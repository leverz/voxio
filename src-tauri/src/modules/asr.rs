use std::{
    fs,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{Mutex, OnceLock},
    thread,
    time::Duration,
};

use reqwest::blocking::{multipart, Client};
use serde::{Deserialize, Serialize};

use crate::{
    config::{Settings, TranscriptionProvider},
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
    pub provider: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeStatus {
    pub local_ready: bool,
    pub cloud_ready: bool,
    pub local_backend: String,
    pub effective_provider: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderProbeResult {
    pub provider: String,
    pub ok: bool,
    pub message: String,
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
            provider: "none".into(),
        })
    }
}

pub fn transcribe_wav_bytes(wav_bytes: &[u8], settings: &Settings) -> Result<TranscriptionResult> {
    match settings.transcription_provider {
        TranscriptionProvider::Local => transcribe_with_local_provider(wav_bytes, settings),
        TranscriptionProvider::Cloud => transcribe_with_cloud_provider(wav_bytes, settings),
        TranscriptionProvider::Auto => {
            transcribe_with_local_provider(wav_bytes, settings)
                .or_else(|_| transcribe_with_cloud_provider(wav_bytes, settings))
        }
    }
}

pub fn prewarm_provider(settings: &Settings) {
    if !matches!(settings.transcription_provider, TranscriptionProvider::Local | TranscriptionProvider::Auto) {
        return;
    }
    let _ = try_prewarm_provider(settings);
}

pub fn runtime_status(settings: &Settings) -> RuntimeStatus {
    let local_backend = detect_local_backend(settings).unwrap_or_else(|| "Unavailable".to_string());
    let local_ready = local_backend != "Unavailable";
    let cloud_ready = std::env::var("OPENAI_API_KEY")
        .ok()
        .is_some_and(|value| !value.trim().is_empty());

    let effective_provider = match settings.transcription_provider {
        TranscriptionProvider::Local => {
            if local_ready {
                "Local".to_string()
            } else {
                "Unavailable".to_string()
            }
        }
        TranscriptionProvider::Cloud => {
            if cloud_ready {
                "Cloud".to_string()
            } else {
                "Unavailable".to_string()
            }
        }
        TranscriptionProvider::Auto => {
            if local_ready {
                "Local".to_string()
            } else if cloud_ready {
                "Cloud".to_string()
            } else {
                "Unavailable".to_string()
            }
        }
    };

    RuntimeStatus {
        local_ready,
        cloud_ready,
        local_backend,
        effective_provider,
    }
}

pub fn probe_provider(settings: &Settings) -> Result<ProviderProbeResult> {
    match settings.transcription_provider {
        TranscriptionProvider::Local => probe_local_provider(settings),
        TranscriptionProvider::Cloud => probe_cloud_provider(settings),
        TranscriptionProvider::Auto => {
            let status = runtime_status(settings);
            if status.effective_provider == "Local" {
                probe_local_provider(settings)
            } else if status.effective_provider == "Cloud" {
                probe_cloud_provider(settings)
            } else {
                Ok(ProviderProbeResult {
                    provider: "Unavailable".to_string(),
                    ok: false,
                    message: "No transcription provider is ready.".to_string(),
                })
            }
        }
    }
}

fn probe_local_provider(settings: &Settings) -> Result<ProviderProbeResult> {
    let Some(local_backend) = detect_local_backend(settings) else {
        return Ok(ProviderProbeResult {
            provider: "Local".to_string(),
            ok: false,
            message: "No local transcription backend is available.".to_string(),
        });
    };

    Ok(ProviderProbeResult {
        provider: "Local".to_string(),
        ok: true,
        message: format!("{local_backend} is ready."),
    })
}

fn probe_cloud_provider(settings: &Settings) -> Result<ProviderProbeResult> {
    let api_key = std::env::var("OPENAI_API_KEY").map_err(|_| {
        VoxioError::Transcription("cloud transcription is not configured. Set OPENAI_API_KEY to enable it.".to_string())
    })?;
    let base_url = std::env::var("OPENAI_BASE_URL")
        .unwrap_or_else(|_| "https://api.openai.com/v1".to_string());

    let client = Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|error| VoxioError::Transcription(format!("failed to build HTTP client: {error}")))?;
    let response = client
        .get(format!("{base_url}/models"))
        .bearer_auth(api_key)
        .send()
        .map_err(|error| VoxioError::Transcription(format!("OpenAI connectivity check failed: {error}")))?;

    if !response.status().is_success() {
        return Ok(ProviderProbeResult {
            provider: "Cloud".to_string(),
            ok: false,
            message: format!(
                "OpenAI connectivity check returned HTTP {}.",
                response.status()
            ),
        });
    }

    Ok(ProviderProbeResult {
        provider: "Cloud".to_string(),
        ok: true,
        message: format!(
            "{} is reachable.",
            settings.openai_transcription_model()
        ),
    })
}

