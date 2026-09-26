//! Authoritative paper-extraction records (data-model.md): Paper Sources,
//! Extraction Candidates and Review Decisions. Candidates and decisions are the
//! only research-derived rows here; decisions are append-only (the table also
//! rejects UPDATE/DELETE with triggers).

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use uuid::Uuid;

use crate::designer::paper::candidate::{Ambiguity, CandidateState, Category, ExtractionCandidate, Proposed, SourceSpan};

#[derive(Debug, Clone, Serialize)]
pub struct PaperRow {
    pub paper_id: Uuid,
    pub content_id: String,
    #[serde(skip)]
    pub blob_ref: String,
    pub title: Option<String>,
    pub authors: Option<String>,
    pub year: Option<i64>,
    pub doi: Option<String>,
    pub page_count: u32,
    pub pages_without_text: Vec<u32>,
    pub authorization_attested: bool,
    pub authorization_attested_at: Option<String>,
    pub extraction_state: String,
    pub imported_at: String,
}

const PAPER_COLS: &str = "paper_id, content_id, blob_ref, title, authors, year, doi, page_count, pages_without_text_json, authorization_attested, authorization_attested_at, extraction_state, imported_at";

fn paper_from(r: &rusqlite::Row) -> rusqlite::Result<PaperRow> {
    Ok(PaperRow {
        paper_id: Uuid::parse_str(&r.get::<_, String>(0)?).expect("stored UUID is valid"),
        content_id: r.get(1)?,
        blob_ref: r.get(2)?,
        title: r.get(3)?,
        authors: r.get(4)?,
        year: r.get(5)?,
        doi: r.get(6)?,
        page_count: r.get::<_, i64>(7)? as u32,
        pages_without_text: serde_json::from_str(&r.get::<_, String>(8)?).unwrap_or_default(),
        authorization_attested: r.get::<_, i64>(9)? == 1,
        authorization_attested_at: r.get(10)?,
        extraction_state: r.get(11)?,
        imported_at: r.get(12)?,
    })
}

pub fn insert_paper(conn: &Connection, p: &PaperRow) -> rusqlite::Result<()> {
    conn.execute(
        &format!("INSERT INTO paper_source ({PAPER_COLS}) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)"),
        params![
            p.paper_id.to_string(),
            p.content_id,
            p.blob_ref,
            p.title,
            p.authors,
            p.year,
            p.doi,
            p.page_count,
            serde_json::to_string(&p.pages_without_text).unwrap(),
            p.authorization_attested as i64,
            p.authorization_attested_at,
            p.extraction_state,
            p.imported_at,
        ],
    )?;
    Ok(())
}

pub fn paper_by_id(conn: &Connection, id: Uuid) -> rusqlite::Result<Option<PaperRow>> {
    conn.query_row(&format!("SELECT {PAPER_COLS} FROM paper_source WHERE paper_id = ?1"), [id.to_string()], paper_from).optional()
}

pub fn paper_by_content(conn: &Connection, content_id: &str) -> rusqlite::Result<Option<PaperRow>> {
    conn.query_row(&format!("SELECT {PAPER_COLS} FROM paper_source WHERE content_id = ?1 LIMIT 1"), [content_id], paper_from).optional()
}

pub fn list_papers(conn: &Connection) -> rusqlite::Result<Vec<PaperRow>> {
    let mut stmt = conn.prepare(&format!("SELECT {PAPER_COLS} FROM paper_source ORDER BY imported_at, rowid"))?;
    let rows = stmt.query_map([], paper_from)?;
    rows.collect()
}

pub fn set_extraction_state(conn: &Connection, id: Uuid, state: &str) -> rusqlite::Result<()> {
    conn.execute("UPDATE paper_source SET extraction_state = ?1 WHERE paper_id = ?2", params![state, id.to_string()])?;
    Ok(())
}

pub fn set_pages(conn: &Connection, id: Uuid, page_count: u32, pages_without_text: &[u32]) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE paper_source SET page_count = ?1, pages_without_text_json = ?2 WHERE paper_id = ?3",
        params![page_count, serde_json::to_string(pages_without_text).unwrap(), id.to_string()],
    )?;
    Ok(())
}

const CAND_COLS: &str = "candidate_id, paper_id, run_id, category, sources_json, proposed_json, ambiguities_json, state, origin";

