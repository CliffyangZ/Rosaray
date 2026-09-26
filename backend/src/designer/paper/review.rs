//! Candidate review (US5, FR-024/025/027): accept, edit, reject, mark
//! non-executable, map to an existing node, keep as a specification-only node,
//! or defer. Every decision is an append-only Review Decision; an edit stores the
//! *after* snapshot so history is replayable. Accepting creates or updates a
//! **draft** AlgoNode that is specification-only, marked `paper_derived`, and
//! carries one method-source evidence record per source. Nothing is published.

use std::path::Path;

use rusqlite::Connection;
use serde_json::{json, Value};
use uuid::Uuid;

use super::candidate::{CandidateState, Category, ExtractionCandidate, Proposed};
use crate::data_repository::sqlite::paper_repo::{self, DecisionRow, PaperRow};
use crate::data_repository::sqlite::kb_event_repo;
use crate::designer::validate::finding::{BundleRef, Finding, Severity, Subject, SubjectType};
use crate::kb::bundle::draft::{revision_of, save_draft, DraftError};
use crate::kb::bundle::model::to_yaml;
use crate::kb::bundle::read::read_bundle;
use crate::kb::bundle::service::{sync_catalog, KbServiceError};
use crate::kb::bundle::write::write_bundle_atomic;
use crate::kb::catalog::query::path_of;
use crate::kb::catalog::repo::{draft_dir, record_draft_session};

pub const ACTIONS: &[&str] = &["accept", "edit", "reject", "mark_non_executable", "map_to_node", "keep_specification_only", "defer"];

#[derive(Debug, Default, Clone)]
pub struct DecisionRequest {
    pub action: String,
    /// For `edit`: `{ category?, proposed? }` replacing those parts of the candidate.
    pub edited: Option<Value>,
    pub map_to: Option<(String, String)>,
    pub rationale: Option<String>,
    /// For accept / keep_specification_only: the draft ID to use (default derived).
    pub draft_id: Option<String>,
}

#[derive(Debug)]
pub enum ReviewError {
    NotFound,
    UnknownAction,
    /// A clinical-claim candidate may only be rejected or marked non-executable (FR-033).
    ClinicalClaimNotExecutable,
    EditInvalid(String),
    MapTargetNotFound,
    MapIncompatible(Vec<Finding>),
    Draft(KbServiceError),
    Db(rusqlite::Error),
    Io(std::io::Error),
}

impl From<rusqlite::Error> for ReviewError {
    fn from(e: rusqlite::Error) -> Self {
        ReviewError::Db(e)
    }
}

impl From<std::io::Error> for ReviewError {
    fn from(e: std::io::Error) -> Self {
        ReviewError::Io(e)
    }
}

#[derive(Debug)]
pub struct DraftRef {
    pub id: String,
    pub revision: String,
    pub created: bool,
}

