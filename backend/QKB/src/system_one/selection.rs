//! Selection / abstention reports: validate against the stored candidate set
//! and current eligibility, then record (accepted *or* rejected) idempotently.

use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};

use super::protocol::*;
use super::query::applicability_of;
use crate::bundle::read::read_bundle;
use crate::catalog::query::path_of;
use crate::catalog::repo::refresh;
use crate::executability::assess;

struct QueryRow {
    source_id: String,
    conditions: DataConditions,
}

fn load_query(conn: &Connection, request_id: &str) -> Result<QueryRow, SoError> {
    conn.query_row("SELECT source_id, data_conditions_json FROM qkb_query WHERE request_id = ?1", [request_id], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
    })
    .optional()?
    .map(|(source_id, json)| QueryRow { source_id, conditions: serde_json::from_str(&json).unwrap_or_default() })
    .ok_or(SoError::UnknownRequest)
}

fn record_json(conn: &Connection, request_id: &str) -> Result<Value, SoError> {
    let row = conn.query_row(
        "SELECT decision, validation, rejection_code, contract_snapshot_json, selected_id, selected_version, selected_content_id,
                reason, selected_version_deleted, recorded_at, source_id
         FROM qkb_selection WHERE request_id = ?1",
        [request_id],
        |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, Option<String>>(5)?,
                r.get::<_, Option<String>>(6)?,
                r.get::<_, String>(7)?,
                r.get::<_, bool>(8)?,
                r.get::<_, String>(9)?,
                r.get::<_, String>(10)?,
            ))
        },
    )?;
    let (decision, validation, code, snap, sid, sver, scid, reason, deleted, at, source) = row;
    let selection = match (sid, sver, scid) {
        (Some(id), Some(version), Some(content_id)) => Some(json!({ "id": id, "version": version, "content_id": content_id })),
        _ => None,
    };
    Ok(json!({
        "protocol_version": PROTOCOL_VERSION,
        "request_id": request_id,
        "validation": validation,
        "rejection_code": code,
        "record": {
            "decision": decision,
            "selection": selection,
            "reason": reason,
            "source_id": source,
            "contract_snapshot": snap.and_then(|s| serde_json::from_str::<Value>(&s).ok()),
            "selected_version_deleted": deleted,
            "recorded_at": at,
        },
    }))
}

/// `POST /system-one/selections`.
pub fn report(conn: &Connection, root: &Path, source_id: &str, body: &Value) -> Result<Value, SoError> {
    check_version(body)?;
    guard(body)?;
    let req: SelectionRequest = serde_json::from_value(body.clone()).map_err(|e| SoError::Malformed(e.to_string()))?;
    let request_id = req.request_id.to_string();
    let query = load_query(conn, &request_id)?;
    if query.source_id != source_id {
        return Err(SoError::AccessDenied);
    }
    let hash = canonical_hash(body);
    if let Some(existing) = conn
        .query_row("SELECT report_hash FROM qkb_selection WHERE request_id = ?1", [&request_id], |r| r.get::<_, String>(0))
        .optional()?
    {
        return if existing == hash { record_json(conn, &request_id) } else { Err(SoError::ConflictingReport) };
    }

    if req.reason.trim().is_empty() || req.reason.chars().count() > MAX_REASON_CHARS {
        return Err(SoError::Malformed(format!("reason must be 1–{MAX_REASON_CHARS} characters")));
    }
    let (selected, decision): (Option<&VersionRef>, &str) = match (req.decision.as_str(), &req.selection) {
        ("selected", Some(s)) => (Some(s), "selected"),
        ("selected", None) => return Err(SoError::Malformed("selection is required when decision is `selected`".into())),
        ("abstained", None) => (None, "abstained"),
        ("abstained", Some(_)) => return Err(SoError::Malformed("selection must be absent when decision is `abstained`".into())),
        _ => return Err(SoError::Malformed("decision must be `selected` or `abstained`".into())),
    };

    let mut rejection: Option<&'static str> = None;
    let mut snapshot: Option<Value> = None;
    if let Some(sel) = selected {
        rejection = check_selection(conn, root, &request_id, sel, &query.conditions, &mut snapshot)?;
    }

    let validation = if rejection.is_some() { "rejected" } else { "accepted" };
    let contract_snapshot = snapshot.map(|s| json!({
        "pipe": s["pipe"], "name": s["name"], "summary": s["summary"], "contract": s["contract"],
        "limitations": s["limitations"], "dependencies": s["dependencies"],
    }));
    conn.execute(
        "INSERT INTO qkb_selection (request_id, report_hash, source_id, decision, selected_id, selected_version, selected_content_id,
            reason, validation, rejection_code, contract_snapshot_json, selected_version_deleted, recorded_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,0,?12)",
        params![
            request_id,
            hash,
            source_id,
            decision,
            selected.map(|s| s.id.clone()),
            selected.map(|s| s.version.clone()),
            selected.map(|s| s.content_id.clone()),
            req.reason.trim(),
            validation,
            rejection,
            contract_snapshot.map(|c| c.to_string()),
            chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        ],
    )?;
    record_json(conn, &request_id)
}

