//! Offline paper → method-candidate pipeline (US5): PDF text, a deterministic
//! rules-based proposer, and the human review workflow. Nothing here contacts
//! the network, publishes, or executes anything (Constitution I, FR-021).
pub mod candidate;
pub mod extract;
pub mod import;
pub mod pdf;
pub mod review;
pub mod rules;
