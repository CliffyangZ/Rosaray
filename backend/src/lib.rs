//! Rosaray local service: a thin HTTP layer over the QKB (`rosaray-qkb`).
//! It adds transport, authorization scopes and error mapping — no method
//! definitions, validation or selection logic live here.
pub mod api;
pub mod bootstrap;
