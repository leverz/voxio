use std::{
    fs,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{Mutex, OnceLock},
    thread,
    time::Duration,
};

use reqwest::blocking::{multipart, Client};
use serde::Deserialize;

use crate::{
    config::Settings,
    error::{Result, VoxioError},
};

use super::audio::AudioFrame;

#[derive(Debug, Clone)]
pub struct AsrConfig {
    pub language: String,
    pub model: String,
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

pub fn transcribe_file(audio_path: &Path, settings: &Settings) -> Result<TranscriptionResult> {
    if let Some(result) = transcribe_with_whisper_server(audio_path, settings)? {
        return Ok(result);
    }

    transcribe_with_openai_whisper(audio_path, settings)
}

fn make_output_dir() -> Result<PathBuf> {
    let mut path = std::env::temp_dir();
    path.push(format!("voxio-transcript-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&path).map_err(|error| {
        VoxioError::Transcription(format!(
            "failed to create transcript output directory {}: {error}",
            path.display()
        ))
    })?;
    Ok(path)
}

fn transcribe_with_whisper_server(
    audio_path: &Path,
    settings: &Settings,
) -> Result<Option<TranscriptionResult>> {
    let whisper_server_bin = std::env::var("VOXIO_WHISPER_SERVER_BIN")
        .unwrap_or_else(|_| "/opt/homebrew/bin/whisper-server".to_string());
    if !Path::new(&whisper_server_bin).exists() {
        return Ok(None);
    }

    let Some(model_path) = resolve_whisper_cpp_model(settings) else {
        return Ok(None);
    };
    let port = std::env::var("VOXIO_WHISPER_SERVER_PORT")
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(8178);

    ensure_whisper_server(&whisper_server_bin, &model_path, port)?;

    let client = Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|error| VoxioError::Transcription(format!("failed to build HTTP client: {error}")))?;
    let form = multipart::Form::new()
        .text("response_format", "json")
        .text(
            "language",
            settings.whisper_language().unwrap_or("auto").to_string(),
        )
        .file("file", audio_path)
        .map_err(|error| {
            VoxioError::Transcription(format!(
                "failed to attach audio file {}: {error}",
                audio_path.display()
            ))
        })?;

    let response = client
        .post(format!("http://127.0.0.1:{port}/inference"))
        .multipart(form)
        .send()
        .map_err(|error| VoxioError::Transcription(format!("whisper-server request failed: {error}")))?;

    if !response.status().is_success() {
        return Err(VoxioError::Transcription(format!(
            "whisper-server returned HTTP {}",
            response.status()
        )));
    }

    let payload: WhisperServerResponse = response
        .json()
        .map_err(|error| VoxioError::Transcription(format!("invalid whisper-server response: {error}")))?;

    Ok(Some(TranscriptionResult {
        text: payload.text.trim().to_string(),
    }))
}

fn transcribe_with_openai_whisper(
    audio_path: &Path,
    settings: &Settings,
) -> Result<TranscriptionResult> {
    let whisper_bin = std::env::var("VOXIO_WHISPER_BIN").unwrap_or_else(|_| "whisper".to_string());
    let output_dir = make_output_dir()?;
    let mut command = Command::new(&whisper_bin);
    command
        .arg(audio_path)
        .arg("--model")
        .arg(settings.whisper_model())
        .arg("--output_format")
        .arg("txt")
        .arg("--output_dir")
        .arg(&output_dir)
        .arg("--verbose")
        .arg("False")
        .arg("--task")
        .arg("transcribe");

    if let Some(language) = settings.whisper_language() {
        command.arg("--language").arg(language);
    }

    let output = command.output().map_err(|error| {
        VoxioError::Transcription(format!(
            "failed to launch whisper command `{whisper_bin}`: {error}"
        ))
    })?;

    if !output.status.success() {
        return Err(VoxioError::Transcription(format!(
            "whisper exited with status {}: {}",
            output
                .status
                .code()
                .map_or_else(|| "unknown".to_string(), |code| code.to_string()),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }

    let stem = audio_path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .ok_or_else(|| VoxioError::Transcription("audio file stem is invalid".to_string()))?;
    let transcript_path = output_dir.join(format!("{stem}.txt"));
    let text = fs::read_to_string(&transcript_path).map_err(|error| {
        VoxioError::Transcription(format!(
            "failed to read whisper transcript at {}: {error}",
            transcript_path.display()
        ))
    })?;

    Ok(TranscriptionResult {
        text: text.trim().to_string(),
    })
}

fn resolve_whisper_cpp_model(settings: &Settings) -> Option<PathBuf> {
    if let Ok(path) = std::env::var("VOXIO_WHISPER_CPP_MODEL") {
        let path = PathBuf::from(path);
        if path.exists() {
            return Some(path);
        }
    }

    let requested = match settings.whisper_model() {
        "small" => vec![
            "models/whisper/ggml-small.bin",
            "models/whisper/ggml-small.en.bin",
            "models/whisper/ggml-tiny-q5_1.bin",
        ],
        _ => vec![
            "models/whisper/ggml-base.bin",
            "models/whisper/ggml-base.en.bin",
            "models/whisper/ggml-tiny-q5_1.bin",
        ],
    };

    for candidate in requested {
        let path = PathBuf::from(candidate);
        if path.exists() {
            return Some(path);
        }
    }

    None
}

#[derive(Deserialize)]
struct WhisperServerResponse {
    text: String,
}

struct WhisperServerHandle {
    child: Child,
    model_path: PathBuf,
    port: u16,
}

static WHISPER_SERVER: OnceLock<Mutex<Option<WhisperServerHandle>>> = OnceLock::new();

fn ensure_whisper_server(server_bin: &str, model_path: &Path, port: u16) -> Result<()> {
    let store = WHISPER_SERVER.get_or_init(|| Mutex::new(None));
    let mut guard = store.lock().expect("whisper server mutex poisoned");

    if let Some(handle) = guard.as_mut() {
        if handle.model_path == model_path && handle.port == port && server_is_healthy(port) {
            return Ok(());
        }

        let _ = handle.child.kill();
        let _ = handle.child.wait();
        *guard = None;
    }

    let child = Command::new(server_bin)
        .arg("-m")
        .arg(model_path)
        .arg("--host")
        .arg("127.0.0.1")
        .arg("--port")
        .arg(port.to_string())
        .arg("-nt")
        .arg("-fa")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| {
            VoxioError::Transcription(format!(
                "failed to launch whisper-server `{server_bin}`: {error}"
            ))
        })?;

    *guard = Some(WhisperServerHandle {
        child,
        model_path: model_path.to_path_buf(),
        port,
    });
    drop(guard);

    wait_for_server_health(port)
}

fn wait_for_server_health(port: u16) -> Result<()> {
    for _ in 0..60 {
        if server_is_healthy(port) {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(100));
    }

    Err(VoxioError::Transcription(format!(
        "whisper-server did not become healthy on port {port}"
    )))
}

fn server_is_healthy(port: u16) -> bool {
    Client::builder()
        .timeout(Duration::from_millis(300))
        .build()
        .ok()
        .and_then(|client| {
            client
                .get(format!("http://127.0.0.1:{port}/health"))
                .send()
                .ok()
        })
        .is_some_and(|response| response.status().is_success())
}
