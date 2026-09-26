//! `/kb` routes (specs/002-paper-pipeline-designer/contracts/kb-api.md):
//! catalog, drafts, publication and exchange. Paths never expose filesystem
//! locations; the service resolves ids to folders itself.

use std::collections::BTreeMap;

use axum::body::Body;
use axum::extract::{DefaultBodyLimit, Multipart, Path, Query, State};
use axum::http::header;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::api::errors::{KbError, KbErrorCode};
use crate::api::AppState;
use crate::data_repository::sqlite::kb_event_repo;
use crate::designer::validate::finding::Finding;
use crate::domain::ServiceError;
use crate::events::{Event, EventPayload};
use crate::kb::bundle::draft::DraftConflict;
use crate::kb::bundle::model::{GraphFile, Kind, TargetDataProfile};
use crate::kb::bundle::read::read_bundle;
use crate::kb::bundle::service::{self, parse_kind, CreateDraft, KbServiceError};
use crate::kb::catalog::query::{findings_for_path, path_of, query_entries, EntryFilter};
use crate::kb::catalog::repo;
use crate::kb::catalog::scan::ScanEvent;
use crate::kb::exchange::export::{plan_export, ExportRefused};
use crate::kb::exchange::import::{self, ImportError};
use crate::kb::publish::{publish, PublishRequest, Release};

/// A knowledge-base API failure: either a KB-specific coded error or one of
/// feature 001's common errors.
pub enum ApiError {
    Kb(KbError),
    Service(ServiceError),
}

impl From<ServiceError> for ApiError {
    fn from(e: ServiceError) -> Self {
        ApiError::Service(e)
    }
}

impl From<KbError> for ApiError {
    fn from(e: KbError) -> Self {
        ApiError::Kb(e)
    }
}

impl From<rusqlite::Error> for ApiError {
    fn from(_: rusqlite::Error) -> Self {
        ApiError::Service(ServiceError::ServiceUnavailable)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        match self {
            ApiError::Kb(e) => e.into_response(),
            ApiError::Service(e) => e.into_response(),
        }
    }
}

fn findings_json(findings: &[Finding]) -> Value {
    json!({ "findings": findings })
}

/// Maps a service-layer failure to the contract's error envelope.
/// `your_base` is the revision the caller sent, for `draft_conflict`.
pub fn map_service_error(e: KbServiceError, your_base: Option<&str>) -> ApiError {
    match e {
        KbServiceError::IdentityConflict(_) => KbError::new(KbErrorCode::IdentityConflict, "identity conflict").into(),
        KbServiceError::NotFound => ServiceError::NotFound.into(),
        KbServiceError::Invalid(findings) => {
            KbError::new(KbErrorCode::BundleInvalid, "bundle invalid").with_details(findings_json(&findings)).into()
        }
        KbServiceError::DraftConflict(DraftConflict { on_disk_revision, changed_files }) => {
            KbError::new(KbErrorCode::DraftConflict, "the draft changed on disk since it was opened")
                .with_details(json!({
                    "on_disk_revision": on_disk_revision,
                    "your_base": your_base,
                    "changed_files": changed_files,
                }))
                .into()
        }
        KbServiceError::PublishedImmutable => {
            KbError::new(KbErrorCode::PublishedImmutable, "published versions cannot be modified").into()
        }
        KbServiceError::NotPublishable(findings) => {
            KbError::new(KbErrorCode::NotPublishable, "not publishable").with_details(findings_json(&findings)).into()
        }
        KbServiceError::UnsafePath(_) => KbError::new(KbErrorCode::BundleInvalid, "bundle invalid").into(),
        KbServiceError::Io(_) | KbServiceError::Db(_) => ServiceError::ServiceUnavailable.into(),
    }
}

fn kind_or_404(s: &str) -> Result<Kind, ApiError> {
    parse_kind(s).ok_or(ApiError::Service(ServiceError::NotFound))
}

// ---- catalog -------------------------------------------------------------------

pub async fn get_entries(
    State(state): State<AppState>,
    Query(filter): Query<EntryFilter>,
) -> Result<Json<Value>, ApiError> {
    let page = {
        let db = state.db.lock().unwrap();
        query_entries(&db, &filter)?
    };
    Ok(Json(json!({ "entries": page.entries, "next": page.next })))
}

