//! Permanent cleanup (constitution III, FR-006, plan R3).
//!
//! After every mutation that can change executability — and at startup — every
//! currently unexecutable version (drafts, knowledge-only, paused, unreadable)
//! is permanently deleted, then every AlgoPipe that depended on a deleted node
//! is deleted too, until no dangling reference remains. Order of effects:
//!
//! 1. `qkb_deletion_record` rows are written as `pending` (one transaction,
//!    which also flags affected selection records),
//! 2. files are removed,
//! 3. the records become `done`.
//!
//! A crash between 1 and 3 is repaired by [`recover_pending`], which replays
//! the file removal. A query-specific data mismatch never reaches this module.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use rusqlite::{params, Connection};
use serde::Serialize;

use crate::catalog::repo::refresh;
use crate::evidence::{deprecation, verification};
use crate::executability::assess_dir;
use crate::trust;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct DeletionRecord {
    pub deletion_id: String,
    pub kind: String,
    pub id: String,
    pub version: String,
    pub content_id: Option<String>,
    pub reason_code: String,
    /// For a cascade: the `kind:id@version` whose deletion caused this one.
    pub root_cause: Option<String>,
    pub affected_dependents: Vec<String>,
    pub trigger: String,
    /// `pending | done`
    pub state: String,
    /// Path relative to the KB root.
    pub path: String,
    pub recorded_at: String,
}