fn detect_local_backend(settings: &Settings) -> Option<String> {
    let whisper_cli_bin = std::env::var("VOXIO_WHISPER_CPP_BIN")
        .unwrap_or_else(|_| "/opt/homebrew/bin/whisper-cli".to_string());
    if Path::new(&whisper_cli_bin).exists() {
        if let Some(model_path) = resolve_whisper_cpp_model(settings) {
            let model_name = model_path
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("model");
            return Some(format!("whisper-cli ({model_name})"));
        }
    }

    let whisper_bin = std::env::var("VOXIO_WHISPER_BIN").unwrap_or_else(|_| "whisper".to_string());
    if command_is_available(&whisper_bin) {
        return Some("openai-whisper".to_string());
    }

    None
}

fn transcribe_with_local_provider(
    wav_bytes: &[u8],
    settings: &Settings,
) -> Result<TranscriptionResult> {
    if let Some(result) = transcribe_with_whisper_cli(wav_bytes, settings)? {
        return Ok(result);
    }

    if let Some(result) = transcribe_with_whisper_server(wav_bytes, settings)? {
        return Ok(result);
    }

    transcribe_with_openai_whisper(wav_bytes, settings)
}

fn transcribe_with_cloud_provider(
    wav_bytes: &[u8],
    settings: &Settings,
) -> Result<TranscriptionResult> {
    if let Some(result) = transcribe_with_openai_api(wav_bytes, settings)? {
        return Ok(result);
    }

    Err(VoxioError::Transcription(
        "cloud transcription is not configured. Set OPENAI_API_KEY to enable it.".to_string(),
    ))
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
    wav_bytes: &[u8],
    settings: &Settings,
) -> Result<Option<TranscriptionResult>> {
    let Some(whisper_server) = resolve_whisper_server(settings)? else {
        return Ok(None);
    };
    if ensure_whisper_server(
        &whisper_server.server_bin,
        &whisper_server.model_path,
        whisper_server.port,
    )
    .is_err()
    {
        return Ok(None);
    }

    let client = Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|error| VoxioError::Transcription(format!("failed to build HTTP client: {error}")))?;
    let audio_part = multipart::Part::bytes(wav_bytes.to_vec())
        .file_name("voxio-recording.wav")
        .mime_str("audio/wav")
        .map_err(|error| VoxioError::Transcription(format!("failed to build audio part: {error}")))?;
    let form = multipart::Form::new()
        .text("response_format", "json")
        .text(
            "language",
            settings.whisper_language().unwrap_or("auto").to_string(),
        )
        .part("file", audio_part);
    let form = if let Some(prompt) = settings.transcription_hint() {
        form.text("prompt", prompt.to_string())
    } else {
        form
    };

    let response = client
        .post(format!("http://127.0.0.1:{}/inference", whisper_server.port))
        .multipart(form)
        .send();

    let response = match response {
        Ok(response) => response,
        Err(_) => return Ok(None),
    };

    if !response.status().is_success() {
        return Ok(None);
    }

    let payload: WhisperServerResponse = response
        .json()
        .map_err(|error| VoxioError::Transcription(format!("invalid whisper-server response: {error}")))?;

    Ok(Some(TranscriptionResult {
        text: payload.text.trim().to_string(),
        provider: "whisper-server".to_string(),
    }))
}