#[derive(Debug)]
pub struct DecisionOutcome {
    pub decision: DecisionRow,
    pub candidate: ExtractionCandidate,
    pub draft: Option<DraftRef>,
    /// Non-blocking findings from a mapping check.
    pub warnings: Vec<Finding>,
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

pub fn decide(
    conn: &Connection,
    root: &Path,
    candidate_id: Uuid,
    req: DecisionRequest,
) -> Result<DecisionOutcome, ReviewError> {
    if !ACTIONS.contains(&req.action.as_str()) {
        return Err(ReviewError::UnknownAction);
    }
    let mut cand = paper_repo::candidate_by_id(conn, candidate_id)?.ok_or(ReviewError::NotFound)?;
    let paper = paper_repo::paper_by_id(conn, cand.paper_id)?.ok_or(ReviewError::NotFound)?;

    // FR-033: an unsafe clinical claim is never turned into a method.
    let executable_action = matches!(req.action.as_str(), "accept" | "keep_specification_only" | "map_to_node");
    if cand.category == Category::ClinicalClaim && executable_action {
        return Err(ReviewError::ClinicalClaimNotExecutable);
    }

    let mut edited_snapshot = None;
    let mut target_ref = None;
    let mut draft = None;
    let mut warnings = Vec::new();

    let new_state = match req.action.as_str() {
        "accept" | "keep_specification_only" => CandidateState::Accepted,
        "edit" => CandidateState::Edited,
        "reject" => CandidateState::Rejected,
        "mark_non_executable" => CandidateState::NonExecutable,
        "map_to_node" => CandidateState::Mapped,
        _ => CandidateState::Deferred,
    };

    if req.action == "edit" {
        let edits = req.edited.clone().ok_or_else(|| ReviewError::EditInvalid("send `edited`".into()))?;
        if let Some(c) = edits.get("category") {
            let category: Category = serde_json::from_value(c.clone()).map_err(|_| ReviewError::EditInvalid("unknown category".into()))?;
            if cand.category == Category::ClinicalClaim && category != Category::ClinicalClaim {
                return Err(ReviewError::ClinicalClaimNotExecutable);
            }
            cand.category = category;
        }
        if let Some(p) = edits.get("proposed") {
            cand.proposed = serde_json::from_value::<Proposed>(p.clone()).map_err(|e| ReviewError::EditInvalid(e.to_string()))?;
        }
        edited_snapshot = Some(json!({ "category": cand.category, "proposed": cand.proposed }));
    }

    if req.action == "map_to_node" {
        let (id, version) = req.map_to.clone().ok_or(ReviewError::MapTargetNotFound)?;
        let findings = check_mapping(conn, root, &cand, &id, &version)?;
        if findings.iter().any(|f| f.is_error()) {
            return Err(ReviewError::MapIncompatible(findings));
        }
        warnings = findings;
        target_ref = Some(format!("{id}@{version}"));
    }

    if matches!(req.action.as_str(), "accept" | "keep_specification_only") {
        let previous = paper_repo::decisions_for_candidate(conn, candidate_id)?
            .into_iter()
            .rev()
            .find_map(|d| d.target_ref.filter(|t| t.starts_with("draft:")));
        let (d, snapshot_ref) = write_draft(conn, root, &cand, &paper, req.draft_id.clone(), previous)?;
        target_ref = Some(snapshot_ref);
        draft = Some(d);
    }

    cand.state = new_state;
    let decision = DecisionRow {
        decision_id: Uuid::new_v4(),
        candidate_id,
        action: req.action.clone(),
        target_ref,
        edited_snapshot,
        rationale: req.rationale.clone(),
        created_at: now(),
    };
    let tx = conn.unchecked_transaction()?;
    paper_repo::insert_decision(&tx, &decision)?;
    paper_repo::update_candidate(&tx, &cand)?;
    kb_event_repo::append(
        &tx,
        "candidate_decision",
        &candidate_id.to_string(),
        &json!({ "action": req.action, "state": cand.state.as_str(), "target": decision.target_ref }),
    )?;
    tx.commit()?;
    Ok(DecisionOutcome { decision, candidate: cand, draft, warnings })
}

// ---- mapping check ---------------------------------------------------------------

fn kind_for(name: &str) -> Option<&'static str> {
    match name {
        "image" => Some("image2d"),
        "mask" => Some("mask2d"),
        _ => None,
    }
}

fn norm(s: &str) -> String {
    s.to_ascii_lowercase().replace(['_', '-', ' '], "")
}

