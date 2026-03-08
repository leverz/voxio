use std::{thread, time::Duration};

use arboard::Clipboard;
use enigo::{
    Direction::{Click, Press, Release},
    Enigo, Key, Keyboard, Settings,
};

use crate::{
    config::InjectionMode,
    error::{Result, VoxioError},
};

#[derive(Debug, Clone)]
pub struct InjectResult {
    pub applied: bool,
}

pub trait TextInjector {
    fn inject(&self, text: &str) -> Result<InjectResult>;
}

pub struct NullInjector;

impl TextInjector for NullInjector {
    fn inject(&self, text: &str) -> Result<InjectResult> {
        let applied = !text.is_empty();
        Ok(InjectResult { applied })
    }
}

pub struct ClipboardInjector;

impl TextInjector for ClipboardInjector {
    fn inject(&self, text: &str) -> Result<InjectResult> {
        if text.trim().is_empty() {
            return Ok(InjectResult { applied: false });
        }

        let mut clipboard = Clipboard::new()
            .map_err(|error| VoxioError::Injection(format!("clipboard unavailable: {error}")))?;
        let previous_text = clipboard.get_text().ok();

        clipboard
            .set_text(text.to_string())
            .map_err(|error| VoxioError::Injection(format!("failed to set clipboard text: {error}")))?;

        let mut enigo = Enigo::new(&Settings::default())
            .map_err(|error| VoxioError::Injection(format!("failed to initialize keyboard driver: {error}")))?;

        paste_clipboard(&mut enigo)?;

        // Give the target app time to read the temporary clipboard payload before restoring it.
        thread::sleep(Duration::from_millis(120));

        if let Some(previous_text) = previous_text {
            let _ = clipboard.set_text(previous_text);
        }

        Ok(InjectResult { applied: true })
    }
}

pub fn build_injector(mode: &InjectionMode) -> Box<dyn TextInjector + Send + Sync> {
    match mode {
        InjectionMode::Auto | InjectionMode::Clipboard | InjectionMode::Accessibility => {
            Box::new(ClipboardInjector)
        }
    }
}

fn paste_clipboard(enigo: &mut Enigo) -> Result<()> {
    #[cfg(target_os = "macos")]
    let modifier = Key::Meta;
    #[cfg(not(target_os = "macos"))]
    let modifier = Key::Control;

    enigo
        .key(modifier, Press)
        .map_err(|error| VoxioError::Injection(format!("failed to press modifier: {error}")))?;
    enigo
        .key(Key::Unicode('v'), Click)
        .map_err(|error| VoxioError::Injection(format!("failed to send paste keystroke: {error}")))?;
    enigo
        .key(modifier, Release)
        .map_err(|error| VoxioError::Injection(format!("failed to release modifier: {error}")))?;

    Ok(())
}
