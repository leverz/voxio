use std::sync::Mutex;

use serde::Serialize;
use uuid::Uuid;

use crate::config::Settings;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppStateSnapshot {
    pub state: DictationState,
    pub session_id: Option<String>,
    pub last_transcript: Option<String>,
    pub last_error: Option<String>,
    pub last_provider: Option<String>,
    pub last_latency_ms: Option<u128>,
    pub requested_backend: Option<String>,
    pub actual_backend: Option<String>,
    pub detected_language: Option<String>,
    pub fallback_used: bool,
    pub fallback_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DictationState {
    Idle,
    Listening,
    Processing,
    Error,
}

#[derive(Debug)]
pub struct SessionState {
    pub state: DictationState,
    pub session_id: Option<Uuid>,
    pub last_transcript: Option<String>,
    pub last_error: Option<String>,
    pub last_provider: Option<String>,
    pub last_latency_ms: Option<u128>,
    pub requested_backend: Option<String>,
    pub actual_backend: Option<String>,
    pub detected_language: Option<String>,
    pub fallback_used: bool,
    pub fallback_reason: Option<String>,
}

impl Default for SessionState {
    fn default() -> Self {
        Self {
            state: DictationState::Idle,
            session_id: None,
            last_transcript: None,
            last_error: None,
            last_provider: None,
            last_latency_ms: None,
            requested_backend: None,
            actual_backend: None,
            detected_language: None,
            fallback_used: false,
            fallback_reason: None,
        }
    }
}

impl SessionState {
    pub fn snapshot(&self) -> AppStateSnapshot {
        AppStateSnapshot {
            state: self.state.clone(),
            session_id: self.session_id.map(|value| value.to_string()),
            last_transcript: self.last_transcript.clone(),
            last_error: self.last_error.clone(),
            last_provider: self.last_provider.clone(),
            last_latency_ms: self.last_latency_ms,
            requested_backend: self.requested_backend.clone(),
            actual_backend: self.actual_backend.clone(),
            detected_language: self.detected_language.clone(),
            fallback_used: self.fallback_used,
            fallback_reason: self.fallback_reason.clone(),
        }
    }
}

pub struct AppState {
    pub session: Mutex<SessionState>,
    pub settings: Mutex<Settings>,
}

impl AppState {
    pub fn new(settings: Settings) -> Self {
        Self {
            session: Mutex::new(SessionState::default()),
            settings: Mutex::new(settings),
        }
    }
}
