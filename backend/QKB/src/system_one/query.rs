//! Query handling: build, persist and replay a candidate set.

use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};

use super::protocol::*;
use crate::bundle::read::{read_bundle, Bundle};
use crate::catalog::query::fts_terms;
use crate::eligibility::applicability;
use crate::executability::assess;
use crate::profile::ImageFacts;

/// The applicability of `pipe` to declared facts, in wire form.
pub fn applicability_of(conn: &Connection, root: &Path, pipe: &Bundle, facts: &ImageFacts) -> Applicability {
    let reasons = applicability(conn, root, pipe, facts);
    Applicability {
        result: if reasons.is_empty() { "applicable" } else { "not_applicable" },
        reasons: reasons.iter().map(|r| serde_json::to_value(r).unwrap_or(Value::Null)).collect(),
    }
}

/// The fixed-version contract of a pipe: its pinned dependency contracts,
/// wiring and data profile. Nothing here is executable content.
pub fn contract_of(conn: &Connection, root: &Path, pipe: &Bundle) -> Value {
    let Some(graph) = &pipe.graph else { return json!({}) };
    let nodes: Vec<Value> = graph
        .nodes
        .iter()
        .map(|n| {
            let dep = crate::catalog::query::path_of(conn, "algonode", &n.node_ref.id, &n.node_ref.version, "published")
                .ok()
                .flatten()
                .and_then(|p| read_bundle(&root.join(p)).0);
            let c = dep.as_ref().and_then(|b| b.contract.as_ref());
            json!({
                "instance_id": n.instance_id,
                "id": n.node_ref.id,
                "version": n.node_ref.version,
                "content_id": n.node_ref.content_id,
                "purpose": c.and_then(|c| c.purpose.clone()),
                "parameters": n.parameters,
                "inputs": c.map(|c| serde_json::to_value(&c.inputs).unwrap_or(Value::Null)),
                "outputs": c.map(|c| serde_json::to_value(&c.outputs).unwrap_or(Value::Null)),
                "prerequisites": c.map(|c| serde_json::to_value(&c.prerequisites).unwrap_or(Value::Null)),
            })
        })
        .collect();
    json!({
        "nodes": nodes,
        "edges": serde_json::to_value(&graph.edges).unwrap_or(Value::Null),
        "target_data_profile": serde_json::to_value(&graph.target_data_profile).unwrap_or(Value::Null),
    })
}

fn snapshot_of(conn: &Connection, root: &Path, pipe: &Bundle) -> Value {
    let lock = pipe.lock.as_ref();
    let dependencies: Vec<Value> = pipe
        .graph
        .as_ref()
        .map(|g| {
            g.nodes
                .iter()
                .map(|n| json!({ "id": n.node_ref.id, "version": n.node_ref.version, "content_id": n.node_ref.content_id }))
                .collect()
        })
        .unwrap_or_default();
    json!({
        "pipe": { "id": pipe.header.id, "version": pipe.header.version, "content_id": lock.map(|l| l.content_id.clone()) },
        "name": pipe.header.name,
        "summary": pipe.header.summary,
        "intended_use": pipe.header.intended_use,
        "contract": contract_of(conn, root, pipe),
        "limitations": pipe.header.limitations,
        "dependencies": dependencies,
        "source": pipe.header.origin.clone().unwrap_or_else(|| "submitted".to_string()),
        "state": "executable",
    })
}

/// Published AlgoPipe catalog paths whose text matches any purpose word, best first.
fn purpose_matches(conn: &Connection, purpose: &str) -> rusqlite::Result<Vec<String>> {
    let terms = fts_terms(purpose);
    if terms.is_empty() {
        return Ok(Vec::new());
    }
    let mut stmt = conn.prepare(
        "SELECT c.path FROM kb_fts f JOIN kb_catalog_entry c ON c.path = f.path
         WHERE kb_fts MATCH ?1 AND c.kind = 'algopipe' AND c.status = 'published'
         ORDER BY f.rank, c.path",
    )?;
    let rows = stmt.query_map([terms.join(" OR ")], |r| r.get::<_, String>(0))?;
    rows.collect()
}

