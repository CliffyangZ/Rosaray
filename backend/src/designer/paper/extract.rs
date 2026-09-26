//! One extraction pass: per-page progress, cooperative cancellation, then the
//! proposer. Cancelling commits nothing — this function returns the candidates
//! for the caller to persist in one transaction, or `None` if cancelled first,
//! so a cancelled pass can never leave partial results (US5-6).

use uuid::Uuid;

use super::candidate::{CandidateProposer, CandidateState, ExtractionCandidate};
use super::pdf::PdfText;
use super::rules::RulesProposer;

pub fn extract_candidates(
    text: &PdfText,
    paper_id: Uuid,
    run_id: Uuid,
    cancelled: &dyn Fn() -> bool,
    progress: &mut dyn FnMut(u32, u32),
) -> Option<Vec<ExtractionCandidate>> {
    for p in &text.pages {
        if cancelled() {
            return None;
        }
        progress(p.page, text.page_count);
    }
    if cancelled() {
        return None;
    }
    let proposed = RulesProposer.propose(&text.pages);
    if cancelled() {
        return None;
    }
    Some(
        proposed
            .into_iter()
            .map(|c| ExtractionCandidate {
                candidate_id: Uuid::new_v4(),
                paper_id,
                run_id,
                category: c.category,
                sources: c.sources,
                proposed: c.proposed,
                ambiguities: c.ambiguities,
                state: CandidateState::Proposed,
                origin: "paper_derived".to_string(),
            })
            .collect(),
    )
}
