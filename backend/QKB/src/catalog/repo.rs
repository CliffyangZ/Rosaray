//! Catalog persistence in the encrypted project SQLite. Derived only: rows can
//! be dropped and regenerated from bundle files at any time (FR-040).

use std::collections::HashMap;
use std::path::Path;

use rusqlite::{params, Connection};

use super::scan::{
    discover_bundle_dirs, signature_with_chains, rel_path, scan_bundle, DirSignature, EntryStatus, ScanEntry, ScanEvent,
};
use crate::contract::finding::{BundleRef, Finding, Severity, Subject, SubjectType};

pub fn upsert_entry(conn: &Connection, e: &ScanEntry) -> rusqlite::Result<()> {
    remove_path(conn, &e.path)?;
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO kb_catalog_entry (
            path, id, version, kind, name, summary, purpose, intended_use, domain, status,
            maturity, release_kind, availability, verification, data_kinds, content_id,
            mtime, size, finding_count, indexed_at, dataset_validation, origin, amendment_count
        ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23)",
        params![
            e.path,
            e.id,
            e.version,
            e.kind.map(|k| k.as_str()),
            e.name,
            e.summary,
            e.purpose,
            e.intended_use,
            e.domain,
            e.status.as_str(),
            e.maturity,
            e.release_kind,
            e.availability,
            e.verification,
            e.data_kinds,
            e.content_id,
            e.signature.mtime_ms,
            e.signature.size as i64,
            e.findings.len() as i64,
            now,
            e.dataset_validation,
            e.origin,
            e.amendment_count,
        ],
    )?;
    conn.execute(
        "INSERT INTO kb_fts (name, summary, purpose, intended_use, path) VALUES (?1,?2,?3,?4,?5)",
        params![
            e.name.clone().unwrap_or_default(),
            e.summary.clone().unwrap_or_default(),
            e.purpose.clone().unwrap_or_default(),
            e.intended_use.clone().unwrap_or_default(),
            e.path,
        ],
    )?;
    for d in &e.dependencies {
        conn.execute(
            "INSERT INTO kb_dependency (from_path, to_id, to_version, to_content_id, resolved) VALUES (?1,?2,?3,?4,0)",
            params![e.path, d.to_id, d.to_version, d.to_content_id],
        )?;
    }
    for f in &e.findings {
        insert_finding(conn, &e.path, f, &now)?;
    }
    Ok(())
}

fn insert_finding(conn: &Connection, path: &str, f: &Finding, at: &str) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO kb_finding (path, bundle_id, bundle_version, severity, code, subject_json, explanation, action, detected_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
        params![
            path,
            f.bundle.id,
            f.bundle.version,
            serde_json::to_value(f.severity).unwrap().as_str().unwrap_or("error"),
            f.code,
            serde_json::to_string(&f.subject).unwrap_or_default(),
            f.explanation,
            f.action,
            at,
        ],
    )?;
    Ok(())
}

/// Removes every derived row for `path`.
pub fn remove_path(conn: &Connection, path: &str) -> rusqlite::Result<()> {
    conn.execute("DELETE FROM kb_fts WHERE path = ?1", [path])?;
    conn.execute("DELETE FROM kb_dependency WHERE from_path = ?1", [path])?;
    conn.execute("DELETE FROM kb_finding WHERE path = ?1", [path])?;
    conn.execute("DELETE FROM kb_catalog_entry WHERE path = ?1", [path])?;
    Ok(())
}

/// Forgets the catalog rows of specific published versions so the next refresh
/// re-reads them (used when history that affects them changed outside their folders).
pub fn forget_versions(conn: &Connection, versions: &[(String, String)]) -> rusqlite::Result<()> {
    for (id, version) in versions {
        let paths: Vec<String> = conn
            .prepare("SELECT path FROM kb_catalog_entry WHERE id = ?1 AND version = ?2")?
            .query_map([id, version], |r| r.get(0))?
            .collect::<Result<_, _>>()?;
        for p in paths {
            remove_path(conn, &p)?;
        }
    }
    Ok(())
}

/// Drops all derived catalog content (the "drop" of drop-and-rebuild).
pub fn clear_derived(conn: &Connection) -> rusqlite::Result<()> {
    for table in ["kb_fts", "kb_dependency", "kb_finding", "kb_catalog_entry"] {
        conn.execute(&format!("DELETE FROM {table}"), [])?;
    }
    Ok(())
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct RefreshOutcome {
    pub scanned: u64,
    pub indexed: u64,
    pub invalid: u64,
}

/// Incremental refresh: re-reads only bundles whose `(mtime, size)` moved,
/// drops rows for vanished folders, then re-resolves dependencies.
pub fn refresh(
    conn: &Connection,
    root: &Path,
    progress: &mut dyn FnMut(ScanEvent),
) -> rusqlite::Result<RefreshOutcome> {
    let dirs = discover_bundle_dirs(root);
    let total = dirs.len() as u64;
    let known: HashMap<String, (DirSignature, Option<String>, Option<String>)> = {
        let mut stmt = conn.prepare("SELECT path, mtime, size, id, version FROM kb_catalog_entry")?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                (DirSignature { mtime_ms: r.get(1)?, size: r.get::<_, i64>(2)? as u64 }, r.get(3)?, r.get(4)?),
            ))
        })?;
        rows.collect::<Result<_, _>>()?
    };

    let mut seen = std::collections::HashSet::new();
    let mut invalid = 0u64;
    for (i, dir) in dirs.iter().enumerate() {
        let path = rel_path(root, dir);
        seen.insert(path.clone());
        let unchanged = known.get(&path).is_some_and(|(sig, id, ver)| {
            *sig == signature_with_chains(root, dir, id.as_deref(), ver.as_deref())
        });
        if unchanged {
            let status: String =
                conn.query_row("SELECT status FROM kb_catalog_entry WHERE path = ?1", [&path], |r| r.get(0))?;
            if status == EntryStatus::Invalid.as_str() {
                invalid += 1;
            }
        } else {
            let entry = scan_bundle(root, dir);
            if entry.status == EntryStatus::Invalid {
                invalid += 1;
            }
            upsert_entry(conn, &entry)?;
        }
        progress(ScanEvent::Progress { scanned: i as u64 + 1, total: Some(total), invalid });
    }
    for path in known.keys() {
        if !seen.contains(path) {
            remove_path(conn, path)?;
        }
    }
    resolve_dependencies(conn, root)?;
    progress(ScanEvent::Complete { indexed: total, invalid });

    Ok(RefreshOutcome {
        scanned: total,
        indexed: total,
        invalid,
    })
}

