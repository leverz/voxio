use std::{fs, path::PathBuf};

use dirs::config_dir;
use serde::{Deserialize, Serialize};

use crate::error::Result;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub hotkey: String,
    pub language: String,
    pub transcription_hint: String,
    pub auto_punctuation: bool,
    pub silence_timeout_ms: u64,
    pub injection_mode: InjectionMode,
    pub transcription_provider: TranscriptionProvider,
    pub cloud_model: CloudModel,
    pub model: ModelSize,
    pub launch_at_login: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            hotkey: "Option+Space".to_string(),
            language: "auto".to_string(),
            transcription_hint: String::new(),
            auto_punctuation: true,
            silence_timeout_ms: 1200,
            injection_mode: InjectionMode::Auto,
            transcription_provider: TranscriptionProvider::Local,
            cloud_model: CloudModel::Fast,
            model: ModelSize::Balanced,
            launch_at_login: false,
        }
    }
}

impl Settings {
    pub fn whisper_language(&self) -> Option<&str> {
        match self.language.as_str() {
            "auto" => None,
            value => Some(value),
        }
    }

    pub fn transcription_hint(&self) -> Option<&str> {
        let value = self.transcription_hint.trim();
        if value.is_empty() {
            None
        } else {
            Some(value)
        }
    }

    pub fn whisper_model(&self) -> &str {
        match self.model {
            ModelSize::Fast => "tiny",
            ModelSize::Balanced => "base",
            ModelSize::Small => "small",
        }
    }

    pub fn openai_transcription_model(&self) -> &str {
        match self.cloud_model {
            CloudModel::Fast => "gpt-4o-mini-transcribe",
            CloudModel::Accurate => "gpt-4o-transcribe",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum InjectionMode {
    Auto,
    Accessibility,
    Clipboard,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TranscriptionProvider {
    Local,
    Cloud,
    Auto,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CloudModel {
    Fast,
    Accurate,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ModelSize {
    #[serde(alias = "tiny")]
    Fast,
    #[serde(alias = "base")]
    Balanced,
    #[serde(alias = "small")]
    Small,
}

pub struct ConfigStore {
    path: PathBuf,
}

impl ConfigStore {
    pub fn new() -> Self {
        let mut path = config_dir().unwrap_or_else(|| PathBuf::from("."));
        path.push("voxio");
        path.push("config.json");
        Self { path }
    }

    pub fn load(&self) -> Result<Settings> {
        if !self.path.exists() {
            return Ok(Settings::default());
        }

        let content = fs::read_to_string(&self.path)?;
        Ok(serde_json::from_str(&content)?)
    }

    pub fn save(&self, settings: &Settings) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }

        let json = serde_json::to_string_pretty(settings)?;
        fs::write(&self.path, json)?;
        Ok(())
    }
}
