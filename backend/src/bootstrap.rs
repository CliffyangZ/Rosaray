//! Opens the QKB for a project directory. Only QKB-owned locations are used:
//! `qkb.sqlite`, `knowledge-base/` and `qkb-credentials/`. Legacy Dataset/Run
//! files in the same directory (`rosaray.sqlite3*`, `blobs/`, `project.*`)
//! are never opened, read or modified (FR-018).

use std::path::Path;
use std::sync::Mutex;

use rosaray_qkb::auth::Credentials;

use crate::api::{AppState, AppStateInner};

/// What startup did, for the launcher's log line.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct StartupReport {
    pub seeded: usize,
    pub deleted_versions: usize,
    pub replayed_deletions: usize,
    pub legacy_files_present: bool,
}

pub const LEGACY_MARKERS: &[&str] = &["rosaray.sqlite3", "blobs", "project.salt", "project.verifier"];

/// Opens (creating what is missing) the QKB under `project_dir`.
pub fn open(project_dir: &Path, catalog_token: String) -> std::io::Result<(AppState, StartupReport)> {
    std::fs::create_dir_all(project_dir)?;
    let kb_root = project_dir.join("knowledge-base");
    rosaray_qkb::ensure_layout(&kb_root)?;
    let conn = rosaray_qkb::store::open(&project_dir.join("qkb.sqlite")).map_err(std::io::Error::other)?;
    let creds = Credentials::load_or_create(&project_dir.join("qkb-credentials"), catalog_token)?;

    let mut report = StartupReport {
        legacy_files_present: LEGACY_MARKERS.iter().any(|m| project_dir.join(m).exists()),
        ..Default::default()
    };
    // Crash recovery first, then the migration/eligibility sweep (FR-006).
    report.replayed_deletions = rosaray_qkb::cleanup::recover_pending(&conn, &kb_root).map_err(std::io::Error::other)?;
    rosaray_qkb::catalog::repo::refresh(&conn, &kb_root, &mut |_| {}).map_err(std::io::Error::other)?;
    if rosaray_qkb::catalog::scan::discover_bundle_dirs(&kb_root).is_empty() {
        report.seeded = rosaray_qkb::seed::install(&conn, &kb_root)?;
    }
    let deleted = rosaray_qkb::cleanup::recompute_and_purge(&conn, &kb_root, "startup").map_err(std::io::Error::other)?;
    report.deleted_versions = deleted.len();

    let state = AppState(std::sync::Arc::new(AppStateInner { creds, db: Mutex::new(conn), kb_root }));
    Ok((state, report))
}
