//! Event Bus (contracts/event-bus.md): a non-persistent WebSocket
//! notification stream, one connection per browser tab. Carries only async
//! status/progress/lifecycle events — never request-response data (FR-050).

use axum::extract::ws::{Message, WebSocket};
use axum::extract::{Query, State, WebSocketUpgrade};
use axum::response::IntoResponse;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::api::session::SessionState;
use crate::api::AppState;

pub const EVENT_CHANNEL_CAPACITY: usize = 256;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum EventPayload {
    SessionStateChanged {
        state: SessionState,
    },
    RunProgress {
        run_id: Uuid,
        stage: String,
        node_id: Option<String>,
    },
    RunCompleted {
        run_id: Uuid,
    },
    RunFailed {
        run_id: Uuid,
        failed_stage: String,
        error_summary: String,
    },
    PreviewReady {
        request_context_id: Uuid,
        image_asset_id: Uuid,
        pipeline_snapshot_id: Uuid,
        target_node_id: String,
    },
    PreviewFailed {
        request_context_id: Uuid,
        image_asset_id: Uuid,
        pipeline_snapshot_id: Uuid,
        target_node_id: String,
    },
    ThumbnailReady {
        image_asset_id: Uuid,
        source_content_identity: String,
    },
}

#[derive(Debug, Clone, Serialize)]
pub struct Event {
    pub context_id: Uuid,
    pub emitted_at: String,
    #[serde(flatten)]
    pub payload: EventPayload,
}

impl Event {
    pub fn new(context_id: Uuid, payload: EventPayload) -> Self {
        Self {
            context_id,
            emitted_at: chrono::Utc::now().to_rfc3339(),
            payload,
        }
    }
}

#[derive(Deserialize)]
pub struct EventsQuery {
    pub session: String,
}

/// `GET /events?session=<session_token>` — authenticates via the same
/// session token as the typed API, then streams every subsequently
/// broadcast `Event` plus one immediate `session_state_changed` on connect.
pub async fn events_ws(
    State(state): State<AppState>,
    Query(query): Query<EventsQuery>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    if query.session != state.session_token {
        return crate::domain::ServiceError::AccessDenied.into_response();
    }
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

async fn handle_socket(mut socket: WebSocket, state: AppState) {
    let mut rx = state.event_tx.subscribe();

    let initial_state = *state.session_state.read().unwrap();
    let hello = Event::new(
        Uuid::nil(),
        EventPayload::SessionStateChanged {
            state: initial_state,
        },
    );
    if send_event(&mut socket, &hello).await.is_err() {
        return;
    }

    loop {
        tokio::select! {
            incoming = rx.recv() => {
                match incoming {
                    Ok(event) => {
                        if send_event(&mut socket, &event).await.is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(_)) => continue,
                    Some(Err(_)) => break,
                }
            }
        }
    }
}

async fn send_event(socket: &mut WebSocket, event: &Event) -> Result<(), axum::Error> {
    let text = serde_json::to_string(event).expect("Event always serializes");
    socket.send(Message::Text(text)).await
}

pub fn new_channel() -> (broadcast::Sender<Event>, broadcast::Receiver<Event>) {
    broadcast::channel(EVENT_CHANNEL_CAPACITY)
}
