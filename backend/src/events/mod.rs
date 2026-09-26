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
        // Feature 002 additions (absent on legacy snapshot previews). A consumer
        // MUST discard the event if `(image, revision, target, port)` is no
        // longer current (FR-019).
        #[serde(skip_serializing_if = "Option::is_none")]
        pipe_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        revision: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        verification_state: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        unverified_nodes: Option<Vec<String>>,
    },
    PreviewFailed {
        request_context_id: Uuid,
        image_asset_id: Uuid,
        pipeline_snapshot_id: Uuid,
        target_node_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        pipe_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        revision: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        verification_state: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        unverified_nodes: Option<Vec<String>>,
    },
    ThumbnailReady {
        image_asset_id: Uuid,
        source_content_identity: String,
    },
    // ---- feature 002 (contracts/events.md) — notification only; no event
    // carries paper text, quotes, file contents or patient details. ----
    KbScanProgress {
        scan_id: Uuid,
        scanned: u64,
        total: Option<u64>,
        invalid: u64,
    },
    KbScanComplete {
        scan_id: Uuid,
        indexed: u64,
        invalid: u64,
    },
    KbBundleChangedExternally {
        kind: String,
        id: String,
        revision: String,
    },
    ExtractionProgress {
        extraction_id: Uuid,
        paper_id: Uuid,
        page: u32,
        page_count: u32,
    },
    ExtractionComplete {
        extraction_id: Uuid,
        candidate_count: u32,
        pages_without_text: Vec<u32>,
    },
    ExtractionCancelled {
        extraction_id: Uuid,
    },
    PreviewStale {
        pipe_id: String,
        revision: String,
        nodes: Vec<String>,
    },
    EligibilityChanged {
        algopipe: AlgopipeRef,
        eligible: bool,
        reasons: Vec<String>,
    },
}

#[derive(Debug, Clone, Serialize)]
pub struct AlgopipeRef {
    pub id: String,
    pub version: String,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn json_of(payload: EventPayload) -> serde_json::Value {
        serde_json::to_value(Event::new(Uuid::nil(), payload)).unwrap()
    }

    #[test]
    fn kb_events_use_the_documented_names_and_payloads() {
        let v = json_of(EventPayload::KbScanProgress {
            scan_id: Uuid::nil(),
            scanned: 3,
            total: None,
            invalid: 1,
        });
        assert_eq!(v["type"], "kb_scan_progress");
        assert_eq!(v["payload"]["scanned"], 3);

        let v = json_of(EventPayload::EligibilityChanged {
            algopipe: AlgopipeRef { id: "a.b".into(), version: "1.0.0".into() },
            eligible: false,
            reasons: vec!["verification_withdrawn".into()],
        });
        assert_eq!(v["type"], "eligibility_changed");
        assert_eq!(v["payload"]["algopipe"]["id"], "a.b");

        for (payload, name) in [
            (EventPayload::KbScanComplete { scan_id: Uuid::nil(), indexed: 1, invalid: 0 }, "kb_scan_complete"),
            (EventPayload::KbBundleChangedExternally { kind: "algonode".into(), id: "a.b".into(), revision: "r".into() }, "kb_bundle_changed_externally"),
            (EventPayload::ExtractionCancelled { extraction_id: Uuid::nil() }, "extraction_cancelled"),
            (EventPayload::PreviewStale { pipe_id: "a.b".into(), revision: "r".into(), nodes: vec![] }, "preview_stale"),
        ] {
            assert_eq!(json_of(payload)["type"], name);
        }
    }
}