#[derive(Debug, thiserror::Error)]
pub enum CleanupError {
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

pub const TRIGGERS: &[&str] = &["submit", "migration", "verification_change", "trust_change", "rescan", "startup"];

struct Entry {
    path: String,
    kind: String,
    id: String,
    version: String,
    content_id: Option<String>,
    status: String,
}

fn identity_from_path(path: &str) -> (String, String, String) {
    // nodes/<id>/<version>, pipes/<id>/<version>, drafts/nodes/<id>, ...
    let segs: Vec<&str> = path.split('/').collect();
    let kind = if segs.contains(&"pipes") { "algopipe" } else { "algonode" };
    let n = segs.len();
    let id = if n >= 2 { segs[n - 2] } else { "unknown" };
    let version = segs.last().copied().unwrap_or("unknown");
    (kind.to_string(), id.to_string(), version.to_string())
}

fn load_entries(conn: &Connection) -> rusqlite::Result<Vec<Entry>> {
    let mut stmt = conn.prepare("SELECT path, kind, id, version, content_id, status FROM kb_catalog_entry ORDER BY path")?;
    let rows = stmt.query_map([], |r| {
        let path: String = r.get(0)?;
        let (k, i, v) = identity_from_path(&path);
        Ok(Entry {
            kind: r.get::<_, Option<String>>(1)?.unwrap_or(k),
            id: r.get::<_, Option<String>>(2)?.unwrap_or(i),
            version: r.get::<_, Option<String>>(3)?.unwrap_or(v),
            content_id: r.get(4)?,
            status: r.get(5)?,
            path,
        })
    })?;
    rows.collect()
}

fn label(kind: &str, id: &str, version: &str) -> String {
    format!("{kind}:{id}@{version}")
}

struct Planned {
    entry: Entry,
    reason: String,
    root_cause: Option<String>,
    affected: Vec<String>,
}

/// Computes the set of versions to delete: own failures plus the dependency
/// closure. Pure with respect to the file system (reads only).
fn plan(conn: &Connection, root: &Path) -> Result<Vec<Planned>, CleanupError> {
    let entries = load_entries(conn)?;
    let mut own: BTreeMap<String, String> = BTreeMap::new(); // path -> reason
    for e in &entries {
        if e.status == "draft" {
            own.insert(e.path.clone(), "draft_removed".to_string());
            continue;
        }
        let a = assess_dir(conn, root, &root.join(&e.path));
        if let Some(code) = a.executability.primary_code() {
            own.insert(e.path.clone(), code.to_string());
        }
    }
    if own.is_empty() {
        return Ok(Vec::new());
    }
    let by_path: HashMap<&str, &Entry> = entries.iter().map(|e| (e.path.as_str(), e)).collect();

    // dependents index: (to_id,to_version) -> pipe paths
    let mut dependents: HashMap<(String, String), Vec<String>> = HashMap::new();
    {
        let mut stmt = conn.prepare("SELECT from_path, to_id, to_version FROM kb_dependency")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))?;
        for row in rows {
            let (from, to_id, to_version) = row?;
            dependents.entry((to_id, to_version)).or_default().push(from);
        }
    }

    // Everything reachable (through dependents) from any failing version is a
    // cascade, even if it also fails on its own: the cause is the dependency.
    let reach_from = |start: &Entry| -> Vec<&Entry> {
        let mut out: Vec<&Entry> = Vec::new();
        let mut seen: BTreeSet<&str> = BTreeSet::new();
        let mut queue: Vec<(&str, &str)> = vec![(start.id.as_str(), start.version.as_str())];
        while let Some((id, ver)) = queue.pop() {
            for dep_path in dependents.get(&(id.to_string(), ver.to_string())).into_iter().flatten() {
                let Some(d) = by_path.get(dep_path.as_str()) else { continue };
                if seen.insert(d.path.as_str()) {
                    out.push(d);
                    queue.push((d.id.as_str(), d.version.as_str()));
                }
            }
        }
        out.sort_by(|a, b| a.path.cmp(&b.path));
        out
    };
    let mut reachable: BTreeSet<String> = BTreeSet::new();
    for path in own.keys() {
        for d in reach_from(by_path[path.as_str()]) {
            reachable.insert(d.path.clone());
        }
    }
    let copy = |e: &Entry| Entry {
        path: e.path.clone(),
        kind: e.kind.clone(),
        id: e.id.clone(),
        version: e.version.clone(),
        content_id: e.content_id.clone(),
        status: e.status.clone(),
    };
    let mut planned: Vec<Planned> = Vec::new();
    let mut cascades: BTreeMap<String, Planned> = BTreeMap::new();
    for (path, reason) in &own {
        if reachable.contains(path) {
            continue; // a cascade, classified below
        }
        let e = by_path[path.as_str()];
        let root_label = label(&e.kind, &e.id, &e.version);
        let reached = reach_from(e);
        let mut affected: Vec<String> = reached.iter().map(|d| label(&d.kind, &d.id, &d.version)).collect();
        affected.sort();
        for d in reached {
            cascades.entry(d.path.clone()).or_insert_with(|| Planned {
                entry: copy(d),
                reason: "cascade".to_string(),
                root_cause: Some(root_label.clone()),
                affected: Vec::new(),
            });
        }
        planned.push(Planned { entry: copy(e), reason: reason.clone(), root_cause: None, affected });
    }
    planned.extend(cascades.into_values());
    Ok(planned)
}

fn make_writable(dir: &Path) {
    if let Ok(rd) = std::fs::read_dir(dir) {
        for entry in rd.filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.is_dir() {
                make_writable(&path);
            } else if let Ok(meta) = std::fs::metadata(&path) {
                let mut perms = meta.permissions();
                #[allow(clippy::permissions_set_readonly_false)]
                perms.set_readonly(false);
                let _ = std::fs::set_permissions(&path, perms);
            }
        }
    }
}

/// Removes `dir` and its now-empty ancestors, never the KB root itself nor its
/// top-level folders (`nodes/`, `pipes/`, ...), which the layout keeps.
fn remove_empty_parents(root: &Path, mut dir: PathBuf) {
    while dir.starts_with(root) && dir != root && dir.parent() != Some(root) {
        let empty = std::fs::read_dir(&dir).map(|mut r| r.next().is_none()).unwrap_or(false);
        if !empty || std::fs::remove_dir(&dir).is_err() {
            break;
        }
        match dir.parent() {
            Some(p) => dir = p.to_path_buf(),
            None => break,
        }
    }
}

