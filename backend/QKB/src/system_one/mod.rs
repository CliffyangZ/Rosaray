//! System One protocol `system-one/1` (constitution V, FR-007…FR-012).
//!
//! A query returns a candidate set of currently executable, published
//! AlgoPipe versions; a selection report is validated against *that* stored
//! set and against current executability, then recorded idempotently.
//! Nothing here creates, edits, publishes or executes a method.

pub mod protocol;
pub mod query;
pub mod selection;

pub use protocol::{SoError, PROTOCOL_VERSION};