fn transcribe_with_whisper_cli(
    wav_bytes: &[u8],
    settings: &Settings,
) -> Result<Option<TranscriptionResult>> {
    let whisper_cli_bin = std::env::var("VOXIO_WHISPER_CPP_BIN")
        .unwrap_or_else(|_| "/opt/homebrew/bin/whisper-cli".to_string());
    if !Path::new(&whisper_cli_bin).exists() {
        return Ok(None);
    }

    let Some(model_path) = resolve_whisper_cpp_model(settings) else {
        return Ok(None);
    };

    let audio_path = write_temp_wav(wav_bytes)?;
    let output_dir = make_output_dir()?;
    let output_prefix = output_dir.join("transcript");
    let mut command = Command::new(&whisper_cli_bin);
    command
        .arg("-m")
        .arg(&model_path)
        .arg("-f")
        .arg(&audio_path)
        .arg("-otxt")
        .arg("-of")
        .arg(&output_prefix)
        .arg("-nt")
        .arg("-np")
        .arg("-fa");

    if let Some(language) = settings.whisper_language() {
        command.arg("-l").arg(language);
    } else {
        command.arg("-l").arg("auto");
    }
    if let Some(prompt) = settings.transcription_hint() {
        command.arg("--prompt").arg(prompt);
    }

    let output = command.output().map_err(|error| {
        VoxioError::Transcription(format!(
            "failed to launch whisper-cli `{whisper_cli_bin}`: {error}"
        ))
    })?;

    if !output.status.success() {
        let _ = fs::remove_file(&audio_path);
        return Ok(None);
    }

    let transcript_path = output_prefix.with_extension("txt");
    let text = fs::read_to_string(&transcript_path).map_err(|error| {
        VoxioError::Transcription(format!(
            "failed to read whisper-cli transcript at {}: {error}",
            transcript_path.display()
        ))
    })?;

    let _ = fs::remove_file(&audio_path);

    Ok(Some(TranscriptionResult {
        text: text.trim().to_string(),
        provider: "whisper-cli".to_string(),
    }))
}

fn transcribe_with_openai_api(
    wav_bytes: &[u8],
    settings: &Settings,
) -> Result<Option<TranscriptionResult>> {
    let api_key = match std::env::var("OPENAI_API_KEY") {
        Ok(value) if !value.trim().is_empty() => value,
        _ => return Ok(None),
    };
    let base_url = std::env::var("OPENAI_BASE_URL")
        .unwrap_or_else(|_| "https://api.openai.com/v1".to_string());

    let client = Client::builder()
        .timeout(Duration::from_secs(90))
        .build()
        .map_err(|error| VoxioError::Transcription(format!("failed to build HTTP client: {error}")))?;
    let audio_part = multipart::Part::bytes(wav_bytes.to_vec())
        .file_name("voxio-recording.wav")
        .mime_str("audio/wav")
        .map_err(|error| VoxioError::Transcription(format!("failed to build audio part: {error}")))?;
    let mut form = multipart::Form::new()
        .text("model", settings.openai_transcription_model().to_string())
        .text("response_format", "json".to_string())
        .part("file", audio_part);

    if let Some(language) = settings.whisper_language() {
        form = form.text("language", language.to_string());
    }
    if let Some(prompt) = settings.transcription_hint() {
        form = form.text("prompt", prompt.to_string());
    }

    let response = client
        .post(format!("{base_url}/audio/transcriptions"))
        .bearer_auth(api_key)
        .multipart(form)
        .send()
        .map_err(|error| VoxioError::Transcription(format!("OpenAI transcription request failed: {error}")))?;

    if !response.status().is_success() {
        return Err(VoxioError::Transcription(format!(
            "OpenAI transcription returned HTTP {}",
            response.status()
        )));
    }

    let payload: OpenAiTranscriptionResponse = response
        .json()
        .map_err(|error| VoxioError::Transcription(format!("invalid OpenAI transcription response: {error}")))?;

    Ok(Some(TranscriptionResult {
        text: payload.text.trim().to_string(),
        provider: settings.openai_transcription_model().to_string(),
    }))
}

fn try_prewarm_provider(settings: &Settings) -> Result<()> {
    if std::env::var("VOXIO_ENABLE_WHISPER_SERVER").ok().as_deref() != Some("1") {
        return Ok(());
    }

    let Some(whisper_server) = resolve_whisper_server(settings)? else {
        return Ok(());
    };

    ensure_whisper_server(
        &whisper_server.server_bin,
        &whisper_server.model_path,
        whisper_server.port,
    )
}

