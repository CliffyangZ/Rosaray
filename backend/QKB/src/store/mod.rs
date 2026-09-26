//! `qkb.sqlite`: the QKB's own database. It never touches the legacy project
//! database (`rosaray.sqlite3`), so legacy Dataset/Run files stay unmodified.

use std::path::Path;

use rusqlite::Connection;

const MIGRATIONS: &[(&str, &str)] = &[("0001_qkb", include_str!("migrations/0001_qkb.sql"))];

fn migrate(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_migrations (name TEXT PRIMARY KEY, applied_at TEXT NOT NULL);",
    )?;
    for (name, sql) in MIGRATIONS {
        let applied: bool =
            conn.query_row("SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE name = ?1)", [name], |r| r.get(0))?;
        if applied {
            continue;
        }
        let tx = conn.unchecked_transaction()?;
        tx.execute_batch(sql)?;
        tx.execute(
            "INSERT INTO schema_migrations (name, applied_at) VALUES (?1, ?2)",
            rusqlite::params![name, chrono::Utc::now().to_rfc3339()],
        )?;
        tx.commit()?;
    }
    Ok(())
}

/// Opens (creating if needed) the QKB database and applies migrations.
pub fn open(path: &Path) -> rusqlite::Result<Connection> {
    let conn = Connection::open(path)?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    migrate(&conn)?;
    Ok(conn)
}

pub fn open_in_memory() -> rusqlite::Result<Connection> {
    let conn = Connection::open_in_memory()?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    migrate(&conn)?;
    Ok(conn)
}
