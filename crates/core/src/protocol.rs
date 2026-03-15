use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BoundSessionState {
    pub session_id: Option<i64>,
    pub session_title: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BoundSessionEntry {
    pub session_id: i64,
    pub session_title: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RuntimeStatus {
    pub connected: bool,
    pub latency_ms: Option<u64>,
    pub runtime_mode: String,
    pub platform: String,
    pub client_id: String,
    pub client_name: String,
    pub bound_session: BoundSessionState,
    pub bound_sessions: Vec<BoundSessionEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ControlMessage {
    Ping(PingMessage),
    Pong(PongMessage),
    Status(StatusMessage),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PingMessage {
    pub request_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PongMessage {
    pub request_id: String,
    pub runtime: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StatusMessage {
    pub component: String,
    pub state: String,
    pub detail: String,
}

impl StatusMessage {
    pub fn new(
        component: impl Into<String>,
        state: impl Into<String>,
        detail: impl Into<String>,
    ) -> Self {
        Self { component: component.into(), state: state.into(), detail: detail.into() }
    }
}