/// Drop-and-rebuild: forget every derived row, then rescan.
pub fn rebuild(
    conn: &Connection,
    root: &Path,
    progress: &mut dyn FnMut(ScanEvent),
) -> rusqlite::Result<RefreshOutcome> {
    clear_derived(conn)?;
    refresh(conn, root, progress)
}

/// Resolves every recorded dependency against published catalog rows by
/// `id@version` plus `content_id`. An unresolved reference is marked, never
/// silently re-pointed at another version (SC-013). Pipes' dependency
/// findings are regenerated.
pub fn resolve_dependencies(conn: &Connection, root: &Path) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE kb_dependency SET resolved = CASE WHEN EXISTS (
            SELECT 1 FROM kb_catalog_entry c
            WHERE c.id = kb_dependency.to_id AND c.version = kb_dependency.to_version
              AND c.status = 'published' AND c.availability IN ('available', 'deprecated')
              AND (kb_dependency.to_content_id IS NULL OR c.content_id = kb_dependency.to_content_id)
         ) THEN 1 ELSE 0 END",
        [],
    )?;

    conn.execute(
        "DELETE FROM kb_finding WHERE code IN ('dependency_unresolved', 'dependency_content_mismatch', 'dependency_deprecated')",
        [],
    )?;
    let now = chrono::Utc::now().to_rfc3339();
    let mut stmt = conn.prepare(
        "SELECT d.from_path, p.id, p.version, d.to_id, d.to_version, d.to_content_id,
                EXISTS (SELECT 1 FROM kb_catalog_entry c WHERE c.id = d.to_id AND c.version = d.to_version
                        AND c.status = 'published') AS exists_published
         FROM kb_dependency d JOIN kb_catalog_entry p ON p.path = d.from_path
         WHERE d.resolved = 0 AND d.to_version NOT IN ('draft', 'latest')",
    )?;
    let rows: Vec<(String, Option<String>, Option<String>, String, String, Option<String>, bool)> = stmt
        .query_map([], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?))
        })?
        .collect::<Result<_, _>>()?;
    for (path, id, version, to_id, to_version, _content, exists) in rows {
        let bundle = BundleRef::new(id.unwrap_or_default(), version);
        let (code, explanation, action) = if exists {
            (
                "dependency_content_mismatch",
                format!("{to_id}@{to_version} exists but its content differs from the one this pipe was built against, or it is unavailable."),
                "Restore the original version, or update the reference deliberately.",
            )
        } else {
            (
                "dependency_unresolved",
                format!("{to_id}@{to_version} is not in the knowledge base."),
                "Import or create that version, or reference one that exists.",
            )
        };
        let f = Finding::build(
            Severity::Error,
            code,
            &bundle,
            Subject::new(SubjectType::NodeInstance, to_id),
            explanation,
            action,
        );
        insert_finding(conn, &path, &f, &now)?;
    }
    // A reference to a deprecated version keeps resolving; the researcher is
    // only told (FR-013), with the replacement when one was named.
    let mut stmt = conn.prepare(
        "SELECT d.from_path, p.id, p.version, d.to_id, d.to_version
         FROM kb_dependency d JOIN kb_catalog_entry p ON p.path = d.from_path
         JOIN kb_catalog_entry c ON c.id = d.to_id AND c.version = d.to_version AND c.status = 'published'
         WHERE d.resolved = 1 AND c.availability = 'deprecated'",
    )?;
    let deprecated: Vec<(String, Option<String>, Option<String>, String, String)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))?
        .collect::<Result<_, _>>()?;
    for (path, id, version, to_id, to_version) in deprecated {
        let state = crate::evidence::deprecation::read(root, &to_id, &to_version)
            .map(|c| crate::evidence::deprecation::state_of(&c))
            .unwrap_or_default();
        let replacement = state
            .replacement
            .as_ref()
            .and_then(|r| Some(format!(" Suggested replacement: {}@{}.", r.get("id")?.as_str()?, r.get("version")?.as_str()?)))
            .unwrap_or_default();
        let f = Finding::build(
            Severity::Warning,
            "dependency_deprecated",
            &BundleRef::new(id.unwrap_or_default(), version),
            Subject::new(SubjectType::NodeInstance, to_id.clone()),
            format!("{to_id}@{to_version} has been deprecated{}.{replacement} This pipe still uses it exactly as published.", state.reason.map(|r| format!(" ({r})")).unwrap_or_default()),
            "Nothing is changed for you; move to another version only when it suits your study.",
        );
        insert_finding(conn, &path, &f, &now)?;
    }
    conn.execute(
        "UPDATE kb_catalog_entry SET finding_count = (SELECT count(*) FROM kb_finding f WHERE f.path = kb_catalog_entry.path)",
        [],
    )?;
    Ok(())
}
