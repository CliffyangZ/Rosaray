//! Paper import (FR-021, spec assumption): a local, *authorized* PDF is stored in
//! the encrypted blob store and recorded as a Paper Source. Nothing is extracted,
//! published or executed by importing, and nothing touches the network.

use rusqlite::Connection;
use serde_json::json;
use uuid::Uuid;

use super::pdf;
use crate::crypto::MasterKey;
use crate::data_repository::blob_store::BlobStore;
use crate::data_repository::sqlite::{kb_event_repo, paper_repo};
use crate::data_repository::sqlite::paper_repo::PaperRow;

#[derive(Debug, Default, Clone)]
pub struct PaperMetadata {
    pub title: Option<String>,
    pub authors: Option<String>,
    pub year: Option<i64>,
    pub doi: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum PaperImportError {
    /// The researcher did not attest they may use this paper.
    #[error("authorization_required")]
    AuthorizationRequired,
    #[error("not a readable PDF")]
    NotReadable,
    #[error("could not store the paper")]
    Store,
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),
}

/// Imports `bytes`; returns the Paper Source and whether it was already present
/// (importing the same file twice is a no-op that returns the existing record).
pub fn import_paper(
    conn: &Connection,
    blobs: &BlobStore,
    key: &MasterKey,
    bytes: &[u8],
    authorization_attested: bool,
    meta: PaperMetadata,
) -> Result<(PaperRow, bool), PaperImportError> {
    if !authorization_attested {
        return Err(PaperImportError::AuthorizationRequired);
    }
    if !bytes.starts_with(b"%PDF") {
        return Err(PaperImportError::NotReadable);
    }
    let text = pdf::extract(bytes).map_err(|_| PaperImportError::NotReadable)?;
    let content_id = crate::domain::content_identity::content_identity(bytes);
    if let Some(existing) = paper_repo::paper_by_content(conn, &content_id)? {
        return Ok((existing, true));
    }
    let blob_ref = blobs.write(key, bytes).map_err(|_| PaperImportError::Store)?;
    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let row = PaperRow {
        paper_id: Uuid::new_v4(),
        content_id,
        blob_ref,
        title: meta.title,
        authors: meta.authors,
        year: meta.year,
        doi: meta.doi,
        page_count: text.page_count,
        pages_without_text: text.pages_without_text,
        authorization_attested: true,
        authorization_attested_at: Some(now.clone()),
        extraction_state: "imported".to_string(),
        imported_at: now,
    };
    paper_repo::insert_paper(conn, &row)?;
    kb_event_repo::append(
        conn,
        "paper_import",
        &row.paper_id.to_string(),
        // Never the file name, path or any text of the paper.
        &json!({ "content_id": row.content_id, "page_count": row.page_count }),
    )?;
    Ok((row, false))
}
