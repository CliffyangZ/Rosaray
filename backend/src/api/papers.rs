//! `/papers` and `/candidates` routes (contracts/kb-api.md §5): offline paper
//! import, extraction, candidate listing and review. No handler here reaches the
//! network, publishes a bundle or runs anything.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use axum::extract::{DefaultBodyLimit, Multipart, Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::api::errors::{KbError, KbErrorCode};
use crate::api::kb::{map_service_error, ApiError};
use crate::api::AppState;
use crate::data_repository::sqlite::paper_repo;
use crate::designer::paper::extract::extract_candidates;
use crate::designer::paper::import::{import_paper, PaperImportError, PaperMetadata};
use crate::designer::paper::pdf;
use crate::designer::paper::review::{decide, DecisionRequest, ReviewError};
use crate::domain::ServiceError;
use crate::events::{Event, EventPayload};

pub async fn post_paper(State(state): State<AppState>, mut multipart: Multipart) -> Result<(StatusCode, Json<Value>), ApiError> {
    let bad = || KbError::new(KbErrorCode::BundleInvalid, "bad upload");
    let (mut file, mut attested) = (None, false);
    let mut meta = PaperMetadata::default();
    while let Some(field) = multipart.next_field().await.map_err(|_| bad())? {
        let name = field.name().unwrap_or("").to_string();
        match name.as_str() {
            "file" => file = Some(field.bytes().await.map_err(|_| bad())?),
            other => {
                let text = field.text().await.map_err(|_| bad())?;
                match other {
                    "authorization_attested" => attested = text.trim() == "true",
                    "title" => meta.title = Some(text).filter(|t| !t.trim().is_empty()),
                    "authors" => meta.authors = Some(text).filter(|t| !t.trim().is_empty()),
                    "doi" => meta.doi = Some(text).filter(|t| !t.trim().is_empty()),
                    "year" => meta.year = text.trim().parse().ok(),
                    _ => {}
                }
            }
        }
    }
    let bytes = file.ok_or_else(bad)?;
    let (row, existed) = {
        let db = state.db.lock().unwrap();
        import_paper(&db, &state.blob_store, &state.master_key, &bytes, attested, meta).map_err(|e| match e {
            PaperImportError::AuthorizationRequired => ApiError::Kb(KbError::new(
                KbErrorCode::AuthorizationRequired,
                "confirm you are authorized to use this paper before importing it",
            )),
            PaperImportError::NotReadable => ApiError::Kb(KbError::new(KbErrorCode::BundleInvalid, "the file is not a readable PDF")),
            PaperImportError::Store => ApiError::Service(ServiceError::ServiceUnavailable),
            PaperImportError::Db(_) => ApiError::Service(ServiceError::ServiceUnavailable),
        })?
    };
    Ok((
        if existed { StatusCode::OK } else { StatusCode::CREATED },
        Json(json!({
            "paper_id": row.paper_id,
            "page_count": row.page_count,
            "pages_without_text": row.pages_without_text,
            "already_imported": existed,
        })),
    ))
}

pub async fn get_papers(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    let db = state.db.lock().unwrap();
    Ok(Json(json!({ "papers": paper_repo::list_papers(&db)? })))
}

pub async fn post_extract(State(state): State<AppState>, Path(paper_id): Path<Uuid>) -> Result<(StatusCode, Json<Value>), ApiError> {
    let paper = {
        let db = state.db.lock().unwrap();
        let paper = paper_repo::paper_by_id(&db, paper_id)?.ok_or(ServiceError::NotFound)?;
        if paper.extraction_state == "extracting" {
            return Err(ServiceError::Conflict.into());
        }
        paper_repo::set_extraction_state(&db, paper_id, "extracting")?;
        paper
    };
    let extraction_id = Uuid::new_v4();
    let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
    state.extractions.lock().unwrap().insert(extraction_id, cancel.clone());

    let worker_state = state.clone();
    tokio::task::spawn_blocking(move || {
        let s = worker_state;
        let finish = |state_name: &str| {
            let db = s.db.lock().unwrap();
            let _ = paper_repo::set_extraction_state(&db, paper_id, state_name);
        };
        let emit = |p: EventPayload| {
            let _ = s.event_tx.send(Event::new(extraction_id, p));
        };
        let result = (|| -> Option<(pdf::PdfText, Option<Vec<crate::designer::paper::candidate::ExtractionCandidate>>)> {
            let bytes = s.blob_store.read(&s.master_key, &paper.blob_ref).ok()?;
            let text = pdf::extract(&bytes).ok()?;
            let delay = s.extraction_delay_ms.load(Ordering::SeqCst);
            let cands = extract_candidates(&text, paper_id, extraction_id, &|| cancel.load(Ordering::SeqCst), &mut |page, count| {
                emit(EventPayload::ExtractionProgress { extraction_id, paper_id, page, page_count: count });
                if delay > 0 {
                    std::thread::sleep(std::time::Duration::from_millis(delay));
                }
            });
            Some((text, cands))
        })();
        s.extractions.lock().unwrap().remove(&extraction_id);
        match result {
            // Cancelled: nothing was committed, so nothing else changes.
            Some((_, None)) => {
                finish("cancelled");
                emit(EventPayload::ExtractionCancelled { extraction_id });
            }
            Some((text, Some(cands))) => {
                let stored = {
                    let db = s.db.lock().unwrap();
                    paper_repo::set_pages(&db, paper_id, text.page_count, &text.pages_without_text).ok();
                    paper_repo::insert_candidates(&db, &cands)
                };
                if stored.is_ok() {
                    finish("extracted");
                    emit(EventPayload::ExtractionComplete {
                        extraction_id,
                        candidate_count: cands.len() as u32,
                        pages_without_text: text.pages_without_text.clone(),
                    });
                } else {
                    finish("failed");
                }
            }
            None => finish("failed"),
        }
    });
    Ok((StatusCode::ACCEPTED, Json(json!({ "extraction_id": extraction_id }))))
}