/// Returns the rejection code, or `None` when the selection stands. Also
/// hands back the candidate snapshot (for the record) when the pair was a candidate.
fn check_selection(
    conn: &Connection,
    root: &Path,
    request_id: &str,
    sel: &VersionRef,
    conditions: &DataConditions,
    snapshot: &mut Option<Value>,
) -> Result<Option<&'static str>, SoError> {
    let candidate: Option<(String, String)> = conn
        .query_row(
            "SELECT content_id, snapshot_json FROM qkb_candidate WHERE request_id = ?1 AND pipe_id = ?2 AND pipe_version = ?3",
            params![request_id, sel.id, sel.version],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let Some((cand_content, snap_json)) = candidate else { return Ok(Some("not_in_candidates")) };
    *snapshot = serde_json::from_str(&snap_json).ok();
    if cand_content != sel.content_id {
        return Ok(Some("identity_mismatch"));
    }

    // Re-check against *now*, never against the stale candidate list.
    refresh(conn, root, &mut |_| {})?;
    let Some(path) = path_of(conn, "algopipe", &sel.id, &sel.version, "published")? else { return Ok(Some("version_deleted")) };
    let (bundle, findings) = read_bundle(&root.join(path));
    let Some(bundle) = bundle else { return Ok(Some("version_deleted")) };
    if bundle.lock.as_ref().map(|l| l.content_id.as_str()) != Some(sel.content_id.as_str()) {
        return Ok(Some("identity_mismatch"));
    }
    let modified = findings.iter().any(|f| f.code == "published_bundle_modified");
    if !assess(conn, root, &bundle, modified).executable {
        return Ok(Some("no_longer_executable"));
    }
    if applicability_of(conn, root, &bundle, &conditions.to_facts()).result != "applicable" {
        return Ok(Some("not_applicable"));
    }
    Ok(None)
}

/// `GET /system-one/selections/:request_id`: query, candidate snapshots and the record.
pub fn read_record(conn: &Connection, request_id: &str) -> Result<Value, SoError> {
    let (source, purpose, conditions, received, status): (String, String, String, String, String) = conn
        .query_row(
            "SELECT source_id, task_purpose, data_conditions_json, received_at, status FROM qkb_query WHERE request_id = ?1",
            [request_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .optional()?
        .ok_or(SoError::UnknownRequest)?;
    let mut stmt = conn.prepare("SELECT rank, snapshot_json, applicability_json FROM qkb_candidate WHERE request_id = ?1 ORDER BY rank")?;
    let rows = stmt.query_map([request_id], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))?;
    let mut candidates = Vec::new();
    for row in rows {
        let (rank, snap, app) = row?;
        let mut c: Value = serde_json::from_str(&snap).unwrap_or(Value::Null);
        if let Some(o) = c.as_object_mut() {
            o.insert("rank".into(), json!(rank));
            o.insert("applicability".into(), serde_json::from_str(&app).unwrap_or(Value::Null));
        }
        candidates.push(c);
    }
    let selection = match record_json(conn, request_id) {
        Ok(v) => Some(v),
        Err(SoError::Db(rusqlite::Error::QueryReturnedNoRows)) => None,
        Err(e) => return Err(e),
    };
    Ok(json!({
        "protocol_version": PROTOCOL_VERSION,
        "request_id": request_id,
        "query": {
            "source_id": source, "task_purpose": purpose,
            "data_conditions": serde_json::from_str::<Value>(&conditions).unwrap_or(Value::Null),
            "received_at": received, "status": status,
        },
        "candidates": candidates,
        "selection": selection,
    }))
}
