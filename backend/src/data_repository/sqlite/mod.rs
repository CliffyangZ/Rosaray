pub mod dataset_repo;
pub mod run_repo;
pub mod thumbnail_repo;

use rusqlite::{Connection, OptionalExtension};

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
];

pub fn open(path: &std::path::Path) -> rusqlite::Result<Connection> {
    let conn = Connection::open(path)?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    run_migrations(&conn)?;
    Ok(conn)
}

const CONTRACT_VERSION: &str = "0.1.0";

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
}
