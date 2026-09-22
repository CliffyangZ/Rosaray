use axum::extract::State;
use axum::Json;
use serde::Serialize;

use crate::api::AppState;

/// `LocalServiceSession.state` (data-model.md) — in-memory only, never
/// persisted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    Connecting,
    Ready,
    Unavailable,
    AccessDenied,
}

#[derive(Serialize)]
pub struct SessionResponse {
    state: SessionState,
}

/// `GET /session` (contracts/local-service-api.md §Session, FR-042).
pub async fn get_session(State(state): State<AppState>) -> Json<SessionResponse> {
    let session_state = *state.session_state.read().unwrap();
    Json(SessionResponse {
        state: session_state,
    })
}