fn emit(state: &AppState, scan_id: Uuid, event: ScanEvent) {
    let payload = match event {
        ScanEvent::Progress { scanned, total, invalid } => EventPayload::KbScanProgress { scan_id, scanned, total, invalid },
        ScanEvent::Complete { indexed, invalid } => EventPayload::KbScanComplete { scan_id, indexed, invalid },
    };
    // No subscribers is fine: events are hints, queries are authoritative.
    let _ = state.event_tx.send(Event::new(scan_id, payload));
}

fn run_scan(state: &AppState, rebuild: bool) -> Result<Json<Value>, ApiError> {
    let scan_id = Uuid::new_v4();
    let outcome = {
        let db = state.db.lock().unwrap();
        let mut progress = |e: ScanEvent| emit(state, scan_id, e);
        if rebuild {
            repo::rebuild(&db, &state.kb_root, &mut progress)?
        } else {
            repo::refresh(&db, &state.kb_root, &mut progress)?
        }
    };
    for (kind, id, revision) in &outcome.externally_changed_drafts {
        let _ = state.event_tx.send(Event::new(
            scan_id,
            EventPayload::KbBundleChangedExternally { kind: kind.clone(), id: id.clone(), revision: revision.clone() },
        ));
    }
    Ok(Json(json!({
        "scanned": outcome.scanned,
        "indexed": outcome.indexed,
        "invalid": outcome.invalid,
        "findings_url": "/kb/findings",
    })))
}

pub async fn post_refresh(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    run_scan(&state, false)
}

pub async fn post_rebuild(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    run_scan(&state, true)
}

#[derive(Deserialize)]
pub struct FindingsQuery {
    pub kind: Option<String>,
    pub id: Option<String>,
    pub version: Option<String>,
    pub status: Option<String>,
}

/// `GET /kb/findings` — derived findings for every matching catalog entry.
pub async fn get_findings(
    State(state): State<AppState>,
    Query(q): Query<FindingsQuery>,
) -> Result<Json<Value>, ApiError> {
    let db = state.db.lock().unwrap();
    let mut stmt = db.prepare(
        "SELECT path, kind, id, version, status FROM kb_catalog_entry
         WHERE (?1 IS NULL OR kind = ?1) AND (?2 IS NULL OR id = ?2)
           AND (?3 IS NULL OR version = ?3) AND (?4 IS NULL OR status = ?4)
         ORDER BY path",
    )?;
    let rows: Vec<(String, Option<String>, Option<String>, Option<String>, String)> = stmt
        .query_map(rusqlite::params![q.kind, q.id, q.version, q.status], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
        })?
        .collect::<Result<_, _>>()?;
    let mut out = Vec::new();
    for (path, kind, id, version, status) in rows {
        let findings = findings_for_path(&db, &path)?;
        if !findings.is_empty() {
            out.push(json!({ "kind": kind, "id": id, "version": version, "status": status, "findings": findings }));
        }
    }
    Ok(Json(json!({ "bundles": out })))
}

#[derive(Deserialize)]
pub struct PublishedQuery {
    /// Evaluate the node's prerequisites against this Image Asset.
    pub image_asset_id: Option<Uuid>,
}

/// `GET /kb/{kind}/{id}/{ver}` — the published definition plus the assembled
/// `inspector` block (five separate status dimensions, three evidence types).
pub async fn get_published(
    State(state): State<AppState>,
    Path((kind, id, ver)): Path<(String, String, String)>,
    Query(q): Query<PublishedQuery>,
) -> Result<Json<Value>, ApiError> {
    let kind = kind_or_404(&kind)?;
    let db = state.db.lock().unwrap();
    let path = path_of(&db, kind.as_str(), &id, &ver, "published")?.ok_or(ServiceError::NotFound)?;
    let image = match q.image_asset_id {
        Some(image_id) => Some(
            crate::data_repository::sqlite::dataset_repo::image_asset_by_id(&db, image_id)?.ok_or(ServiceError::NotFound)?,
        ),
        None => None,
    };
    let mut view = bundle_view(&db, &state, kind, &path, None)?;
    view["inspector"] = crate::kb::inspector::inspect(&db, &state.kb_root, kind, &id, &ver, image.as_ref())
        .map_err(|e| map_service_error(e, None))?;
    Ok(Json(view))
}

