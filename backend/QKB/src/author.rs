//! Author-side operations that change a version's executability. Each ends by
//! re-deriving executability and permanently deleting whatever no longer
//! qualifies (FR-006); the deletion records are returned to the caller.

use std::path::Path;

use rusqlite::Connection;
use serde_json::json;

use crate::bundle::model::{Kind, Trust};
use crate::bundle::read::read_bundle;
use crate::catalog::query::path_of;
use crate::catalog::repo::refresh;
use crate::catalog::status::verification_subject;
use crate::cleanup::{recompute_and_purge, CleanupError, DeletionRecord};
use crate::evidence::tech_verify::{run_bundle_tests, suite_of, TestError};
use crate::evidence::verification::{record_event, Event as VEvent, VerificationType};
use crate::trust;

#[derive(Debug, thiserror::Error)]
pub enum AuthorError {
    #[error("not_found")]
    NotFound,
    #[error("verification_invalid: {0}")]
    VerificationInvalid(String),
    #[error("cleanup: {0}")]
    Cleanup(#[from] CleanupError),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),
}

fn load_node(conn: &Connection, root: &Path, id: &str, version: &str) -> Result<crate::bundle::read::Bundle, AuthorError> {
    refresh(conn, root, &mut |_| {})?;
    let path = path_of(conn, "algonode", id, version, "published")?.ok_or(AuthorError::NotFound)?;
    let (bundle, _) = read_bundle(&root.join(path));
    bundle.filter(|b| b.kind == Kind::Algonode && b.lock.is_some()).ok_or(AuthorError::NotFound)
}

/// Sets the local trust decision for one exact node version, then re-derives.
/// `Trust::Untrusted` makes the node (and dependent pipes) non-executable.
pub fn set_trust(conn: &Connection, root: &Path, id: &str, version: &str, new: Trust) -> Result<Vec<DeletionRecord>, AuthorError> {
    let bundle = load_node(conn, root, id, version)?;
    let cid = bundle.lock.as_ref().map(|l| l.content_id.clone()).ok_or(AuthorError::NotFound)?;
    trust::record(root, id, version, &cid, new, "author")?;
    crate::event_repo::append(conn, "trust_changed", &format!("{id}@{version}"), &json!({ "trust": new }))?;
    Ok(recompute_and_purge(conn, root, "trust_change")?)
}

/// Withdraws the current technical verification of a node version.
pub fn withdraw_verification(conn: &Connection, root: &Path, id: &str, version: &str, reason: &str) -> Result<Vec<DeletionRecord>, AuthorError> {
    let bundle = load_node(conn, root, id, version)?;
    let subject = verification_subject(&bundle).ok_or(AuthorError::NotFound)?;
    record_event(root, VerificationType::Technical, VEvent::Withdrawn, &subject, None, None, Some(reason.to_string()))
        .map_err(|e| AuthorError::Io(std::io::Error::other(format!("{e:?}"))))?;
    crate::event_repo::append(conn, "verification_recorded", &format!("{id}@{version}"), &json!({ "type": "technical", "event": "withdrawn" }))?;
    Ok(recompute_and_purge(conn, root, "verification_change")?)
}

/// Re-runs a node's own tests. A pass is *produced by the run*, never asserted;
/// a failure is recorded honestly and the version is then deleted.
pub fn reverify(conn: &Connection, root: &Path, id: &str, version: &str) -> Result<(bool, Vec<DeletionRecord>), AuthorError> {
    let bundle = load_node(conn, root, id, version)?;
    let subject = verification_subject(&bundle).ok_or(AuthorError::NotFound)?;
    let report = run_bundle_tests(&bundle).map_err(|e| match e {
        TestError::Unreadable(m) => AuthorError::VerificationInvalid(m),
        other => AuthorError::VerificationInvalid(other.to_string()),
    })?;
    let event = if report.all_passed { VEvent::Passed } else { VEvent::Failed };
    record_event(root, VerificationType::Technical, event, &subject, Some(suite_of(&report)), None, None)
        .map_err(|e| AuthorError::Io(std::io::Error::other(format!("{e:?}"))))?;
    crate::event_repo::append(conn, "verification_recorded", &format!("{id}@{version}"), &json!({ "type": "technical", "event": event.as_str() }))?;
    let deleted = recompute_and_purge(conn, root, "verification_change")?;
    Ok((report.all_passed, deleted))
}
