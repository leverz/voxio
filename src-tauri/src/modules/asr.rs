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
    config::{LocalBackend, Settings, TranscriptionProvider},
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
    pub route: RouteDecision,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RouteDecision {
    pub requested_provider: String,
    pub effective_provider: String,
    pub requested_backend: String,
    pub actual_backend: String,
    pub detected_language: String,
    pub fallback_used: bool,
    pub fallback_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackendStatus {
    pub name: String,
    pub ready: bool,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeStatus {
    pub local_ready: bool,
    pub cloud_ready: bool,
    pub local_backend: String,
    pub effective_provider: String,
    pub whisper: BackendStatus,
    pub sense_voice: BackendStatus,
    pub cloud: BackendStatus,
    pub local_strategy: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderProbeResult {
    pub provider: String,
    pub ok: bool,
    pub message: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProbeTarget {
    Current,
    AutoRoute,
    Whisper,
    SenseVoice,
    Cloud,
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
            route: RouteDecision::default(),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LocalBackendKind {
    Whisper,
    SenseVoice,
}

impl LocalBackendKind {
    fn label(self) -> &'static str {
        match self {
            Self::Whisper => "Whisper",
            Self::SenseVoice => "SenseVoice",
        }
    }
}

pub fn transcribe_wav_bytes(wav_bytes: &[u8], settings: &Settings) -> Result<TranscriptionResult> {
    match settings.transcription_provider {
        TranscriptionProvider::Local => transcribe_with_local_provider(wav_bytes, settings),
        TranscriptionProvider::Cloud => transcribe_with_cloud_provider(wav_bytes, settings),
        TranscriptionProvider::Auto => match transcribe_with_local_provider(wav_bytes, settings) {
            Ok(result) => Ok(result),
            Err(local_error) => transcribe_with_cloud_provider(wav_bytes, settings)
                .map(|mut result| {
                    result.route.requested_provider = "Auto fallback".to_string();
                    result.route.effective_provider = "Cloud".to_string();
                    result.route.fallback_used = true;
                    result.route.fallback_reason = Some(format!(
                        "Local transcription failed, so Auto fallback retried with cloud: {}",
                        local_error
                    ));
                    result
                })
                .map_err(|_| local_error),
        },
    }
}

pub fn prewarm_provider(settings: &Settings) {
    if !matches!(
        settings.transcription_provider,
        TranscriptionProvider::Local | TranscriptionProvider::Auto
    ) {
        return;
    }
    let _ = try_prewarm_provider(settings);
}

pub fn runtime_status(settings: &Settings) -> RuntimeStatus {
    let whisper = whisper_backend_status(settings);
    let sense_voice = sensevoice_backend_status();
    let cloud = cloud_backend_status(settings);
    let local_ready = whisper.ready || sense_voice.ready;

    let effective_provider = match settings.transcription_provider {
        TranscriptionProvider::Local => {
            if local_ready {
                "Local".to_string()
            } else {
                "Unavailable".to_string()
            }
        }
        TranscriptionProvider::Cloud => {
            if cloud.ready {
                "Cloud".to_string()
            } else {
                "Unavailable".to_string()
            }
        }
        TranscriptionProvider::Auto => {
            if local_ready {
                "Local".to_string()
            } else if cloud.ready {
                "Cloud".to_string()
            } else {
                "Unavailable".to_string()
            }
        }
    };

    let local_backend = current_local_backend_label(settings, &whisper, &sense_voice);
    let local_strategy = describe_local_strategy(settings);

    RuntimeStatus {
        local_ready,
        cloud_ready: cloud.ready,
        local_backend,
        effective_provider,
        whisper,
        sense_voice,
        cloud,
        local_strategy,
    }
}

pub fn probe_provider(settings: &Settings, target: ProbeTarget) -> Result<ProviderProbeResult> {
    match target {
        ProbeTarget::Current => match settings.transcription_provider {
            TranscriptionProvider::Local => probe_local_provider(settings),
            TranscriptionProvider::Cloud => probe_cloud_provider(settings),
            TranscriptionProvider::Auto => probe_auto_route(settings),
        },
        ProbeTarget::AutoRoute => probe_auto_route(settings),
        ProbeTarget::Whisper => probe_whisper_provider(settings),
        ProbeTarget::SenseVoice => probe_sensevoice_provider(),
        ProbeTarget::Cloud => probe_cloud_provider(settings),
    }
}

fn probe_local_provider(settings: &Settings) -> Result<ProviderProbeResult> {
    match settings.local_backend {
        LocalBackend::Whisper => probe_whisper_provider(settings),
        LocalBackend::SenseVoice => probe_sensevoice_provider(),
        LocalBackend::Auto => probe_auto_route(settings),
    }
}

fn probe_auto_route(settings: &Settings) -> Result<ProviderProbeResult> {
    let whisper = whisper_backend_status(settings);
    let sense_voice = sensevoice_backend_status();
    let strategy = describe_local_strategy(settings);
    let backend = resolve_auto_route_backend(settings, &whisper, &sense_voice)
        .map(|backend| backend.label())
        .unwrap_or("Unavailable");

    Ok(ProviderProbeResult {
        provider: "Auto route".to_string(),
        ok: backend != "Unavailable",
        message: format!("{strategy}. Current route target: {backend}."),
    })
}

fn probe_whisper_provider(settings: &Settings) -> Result<ProviderProbeResult> {
    let status = whisper_backend_status(settings);
    Ok(ProviderProbeResult {
        provider: "Whisper".to_string(),
        ok: status.ready,
        message: status.detail,
    })
}

fn probe_sensevoice_provider() -> Result<ProviderProbeResult> {
    let status = sensevoice_backend_status();
    Ok(ProviderProbeResult {
        provider: "SenseVoice".to_string(),
        ok: status.ready,
        message: status.detail,
    })
}

fn probe_cloud_provider(settings: &Settings) -> Result<ProviderProbeResult> {
    let api_key = std::env::var("OPENAI_API_KEY").map_err(|_| {
        VoxioError::Transcription(
            "cloud transcription is not configured. Set OPENAI_API_KEY to enable it.".to_string(),
        )
    })?;
    let base_url = std::env::var("OPENAI_BASE_URL")
        .unwrap_or_else(|_| "https://api.openai.com/v1".to_string());

    let client = Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|error| {
            VoxioError::Transcription(format!("failed to build HTTP client: {error}"))
        })?;
    let response = client
        .get(format!("{base_url}/models"))
        .bearer_auth(api_key)
        .send()
        .map_err(|error| {
            VoxioError::Transcription(format!("OpenAI connectivity check failed: {error}"))
        })?;

    Ok(ProviderProbeResult {
        provider: "Cloud".to_string(),
        ok: response.status().is_success(),
        message: if response.status().is_success() {
            format!("{} is reachable.", settings.openai_transcription_model())
        } else {
            format!(
                "OpenAI connectivity check returned HTTP {}.",
                response.status()
            )
        },
    })
}

fn transcribe_with_local_provider(
    wav_bytes: &[u8],
    settings: &Settings,
) -> Result<TranscriptionResult> {
    let requested_backend = settings.local_backend;
    let requested_label = requested_backend_label(requested_backend);
    let mut route = RouteDecision {
        requested_provider: "Local".to_string(),
        effective_provider: "Local".to_string(),
        requested_backend: requested_label.to_string(),
        actual_backend: "Unavailable".to_string(),
        detected_language: requested_language_label(settings).to_string(),
        fallback_used: false,
        fallback_reason: None,
    };

    match requested_backend {
        LocalBackend::SenseVoice => {
            if let Some(result) = transcribe_with_sensevoice(wav_bytes)? {
                route.actual_backend = LocalBackendKind::SenseVoice.label().to_string();
                route.detected_language =
                    detect_language_from_text(&result.text, result.detected_language.as_deref());
                return Ok(TranscriptionResult {
                    text: result.text,
                    provider: result.provider,
                    route,
                });
            }

            let reason = sensevoice_backend_status().detail;
            return Err(VoxioError::Transcription(format!(
                "SenseVoice is pinned. {reason}"
            )));
        }
        LocalBackend::Whisper => {
            if let Some(result) = transcribe_with_whisper_family(wav_bytes, settings)? {
                route.actual_backend = LocalBackendKind::Whisper.label().to_string();
                route.detected_language = detect_language_from_text(&result.text, None);
                return Ok(TranscriptionResult {
                    text: result.text,
                    provider: result.provider,
                    route,
                });
            }

            let reason = whisper_backend_status(settings).detail;
            return Err(VoxioError::Transcription(format!(
                "Whisper is pinned. {reason}"
            )));
        }
        LocalBackend::Auto => {}
    }

    let whisper = whisper_backend_status(settings);
    let sense_voice = sensevoice_backend_status();
    let primary = resolve_auto_route_backend(settings, &whisper, &sense_voice);
    let fallback = primary.and_then(|backend| fallback_backend(backend, &whisper, &sense_voice));

    if let Some(primary_backend) = primary {
        let primary_result = match primary_backend {
            LocalBackendKind::SenseVoice => transcribe_with_sensevoice(wav_bytes)?.map(|result| {
                let language =
                    detect_language_from_text(&result.text, result.detected_language.as_deref());
                (result, language)
            }),
            LocalBackendKind::Whisper => {
                transcribe_with_whisper_family(wav_bytes, settings)?.map(|result| {
                    let language = detect_language_from_text(&result.text, None);
                    (result, language)
                })
            }
        };

        if let Some((result, language)) = primary_result {
            route.actual_backend = primary_backend.label().to_string();
            route.detected_language = language.clone();

            if should_retry_with_fallback(primary_backend, &language, &result.text) {
                if let Some(fallback_backend) = fallback {
                    if let Some((fallback_result, fallback_language)) =
                        run_fallback_backend(fallback_backend, wav_bytes, settings)?
                    {
                        route.actual_backend = fallback_backend.label().to_string();
                        route.detected_language = fallback_language;
                        route.fallback_used = true;
                        route.fallback_reason = Some(
                            match primary_backend {
                                LocalBackendKind::SenseVoice => {
                                    "SenseVoice result looked English-heavy; retried with Whisper."
                                }
                                LocalBackendKind::Whisper => {
                                    "Whisper result looked Chinese-heavy; retried with SenseVoice."
                                }
                            }
                            .to_string(),
                        );

                        return Ok(TranscriptionResult {
                            text: fallback_result.text,
                            provider: fallback_result.provider,
                            route,
                        });
                    }
                }
            }

            return Ok(TranscriptionResult {
                text: result.text,
                provider: result.provider,
                route,
            });
        }

        if let Some(fallback_backend) = fallback {
            if let Some((fallback_result, fallback_language)) =
                run_fallback_backend(fallback_backend, wav_bytes, settings)?
            {
                route.actual_backend = fallback_backend.label().to_string();
                route.detected_language = fallback_language;
                route.fallback_used = true;
                route.fallback_reason = Some(format!(
                    "{} was unavailable or failed, so Auto route retried with {}.",
                    primary_backend.label(),
                    fallback_backend.label()
                ));

                return Ok(TranscriptionResult {
                    text: fallback_result.text,
                    provider: fallback_result.provider,
                    route,
                });
            }
        }

        route.actual_backend = primary_backend.label().to_string();
        return Err(VoxioError::Transcription(format!(
            "Auto route selected {}, but no local backend completed transcription.",
            primary_backend.label()
        )));
    }

    Err(VoxioError::Transcription(
        "No local transcription backend is available.".to_string(),
    ))
}

fn transcribe_with_cloud_provider(
    wav_bytes: &[u8],
    settings: &Settings,
) -> Result<TranscriptionResult> {
    if let Some(result) = transcribe_with_openai_api(wav_bytes, settings)? {
        let detected_language = detect_language_from_text(&result.text, None);
        return Ok(TranscriptionResult {
            text: result.text,
            provider: result.provider,
            route: RouteDecision {
                requested_provider: "Cloud only".to_string(),
                effective_provider: "Cloud".to_string(),
                requested_backend: "Cloud".to_string(),
                actual_backend: "Cloud".to_string(),
                detected_language,
                fallback_used: false,
                fallback_reason: None,
            },
        });
    }

    Err(VoxioError::Transcription(
        "cloud transcription is not configured. Set OPENAI_API_KEY to enable it.".to_string(),
    ))
}

fn transcribe_with_whisper_family(
    wav_bytes: &[u8],
    settings: &Settings,
) -> Result<Option<BackendTranscription>> {
    if let Some(result) = transcribe_with_whisper_cli(wav_bytes, settings)? {
        return Ok(Some(result));
    }

    if let Some(result) = transcribe_with_whisper_server(wav_bytes, settings)? {
        return Ok(Some(result));
    }

    if detect_python_whisper_backend().ready {
        return transcribe_with_openai_whisper(wav_bytes, settings).map(Some);
    }

    Ok(None)
}

fn transcribe_with_whisper_server(
    wav_bytes: &[u8],
    settings: &Settings,
) -> Result<Option<BackendTranscription>> {
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
        .map_err(|error| {
            VoxioError::Transcription(format!("failed to build HTTP client: {error}"))
        })?;
    let audio_part = multipart::Part::bytes(wav_bytes.to_vec())
        .file_name("voxio-recording.wav")
        .mime_str("audio/wav")
        .map_err(|error| {
            VoxioError::Transcription(format!("failed to build audio part: {error}"))
        })?;
    let form = multipart::Form::new()
        .text("response_format", "json")
        .text(
            "language",
            settings.whisper_language().unwrap_or("auto").to_string(),
        )
        .part("file", audio_part);
    let form = if let Some(prompt) = settings.effective_transcription_prompt() {
        form.text("prompt", prompt)
    } else {
        form
    };

    let response = match client
        .post(format!(
            "http://127.0.0.1:{}/inference",
            whisper_server.port
        ))
        .multipart(form)
        .send()
    {
        Ok(response) => response,
        Err(_) => return Ok(None),
    };

    if !response.status().is_success() {
        return Ok(None);
    }

    let payload: WhisperServerResponse = response.json().map_err(|error| {
        VoxioError::Transcription(format!("invalid whisper-server response: {error}"))
    })?;

    Ok(Some(BackendTranscription {
        text: payload.text.trim().to_string(),
        provider: "whisper-server".to_string(),
        detected_language: None,
    }))
}

fn transcribe_with_whisper_cli(
    wav_bytes: &[u8],
    settings: &Settings,
) -> Result<Option<BackendTranscription>> {
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
    if let Some(prompt) = settings.effective_transcription_prompt() {
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

    Ok(Some(BackendTranscription {
        text: text.trim().to_string(),
        provider: "whisper-cli".to_string(),
        detected_language: None,
    }))
}

fn transcribe_with_openai_api(
    wav_bytes: &[u8],
    settings: &Settings,
) -> Result<Option<BackendTranscription>> {
    let api_key = match std::env::var("OPENAI_API_KEY") {
        Ok(value) if !value.trim().is_empty() => value,
        _ => return Ok(None),
    };
    let base_url = std::env::var("OPENAI_BASE_URL")
        .unwrap_or_else(|_| "https://api.openai.com/v1".to_string());

    let client = Client::builder()
        .timeout(Duration::from_secs(90))
        .build()
        .map_err(|error| {
            VoxioError::Transcription(format!("failed to build HTTP client: {error}"))
        })?;
    let audio_part = multipart::Part::bytes(wav_bytes.to_vec())
        .file_name("voxio-recording.wav")
        .mime_str("audio/wav")
        .map_err(|error| {
            VoxioError::Transcription(format!("failed to build audio part: {error}"))
        })?;
    let mut form = multipart::Form::new()
        .text("model", settings.openai_transcription_model().to_string())
        .text("response_format", "json".to_string())
        .part("file", audio_part);

    if let Some(language) = settings.whisper_language() {
        form = form.text("language", language.to_string());
    }
    if let Some(prompt) = settings.effective_transcription_prompt() {
        form = form.text("prompt", prompt);
    }

    let response = client
        .post(format!("{base_url}/audio/transcriptions"))
        .bearer_auth(api_key)
        .multipart(form)
        .send()
        .map_err(|error| {
            VoxioError::Transcription(format!("OpenAI transcription request failed: {error}"))
        })?;

    if !response.status().is_success() {
        return Err(VoxioError::Transcription(format!(
            "OpenAI transcription returned HTTP {}",
            response.status()
        )));
    }

    let payload: OpenAiTranscriptionResponse = response.json().map_err(|error| {
        VoxioError::Transcription(format!("invalid OpenAI transcription response: {error}"))
    })?;

    Ok(Some(BackendTranscription {
        text: payload.text.trim().to_string(),
        provider: settings.openai_transcription_model().to_string(),
        detected_language: None,
    }))
}

fn transcribe_with_sensevoice(wav_bytes: &[u8]) -> Result<Option<BackendTranscription>> {
    let coli_bin = std::env::var("VOXIO_COLI_BIN").unwrap_or_else(|_| "coli".to_string());
    if !command_is_available(&coli_bin) {
        return Ok(None);
    }

    let audio_path = write_temp_wav(wav_bytes)?;
    let output = Command::new(&coli_bin)
        .arg("asr")
        .arg("-j")
        .arg("--model")
        .arg("sensevoice")
        .arg(&audio_path)
        .output()
        .map_err(|error| {
            VoxioError::Transcription(format!("failed to launch coli `{coli_bin}`: {error}"))
        })?;
    let _ = fs::remove_file(&audio_path);

    if !output.status.success() {
        return Ok(None);
    }

    let payload: ColiAsrResponse = serde_json::from_slice(&output.stdout).map_err(|error| {
        VoxioError::Transcription(format!("invalid coli ASR response: {error}"))
    })?;
    let provider = payload
        .model
        .as_deref()
        .map(|model| format!("coli ({model})"))
        .unwrap_or_else(|| "coli (sensevoice)".to_string());

    Ok(Some(BackendTranscription {
        text: payload.text.trim().to_string(),
        provider,
        detected_language: payload.lang,
    }))
}

fn try_prewarm_provider(settings: &Settings) -> Result<()> {
    if settings.local_backend == LocalBackend::SenseVoice {
        return Ok(());
    }
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
) -> Result<BackendTranscription> {
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
    if let Some(prompt) = settings.effective_transcription_prompt() {
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

    Ok(BackendTranscription {
        text: text.trim().to_string(),
        provider: "openai-whisper".to_string(),
        detected_language: None,
    })
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

fn current_local_backend_label(
    settings: &Settings,
    whisper: &BackendStatus,
    sense_voice: &BackendStatus,
) -> String {
    match settings.local_backend {
        LocalBackend::Whisper => whisper.name.clone(),
        LocalBackend::SenseVoice => sense_voice.name.clone(),
        LocalBackend::Auto => resolve_auto_route_backend(settings, whisper, sense_voice)
            .map(|backend| backend.label().to_string())
            .unwrap_or_else(|| "Unavailable".to_string()),
    }
}

fn describe_local_strategy(settings: &Settings) -> String {
    match settings.local_backend {
        LocalBackend::Whisper => "Pinned: Whisper only".to_string(),
        LocalBackend::SenseVoice => "Pinned: SenseVoice only".to_string(),
        LocalBackend::Auto => match settings.language.as_str() {
            "zh" => "Auto route: Chinese -> SenseVoice first".to_string(),
            "en" => "Auto route: English -> Whisper first".to_string(),
            _ => "Auto route: Auto-detect prefers SenseVoice first, then Whisper".to_string(),
        },
    }
}

fn whisper_backend_status(settings: &Settings) -> BackendStatus {
    let whisper_cli_bin = std::env::var("VOXIO_WHISPER_CPP_BIN")
        .unwrap_or_else(|_| "/opt/homebrew/bin/whisper-cli".to_string());
    if Path::new(&whisper_cli_bin).exists() {
        if let Some(model_path) = resolve_whisper_cpp_model(settings) {
            let model_name = model_path
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("model");
            return BackendStatus {
                name: "Whisper".to_string(),
                ready: true,
                detail: format!("whisper-cli is ready with {model_name}."),
            };
        }
        return BackendStatus {
            name: "Whisper".to_string(),
            ready: false,
            detail: "Whisper CLI was found, but no complete local model is ready.".to_string(),
        };
    }

    let python_status = detect_python_whisper_backend();
    if python_status.ready {
        return python_status;
    }

    BackendStatus {
        name: "Whisper".to_string(),
        ready: false,
        detail: "Whisper backend is unavailable.".to_string(),
    }
}

fn sensevoice_backend_status() -> BackendStatus {
    let coli_bin = std::env::var("VOXIO_COLI_BIN").unwrap_or_else(|_| "coli".to_string());
    if command_is_available(&coli_bin) {
        BackendStatus {
            name: "SenseVoice".to_string(),
            ready: true,
            detail: "SenseVoice is ready through coli.".to_string(),
        }
    } else {
        BackendStatus {
            name: "SenseVoice".to_string(),
            ready: false,
            detail: "SenseVoice is unavailable. Install `@marswave/coli` to enable it.".to_string(),
        }
    }
}

fn cloud_backend_status(settings: &Settings) -> BackendStatus {
    let ready = std::env::var("OPENAI_API_KEY")
        .ok()
        .is_some_and(|value| !value.trim().is_empty());
    BackendStatus {
        name: "Cloud".to_string(),
        ready,
        detail: if ready {
            format!("{} is configured.", settings.openai_transcription_model())
        } else {
            "OPENAI_API_KEY is missing.".to_string()
        },
    }
}

fn detect_python_whisper_backend() -> BackendStatus {
    let whisper_bin = std::env::var("VOXIO_WHISPER_BIN").unwrap_or_else(|_| "whisper".to_string());
    if command_is_available(&whisper_bin) {
        BackendStatus {
            name: "Whisper".to_string(),
            ready: true,
            detail: "openai-whisper is available.".to_string(),
        }
    } else {
        BackendStatus {
            name: "Whisper".to_string(),
            ready: false,
            detail: "openai-whisper is unavailable.".to_string(),
        }
    }
}

fn requested_backend_label(backend: LocalBackend) -> &'static str {
    match backend {
        LocalBackend::Auto => "Auto route",
        LocalBackend::Whisper => "Whisper",
        LocalBackend::SenseVoice => "SenseVoice",
    }
}

fn requested_language_label(settings: &Settings) -> &'static str {
    match settings.language.as_str() {
        "zh" => "zh",
        "en" => "en",
        _ => "auto",
    }
}

fn resolve_auto_route_backend(
    settings: &Settings,
    whisper: &BackendStatus,
    sense_voice: &BackendStatus,
) -> Option<LocalBackendKind> {
    let ordered = match settings.language.as_str() {
        "en" => [LocalBackendKind::Whisper, LocalBackendKind::SenseVoice],
        _ => [LocalBackendKind::SenseVoice, LocalBackendKind::Whisper],
    };

    ordered.into_iter().find(|backend| match backend {
        LocalBackendKind::Whisper => whisper.ready,
        LocalBackendKind::SenseVoice => sense_voice.ready,
    })
}

fn fallback_backend(
    primary: LocalBackendKind,
    whisper: &BackendStatus,
    sense_voice: &BackendStatus,
) -> Option<LocalBackendKind> {
    match primary {
        LocalBackendKind::Whisper if sense_voice.ready => Some(LocalBackendKind::SenseVoice),
        LocalBackendKind::SenseVoice if whisper.ready => Some(LocalBackendKind::Whisper),
        _ => None,
    }
}

fn should_retry_with_fallback(
    primary: LocalBackendKind,
    detected_language: &str,
    text: &str,
) -> bool {
    if text.trim().is_empty() || text.trim().chars().count() < 2 {
        return true;
    }

    match primary {
        LocalBackendKind::SenseVoice => detected_language == "en",
        LocalBackendKind::Whisper => detected_language == "zh",
    }
}

fn run_fallback_backend(
    backend: LocalBackendKind,
    wav_bytes: &[u8],
    settings: &Settings,
) -> Result<Option<(BackendTranscription, String)>> {
    match backend {
        LocalBackendKind::SenseVoice => Ok(transcribe_with_sensevoice(wav_bytes)?.map(|result| {
            let language =
                detect_language_from_text(&result.text, result.detected_language.as_deref());
            (result, language)
        })),
        LocalBackendKind::Whisper => Ok(transcribe_with_whisper_family(wav_bytes, settings)?.map(
            |result| {
                let language = detect_language_from_text(&result.text, None);
                (result, language)
            },
        )),
    }
}

fn detect_language_from_text(text: &str, provider_language: Option<&str>) -> String {
    if let Some(language) = provider_language {
        let normalized = normalize_language_tag(language);
        if normalized != "auto" {
            return normalized.to_string();
        }
    }

    let mut han = 0usize;
    let mut latin = 0usize;
    for character in text.chars() {
        if ('\u{4E00}'..='\u{9FFF}').contains(&character) {
            han += 1;
        } else if character.is_ascii_alphabetic() {
            latin += 1;
        }
    }

    if han == 0 && latin == 0 {
        "unknown".to_string()
    } else if han > 0 && latin <= han * 4 {
        "zh".to_string()
    } else if han >= latin {
        "zh".to_string()
    } else {
        "en".to_string()
    }
}

fn normalize_language_tag(language: &str) -> &str {
    match language {
        "<|zh|>" | "zh" | "zh-cn" | "zh-CN" => "zh",
        "<|en|>" | "en" | "en-us" | "en-US" => "en",
        _ => "auto",
    }
}

#[derive(Debug, Deserialize)]
struct WhisperServerResponse {
    text: String,
}

#[derive(Debug, Deserialize)]
struct OpenAiTranscriptionResponse {
    text: String,
}

#[derive(Debug, Deserialize)]
struct ColiAsrResponse {
    text: String,
    model: Option<String>,
    lang: Option<String>,
}

#[derive(Debug, Clone)]
struct BackendTranscription {
    text: String,
    provider: String,
    detected_language: Option<String>,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{CloudModel, InjectionMode, ModelSize, Settings, TranscriptionProvider};

    fn make_settings(language: &str, local_backend: LocalBackend) -> Settings {
        Settings {
            hotkey: "Option+Space".to_string(),
            language: language.to_string(),
            local_backend,
            transcription_hint: String::new(),
            vocabulary_terms: String::new(),
            auto_punctuation: true,
            silence_timeout_ms: 1200,
            injection_mode: InjectionMode::Auto,
            transcription_provider: TranscriptionProvider::Local,
            cloud_model: CloudModel::Fast,
            model: ModelSize::Balanced,
            launch_at_login: false,
        }
    }

    fn backend_status(name: &str, ready: bool) -> BackendStatus {
        BackendStatus {
            name: name.to_string(),
            ready,
            detail: String::new(),
        }
    }

    #[test]
    fn detects_language_from_provider_tag_first() {
        assert_eq!(detect_language_from_text("hello 世界", Some("<|en|>")), "en");
        assert_eq!(detect_language_from_text("hello 世界", Some("<|zh|>")), "zh");
    }

    #[test]
    fn detects_language_from_text_mix() {
        assert_eq!(detect_language_from_text("今天 meeting", None), "zh");
        assert_eq!(detect_language_from_text("hello world", None), "en");
        assert_eq!(detect_language_from_text("1234", None), "unknown");
    }

    #[test]
    fn auto_route_prefers_sensevoice_for_chinese_and_auto() {
        let whisper = backend_status("Whisper", true);
        let sense_voice = backend_status("SenseVoice", true);

        assert_eq!(
            resolve_auto_route_backend(&make_settings("zh", LocalBackend::Auto), &whisper, &sense_voice),
            Some(LocalBackendKind::SenseVoice)
        );
        assert_eq!(
            resolve_auto_route_backend(&make_settings("auto", LocalBackend::Auto), &whisper, &sense_voice),
            Some(LocalBackendKind::SenseVoice)
        );
    }

    #[test]
    fn auto_route_prefers_whisper_for_english() {
        let whisper = backend_status("Whisper", true);
        let sense_voice = backend_status("SenseVoice", true);

        assert_eq!(
            resolve_auto_route_backend(&make_settings("en", LocalBackend::Auto), &whisper, &sense_voice),
            Some(LocalBackendKind::Whisper)
        );
    }

    #[test]
    fn auto_route_skips_unavailable_primary_backend() {
        let whisper = backend_status("Whisper", true);
        let sense_voice = backend_status("SenseVoice", false);

        assert_eq!(
            resolve_auto_route_backend(&make_settings("zh", LocalBackend::Auto), &whisper, &sense_voice),
            Some(LocalBackendKind::Whisper)
        );
    }

    #[test]
    fn retry_with_fallback_only_on_mismatch_or_empty_text() {
        assert!(should_retry_with_fallback(
            LocalBackendKind::SenseVoice,
            "en",
            "hello world"
        ));
        assert!(should_retry_with_fallback(
            LocalBackendKind::Whisper,
            "zh",
            "今天开会"
        ));
        assert!(should_retry_with_fallback(
            LocalBackendKind::SenseVoice,
            "unknown",
            ""
        ));

        assert!(!should_retry_with_fallback(
            LocalBackendKind::SenseVoice,
            "zh",
            "今天开会"
        ));
        assert!(!should_retry_with_fallback(
            LocalBackendKind::Whisper,
            "en",
            "hello world"
        ));
    }
}