fn transcribe_with_openai_whisper(
    wav_bytes: &[u8],
    settings: &Settings,
) -> Result<TranscriptionResult> {
    let audio_path = write_temp_wav(wav_bytes)?;
    let whisper_bin = std::env::var("VOXIO_WHISPER_BIN").unwrap_or_else(|_| "whisper".to_string());
    let output_dir = make_output_dir()?;
    let mut command = Command::new(&whisper_bin);
    command
        .arg(&audio_path)
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
    if let Some(prompt) = settings.transcription_hint() {
        command.arg("--initial_prompt").arg(prompt);
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
    let _ = fs::remove_file(&audio_path);

    Ok(TranscriptionResult {
        text: text.trim().to_string(),
        provider: "openai-whisper".to_string(),
    })
}

fn write_temp_wav(wav_bytes: &[u8]) -> Result<PathBuf> {
    let mut wav_path = std::env::temp_dir();
    wav_path.push(format!("voxio-recording-{}.wav", uuid::Uuid::new_v4()));
    fs::write(&wav_path, wav_bytes).map_err(|error| {
        VoxioError::Transcription(format!(
            "failed to write temporary wav file {}: {error}",
            wav_path.display()
        ))
    })?;
    Ok(wav_path)
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
            "models/whisper/ggml-base-q5_1.bin",
            "models/whisper/ggml-base.bin",
            "models/whisper/ggml-base.en.bin",
            "models/whisper/ggml-tiny-q5_1.bin",
        ],
        "tiny" => vec![
            "models/whisper/ggml-tiny-q5_1.bin",
            "models/whisper/ggml-tiny.bin",
        ],
        _ => vec![
            "models/whisper/ggml-base-q5_1.bin",
            "models/whisper/ggml-base.bin",
            "models/whisper/ggml-base.en.bin",
            "models/whisper/ggml-tiny-q5_1.bin",
        ],
    };

    for candidate in requested {
        if let Some(path) = resolve_existing_model_path(candidate) {
            return Some(path);
        }
    }

    None
}

fn resolve_existing_model_path(candidate: &str) -> Option<PathBuf> {
    let path = PathBuf::from(candidate);
    if path.is_absolute() && model_file_looks_complete(&path) {
        return Some(path);
    }

    let cwd_path = std::env::current_dir().ok()?.join(&path);
    if model_file_looks_complete(&cwd_path) {
        return Some(cwd_path);
    }

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let manifest_relative = manifest_dir.join(&path);
    if model_file_looks_complete(&manifest_relative) {
        return Some(manifest_relative);
    }

    let workspace_relative = manifest_dir.parent().map(|root| root.join(&path));
    if let Some(workspace_relative) = workspace_relative {
        if model_file_looks_complete(&workspace_relative) {
            return Some(workspace_relative);
        }
    }

    None
}

fn model_file_looks_complete(path: &Path) -> bool {
    if !path.exists() {
        return false;
    }

    let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };

    let Ok(metadata) = fs::metadata(path) else {
        return false;
    };

    let min_size = match file_name {
        "ggml-tiny-q5_1.bin" => 20 * 1024 * 1024,
        "ggml-tiny.bin" => 70 * 1024 * 1024,
        "ggml-base-q5_1.bin" => 50 * 1024 * 1024,
        "ggml-base.bin" | "ggml-base.en.bin" => 120 * 1024 * 1024,
        "ggml-small.bin" | "ggml-small.en.bin" => 230 * 1024 * 1024,
        _ => 1,
    };

    metadata.len() >= min_size
}

fn command_is_available(command: &str) -> bool {
    if Path::new(command).is_absolute() {
        return Path::new(command).exists();
    }

    std::env::var_os("PATH").is_some_and(|paths| {
        std::env::split_paths(&paths).any(|entry| entry.join(command).exists())
    })
}

#[derive(Deserialize)]
struct WhisperServerResponse {
    text: String,
}

#[derive(Deserialize)]
struct OpenAiTranscriptionResponse {
    text: String,
}

struct WhisperServerHandle {
    child: Child,
    model_path: PathBuf,
    port: u16,
}

struct WhisperServerConfig {
    server_bin: String,
    model_path: PathBuf,
    port: u16,
}

static WHISPER_SERVER: OnceLock<Mutex<Option<WhisperServerHandle>>> = OnceLock::new();

fn resolve_whisper_server(settings: &Settings) -> Result<Option<WhisperServerConfig>> {
    if std::env::var("VOXIO_ENABLE_WHISPER_SERVER").ok().as_deref() != Some("1") {
        return Ok(None);
    }

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

    Ok(Some(WhisperServerConfig {
        server_bin: whisper_server_bin,
        model_path,
        port,
    }))
}

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
