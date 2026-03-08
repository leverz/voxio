use crate::error::Result;

use super::audio::AudioFrame;

#[derive(Debug, Clone)]
pub struct AsrConfig {
    pub language: String,
}

#[derive(Debug, Clone)]
pub struct TranscriptionResult {
    pub text: String,
}

pub trait AsrProvider {
    fn start_stream(&mut self, config: AsrConfig) -> Result<()>;
    fn push_audio(&mut self, frame: AudioFrame) -> Result<()>;
    fn stop(&mut self) -> Result<TranscriptionResult>;
}

pub struct NullAsrProvider;

impl AsrProvider for NullAsrProvider {
    fn start_stream(&mut self, _config: AsrConfig) -> Result<()> {
        Ok(())
    }

    fn push_audio(&mut self, _frame: AudioFrame) -> Result<()> {
        Ok(())
    }

    fn stop(&mut self) -> Result<TranscriptionResult> {
        Ok(TranscriptionResult {
            text: "Speech recognition pipeline is scaffolded but not connected yet.".into(),
        })
    }
}