/// Cancels an in-flight extraction. Cancellation is racy against completion, so
/// an unknown or already-finished id is treated as already resolved.
pub async fn delete_extract(State(state): State<AppState>, Path((_paper_id, extraction_id)): Path<(Uuid, Uuid)>) -> StatusCode {
    if let Some(flag) = state.extractions.lock().unwrap().get(&extraction_id) {
        flag.store(true, Ordering::SeqCst);
    }
    StatusCode::NO_CONTENT
}

#[derive(Deserialize)]
pub struct CandidateQuery {
    pub state: Option<String>,
    pub category: Option<String>,
}

pub async fn get_candidates(
    State(state): State<AppState>,
    Path(paper_id): Path<Uuid>,
    Query(q): Query<CandidateQuery>,
) -> Result<Json<Value>, ApiError> {
    let db = state.db.lock().unwrap();
    let paper = paper_repo::paper_by_id(&db, paper_id)?.ok_or(ServiceError::NotFound)?;
    let candidates = paper_repo::candidates_for_paper(&db, paper_id, q.state.as_deref(), q.category.as_deref())?;
    Ok(Json(json!({ "paper": paper, "candidates": candidates })))
}

#[derive(Deserialize)]
pub struct DecisionBody {
    pub action: String,
    #[serde(default)]
    pub edited: Option<Value>,
    #[serde(default)]
    pub map_to: Option<MapTo>,
    #[serde(default)]
    pub rationale: Option<String>,
    #[serde(default)]
    pub draft_id: Option<String>,
}

#[derive(Deserialize)]
pub struct MapTo {
    pub id: String,
    pub version: String,
}

fn map_review_error(e: ReviewError) -> ApiError {
    match e {
        ReviewError::NotFound => ServiceError::NotFound.into(),
        ReviewError::UnknownAction => KbError::new(KbErrorCode::BundleInvalid, "unknown action").into(),
        ReviewError::ClinicalClaimNotExecutable => KbError::new(
            KbErrorCode::ClinicalClaimNotExecutable,
            "a clinical statement can only be rejected or marked non-executable",
        )
        .into(),
        ReviewError::EditInvalid(msg) => KbError::new(KbErrorCode::BundleInvalid, "the edit is not valid").with_details(json!({ "reason": msg })).into(),
        ReviewError::MapTargetNotFound => ServiceError::NotFound.into(),
        ReviewError::MapIncompatible(findings) => KbError::new(KbErrorCode::BundleInvalid, "the candidate is not compatible with that node")
            .with_details(json!({ "findings": findings }))
            .into(),
        ReviewError::Draft(e) => map_service_error(e, None),
        ReviewError::Db(_) | ReviewError::Io(_) => ServiceError::ServiceUnavailable.into(),
    }
}

pub async fn post_decision(
    State(state): State<AppState>,
    Path(candidate_id): Path<Uuid>,
    Json(body): Json<DecisionBody>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let db = state.db.lock().unwrap();
    let out = decide(
        &db,
        &state.kb_root,
        candidate_id,
        DecisionRequest {
            action: body.action,
            edited: body.edited,
            map_to: body.map_to.map(|m| (m.id, m.version)),
            rationale: body.rationale,
            draft_id: body.draft_id,
        },
    )
    .map_err(map_review_error)?;
    let status = if out.draft.as_ref().is_some_and(|d| d.created) { StatusCode::CREATED } else { StatusCode::OK };
    Ok((
        status,
        Json(json!({
            "decision": out.decision,
            "candidate": out.candidate,
            "draft": out.draft.map(|d| json!({ "id": d.id, "revision": d.revision })),
            "warnings": out.warnings,
        })),
    ))
}

pub async fn get_decisions(State(state): State<AppState>, Path(candidate_id): Path<Uuid>) -> Result<Json<Value>, ApiError> {
    let db = state.db.lock().unwrap();
    paper_repo::candidate_by_id(&db, candidate_id)?.ok_or(ServiceError::NotFound)?;
    Ok(Json(json!({ "decisions": paper_repo::decisions_for_candidate(&db, candidate_id)? })))
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/papers", post(post_paper).get(get_papers).layer(DefaultBodyLimit::max(200 * 1024 * 1024)))
        .route("/papers/:paper_id/extract", post(post_extract))
        .route("/papers/:paper_id/extract/:extraction_id", delete(delete_extract))
        .route("/papers/:paper_id/candidates", get(get_candidates))
        .route("/candidates/:candidate_id/decisions", post(post_decision).get(get_decisions))
}
