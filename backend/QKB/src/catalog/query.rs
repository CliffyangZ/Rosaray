//! Catalog search and filtering (FR-011). Maturity, verification, availability
//! and deprecation are separate fields, never collapsed into one flag.

use rusqlite::{params_from_iter, types::Value, Connection};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Deserialize)]
pub struct EntryFilter {
    pub kind: Option<String>,
    /// Free text, matched against name / summary / purpose / intended use.
    pub q: Option<String>,
    pub domain: Option<String>,
    pub maturity: Option<String>,
    /// `knowledge | executable | draft`
    pub release: Option<String>,
    pub availability: Option<String>,
    pub verification: Option<String>,
    pub intended_use: Option<String>,
    pub data_kind: Option<String>,
    /// `draft | published | invalid`, or `valid` for published + draft.
    pub status: Option<String>,
    pub limit: Option<u32>,
    /// Opaque cursor from a previous page's `next`.
    pub after: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DependencySummary {
    pub total: i64,
    pub unresolved: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct CatalogEntry {
    pub kind: Option<String>,
    pub id: Option<String>,
    pub version: Option<String>,
    pub name: Option<String>,
    pub summary: Option<String>,
    pub domain: Option<String>,
    pub status: String,
    pub maturity: Option<String>,
    pub release_kind: Option<String>,
    pub availability: String,
    pub verification: Option<String>,
    pub dataset_validation: String,
    /// `paper_derived` when the bundle came from a reviewed paper candidate.
    pub origin: Option<String>,
    /// Amendments recorded against this version (or, for a pipe, its dependencies).
    pub amendment_count: i64,
    pub dependency_summary: Option<DependencySummary>,
    pub content_id: Option<String>,
    pub finding_count: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct EntryPage {
    pub entries: Vec<CatalogEntry>,
    pub next: Option<String>,
}

pub const DEFAULT_LIMIT: u32 = 50;
pub const MAX_LIMIT: u32 = 500;

/// Turns free text into a safe FTS5 query: each alphanumeric word becomes a
/// quoted prefix term, ANDed. Returns `None` when nothing searchable remains.
pub fn fts_query(q: &str) -> Option<String> {
    let terms: Vec<String> = q
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(|t| format!("\"{t}\"*"))
        .collect();
    (!terms.is_empty()).then(|| terms.join(" "))
}

pub fn query_entries(conn: &Connection, f: &EntryFilter) -> rusqlite::Result<EntryPage> {
    let mut sql = String::from(
        "SELECT c.path, c.kind, c.id, c.version, c.name, c.summary, c.domain, c.status, c.maturity,
                c.release_kind, c.availability, c.verification, c.content_id, c.finding_count,
                (SELECT count(*) FROM kb_dependency d WHERE d.from_path = c.path),
                (SELECT count(*) FROM kb_dependency d WHERE d.from_path = c.path AND d.resolved = 0),
                c.kind = 'algopipe', c.dataset_validation, c.origin, c.amendment_count
         FROM kb_catalog_entry c WHERE 1=1",
    );
    let mut args: Vec<Value> = Vec::new();
    let mut eq = |sql: &mut String, col: &str, v: &Option<String>| {
        if let Some(v) = v.as_ref().filter(|v| !v.is_empty()) {
            args.push(Value::Text(v.clone()));
            sql.push_str(&format!(" AND c.{col} = ?{}", args.len()));
        }
    };
    eq(&mut sql, "kind", &f.kind);
    eq(&mut sql, "domain", &f.domain);
    eq(&mut sql, "maturity", &f.maturity);
    eq(&mut sql, "release_kind", &f.release);
    eq(&mut sql, "availability", &f.availability);
    eq(&mut sql, "verification", &f.verification);
    match f.status.as_deref() {
        Some("valid") => sql.push_str(" AND c.status IN ('draft', 'published')"),
        Some(s) if !s.is_empty() => {
            args.push(Value::Text(s.to_string()));
            sql.push_str(&format!(" AND c.status = ?{}", args.len()));
        }
        _ => {}
    }
    if let Some(v) = f.intended_use.as_ref().filter(|v| !v.is_empty()) {
        args.push(Value::Text(format!("%{}%", v.replace('%', "\\%").replace('_', "\\_"))));
        sql.push_str(&format!(" AND c.intended_use LIKE ?{} ESCAPE '\\'", args.len()));
    }
    if let Some(v) = f.data_kind.as_ref().filter(|v| !v.is_empty()) {
        args.push(Value::Text(format!("% {v} %")));
        sql.push_str(&format!(" AND (' ' || COALESCE(c.data_kinds, '') || ' ') LIKE ?{}", args.len()));
    }
    if let Some(q) = f.q.as_ref().filter(|q| !q.trim().is_empty()) {
        match fts_query(q) {
            Some(m) => {
                args.push(Value::Text(m));
                sql.push_str(&format!(" AND c.path IN (SELECT path FROM kb_fts WHERE kb_fts MATCH ?{})", args.len()));
            }
            // Only punctuation: nothing can match.
            None => sql.push_str(" AND 0"),
        }
    }
    if let Some(after) = f.after.as_ref().filter(|a| !a.is_empty()) {
        args.push(Value::Text(after.clone()));
        sql.push_str(&format!(" AND c.path > ?{}", args.len()));
    }
    let limit = f.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT) as i64;
    args.push(Value::Integer(limit + 1));
    sql.push_str(&format!(" ORDER BY c.path ASC LIMIT ?{}", args.len()));

    let mut stmt = conn.prepare(&sql)?;
    let mut rows: Vec<(String, CatalogEntry)> = stmt
        .query_map(params_from_iter(args.iter()), |r| {
            let is_pipe: bool = r.get(16)?;
            let total: i64 = r.get(14)?;
            Ok((
                r.get::<_, String>(0)?,
                CatalogEntry {
                    kind: r.get(1)?,
                    id: r.get(2)?,
                    version: r.get(3)?,
                    name: r.get(4)?,
                    summary: r.get(5)?,
                    domain: r.get(6)?,
                    status: r.get(7)?,
                    maturity: r.get(8)?,
                    release_kind: r.get(9)?,
                    availability: r.get(10)?,
                    verification: r.get(11)?,
                    dataset_validation: r.get(17)?,
                    origin: r.get(18)?,
                    amendment_count: r.get(19)?,
                    dependency_summary: is_pipe.then(|| DependencySummary { total, unresolved: r.get(15).unwrap_or(0) }),
                    content_id: r.get(12)?,
                    finding_count: r.get(13)?,
                },
            ))
        })?
        .collect::<Result<_, _>>()?;

    let next = if rows.len() as i64 > limit {
        rows.truncate(limit as usize);
        rows.last().map(|(path, _)| path.clone())
    } else {
        None
    };
    Ok(EntryPage {
        entries: rows.into_iter().map(|(_, e)| e).collect(),
        next,
    })
}

/// Findings recorded for a bundle, oldest first.
pub fn findings_for_path(conn: &Connection, path: &str) -> rusqlite::Result<Vec<crate::designer::validate::finding::Finding>> {
    use crate::designer::validate::finding::{BundleRef, Finding, Severity, Subject};
    let mut stmt = conn.prepare(
        "SELECT bundle_id, bundle_version, severity, code, subject_json, explanation, action
         FROM kb_finding WHERE path = ?1 ORDER BY finding_id",
    )?;
    let rows = stmt.query_map([path], |r| {
        let severity = match r.get::<_, String>(2)?.as_str() {
            "warning" => Severity::Warning,
            "info" => Severity::Info,
            _ => Severity::Error,
        };
        let subject: Subject = serde_json::from_str(&r.get::<_, String>(4)?).unwrap_or_else(|_| Subject::bundle(""));
        Ok(Finding {
            severity,
            code: r.get(3)?,
            bundle: BundleRef::new(r.get::<_, Option<String>>(0)?.unwrap_or_default(), r.get(1)?),
            subject,
            explanation: r.get(5)?,
            action: r.get(6)?,
        })
    })?;
    rows.collect()
}

/// The folder path of the catalog row for `kind/id@version` in `status`.
pub fn path_of(
    conn: &Connection,
    kind: &str,
    id: &str,
    version: &str,
    status: &str,
) -> rusqlite::Result<Option<String>> {
    use rusqlite::OptionalExtension;
    conn.query_row(
        "SELECT path FROM kb_catalog_entry WHERE kind = ?1 AND id = ?2 AND version = ?3 AND status = ?4 ORDER BY path LIMIT 1",
        rusqlite::params![kind, id, version, status],
        |r| r.get(0),
    )
    .optional()
}

/// Resolves pipe references against the published catalog rows, so pipe
/// validation sees exactly what the catalog knows (`id@version` + `content_id`).
pub struct CatalogResolver<'a> {
    pub conn: &'a Connection,
}

impl crate::designer::validate::bundle::DependencyResolver for CatalogResolver<'_> {
    fn resolve(&self, id: &str, version: &str) -> Option<crate::designer::validate::bundle::ResolvedRef> {
        use rusqlite::OptionalExtension;
        self.conn
            .query_row(
                "SELECT content_id FROM kb_catalog_entry
                 WHERE id = ?1 AND version = ?2 AND status = 'published' AND availability IN ('available', 'deprecated') LIMIT 1",
                rusqlite::params![id, version],
                |r| r.get::<_, Option<String>>(0),
            )
            .optional()
            .ok()
            .flatten()
            .map(|content_id| crate::designer::validate::bundle::ResolvedRef { content_id })
    }
}