fn stored_response(conn: &Connection, request_id: &str) -> Result<Value, SoError> {
    let status: String = conn.query_row("SELECT status FROM qkb_query WHERE request_id = ?1", [request_id], |r| r.get(0))?;
    let mut stmt =
        conn.prepare("SELECT rank, snapshot_json, applicability_json FROM qkb_candidate WHERE request_id = ?1 ORDER BY rank")?;
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
    Ok(json!({
        "protocol_version": PROTOCOL_VERSION,
        "request_id": request_id,
        "status": status,
        "candidates": candidates,
    }))
}

/// `POST /system-one/queries`. `source_id` comes from the authenticated caller.
pub fn answer(conn: &Connection, root: &Path, source_id: &str, body: &Value) -> Result<Value, SoError> {
    check_version(body)?;
    guard(body)?;
    let req: QueryRequest = serde_json::from_value(body.clone()).map_err(|e| SoError::Malformed(e.to_string()))?;
    let purpose = req.task_purpose.trim();
    if purpose.is_empty() || purpose.chars().count() > MAX_PURPOSE_CHARS {
        return Err(SoError::Malformed(format!("task_purpose must be 1–{MAX_PURPOSE_CHARS} characters")));
    }
    let request_id = req.request_id.to_string();
    let hash = canonical_hash(body);

    if let Some((existing_hash, existing_source)) = conn
        .query_row("SELECT request_hash, source_id FROM qkb_query WHERE request_id = ?1", [&request_id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })
        .optional()?
    {
        if existing_source != source_id {
            return Err(SoError::AccessDenied);
        }
        return if existing_hash == hash { stored_response(conn, &request_id) } else { Err(SoError::ConflictingRequest) };
    }

    let facts = req.data_conditions.to_facts();
    let limit = req.max_candidates.unwrap_or(DEFAULT_MAX_CANDIDATES).clamp(1, HARD_MAX_CANDIDATES) as usize;
    let include_inapplicable = req.include_inapplicable.unwrap_or(false);

    let mut candidates: Vec<(Value, Value)> = Vec::new();
    for path in purpose_matches(conn, purpose)? {
        if candidates.len() >= limit {
            break;
        }
        let (bundle, findings) = read_bundle(&root.join(&path));
        let Some(bundle) = bundle else { continue };
        let modified = findings.iter().any(|f| f.code == "published_bundle_modified");
        // Defensive: the cleanup invariant already guarantees this.
        if !assess(conn, root, &bundle, modified).executable {
            continue;
        }
        let app = applicability_of(conn, root, &bundle, &facts);
        if app.result != "applicable" && !include_inapplicable {
            continue;
        }
        candidates.push((snapshot_of(conn, root, &bundle), serde_json::to_value(&app).unwrap_or(Value::Null)));
    }

    let status = if candidates.is_empty() { "no_candidates" } else { "answered" };
    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "INSERT INTO qkb_query (request_id, protocol_version, source_id, request_hash, task_purpose, data_conditions_json, received_at, status)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
        params![
            request_id,
            PROTOCOL_VERSION,
            source_id,
            hash,
            purpose,
            serde_json::to_string(&req.data_conditions).unwrap_or_else(|_| "{}".into()),
            now,
            status,
        ],
    )?;
    for (i, (snap, app)) in candidates.iter().enumerate() {
        let pipe = &snap["pipe"];
        tx.execute(
            "INSERT INTO qkb_candidate (request_id, rank, pipe_id, pipe_version, content_id, snapshot_json, applicability_json)
             VALUES (?1,?2,?3,?4,?5,?6,?7)",
            params![
                request_id,
                (i + 1) as i64,
                pipe["id"].as_str().unwrap_or_default(),
                pipe["version"].as_str().unwrap_or_default(),
                pipe["content_id"].as_str().unwrap_or_default(),
                snap.to_string(),
                app.to_string(),
            ],
        )?;
    }
    tx.commit()?;
    stored_response(conn, &request_id)
}
