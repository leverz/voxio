use std::{
    sync::mpsc,
    thread,
};

use tauri::{AppHandle, Manager, State};
use uuid::Uuid;

use crate::{
    app::{current_snapshot, detect_permissions, emit_state_changed, PermissionStatus},
    config::{ConfigStore, Settings},
    error::{Result, VoxioError},
    modules::{
        asr::transcribe_wav_bytes,
        audio::{
            clear_active_recording, start_recording, stop_recording, store_active_recording,
            take_active_recording,
        },
        injector::build_injector,
    },
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
            let recording = start_recording()?;
            store_active_recording(recording);

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
            let session_id = session.session_id;
            drop(session);
            emit_state_changed(&app, processing_snapshot);

            let settings = {
                let settings = state.settings.lock().expect("settings mutex poisoned");
                settings.clone()
            };
            let recording = take_active_recording()
                .ok_or_else(|| VoxioError::Recording("no active recording session".to_string()))?;
            let artifact = match stop_recording(recording) {
                Ok(artifact) => artifact,
                Err(error) => {
                    let snapshot = finalize_processing(
                        &app,
                        session_id,
                        Err(error),
                    );
                    return Ok(snapshot);
                }
            };

            let app_handle = app.clone();
            thread::spawn(move || {
                let result = (|| -> Result<String> {
                    let _sample_count = artifact.sample_count;
                    let transcription = transcribe_wav_bytes(&artifact.wav_bytes, &settings)?;
                    let transcript = transcription.text.trim().to_string();
                    if transcript.is_empty() {
                        return Err(VoxioError::Transcription(
                            "whisper returned an empty transcript".to_string(),
                        ));
                    }

                    inject_transcript(&app_handle, transcript.clone(), settings.injection_mode.clone())?;

                    Ok(transcript)
                })();

                finalize_processing(&app_handle, session_id, result);
            });

            Ok(current_snapshot(&app))
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

fn inject_transcript(
    app: &AppHandle,
    transcript: String,
    injection_mode: crate::config::InjectionMode,
) -> Result<()> {
    let (sender, receiver) = mpsc::channel();

    app.run_on_main_thread(move || {
        let result = (|| -> Result<()> {
            let injector = build_injector(&injection_mode);
            let inject_result = injector.inject(&transcript)?;
            if !inject_result.applied {
                return Err(VoxioError::Injection(
                    "no text was available to inject".to_string(),
                ));
            }

            Ok(())
        })();

        let _ = sender.send(result);
    })
    .map_err(|error| VoxioError::Injection(format!("failed to schedule text injection: {error}")))?;

    receiver
        .recv()
        .map_err(|error| VoxioError::Injection(format!("failed to receive injection result: {error}")))?
}

fn finalize_processing(
    app: &AppHandle,
    session_id: Option<Uuid>,
    result: Result<String>,
) -> crate::state::AppStateSnapshot {
    let state = app.state::<AppState>();
    let mut session = state.session.lock().expect("session mutex poisoned");

    let session_still_active =
        session.state == DictationState::Processing && session.session_id == session_id;
    if !session_still_active {
        return session.snapshot();
    }

    match result {
        Ok(transcript) => {
            session.last_transcript = Some(transcript);
            session.state = DictationState::Idle;
            session.last_error = None;
        }
        Err(error) => {
            session.state = DictationState::Error;
            session.last_transcript = None;
            session.last_error = Some(error.to_string());
        }
    }

    let snapshot = session.snapshot();
    drop(session);
    emit_state_changed(app, snapshot.clone());
    snapshot
}

#[tauri::command]
pub fn cancel_dictation(app: AppHandle) -> Result<crate::state::AppStateSnapshot> {
    let state = app.state::<AppState>();
    clear_active_recording();
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
