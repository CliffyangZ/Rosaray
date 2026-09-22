//! Persistence for the on-demand `ThumbnailArtifact` cache (data-model.md
//! ThumbnailArtifact, research.md §7). Reconstructible and non-authoritative
//! — never referenced by a `RunRecord`/`MetricSet`.

use rusqlite::{params, Connection, OptionalExtension};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThumbnailState {
    Ready,
    Stale,
    Generating,
    Placeholder,
}

impl ThumbnailState {
    fn as_str(self) -> &'static str {
        match self {
            ThumbnailState::Ready => "ready",
            ThumbnailState::Stale => "stale",
            ThumbnailState::Generating => "generating",
            ThumbnailState::Placeholder => "placeholder",
        }
    }

    fn parse(s: &str) -> Self {
        match s {
            "ready" => ThumbnailState::Ready,
            "stale" => ThumbnailState::Stale,
            "generating" => ThumbnailState::Generating,
            _ => ThumbnailState::Placeholder,
        }
    }
}

pub struct ThumbnailRow {
    pub content_identity: Option<String>,
    pub state: ThumbnailState,
}

pub fn get(
    conn: &Connection,
    source_content_identity: &str,
) -> rusqlite::Result<Option<ThumbnailRow>> {
    conn.query_row(
        "SELECT content_identity, state FROM thumbnail_artifacts WHERE source_content_identity = ?1",
        params![source_content_identity],
        |row| {
            Ok(ThumbnailRow {
                content_identity: row.get(0)?,
                state: ThumbnailState::parse(&row.get::<_, String>(1)?),
            })
        },
    )
    .optional()
}

pub fn upsert(
    conn: &Connection,
    source_content_identity: &str,
    content_identity: Option<&str>,
    state: ThumbnailState,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO thumbnail_artifacts (source_content_identity, content_identity, state)
         VALUES (?1, ?2, ?3)
         ON CONFLICT(source_content_identity) DO UPDATE SET content_identity = ?2, state = ?3",
        params![source_content_identity, content_identity, state.as_str()],
    )?;
    Ok(())
}
