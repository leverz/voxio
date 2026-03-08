use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::{
    config::ConfigStore,
    error::Result,
    modules::audio,
    state::{AppState, AppStateSnapshot},
};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionStatus {
    pub microphone: bool,
    pub accessibility: bool,
    pub input_monitoring: bool,
}

impl Default for PermissionStatus {
    fn default() -> Self {
        Self {
            microphone: false,
            accessibility: false,
            input_monitoring: false,
        }
    }
}

pub fn bootstrap(app: &mut tauri::App) -> Result<()> {
    let store = ConfigStore::new();
    let settings = store.load()?;
    app.manage(AppState::new(settings));
    app.manage(store);
    Ok(())
}

pub fn emit_state_changed(app: &AppHandle, snapshot: AppStateSnapshot) {
    let _ = app.emit(
        "voxio://state-changed",
        serde_json::json!({ "snapshot": snapshot }),
    );
}

pub fn current_snapshot(app: &AppHandle) -> AppStateSnapshot {
    let state = app.state::<AppState>();
    let session = state.session.lock().expect("session mutex poisoned");
    session.snapshot()
}

pub fn detect_permissions() -> PermissionStatus {
    let microphone_ready = audio::has_input_device();
    let accessibility_ready = detect_accessibility_permission();

    PermissionStatus {
        microphone: microphone_ready,
        accessibility: accessibility_ready,
        input_monitoring: accessibility_ready,
    }
}

#[cfg(target_os = "macos")]
fn detect_accessibility_permission() -> bool {
    unsafe { macos::ax_is_process_trusted() }
}

#[cfg(not(target_os = "macos"))]
fn detect_accessibility_permission() -> bool {
    false
}

#[cfg(target_os = "macos")]
mod macos {
    #[link(name = "ApplicationServices", kind = "framework")]
    unsafe extern "C" {
        fn AXIsProcessTrusted() -> bool;
    }

    pub unsafe fn ax_is_process_trusted() -> bool {
        AXIsProcessTrusted()
    }
}