fn candidate_from(r: &rusqlite::Row) -> rusqlite::Result<ExtractionCandidate> {
    let category: String = r.get(3)?;
    let state: String = r.get(7)?;
    Ok(ExtractionCandidate {
        candidate_id: Uuid::parse_str(&r.get::<_, String>(0)?).expect("stored UUID is valid"),
        paper_id: Uuid::parse_str(&r.get::<_, String>(1)?).expect("stored UUID is valid"),
        run_id: Uuid::parse_str(&r.get::<_, String>(2)?).expect("stored UUID is valid"),
        category: serde_json::from_value(serde_json::Value::String(category)).unwrap_or(Category::RejectedContent),
        sources: serde_json::from_str::<Vec<SourceSpan>>(&r.get::<_, String>(4)?).unwrap_or_default(),
        proposed: serde_json::from_str::<Proposed>(&r.get::<_, String>(5)?).unwrap_or_default(),
        ambiguities: serde_json::from_str::<Vec<Ambiguity>>(&r.get::<_, String>(6)?).unwrap_or_default(),
        state: CandidateState::parse(&state).unwrap_or(CandidateState::Proposed),
        origin: r.get(8)?,
    })
}

/// Inserts every candidate of one extraction run in a single transaction.
pub fn insert_candidates(conn: &Connection, candidates: &[ExtractionCandidate]) -> rusqlite::Result<()> {
    let tx = conn.unchecked_transaction()?;
    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    for c in candidates {
        tx.execute(
            &format!("INSERT INTO extraction_candidate ({CAND_COLS}, created_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)"),
            params![
                c.candidate_id.to_string(),
                c.paper_id.to_string(),
                c.run_id.to_string(),
                c.category.as_str(),
                serde_json::to_string(&c.sources).unwrap(),
                serde_json::to_string(&c.proposed).unwrap(),
                serde_json::to_string(&c.ambiguities).unwrap(),
                c.state.as_str(),
                c.origin,
                now,
            ],
        )?;
    }
    tx.commit()
}

pub fn candidates_for_paper(
    conn: &Connection,
    paper_id: Uuid,
    state: Option<&str>,
    category: Option<&str>,
) -> rusqlite::Result<Vec<ExtractionCandidate>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {CAND_COLS} FROM extraction_candidate
         WHERE paper_id = ?1 AND (?2 IS NULL OR state = ?2) AND (?3 IS NULL OR category = ?3)
         ORDER BY created_at, rowid"
    ))?;
    let rows = stmt.query_map(params![paper_id.to_string(), state, category], candidate_from)?;
    rows.collect()
}

pub fn candidate_by_id(conn: &Connection, id: Uuid) -> rusqlite::Result<Option<ExtractionCandidate>> {
    conn.query_row(&format!("SELECT {CAND_COLS} FROM extraction_candidate WHERE candidate_id = ?1"), [id.to_string()], candidate_from).optional()
}

pub fn update_candidate(conn: &Connection, c: &ExtractionCandidate) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE extraction_candidate SET state = ?1, category = ?2, proposed_json = ?3, sources_json = ?4 WHERE candidate_id = ?5",
        params![
            c.state.as_str(),
            c.category.as_str(),
            serde_json::to_string(&c.proposed).unwrap(),
            serde_json::to_string(&c.sources).unwrap(),
            c.candidate_id.to_string()
        ],
    )?;
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
pub struct DecisionRow {
    pub decision_id: Uuid,
    pub candidate_id: Uuid,
    pub action: String,
    pub target_ref: Option<String>,
    pub edited_snapshot: Option<serde_json::Value>,
    pub rationale: Option<String>,
    pub created_at: String,
}

pub fn insert_decision(conn: &Connection, d: &DecisionRow) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO review_decision (decision_id, candidate_id, action, target_ref, edited_snapshot_json, rationale, created_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7)",
        params![
            d.decision_id.to_string(),
            d.candidate_id.to_string(),
            d.action,
            d.target_ref,
            d.edited_snapshot.as_ref().map(|v| v.to_string()),
            d.rationale,
            d.created_at
        ],
    )?;
    Ok(())
}

pub fn decisions_for_candidate(conn: &Connection, candidate_id: Uuid) -> rusqlite::Result<Vec<DecisionRow>> {
    let mut stmt = conn.prepare(
        "SELECT decision_id, candidate_id, action, target_ref, edited_snapshot_json, rationale, created_at
         FROM review_decision WHERE candidate_id = ?1 ORDER BY created_at, rowid",
    )?;
    let rows = stmt.query_map([candidate_id.to_string()], |r| {
        Ok(DecisionRow {
            decision_id: Uuid::parse_str(&r.get::<_, String>(0)?).expect("stored UUID is valid"),
            candidate_id: Uuid::parse_str(&r.get::<_, String>(1)?).expect("stored UUID is valid"),
            action: r.get(2)?,
            target_ref: r.get(3)?,
            edited_snapshot: r.get::<_, Option<String>>(4)?.and_then(|s| serde_json::from_str(&s).ok()),
            rationale: r.get(5)?,
            created_at: r.get(6)?,
        })
    })?;
    rows.collect()
}