/// Idempotently removes everything belonging to one deleted version.
fn remove_version_files(root: &Path, rec: &DeletionRecord) -> std::io::Result<()> {
    let dir = root.join(&rec.path);
    if dir.exists() {
        make_writable(&dir);
        std::fs::remove_dir_all(&dir)?;
    }
    if let Some(parent) = dir.parent() {
        remove_empty_parents(root, parent.to_path_buf());
    }
    if rec.path.starts_with("drafts/") {
        return Ok(());
    }
    // Version-scoped history: amendment chain and local trust decision.
    let chain = deprecation::chain_dir(root, &rec.id, &rec.version);
    if chain.exists() {
        make_writable(&chain);
        std::fs::remove_dir_all(&chain)?;
        remove_empty_parents(root, chain.parent().map(Path::to_path_buf).unwrap_or_default());
    }
    if let Some(cid) = &rec.content_id {
        trust::forget(root, &rec.id, &rec.version, cid)?;
    }
    // A node's verification chain is per node id and hash-chained; it is
    // removed only when the last version of that node is gone.
    if rec.kind == "algonode" && !root.join("nodes").join(&rec.id).exists() {
        let vdir = verification::chain_dir(root, &rec.id);
        if vdir.exists() {
            make_writable(&vdir);
            std::fs::remove_dir_all(&vdir)?;
        }
    }
    Ok(())
}

fn record_from_row(r: &rusqlite::Row) -> rusqlite::Result<DeletionRecord> {
    let affected: String = r.get(7)?;
    Ok(DeletionRecord {
        deletion_id: r.get(0)?,
        kind: r.get(1)?,
        id: r.get(2)?,
        version: r.get(3)?,
        content_id: r.get(4)?,
        reason_code: r.get(5)?,
        root_cause: r.get(6)?,
        affected_dependents: serde_json::from_str(&affected).unwrap_or_default(),
        trigger: r.get(8)?,
        state: r.get(9)?,
        path: r.get(10)?,
        recorded_at: r.get(11)?,
    })
}

const RECORD_COLUMNS: &str =
    "deletion_id, kind, id, version, content_id, reason_code, root_cause, affected_dependents_json, trigger, state, path, recorded_at";

/// Replays deletions that were recorded but interrupted before the files
/// were removed. Safe to call at any time.
pub fn recover_pending(conn: &Connection, root: &Path) -> Result<usize, CleanupError> {
    let pending: Vec<DeletionRecord> = {
        let mut stmt = conn.prepare(&format!("SELECT {RECORD_COLUMNS} FROM qkb_deletion_record WHERE state = 'pending'"))?;
        let rows = stmt.query_map([], record_from_row)?;
        rows.collect::<Result<_, _>>()?
    };
    for rec in &pending {
        remove_version_files(root, rec)?;
        conn.execute("UPDATE qkb_deletion_record SET state = 'done' WHERE deletion_id = ?1", [&rec.deletion_id])?;
    }
    if !pending.is_empty() {
        refresh(conn, root, &mut |_| {})?;
    }
    Ok(pending.len())
}

fn remove_empty_tree(dir: &Path) {
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.filter_map(|e| e.ok()) {
            if e.path().is_dir() {
                remove_empty_tree(&e.path());
            }
        }
    }
    let _ = std::fs::remove_dir(dir);
}

/// Removes one version folder without recording (used only to roll back a
/// submission that never became visible as accepted).
pub fn force_remove_version_dir(root: &Path, dir: &Path) {
    if dir.exists() {
        make_writable(dir);
        let _ = std::fs::remove_dir_all(dir);
    }
    if let Some(parent) = dir.parent() {
        remove_empty_parents(root, parent.to_path_buf());
    }
}

/// Removes leftover staging folders from interrupted submissions.
pub fn remove_stale_staging(root: &Path) {
    if let Ok(rd) = std::fs::read_dir(root) {
        for e in rd.filter_map(|e| e.ok()) {
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with(".tmp-staging-") {
                make_writable(&e.path());
                let _ = std::fs::remove_dir_all(e.path());
            }
        }
    }
}