/// Node definitions for the graph validator: published versions by
/// `id@version`, or the draft of `id` when the reference says `draft`.
pub struct CatalogDefinitions<'a> {
    pub conn: &'a Connection,
    pub root: &'a std::path::Path,
}

impl crate::designer::validate::graph::DefinitionSource for CatalogDefinitions<'_> {
    fn definition(&self, id: &str, version: &str) -> Option<crate::designer::validate::graph::Definition> {
        use rusqlite::OptionalExtension;
        let is_version = semver::Version::parse(version).is_ok();
        let (status, ver) = if is_version { ("published", version) } else { ("draft", "") };
        let row: Option<(String, String)> = self
            .conn
            .query_row(
                "SELECT path, availability FROM kb_catalog_entry
                 WHERE kind = 'algonode' AND id = ?1 AND status = ?2 AND (?3 = '' OR version = ?3) ORDER BY path LIMIT 1",
                rusqlite::params![id, status, ver],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .ok()
            .flatten();
        let (path, availability) = row?;
        let (bundle, _) = crate::kb::bundle::read::read_bundle(&self.root.join(path));
        let bundle = bundle?;
        let contract = bundle.contract.clone()?;
        let maturity = crate::kb::catalog::status::maturity_of(&bundle, crate::kb::trust::effective_trust(self.root, &bundle));
        Some(crate::designer::validate::graph::Definition {
            contract,
            deprecated: availability == "deprecated",
            implemented: maturity.is_some_and(|m| m != crate::kb::catalog::status::SPECIFICATION_ONLY),
        })
    }
}