#[derive(Deserialize)]
pub struct DeprecateBody {
    pub reason: String,
    #[serde(default)]
    pub replacement: Option<ReplacementRef>,
}

#[derive(Deserialize)]
pub struct ReplacementRef {
    pub id: String,
    pub version: String,
}

/// `POST /kb/{kind}/{id}/{ver}/deprecate` — appends a notice to the version's
/// amendment chain. Nothing is replaced and no bundle file is touched
/// (FR-013, FR-043).
pub async fn post_deprecate(
    State(state): State<AppState>,
    Path((kind, id, ver)): Path<(String, String, String)>,
    Json(body): Json<DeprecateBody>,
) -> Result<Json<Value>, ApiError> {
    use crate::kb::evidence::deprecation;
    let kind = kind_or_404(&kind)?;
    if body.reason.trim().is_empty() {
        return Err(KbError::new(KbErrorCode::BundleInvalid, "a reason is required").into());
    }
    let db = state.db.lock().unwrap();
    path_of(&db, kind.as_str(), &id, &ver, "published")?.ok_or(ServiceError::NotFound)?;
    let head = deprecation::deprecate(
        &state.kb_root,
        &id,
        &ver,
        body.reason.trim(),
        body.replacement.as_ref().map(|r| (r.id.as_str(), r.version.as_str())),
    )
    .map_err(|_| ServiceError::ServiceUnavailable)?;
    kb_event_repo::append(&db, "amendment_recorded", &format!("{id}@{ver}"), &json!({ "kind": "deprecation", "seq": head.seq }))?;
    service::sync_catalog(&db, &state.kb_root)?;
    Ok(Json(json!({ "seq": head.seq, "chain_head": head.hash })))
}

fn bundle_view(
    db: &rusqlite::Connection,
    state: &AppState,
    kind: Kind,
    rel_path: &str,
    revision: Option<&str>,
) -> Result<Value, ApiError> {
    let dir = state.kb_root.join(rel_path);
    let (bundle, _) = read_bundle(&dir);
    let bundle = bundle.ok_or(ServiceError::NotFound)?;
    let findings = findings_for_path(db, rel_path)?;
    Ok(json!({
        "kind": kind.as_str(),
        "id": bundle.header.id,
        "version": bundle.header.version,
        "status": if bundle.is_published() { "published" } else { "draft" },
        "header": bundle.header,
        "description": bundle.body,
        "contract": bundle.contract,
        "graph": bundle.graph,
        "implementation": bundle.implementation,
        "content_id": bundle.lock.as_ref().map(|l| l.content_id.clone()),
        "revision": revision,
        "findings": findings,
    }))
}

// ---- drafts --------------------------------------------------------------------

