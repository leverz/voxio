use tauri::{AppHandle, Manager, State};
use uuid::Uuid;

use crate::{
    app::{current_snapshot, detect_permissions, emit_state_changed, PermissionStatus},
    config::{ConfigStore, Settings},
    error::{Result, VoxioError},
    modules::injector::build_injector,
    state::{AppState, DictationState},
};

#[tauri::command]
pub fn get_app_state(state: State<'_, AppState>) -> Result<crate::state::AppStateSnapshot> {
    let session = state.session.lock().expect("session mutex poisoned");
    Ok(session.snapshot())
}

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> Result<Settings> {
    let settings = state.settings.lock().expect("settings mutex poisoned");
    Ok(settings.clone())
}

#[tauri::command]
pub fn update_settings(
    state: State<'_, AppState>,
    store: State<'_, ConfigStore>,
    payload: Settings,
) -> Result<Settings> {
    validate_settings(&payload)?;
    store.save(&payload)?;

    let mut settings = state.settings.lock().expect("settings mutex poisoned");
    *settings = payload.clone();

    Ok(payload)
}

#[tauri::command]
pub fn request_permissions() -> Result<PermissionStatus> {
    Ok(detect_permissions())
}

#[tauri::command]
pub fn toggle_dictation(app: AppHandle) -> Result<crate::state::AppStateSnapshot> {
    let snapshot = current_snapshot(&app);

    match snapshot.state {
        crate::state::DictationState::Idle | crate::state::DictationState::Error => {
            start_dictation(app)
        }
        crate::state::DictationState::Listening => stop_dictation(app),
        crate::state::DictationState::Processing => Ok(snapshot),
    }
}

#[tauri::command]
pub fn start_dictation(app: AppHandle) -> Result<crate::state::AppStateSnapshot> {
    let state = app.state::<AppState>();
    let mut session = state.session.lock().expect("session mutex poisoned");

    match session.state {
        DictationState::Idle | DictationState::Error => {
            session.state = DictationState::Listening;
            session.session_id = Some(Uuid::new_v4());
            session.last_error = None;
            let snapshot = session.snapshot();
            drop(session);
            emit_state_changed(&app, snapshot.clone());
            Ok(snapshot)
        }
        DictationState::Listening | DictationState::Processing => Err(VoxioError::Validation(
            "A dictation session is already active.".to_string(),
        )),
    }
}

#[tauri::command]
pub fn stop_dictation(app: AppHandle) -> Result<crate::state::AppStateSnapshot> {
    let state = app.state::<AppState>();
    let mut session = state.session.lock().expect("session mutex poisoned");

    match session.state {
        DictationState::Listening => {
            session.state = DictationState::Processing;
            let processing_snapshot = session.snapshot();
            drop(session);
            emit_state_changed(&app, processing_snapshot);

            let settings = {
                let settings = state.settings.lock().expect("settings mutex poisoned");
                settings.clone()
            };
            let transcript = "Voxio placeholder transcript. Speech recognition is not connected yet.";
            let injector = build_injector(&settings.injection_mode);
            let inject_result = injector.inject(transcript);

            let mut session = state.session.lock().expect("session mutex poisoned");
            session.last_transcript = Some(transcript.into());

            match inject_result {
                Ok(result) if result.applied => {
                    session.state = DictationState::Idle;
                    session.last_error = None;
                }
                Ok(_) => {
                    session.state = DictationState::Error;
                    session.last_error = Some("No text was available to inject.".into());
                }
                Err(error) => {
                    session.state = DictationState::Error;
                    session.last_error = Some(error.to_string());
                }
            }

            let final_snapshot = session.snapshot();
            drop(session);
            emit_state_changed(&app, final_snapshot.clone());
            Ok(final_snapshot)
        }
        DictationState::Idle => Err(VoxioError::Validation(
            "No active dictation session to stop.".to_string(),
        )),
        DictationState::Processing => Err(VoxioError::Validation(
            "Speech is already processing.".to_string(),
        )),
        DictationState::Error => Ok(current_snapshot(&app)),
    }
}

#[tauri::command]
pub fn cancel_dictation(app: AppHandle) -> Result<crate::state::AppStateSnapshot> {
    let state = app.state::<AppState>();
    let mut session = state.session.lock().expect("session mutex poisoned");
    session.state = DictationState::Idle;
    session.session_id = None;
    session.last_error = None;
    let snapshot = session.snapshot();
    drop(session);
    emit_state_changed(&app, snapshot.clone());
    Ok(snapshot)
}

fn validate_settings(settings: &Settings) -> Result<()> {
    if settings.hotkey.trim().is_empty() {
        return Err(VoxioError::Validation(
            "Hotkey must not be empty.".to_string(),
        ));
    }

    if !(500..=5000).contains(&settings.silence_timeout_ms) {
        return Err(VoxioError::Validation(
            "Silence timeout must be between 500 and 5000 ms.".to_string(),
        ));
    }

    Ok(())
}