/// Checks a candidate against an existing published node's contract. Errors block
/// the mapping; warnings are returned alongside a successful one.
fn check_mapping(conn: &Connection, root: &Path, cand: &ExtractionCandidate, id: &str, version: &str) -> Result<Vec<Finding>, ReviewError> {
    let path = path_of(conn, "algonode", id, version, "published")?.ok_or(ReviewError::MapTargetNotFound)?;
    let (bundle, _) = read_bundle(&root.join(path));
    let contract = bundle.and_then(|b| b.contract).ok_or(ReviewError::MapTargetNotFound)?;
    let bref = BundleRef::new(id, Some(version.to_string()));
    let mut out = Vec::new();

    for (items, ports, dir) in [(&cand.proposed.inputs, &contract.inputs, "input"), (&cand.proposed.outputs, &contract.outputs, "output")] {
        for item in items {
            let Some(want) = kind_for(&item.name) else { continue };
            if !ports.iter().any(|p| p.artifact_kind.as_deref() == Some(want)) {
                out.push(Finding::build(
                    Severity::Error,
                    "port_incompatible.type",
                    &bref,
                    Subject::new(SubjectType::Port, item.name.clone()),
                    format!("The paper's step has a {} \"{}\" but {id} has no {dir} of kind {want}.", dir, item.name),
                    "Map it to a node whose ports match, or keep it as a new specification-only node.",
                ));
            }
        }
    }
    for item in &cand.proposed.parameters {
        let Some(param) = contract.parameters.iter().find(|p| norm(&p.parameter_id) == norm(&item.name)) else {
            out.push(Finding::build(
                Severity::Warning,
                "parameter_unmapped",
                &bref,
                Subject::new(SubjectType::Parameter, item.name.clone()),
                format!("{id} has no parameter matching \"{}\".", item.name),
                "Check whether this value belongs to another node, or keep it as a note.",
            ));
            continue;
        };
        if let (Some(have), Some(want)) = (item.unit.as_deref(), param.unit.as_deref()) {
            if !have.eq_ignore_ascii_case(want) && !have.trim_end_matches('s').eq_ignore_ascii_case(want.trim_end_matches('s')) {
                out.push(Finding::build(
                    Severity::Error,
                    "port_incompatible.unit",
                    &bref,
                    Subject::new(SubjectType::Parameter, item.name.clone()),
                    format!("The paper gives {} in \"{have}\" but {id} expects \"{want}\".", item.name),
                    "Convert the value, or map to a node that uses the paper's unit.",
                ));
            }
        }
    }
    Ok(out)
}

// ---- draft creation -----------------------------------------------------------------

fn slug(category: Category) -> String {
    category.as_str().replace('_', "-")
}

