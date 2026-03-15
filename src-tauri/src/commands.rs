use std::{sync::mpsc, thread, time::Instant};

use tauri::{AppHandle, Manager, State};
use uuid::Uuid;

use crate::{
    app::{current_snapshot, detect_permissions, emit_state_changed, PermissionStatus},
    config::{ConfigStore, LocalBackend, Settings, TranscriptionProvider},
    error::{Result, VoxioError},
    modules::{
        asr::{
            prewarm_provider, probe_provider, runtime_status, transcribe_wav_bytes, ProbeTarget,
            ProviderProbeResult, RuntimeStatus,
        },
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
pub fn get_runtime_status(state: State<'_, AppState>) -> Result<RuntimeStatus> {
    let settings = state.settings.lock().expect("settings mutex poisoned");
    Ok(runtime_status(&settings))
}

#[tauri::command]
pub fn test_transcription_provider(
    state: State<'_, AppState>,
    target: Option<ProbeTarget>,
) -> Result<ProviderProbeResult> {
    let settings = state.settings.lock().expect("settings mutex poisoned");
    probe_provider(&settings, target.unwrap_or(ProbeTarget::Current))
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
            session.requested_backend = None;
            session.actual_backend = None;
            session.detected_language = None;
            session.fallback_used = false;
            session.fallback_reason = None;
            let snapshot = session.snapshot();
            let settings = {
                let settings = state.settings.lock().expect("settings mutex poisoned");
                settings.clone()
            };
            drop(session);
            emit_state_changed(&app, snapshot.clone());
            thread::spawn(move || {
                prewarm_provider(&settings);
            });
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
            session.requested_backend = Some(requested_backend_label(&state));
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
                    let snapshot = finalize_processing(&app, session_id, Err(error));
                    return Ok(snapshot);
                }
            };

            let app_handle = app.clone();
            thread::spawn(move || {
                let result =
                    (|| -> Result<(String, String, u128, crate::modules::asr::RouteDecision)> {
                        let started_at = Instant::now();
                        let _sample_count = artifact.sample_count;
                        let transcription = transcribe_wav_bytes(&artifact.wav_bytes, &settings)?;
                        let transcript = transcription.text.trim().to_string();
                        if transcript.is_empty() {
                            return Err(VoxioError::Transcription(
                                "whisper returned an empty transcript".to_string(),
                            ));
                        }

                        inject_transcript(
                            &app_handle,
                            transcript.clone(),
                            settings.injection_mode.clone(),
                        )?;

                        Ok((
                            transcript,
                            transcription.provider,
                            started_at.elapsed().as_millis(),
                            transcription.route,
                        ))
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
    .map_err(|error| {
        VoxioError::Injection(format!("failed to schedule text injection: {error}"))
    })?;

    receiver.recv().map_err(|error| {
        VoxioError::Injection(format!("failed to receive injection result: {error}"))
    })?
}

fn finalize_processing(
    app: &AppHandle,
    session_id: Option<Uuid>,
    result: Result<(String, String, u128, crate::modules::asr::RouteDecision)>,
) -> crate::state::AppStateSnapshot {
    let state = app.state::<AppState>();
    let mut session = state.session.lock().expect("session mutex poisoned");

    let session_still_active =
        session.state == DictationState::Processing && session.session_id == session_id;
    if !session_still_active {
        return session.snapshot();
    }

    match result {
        Ok((transcript, provider, latency_ms, route)) => {
            session.last_transcript = Some(transcript);
            session.state = DictationState::Idle;
            session.last_error = None;
            session.last_provider = Some(provider);
            session.last_latency_ms = Some(latency_ms);
            session.requested_backend = Some(route.requested_backend);
            session.actual_backend = Some(route.actual_backend);
            session.detected_language = Some(route.detected_language);
            session.fallback_used = route.fallback_used;
            session.fallback_reason = route.fallback_reason;
        }
        Err(error) => {
            session.state = DictationState::Error;
            session.last_transcript = None;
            session.last_error = Some(error.to_string());
            session.last_provider = None;
            session.last_latency_ms = None;
            session.requested_backend = None;
            session.actual_backend = None;
            session.detected_language = None;
            session.fallback_used = false;
            session.fallback_reason = None;
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
    session.last_provider = None;
    session.last_latency_ms = None;
    session.requested_backend = None;
    session.actual_backend = None;
    session.detected_language = None;
    session.fallback_used = false;
    session.fallback_reason = None;
    let snapshot = session.snapshot();
    drop(session);
    emit_state_changed(&app, snapshot.clone());
    Ok(snapshot)
}

fn requested_backend_label(state: &AppState) -> String {
    let settings = state.settings.lock().expect("settings mutex poisoned");
    match settings.transcription_provider {
        TranscriptionProvider::Cloud => "Cloud".to_string(),
        TranscriptionProvider::Auto => format!(
            "Auto fallback ({})",
            match settings.local_backend {
                LocalBackend::Auto => "Auto route",
                LocalBackend::Whisper => "Whisper",
                LocalBackend::SenseVoice => "SenseVoice",
            }
        ),
        TranscriptionProvider::Local => match settings.local_backend {
            LocalBackend::Auto => "Auto route".to_string(),
            LocalBackend::Whisper => "Whisper".to_string(),
            LocalBackend::SenseVoice => "SenseVoice".to_string(),
        },
    }
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

    if settings.transcription_hint.chars().count() > 300 {
        return Err(VoxioError::Validation(
            "Prompt hint must be 300 characters or fewer.".to_string(),
        ));
    }

    if settings.vocabulary_terms.chars().count() > 500 {
        return Err(VoxioError::Validation(
            "Vocabulary must be 500 characters or fewer.".to_string(),
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{CloudModel, InjectionMode, ModelSize};

    fn make_settings(
        transcription_provider: TranscriptionProvider,
        local_backend: LocalBackend,
    ) -> Settings {
        Settings {
            hotkey: "Option+Space".to_string(),
            language: "auto".to_string(),
            local_backend,
            transcription_hint: String::new(),
            vocabulary_terms: String::new(),
            auto_punctuation: true,
            silence_timeout_ms: 1200,
            injection_mode: InjectionMode::Auto,
            transcription_provider,
            cloud_model: CloudModel::Fast,
            model: ModelSize::Balanced,
            launch_at_login: false,
        }
    }

    #[test]
    fn requested_backend_label_matches_local_modes() {
        assert_eq!(
            requested_backend_label(&AppState::new(make_settings(
                TranscriptionProvider::Local,
                LocalBackend::Auto
            ))),
            "Auto route"
        );
        assert_eq!(
            requested_backend_label(&AppState::new(make_settings(
                TranscriptionProvider::Local,
                LocalBackend::Whisper
            ))),
            "Whisper"
        );
        assert_eq!(
            requested_backend_label(&AppState::new(make_settings(
                TranscriptionProvider::Local,
                LocalBackend::SenseVoice
            ))),
            "SenseVoice"
        );
    }

    #[test]
    fn requested_backend_label_matches_auto_fallback_modes() {
        assert_eq!(
            requested_backend_label(&AppState::new(make_settings(
                TranscriptionProvider::Auto,
                LocalBackend::Auto
            ))),
            "Auto fallback (Auto route)"
        );
        assert_eq!(
            requested_backend_label(&AppState::new(make_settings(
                TranscriptionProvider::Auto,
                LocalBackend::Whisper
            ))),
            "Auto fallback (Whisper)"
        );
    }

    #[test]
    fn requested_backend_label_matches_cloud_mode() {
        assert_eq!(
            requested_backend_label(&AppState::new(make_settings(
                TranscriptionProvider::Cloud,
                LocalBackend::SenseVoice
            ))),
            "Cloud"
        );
    }
}
