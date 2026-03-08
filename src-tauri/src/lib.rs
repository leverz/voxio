pub mod app;
pub mod commands;
pub mod config;
pub mod error;
pub mod modules;
pub mod state;

use tauri::Builder;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    Builder::default()
        .setup(|app| {
            app::bootstrap(app)?;
            Ok(())
        })
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .invoke_handler(tauri::generate_handler![
            commands::cancel_dictation,
            commands::get_app_state,
            commands::get_settings,
            commands::request_permissions,
            commands::start_dictation,
            commands::stop_dictation,
            commands::toggle_dictation,
            commands::update_settings,
        ])
        .run(tauri::generate_context!())
        .expect("error while running voxio");
}
