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
}

#[derive(Debug, Clone, Serialize)]
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
}

impl Default for SessionState {
    fn default() -> Self {
        Self {
            state: DictationState::Idle,
            session_id: None,
            last_transcript: None,
            last_error: None,
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
