pub mod dataset_repo;
pub mod housekeeping_repo;
pub mod kb_event_repo;
pub mod paper_repo;
pub mod run_repo;
pub mod thumbnail_repo;

use rusqlite::{Connection, OptionalExtension};

use crate::crypto::MasterKey;

/// Ordered, embedded schema migrations. Applied at most once each, tracked
/// via `schema_migrations`, so re-opening an existing project database is
/// idempotent regardless of which version created it.
const MIGRATIONS: &[(&str, &str)] = &[
    (
        "0001_projects",
        include_str!("migrations/0001_projects.sql"),
    ),
    (
        "0002_dataset_core",
        include_str!("migrations/0002_dataset_core.sql"),
    ),
    (
        "0003_import_validation",
        include_str!("migrations/0003_import_validation.sql"),
    ),
    (
        "0004_thumbnails",
        include_str!("migrations/0004_thumbnails.sql"),
    ),
    ("0005_runs", include_str!("migrations/0005_runs.sql")),
    (
        "0006_export_bundles",
        include_str!("migrations/0006_export_bundles.sql"),
    ),
    (
        "0007_knowledge_base",
        include_str!("migrations/0007_knowledge_base.sql"),
    ),
    (
        "0008_kb_status",
        include_str!("migrations/0008_kb_status.sql"),
    ),
    (
        "0009_kb_origin",
        include_str!("migrations/0009_kb_origin.sql"),
    ),
    (
        "0010_kb_amendments",
        include_str!("migrations/0010_kb_amendments.sql"),
    ),
];

/// Opens (creating if needed) the project database, encrypted at rest with
/// SQLCipher under a key derived from the project master key (FR-033,
/// Constitution Principle VI). A database written by an earlier, plaintext
/// build is encrypted in place on first open. A wrong key is an error —
/// never a silently empty database.
pub fn open(path: &std::path::Path, master: &MasterKey) -> rusqlite::Result<Connection> {
    let key = master.database_key_hex();
    let conn = match open_keyed(path, &key) {
        Ok(conn) => conn,
        Err(keyed_err) => {
            // Only a readable *plaintext* database is migrated; anything
            // else (wrong key, corruption) surfaces the original error.
            if !is_plaintext_database(path) {
                return Err(keyed_err);
            }
            encrypt_plaintext_database(path, &key)?;
            open_keyed(path, &key)?
        }
    };
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    run_migrations(&conn)?;
    Ok(conn)
}

fn open_keyed(path: &std::path::Path, key_hex: &str) -> rusqlite::Result<Connection> {
    let conn = Connection::open(path)?;
    conn.execute_batch(&format!("PRAGMA key = \"x'{key_hex}'\";"))?;
    // The key is only checked on first read.
    conn.query_row("SELECT count(*) FROM sqlite_master", [], |r| r.get::<_, i64>(0))?;
    Ok(conn)
}

fn is_plaintext_database(path: &std::path::Path) -> bool {
    path.is_file()
        && Connection::open(path)
            .and_then(|c| c.query_row("SELECT count(*) FROM sqlite_master", [], |r| r.get::<_, i64>(0)))
            .is_ok()
}

fn encrypt_plaintext_database(path: &std::path::Path, key_hex: &str) -> rusqlite::Result<()> {
    let tmp = path.with_extension("sqlite3.encrypting");
    let _ = std::fs::remove_file(&tmp);
    {
        let plain = Connection::open(path)?;
        plain.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
        let tmp_sql = tmp.to_string_lossy().replace('\'', "''");
        plain.execute_batch(&format!(
            "ATTACH DATABASE '{tmp_sql}' AS encrypted KEY \"x'{key_hex}'\";
             SELECT sqlcipher_export('encrypted');
             DETACH DATABASE encrypted;"
        ))?;
    }
    // Swap in the encrypted copy; stale plaintext WAL/SHM must go too.
    for suffix in ["-wal", "-shm"] {
        let mut side = path.as_os_str().to_owned();
        side.push(suffix);
        let _ = std::fs::remove_file(side);
    }
    std::fs::rename(&tmp, path).map_err(|e| {
        rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_IOERR),
            Some(e.to_string()),
        )
    })
}

pub const CONTRACT_VERSION: &str = "0.1.0";

/// Ensures exactly one `Project` row exists (this feature is single-project
/// per data directory; multi-project support is out of scope, plan.md
/// Assumptions) and returns its id.
pub fn ensure_default_project(conn: &Connection) -> rusqlite::Result<uuid::Uuid> {
    let existing: Option<String> = conn
        .query_row("SELECT id FROM projects LIMIT 1", [], |row| row.get(0))
        .optional()?;
    if let Some(id) = existing {
        return Ok(uuid::Uuid::parse_str(&id).expect("stored UUID is always valid"));
    }
    let id = uuid::Uuid::new_v4();
    conn.execute(
        "INSERT INTO projects (id, display_name, created_at, contract_version) VALUES (?1, ?2, ?3, ?4)",
        (
            id.to_string(),
            "Rosaray Project",
            chrono::Utc::now().to_rfc3339(),
            CONTRACT_VERSION,
        ),
    )?;
    Ok(id)
}

pub fn run_migrations(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_migrations (name TEXT PRIMARY KEY, applied_at TEXT NOT NULL)",
    )?;
    for (name, sql) in MIGRATIONS {
        let already_applied: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE name = ?1)",
                [name],
                |row| row.get(0),
            )
            .unwrap_or(false);
        if already_applied {
            continue;
        }
        conn.execute_batch(sql)?;
        conn.execute(
            "INSERT INTO schema_migrations (name, applied_at) VALUES (?1, datetime('now'))",
            [name],
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrations_are_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        run_migrations(&conn).unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM projects", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    fn master(passphrase: &str) -> MasterKey {
        MasterKey::derive(passphrase, &[7u8; 16]).unwrap()
    }

    #[test]
    fn database_file_is_encrypted_and_needs_the_right_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("p.sqlite3");
        {
            let conn = open(&path, &master("right")).unwrap();
            ensure_default_project(&conn).unwrap();
            conn.execute("UPDATE projects SET display_name = 'Confidential Study'", []).unwrap();
        }
        let raw = std::fs::read(&path).unwrap();
        assert!(!raw.starts_with(b"SQLite format 3"));
        assert!(!raw.windows(18).any(|w| w == b"Confidential Study"));

        assert!(open(&path, &master("wrong")).is_err());
        let conn = open(&path, &master("right")).unwrap();
        let name: String = conn
            .query_row("SELECT display_name FROM projects", [], |r| r.get(0))
            .unwrap();
        assert_eq!(name, "Confidential Study");
    }

    #[test]
    fn plaintext_database_from_an_earlier_build_is_encrypted_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("old.sqlite3");
        {
            let plain = Connection::open(&path).unwrap();
            plain.pragma_update(None, "journal_mode", "WAL").unwrap();
            run_migrations(&plain).unwrap();
            ensure_default_project(&plain).unwrap();
            plain.execute("UPDATE projects SET display_name = 'Legacy Study'", []).unwrap();
        }
        let conn = open(&path, &master("pw")).unwrap();
        let name: String = conn
            .query_row("SELECT display_name FROM projects", [], |r| r.get(0))
            .unwrap();
        assert_eq!(name, "Legacy Study");
        drop(conn);
        assert!(!std::fs::read(&path).unwrap().starts_with(b"SQLite format 3"));
        assert!(open(&path, &master("other")).is_err());
    }
}