/// Re-derives executability for the whole KB and permanently deletes what is
/// no longer executable (with dependency closure). Returns the records written.
pub fn recompute_and_purge(conn: &Connection, root: &Path, trigger: &str) -> Result<Vec<DeletionRecord>, CleanupError> {
    debug_assert!(TRIGGERS.contains(&trigger));
    recover_pending(conn, root)?;
    let mut all = Vec::new();
    refresh(conn, root, &mut |_| {})?;
    // Deleting a node can only remove more pipes; the loop reaches a fixpoint.
    for _ in 0..64 {
        let planned = plan(conn, root)?;
        if planned.is_empty() {
            break;
        }
        let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let mut records = Vec::new();
        {
            let tx = conn.unchecked_transaction()?;
            for p in &planned {
                let rec = DeletionRecord {
                    deletion_id: uuid::Uuid::new_v4().to_string(),
                    kind: p.entry.kind.clone(),
                    id: p.entry.id.clone(),
                    version: p.entry.version.clone(),
                    content_id: p.entry.content_id.clone(),
                    reason_code: p.reason.clone(),
                    root_cause: p.root_cause.clone(),
                    affected_dependents: p.affected.clone(),
                    trigger: trigger.to_string(),
                    state: "pending".to_string(),
                    path: p.entry.path.clone(),
                    recorded_at: now.clone(),
                };
                tx.execute(
                    &format!("INSERT INTO qkb_deletion_record ({RECORD_COLUMNS}) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)"),
                    params![
                        rec.deletion_id,
                        rec.kind,
                        rec.id,
                        rec.version,
                        rec.content_id,
                        rec.reason_code,
                        rec.root_cause,
                        serde_json::to_string(&rec.affected_dependents).unwrap_or_else(|_| "[]".into()),
                        rec.trigger,
                        rec.state,
                        rec.path,
                        rec.recorded_at,
                    ],
                )?;
                if let (Some(cid), "algopipe") = (&rec.content_id, rec.kind.as_str()) {
                    tx.execute(
                        "UPDATE qkb_selection SET selected_version_deleted = 1
                         WHERE selected_id = ?1 AND selected_version = ?2 AND selected_content_id = ?3",
                        params![rec.id, rec.version, cid],
                    )?;
                }
                records.push(rec);
            }
            tx.commit()?;
        }
        for rec in &records {
            remove_version_files(root, rec)?;
            conn.execute("UPDATE qkb_deletion_record SET state = 'done' WHERE deletion_id = ?1", [&rec.deletion_id])?;
        }
        refresh(conn, root, &mut |_| {})?;
        all.extend(records.into_iter().map(|mut r| {
            r.state = "done".into();
            r
        }));
    }
    // Legacy layout leftovers: the drafts tree holds nothing once emptied.
    remove_empty_tree(&root.join("drafts"));
    remove_stale_staging(root);
    Ok(all)
}

/// Deletion records, newest first, optionally narrowed to one identity.
pub fn list_deletions(conn: &Connection, identity: Option<(&str, &str)>) -> rusqlite::Result<Vec<DeletionRecord>> {
    let sql = match identity {
        Some(_) => format!("SELECT {RECORD_COLUMNS} FROM qkb_deletion_record WHERE id = ?1 AND (version = ?2 OR ?2 = '') ORDER BY recorded_at DESC, rowid DESC"),
        None => format!("SELECT {RECORD_COLUMNS} FROM qkb_deletion_record ORDER BY recorded_at DESC, rowid DESC"),
    };
    let mut stmt = conn.prepare(&sql)?;
    let rows = match identity {
        Some((id, ver)) => stmt.query_map(params![id, ver], record_from_row)?.collect(),
        None => stmt.query_map([], record_from_row)?.collect(),
    };
    rows
}
