//! Append-only `kb_event` audit log (FR-032). Deliberately exposes no update
//! or delete: the table also rejects both with triggers.

use rusqlite::{params, Connection};
use serde::Serialize;
use serde_json::Value;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct KbEvent {
    pub event_id: Uuid,
    pub at: String,
    pub actor: String,
    #[serde(rename = "type")]
    pub event_type: String,
    pub subject: String,
    pub detail: Value,
}

/// Actor recorded for the single local researcher (spec assumption: one
/// active researcher, no accounts).
pub const LOCAL_ACTOR: &str = "researcher-local";

/// Appends one event and returns its id.
pub fn append(conn: &Connection, event_type: &str, subject: &str, detail: &Value) -> rusqlite::Result<Uuid> {
    let id = Uuid::new_v4();
    conn.execute(
        "INSERT INTO kb_event (event_id, at, actor, type, subject, detail_json) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            id.to_string(),
            chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            LOCAL_ACTOR,
            event_type,
            subject,
            detail.to_string(),
        ],
    )?;
    Ok(id)
}

#[derive(Debug, Default, Clone)]
pub struct EventFilter {
    pub event_type: Option<String>,
    pub subject: Option<String>,
    pub limit: Option<u32>,
}

/// Oldest first (insertion order), optionally narrowed by type/subject.
pub fn list(conn: &Connection, filter: &EventFilter) -> rusqlite::Result<Vec<KbEvent>> {
    let mut stmt = conn.prepare(
        "SELECT event_id, at, actor, type, subject, detail_json FROM kb_event
         WHERE (?1 IS NULL OR type = ?1) AND (?2 IS NULL OR subject = ?2)
         ORDER BY at ASC, rowid ASC LIMIT ?3",
    )?;
    let rows = stmt.query_map(
        params![filter.event_type, filter.subject, filter.limit.map(i64::from).unwrap_or(-1)],
        |row| {
            Ok(KbEvent {
                event_id: Uuid::parse_str(&row.get::<_, String>(0)?).expect("stored UUID is valid"),
                at: row.get(1)?,
                actor: row.get(2)?,
                event_type: row.get(3)?,
                subject: row.get(4)?,
                detail: serde_json::from_str(&row.get::<_, String>(5)?).unwrap_or(Value::Null),
            })
        },
    )?;
    rows.collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::MasterKey;

    fn db() -> (tempfile::TempDir, Connection) {
        let dir = tempfile::tempdir().unwrap();
        let key = MasterKey::derive("pw", &crate::crypto::generate_salt()).unwrap();
        let conn = crate::data_repository::sqlite::open(&dir.path().join("t.sqlite3"), &key).unwrap();
        (dir, conn)
    }

    #[test]
    fn appends_and_lists_in_order_with_filters() {
        let (_d, conn) = db();
        append(&conn, "draft_saved", "rosaray.a", &serde_json::json!({"rev": 1})).unwrap();
        append(&conn, "published", "rosaray.a@1.0.0", &serde_json::json!({})).unwrap();
        append(&conn, "draft_saved", "rosaray.b", &serde_json::json!({})).unwrap();
        let all = list(&conn, &EventFilter::default()).unwrap();
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].detail["rev"], 1);
        let saved = list(&conn, &EventFilter { event_type: Some("draft_saved".into()), ..Default::default() }).unwrap();
        assert_eq!(saved.len(), 2);
        let one = list(&conn, &EventFilter { subject: Some("rosaray.a@1.0.0".into()), ..Default::default() }).unwrap();
        assert_eq!(one[0].event_type, "published");
    }

    #[test]
    fn rows_cannot_be_updated_or_deleted() {
        let (_d, conn) = db();
        append(&conn, "published", "x", &serde_json::json!({})).unwrap();
        assert!(conn.execute("UPDATE kb_event SET actor = 'x'", []).is_err());
        assert!(conn.execute("DELETE FROM kb_event", []).is_err());
        assert_eq!(list(&conn, &EventFilter::default()).unwrap().len(), 1);
    }
}