fn label(category: Category) -> String {
    let s = category.as_str().replace('_', " ");
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

fn yaml_of(v: &Value) -> String {
    to_yaml(v).expect("json value serializes as YAML")
}

fn draft_files(cand: &ExtractionCandidate, paper: &PaperRow, id: &str) -> Vec<(String, Vec<u8>)> {
    let ev_id = |i: usize| format!("paper-{}", i + 1);
    let evidence_for = |item: &super::candidate::Item| -> Option<String> {
        let sp = item.source_span.as_ref()?;
        cand.sources.iter().position(|s| s.page == sp.page && s.start == sp.start).map(ev_id)
    };
    let port = |item: &super::candidate::Item| {
        // Only what the paper says: no kind, unit or shape is invented for a port.
        json!({ "port_id": item.name, "name": item.name })
    };
    let params: Vec<Value> = cand
        .proposed
        .parameters
        .iter()
        .map(|p| {
            let mut v = json!({
                "parameter_id": p.name,
                "type": if p.value.as_ref().is_some_and(Value::is_number) { "number" } else { "string" },
                "meaning": p.source_span.as_ref().map(|s| s.quote.clone()),
            });
            if let Some(u) = &p.unit {
                v["unit"] = json!(u);
            }
            if let Some(val) = &p.value {
                v["default"] = val.clone();
                v["source"] = json!("paper");
                if let Some(e) = evidence_for(p) {
                    v["evidence_ref"] = json!(e);
                }
            }
            v
        })
        .collect();

    let mut contract = json!({
        "schema": "quantify-kb/1",
        "category": cand.category.as_str(),
        "purpose": cand.sources.first().map(|s| s.quote.clone()),
        "inputs": cand.proposed.inputs.iter().map(port).collect::<Vec<_>>(),
        "outputs": cand.proposed.outputs.iter().map(port).collect::<Vec<_>>(),
        "parameters": params,
        "prerequisites": [],
    });
    if let Some(f) = &cand.proposed.formula {
        contract["method"] = json!(f.value);
    }

    let mut head = serde_yaml_ng::Mapping::new();
    let mut put = |k: &str, v: serde_yaml_ng::Value| {
        head.insert(serde_yaml_ng::Value::String(k.into()), v);
    };
    let s = |v: &str| serde_yaml_ng::Value::String(v.to_string());
    put("schema", s("quantify-kb/1"));
    put("kind", s("algonode"));
    put("id", s(id));
    put("version", s("0.1.0"));
    put("name", s(&format!("{} (paper-derived)", label(cand.category))));
    put("summary", s("A method step extracted from a paper and reviewed by a researcher. Specification only."));
    put("status", s("draft"));
    put("origin", s("paper_derived"));
    put("research_use_only", serde_yaml_ng::Value::Bool(true));
    put("intended_use", s(""));
    put("limitations", s(""));
    let mut body = String::from("Paper-derived candidate. Research use only — not a clinical statement. This is a specification: it has no implementation and cannot run.\n\n");
    for (i, src) in cand.sources.iter().enumerate() {
        body.push_str(&format!("Source {} (page {}{}):\n\n> {}\n\n", i + 1, src.page, src.section.as_ref().map(|s| format!(", {s}")).unwrap_or_default(), src.quote));
    }
    if !cand.ambiguities.is_empty() {
        body.push_str("Unresolved in the source:\n\n");
        for a in &cand.ambiguities {
            body.push_str(&format!("- {} ({}): {}\n", a.code, a.field, a.note));
        }
    }
    let md = format!("---\n{}---\n{body}", serde_yaml_ng::to_string(&head).expect("mapping serializes"));

    let mut files = vec![
        ("ALGONODE.md".to_string(), md.into_bytes()),
        ("contract.yaml".to_string(), yaml_of(&contract).into_bytes()),
    ];
    for (i, src) in cand.sources.iter().enumerate() {
        let rec = json!({
            "evidence_id": ev_id(i),
            "type": "method_source",
            "applies_to": { "id": id, "version": "0.1.0" },
            "locator": {
                "paper_source_hash": paper.content_id,
                "page": src.page,
                "section": src.section,
                "quote_ref": format!("p{}:{}-{}", src.page, src.start, src.end),
                "quote": src.quote,
            },
            // A paper quote is not ours to redistribute.
            "distributable": false,
            "patient_data_status": "none",
        });
        files.push((format!("references/{}.yaml", ev_id(i)), yaml_of(&rec).into_bytes()));
    }
    files
}

fn write_draft(
    conn: &Connection,
    root: &Path,
    cand: &ExtractionCandidate,
    paper: &PaperRow,
    requested: Option<String>,
    previous: Option<String>,
) -> Result<(DraftRef, String), ReviewError> {
    // Re-deciding updates the draft this candidate already made — but never over a
    // researcher's own edits: the recorded revision must still match.
    if let Some(prev) = previous.as_deref().and_then(|p| p.strip_prefix("draft:")) {
        let (id, rev) = prev.split_once('@').unwrap_or((prev, ""));
        let dir = draft_dir(root, "algonode", id);
        if dir.is_dir() {
            let files = draft_files(cand, paper, id);
            let revision = save_draft(&dir, rev, &files, &[]).map_err(|e| match e {
                DraftError::Conflict(c) => ReviewError::Draft(KbServiceError::DraftConflict(c)),
                DraftError::Write(w) => ReviewError::Draft(w.into()),
                DraftError::Io(e) => ReviewError::Io(e),
            })?;
            record_draft_session(conn, "algonode", id, &revision)?;
            sync_catalog(conn, root)?;
            return Ok((DraftRef { id: id.to_string(), revision: revision.clone(), created: false }, format!("draft:{id}@{revision}")));
        }
    }
    let id = requested.unwrap_or_else(|| format!("paper.{}-{}", slug(cand.category), &cand.candidate_id.to_string()[..8]));
    let dir = draft_dir(root, "algonode", &id);
    if dir.exists() {
        return Err(ReviewError::Draft(KbServiceError::IdentityConflict(format!("a draft with id {id} already exists"))));
    }
    if let Some(finding) = crate::kb::identity::validate_identity(&id, "0.1.0").into_iter().next() {
        return Err(ReviewError::Draft(KbServiceError::Invalid(vec![finding])));
    }
    write_bundle_atomic(&dir, &draft_files(cand, paper, &id)).map_err(|e| ReviewError::Draft(e.into()))?;
    let revision = revision_of(&dir)?;
    record_draft_session(conn, "algonode", &id, &revision)?;
    kb_event_repo::append(conn, "draft_saved", &format!("algonode:{id}"), &json!({ "revision": revision, "origin": "paper_derived" }))?;
    sync_catalog(conn, root)?;
    Ok((DraftRef { id: id.clone(), revision: revision.clone(), created: true }, format!("draft:{id}@{revision}")))
}
