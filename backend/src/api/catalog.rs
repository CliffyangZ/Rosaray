//! Read-only catalog (frontend). Everything listed is published and currently
//! executable — the QKB deletes anything else (constitution III).

use axum::extract::{Path, Query, State};
use axum::Json;
use rosaray_qkb::bundle::read::read_bundle;
use rosaray_qkb::catalog::query::{path_of, query_entries, EntryFilter};
use rosaray_qkb::catalog::status::derive;
use serde_json::{json, Value};

use super::{ApiError, AppState};

pub async fn session() -> Json<Value> {
    Json(json!({ "state": "ready" }))
}

pub async fn entries(State(state): State<AppState>, Query(mut filter): Query<EntryFilter>) -> Result<Json<Value>, ApiError> {
    // Only executable, published versions are ever exposed.
    filter.status = Some("published".into());
    filter.release = None;
    filter.availability = None;
    let page = state
        .blocking(move |conn, _root| {
            Ok(query_entries(conn, &filter)?)
        })
        .await?;
    let entries: Vec<Value> = page
        .entries
        .iter()
        .map(|e| {
            json!({
                "kind": e.kind, "id": e.id, "version": e.version, "content_id": e.content_id,
                "name": e.name, "summary": e.summary, "purpose": e.purpose, "domain": e.domain,
                "data_kinds": e.data_kinds.as_deref().map(|k| k.split_whitespace().collect::<Vec<_>>()).unwrap_or_default(),
                "state": "executable",
            })
        })
        .collect();
    Ok(Json(json!({ "entries": entries, "next_cursor": page.next })))
}

fn parse_kind(kind: &str) -> Result<&'static str, ApiError> {
    match kind {
        "algonode" | "nodes" => Ok("algonode"),
        "algopipe" | "pipes" => Ok("algopipe"),
        _ => Err(ApiError::not_found()),
    }
}

pub async fn versions(State(state): State<AppState>, Path((kind, id)): Path<(String, String)>) -> Result<Json<Value>, ApiError> {
    let kind = parse_kind(&kind)?;
    let (kind_s, id_s) = (kind.to_string(), id.clone());
    let list = state
        .blocking(move |conn, _root| {
            let mut stmt = conn.prepare(
                "SELECT version, content_id FROM kb_catalog_entry WHERE kind = ?1 AND id = ?2 AND status = 'published' ORDER BY version",
            )?;
            let rows = stmt.query_map([&kind_s, &id_s], |r| Ok(json!({ "version": r.get::<_, String>(0)?, "content_id": r.get::<_, Option<String>>(1)? })))?;
            Ok(rows.collect::<Result<Vec<_>, _>>()?)
        })
        .await?;
    if list.is_empty() {
        return Err(ApiError::not_found());
    }
    Ok(Json(json!({ "kind": kind, "id": id, "versions": list })))
}

pub async fn detail(State(state): State<AppState>, Path((kind, id, version)): Path<(String, String, String)>) -> Result<Json<Value>, ApiError> {
    let kind = parse_kind(&kind)?;
    state
        .blocking(move |conn, root| {
            let path = path_of(conn, kind, &id, &version, "published")?.ok_or_else(ApiError::not_found)?;
            let (bundle, findings) = read_bundle(&root.join(&path));
            let bundle = bundle.ok_or_else(ApiError::not_found)?;
            let modified = findings.iter().any(|f| f.code == "published_bundle_modified");
            if !rosaray_qkb::executability::assess(conn, root, &bundle, modified).executable {
                return Err(ApiError::not_found());
            }
            let derived = derive(root, &bundle, modified);
            let lock = bundle.lock.as_ref();
            let dependencies: Vec<Value> = bundle
                .graph
                .as_ref()
                .map(|g| g.nodes.iter().map(|n| json!({ "instance_id": n.instance_id, "id": n.node_ref.id, "version": n.node_ref.version, "content_id": n.node_ref.content_id })).collect())
                .unwrap_or_default();
            let contract = if kind == "algonode" {
                bundle.contract.as_ref().and_then(|c| serde_json::to_value(c).ok())
            } else {
                Some(rosaray_qkb::system_one::query::contract_of(conn, root, &bundle))
            };
            Ok(Json(json!({
                "kind": kind,
                "id": bundle.header.id,
                "version": bundle.header.version,
                "content_id": lock.map(|l| l.content_id.clone()),
                "name": bundle.header.name,
                "summary": bundle.header.summary,
                "intended_use": bundle.header.intended_use,
                "limitations": bundle.header.limitations,
                "domain": bundle.header.domain,
                "contract": contract,
                "dependencies": dependencies,
                "implementation": bundle.implementation.as_ref().map(|i| json!({ "id": i.implementation_id, "version": i.implementation_version })),
                "release_kind": lock.and_then(|l| l.release_kind.clone()),
                "verification": derived.verification.as_str(),
                "state": "executable",
            })))
        })
        .await
}