pub async fn get_draft(
    State(state): State<AppState>,
    Path((kind, id)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    let kind = kind_or_404(&kind)?;
    let db = state.db.lock().unwrap();
    let view = service::read_draft(&db, &state.kb_root, kind, &id).map_err(|e| map_service_error(e, None))?;
    Ok(Json(json!({
        "kind": kind.as_str(),
        "id": id,
        "revision": view.revision,
        "header": view.bundle.as_ref().map(|b| &b.header),
        "graph": view.bundle.as_ref().and_then(|b| b.graph.as_ref()),
        "files": view.files,
        "findings": view.findings,
    })))
}

#[derive(Deserialize)]
pub struct CreateDraftBody {
    pub id: String,
    pub name: Option<String>,
    pub summary: Option<String>,
    pub version: Option<String>,
    /// `"blank"` or `{ id, version }`.
    pub from: Option<Value>,
}

pub async fn post_draft(
    State(state): State<AppState>,
    Path(kind): Path<String>,
    Json(body): Json<CreateDraftBody>,
) -> Result<(axum::http::StatusCode, Json<Value>), ApiError> {
    let kind = kind_or_404(&kind)?;
    let from = match &body.from {
        None => None,
        Some(Value::String(s)) if s == "blank" => None,
        Some(v) => match (v.get("id").and_then(Value::as_str), v.get("version").and_then(Value::as_str)) {
            (Some(i), Some(ver)) => Some((i.to_string(), ver.to_string())),
            _ => return Err(KbError::new(KbErrorCode::BundleInvalid, "`from` must be \"blank\" or { id, version }").into()),
        },
    };
    let db = state.db.lock().unwrap();
    let handle = service::create_draft(
        &db,
        &state.kb_root,
        kind,
        CreateDraft { id: body.id, name: body.name, summary: body.summary, from, version: body.version },
    )
    .map_err(|e| map_service_error(e, None))?;
    Ok((
        axum::http::StatusCode::CREATED,
        Json(json!({
            "draft": { "kind": handle.kind.as_str(), "id": handle.id },
            "revision": handle.revision,
            "findings": handle.findings,
        })),
    ))
}

#[derive(Deserialize)]
pub struct PutDraftBody {
    pub base_revision: String,
    /// Raw file contents keyed by relative path.
    #[serde(default)]
    pub files: Option<BTreeMap<String, String>>,
    /// Structured AlgoPipe save.
    #[serde(default)]
    pub graph: Option<GraphFile>,
    #[serde(default)]
    pub profile: Option<TargetDataProfile>,
    #[serde(default)]
    pub description: Option<String>,
}

pub async fn put_draft(
    State(state): State<AppState>,
    Path((kind, id)): Path<(String, String)>,
    Json(body): Json<PutDraftBody>,
) -> Result<Json<Value>, ApiError> {
    let kind = kind_or_404(&kind)?;
    let db = state.db.lock().unwrap();
    let base = body.base_revision.clone();
    let previous_graph = if kind == Kind::Algopipe {
        read_bundle(&crate::kb::catalog::repo::draft_dir(&state.kb_root, "algopipe", &id)).0.and_then(|b| b.graph)
    } else {
        None
    };
    let new_graph = body.graph.clone();
    let outcome = if let (Some(graph), Kind::Algopipe) = (body.graph, kind) {
        let mut outcome = service::save_pipe_structured(
            &db,
            &state.kb_root,
            &id,
            &body.base_revision,
            graph,
            body.profile,
            body.description,
        )
        .map_err(|e| map_service_error(e, Some(&base)))?;
        if let Some(files) = body.files.filter(|f| !f.is_empty()) {
            outcome = service::save_draft_files(&db, &state.kb_root, kind, &id, &outcome.revision, &files)
                .map_err(|e| map_service_error(e, Some(&base)))?;
        }
        outcome
    } else if let Some(files) = body.files {
        service::save_draft_files(&db, &state.kb_root, kind, &id, &body.base_revision, &files)
            .map_err(|e| map_service_error(e, Some(&base)))?
    } else {
        return Err(KbError::new(KbErrorCode::BundleInvalid, "nothing to save: send `files` or a structured `graph`").into());
    };
    // A computational edit makes earlier Preview results stale (FR-018).
    if let (Some(prev), Some(new)) = (previous_graph, new_graph) {
        let stale = crate::designer::validate::graph::stale_cone(&prev, &new);
        if !stale.is_empty() {
            state.preview_board.mark_stale(&id, &stale);
            let _ = state.event_tx.send(Event::new(
                Uuid::nil(),
                EventPayload::PreviewStale { pipe_id: id.clone(), revision: outcome.revision.clone(), nodes: stale },
            ));
        }
    }
    Ok(Json(json!({ "revision": outcome.revision, "findings": outcome.findings })))
}

pub async fn delete_draft(
    State(state): State<AppState>,
    Path((kind, id)): Path<(String, String)>,
) -> Result<axum::http::StatusCode, ApiError> {
    let kind = kind_or_404(&kind)?;
    let db = state.db.lock().unwrap();
    service::discard_draft(&db, &state.kb_root, kind, &id).map_err(|e| map_service_error(e, None))?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct PublishBody {
    pub base_revision: String,
    pub release: String,
    #[serde(default)]
    pub version_description: Option<String>,
    #[serde(default)]
    pub acknowledge_unvalidated: bool,
}

pub async fn post_publish(
    State(state): State<AppState>,
    Path((kind, id)): Path<(String, String)>,
    Json(body): Json<PublishBody>,
) -> Result<Json<Value>, ApiError> {
    let kind = kind_or_404(&kind)?;
    let release = match body.release.as_str() {
        "knowledge" => Release::Knowledge,
        "executable" => Release::Executable,
        _ => return Err(KbError::new(KbErrorCode::BundleInvalid, "release must be `knowledge` or `executable`").into()),
    };
    let db = state.db.lock().unwrap();
    let base = body.base_revision.clone();
    let outcome = publish(
        &db,
        &state.kb_root,
        PublishRequest {
            kind,
            id,
            base_revision: body.base_revision,
            release,
            version_description: body.version_description,
            acknowledge_unvalidated: body.acknowledge_unvalidated,
        },
    )
    .map_err(|e| map_service_error(e, Some(&base)))?;
    Ok(Json(serde_json::to_value(outcome).expect("outcome serializes")))
}

// ---- exchange ------------------------------------------------------------------

#[derive(Deserialize)]
pub struct ExportPreviewBody {
    pub kind: String,
    pub id: String,
    pub version: String,
}

fn manifest_response(plan: &crate::kb::exchange::export::ExportPlan) -> Value {
    json!({
        "manifest": plan.manifest,
        "manifest_hash": plan.manifest_hash,
        "findings": plan.findings,
        "blocked": plan.blocked,
    })
}

pub async fn post_export_preview(
    State(state): State<AppState>,
    Json(body): Json<ExportPreviewBody>,
) -> Result<Json<Value>, ApiError> {
    let kind = kind_or_404(&body.kind)?;
    let db = state.db.lock().unwrap();
    let plan = plan_export(&db, &state.kb_root, kind, &body.id, &body.version).map_err(|e| map_service_error(e, None))?;
    Ok(Json(manifest_response(&plan)))
}

#[derive(Deserialize)]
pub struct ExportBody {
    pub kind: String,
    pub id: String,
    pub version: String,
    pub manifest_hash: String,
}

pub async fn post_export(
    State(state): State<AppState>,
    Json(body): Json<ExportBody>,
) -> Result<Json<Value>, ApiError> {
    let kind = kind_or_404(&body.kind)?;
    let db = state.db.lock().unwrap();
    let plan = plan_export(&db, &state.kb_root, kind, &body.id, &body.version).map_err(|e| map_service_error(e, None))?;
    let bytes = match plan.write_archive(&body.manifest_hash) {
        Ok(b) => b,
        Err(ExportRefused::PatientDataBlocked) => {
            return Err(KbError::new(KbErrorCode::PatientDataBlocked, "the bundle contains data that cannot be exchanged")
                .with_details(findings_json(&plan.findings))
                .into())
        }
        Err(ExportRefused::ManifestChanged) => return Err(ServiceError::Conflict.into()),
    };
    let export_id = Uuid::new_v4();
    let dir = state.exports_dir.join("kb");
    std::fs::create_dir_all(&dir).map_err(|_| ServiceError::ServiceUnavailable)?;
    std::fs::write(dir.join(format!("{export_id}.algobundle")), &bytes).map_err(|_| ServiceError::ServiceUnavailable)?;
    kb_event_repo::append(
        &db,
        "exported",
        &format!("{}@{}", body.id, body.version),
        &json!({ "kind": kind.as_str(), "manifest_hash": plan.manifest_hash, "files": plan.manifest.entries.len(), "size_bytes": bytes.len() }),
    )?;
    Ok(Json(json!({ "export_id": export_id, "manifest_hash": plan.manifest_hash, "size_bytes": bytes.len() })))
}

pub async fn get_export_content(
    State(state): State<AppState>,
    Path(export_id): Path<Uuid>,
) -> Result<Response, ApiError> {
    let path = state.exports_dir.join("kb").join(format!("{export_id}.algobundle"));
    let bytes = std::fs::read(path).map_err(|_| ServiceError::NotFound)?;
    Ok((
        [
            (header::CONTENT_TYPE, "application/zip".to_string()),
            (header::CONTENT_DISPOSITION, format!("attachment; filename=\"{export_id}.algobundle\"")),
        ],
        Body::from(bytes),
    )
        .into_response())
}

fn map_import_error(e: ImportError) -> ApiError {
    match e {
        ImportError::ArchiveRejected(reason) => {
            KbError::new(KbErrorCode::BundleInvalid, "the archive was rejected").with_details(json!({ "reason": reason })).into()
        }
        ImportError::PatientDataBlocked(findings) => {
            KbError::new(KbErrorCode::PatientDataBlocked, "the archive contains data that cannot be exchanged")
                .with_details(findings_json(&findings))
                .into()
        }
        ImportError::IdentityConflict(conflicts) => {
            KbError::new(KbErrorCode::IdentityConflict, "identity conflict").with_details(json!({ "conflicts": conflicts })).into()
        }
        ImportError::NotFound => ServiceError::NotFound.into(),
        ImportError::Io(_) | ImportError::Db(_) => ServiceError::ServiceUnavailable.into(),
    }
}

pub async fn post_import(State(state): State<AppState>, mut multipart: Multipart) -> Result<Json<Value>, ApiError> {
    let mut bytes = None;
    while let Some(field) = multipart.next_field().await.map_err(|_| KbError::new(KbErrorCode::BundleInvalid, "bad upload"))? {
        if field.name() == Some("bundle") {
            bytes = Some(field.bytes().await.map_err(|_| KbError::new(KbErrorCode::BundleInvalid, "bad upload"))?);
        }
    }
    let bytes = bytes.ok_or_else(|| KbError::new(KbErrorCode::BundleInvalid, "missing `bundle` file"))?;
    let staged = {
        let db = state.db.lock().unwrap();
        import::stage(&db, &state.kb_root, &bytes).map_err(map_import_error)?
    };
    Ok(Json(json!({ "staged_id": staged.staged_id, "plan": staged.plan, "findings": staged.findings })))
}

pub async fn post_import_confirm(
    State(state): State<AppState>,
    Path(staged_id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    let outcome = {
        let db = state.db.lock().unwrap();
        import::confirm(&db, &state.kb_root, staged_id).map_err(map_import_error)?
    };
    Ok(Json(json!({ "applied": outcome.applied, "identical": outcome.identical })))
}

pub async fn post_import_cancel(
    State(state): State<AppState>,
    Path(staged_id): Path<Uuid>,
) -> Result<axum::http::StatusCode, ApiError> {
    import::cancel(&state.kb_root, staged_id).map_err(map_import_error)?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct VerificationBody {
    /// `technical` or `dataset`
    #[serde(rename = "type")]
    pub vtype: String,
    /// `passed`, `failed` or `withdrawn`
    pub event: String,
    pub implementation_id: String,
    pub implementation_version: String,
    #[serde(default)]
    pub asset_content_ids: Vec<String>,
    #[serde(default)]
    pub suite: Option<Value>,
    #[serde(default)]
    pub scope: Option<Value>,
    #[serde(default)]
    pub reason: Option<String>,
}

/// `POST /kb/nodes/{id}/{ver}/verification` — appends to the node's
/// verification chain. A technical `passed` is only ever *produced by* a run of
/// the bundle's own tests through its built-in executor; it cannot be asserted.
pub async fn post_verification(
    State(state): State<AppState>,
    Path((kind, id, ver)): Path<(String, String, String)>,
    Json(body): Json<VerificationBody>,
) -> Result<Json<Value>, ApiError> {
    use crate::kb::evidence::tech_verify::{run_bundle_tests, suite_of, TestError};
    use crate::kb::evidence::verification::{record_event, Event as VEvent, Subject as VSubject, VerificationType};
    if kind_or_404(&kind)? != Kind::Algonode {
        return Err(ServiceError::NotFound.into());
    }
    let vtype = match body.vtype.as_str() {
        "technical" => VerificationType::Technical,
        "dataset" => VerificationType::Dataset,
        _ => return Err(KbError::new(KbErrorCode::BundleInvalid, "type must be `technical` or `dataset`").into()),
    };
    let event = VEvent::parse(&body.event).ok_or_else(|| KbError::new(KbErrorCode::BundleInvalid, "event must be passed, failed or withdrawn"))?;
    let db = state.db.lock().unwrap();
    let before = crate::kb::eligibility::snapshot_for_node(&db, &state.kb_root, &id);
    let path = path_of(&db, "algonode", &id, &ver, "published")?.ok_or(ServiceError::NotFound)?;
    let (bundle, _) = read_bundle(&state.kb_root.join(&path));
    let bundle = bundle.ok_or(ServiceError::NotFound)?;
    let content_id = bundle.lock.as_ref().map(|l| l.content_id.clone()).ok_or(ServiceError::NotFound)?;
    let subject = VSubject {
        node_id: id.clone(),
        definition_content_id: content_id,
        implementation_id: body.implementation_id.clone(),
        implementation_version: body.implementation_version.clone(),
        asset_content_ids: body.asset_content_ids.clone(),
    };

    let mut suite = body.suite.clone();
    if vtype == VerificationType::Technical && event == VEvent::Passed {
        // Produced by a run, not asserted: the request's claim must describe this bundle.
        let declared = bundle.implementation.as_ref();
        if declared.map(|i| (i.implementation_id.as_str(), i.implementation_version.as_str()))
            != Some((body.implementation_id.as_str(), body.implementation_version.as_str()))
        {
            return Err(KbError::new(KbErrorCode::VerificationInvalid, "the named implementation is not the one this node declares").into());
        }
        let report = run_bundle_tests(&bundle).map_err(|e| match e {
            TestError::Unreadable(_) => ApiError::Kb(KbError::new(KbErrorCode::VerificationInvalid, "the test cases could not be read")),
            _ => ApiError::Kb(KbError::new(KbErrorCode::VerificationInvalid, e.to_string())),
        })?;
        if !report.all_passed {
            // The run happened and failed; that result is recorded honestly.
            let head = record_event(&state.kb_root, VerificationType::Technical, VEvent::Failed, &subject, Some(suite_of(&report)), None, Some("the node's own tests failed".into()))
                .map_err(|_| ServiceError::ServiceUnavailable)?;
            kb_event_repo::append(&db, "verification_recorded", &format!("{id}@{ver}"), &json!({ "type": "technical", "event": "failed", "seq": head.seq }))?;
            service::sync_catalog(&db, &state.kb_root)?;
            let after = crate::kb::eligibility::snapshot_for_node(&db, &state.kb_root, &id);
            emit_eligibility_changes(&state, &before, &after);
            return Err(KbError::new(KbErrorCode::VerificationInvalid, "the node's tests did not all pass")
                .with_details(json!({ "cases": report.cases.iter().filter(|c| !c.passed).collect::<Vec<_>>() }))
                .into());
        }
        suite = Some(suite_of(&report));
    }
    if vtype == VerificationType::Dataset && event == VEvent::Passed && body.scope.is_none() {
        return Err(KbError::new(KbErrorCode::BundleInvalid, "a dataset validation must state its scope").into());
    }
    let head = record_event(&state.kb_root, vtype, event, &subject, suite, body.scope.clone(), body.reason.clone())
        .map_err(|_| ServiceError::ServiceUnavailable)?;
    kb_event_repo::append(&db, "verification_recorded", &format!("{id}@{ver}"), &json!({ "type": vtype.as_str(), "event": event.as_str(), "seq": head.seq }))?;
    service::sync_catalog(&db, &state.kb_root)?;
    let after = crate::kb::eligibility::snapshot_for_node(&db, &state.kb_root, &id);
    emit_eligibility_changes(&state, &before, &after);
    let row: (Option<String>, Option<String>, String) = db.query_row(
        "SELECT maturity, verification, dataset_validation FROM kb_catalog_entry WHERE path = ?1",
        [&path],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    Ok(Json(json!({
        "seq": head.seq,
        "chain_head": head.hash,
        "maturity": row.0,
        "technical_verification": row.1,
        "dataset_validation": row.2,
    })))
}

/// `GET /kb/nodes/{id}/{ver}/verification` — the node's verification history.
pub async fn get_verification(
    State(state): State<AppState>,
    Path((kind, id, ver)): Path<(String, String, String)>,
) -> Result<Json<Value>, ApiError> {
    if kind_or_404(&kind)? != Kind::Algonode {
        return Err(ServiceError::NotFound.into());
    }
    let db = state.db.lock().unwrap();
    path_of(&db, "algonode", &id, &ver, "published")?.ok_or(ServiceError::NotFound)?;
    let chain = crate::kb::evidence::chain::read_chain(&crate::kb::evidence::verification::chain_dir(&state.kb_root, &id))
        .map_err(|_| ServiceError::ServiceUnavailable)?;
    Ok(Json(json!({ "records": chain.records, "intact": chain.broken.is_none() })))
}

/// Emits `eligibility_changed` for each executable pipe whose derived eligibility
/// moved between the two snapshots (suspension and restoration are visible, FR-052).
fn emit_eligibility_changes(
    state: &AppState,
    before: &[((String, String), crate::kb::eligibility::Eligibility)],
    after: &[((String, String), crate::kb::eligibility::Eligibility)],
) {
    for ((id, version), e) in crate::kb::eligibility::changes(before, after) {
        let _ = state.event_tx.send(Event::new(
            Uuid::nil(),
            EventPayload::EligibilityChanged {
                algopipe: crate::events::AlgopipeRef { id, version },
                eligible: e.eligible,
                reasons: e.reasons.iter().map(|r| r.code.to_string()).collect(),
            },
        ));
    }
}

#[derive(Deserialize)]
pub struct AmendBody {
    /// `correction` or `withdrawal`
    pub kind: String,
    pub evidence_id: String,
    pub reason: String,
    pub resulting_status: String,
}

/// `POST /kb/{kind}/{id}/{ver}/amendments` — appends to the version's history.
/// Bundle files and Run rows are untouched (FR-051, SC-016).
pub async fn post_amendment(
    State(state): State<AppState>,
    Path((kind, id, ver)): Path<(String, String, String)>,
    Json(body): Json<AmendBody>,
) -> Result<Json<Value>, ApiError> {
    use crate::kb::evidence::amendment::{amend, AmendError, AmendmentRequest};
    let kind = kind_or_404(&kind)?;
    let db = state.db.lock().unwrap();
    let before = crate::kb::eligibility::snapshot_for_node(&db, &state.kb_root, &id);
    let out = amend(
        &db,
        &state.kb_root,
        kind.as_str(),
        &id,
        &ver,
        AmendmentRequest { kind: body.kind, evidence_id: body.evidence_id, reason: body.reason, resulting_status: body.resulting_status },
    )
    .map_err(|e| match e {
        AmendError::UnknownKind | AmendError::Incomplete | AmendError::UnknownEvidence => {
            ApiError::Kb(KbError::new(KbErrorCode::BundleInvalid, e.to_string()))
        }
        AmendError::Service(s) => map_service_error(s, None),
        AmendError::Chain(_) | AmendError::Db(_) => ApiError::Service(ServiceError::ServiceUnavailable),
    })?;
    let after = crate::kb::eligibility::snapshot_for_node(&db, &state.kb_root, &id);
    emit_eligibility_changes(&state, &before, &after);
    Ok(Json(json!({ "amendment_id": out.amendment_id, "affected_versions": out.affected_versions, "seq": out.seq, "chain_head": out.chain_head })))
}

pub async fn get_amendments(
    State(state): State<AppState>,
    Path((kind, id, ver)): Path<(String, String, String)>,
) -> Result<Json<Value>, ApiError> {
    let kind = kind_or_404(&kind)?;
    let db = state.db.lock().unwrap();
    path_of(&db, kind.as_str(), &id, &ver, "published")?.ok_or(ServiceError::NotFound)?;
    let own = crate::kb::evidence::amendment::list(&state.kb_root, &id, &ver).map_err(|_| ServiceError::ServiceUnavailable)?;
    Ok(Json(json!({ "amendments": own })))
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/kb/entries", get(get_entries))
        .route("/kb/findings", get(get_findings))
        .route("/kb/refresh", post(post_refresh))
        .route("/kb/rebuild", post(post_rebuild))
        .route("/kb/export/preview", post(post_export_preview))
        .route("/kb/export", post(post_export))
        .route("/kb/export/:export_id/content", get(get_export_content))
        .route("/kb/import", post(post_import).layer(DefaultBodyLimit::max(80 * 1024 * 1024)))
        .route("/kb/import/:staged_id/confirm", post(post_import_confirm))
        .route("/kb/import/:staged_id/cancel", post(post_import_cancel))
        .route("/kb/:kind/drafts", post(post_draft))
        .route("/kb/:kind/drafts/:id", put(put_draft).delete(delete_draft))
        .route("/kb/:kind/drafts/:id/publish", post(post_publish))
        .route("/kb/:kind/:id/draft", get(get_draft))
        .route("/kb/:kind/:id/:ver", get(get_published))
        .route("/kb/:kind/:id/:ver/deprecate", post(post_deprecate))
        .route("/kb/:kind/:id/:ver/amendments", post(post_amendment).get(get_amendments))
        .route("/kb/:kind/:id/:ver/verification", post(post_verification).get(get_verification))
}
