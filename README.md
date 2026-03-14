# Voxio

Voxio is a desktop voice typing app built with Tauri 2, React, TypeScript, and Rust. It is designed around fast local transcription first, with optional cloud transcription when a local path is unavailable or not preferred.

## Current capabilities

- global hotkey driven dictation flow
- local audio capture and transcription pipeline
- configurable transcription provider selection
- local backend selection for Whisper, SenseVoice, or automatic routing
- clipboard-based text injection
- persisted local settings
- runtime readiness checks for transcription backends and permissions

## Roadmap

- continuous microphone capture and VAD
- lower-latency streaming ASR sessions
- more native text insertion paths
- richer onboarding and permission guidance
- packaging and release automation

## Architecture

- `src/`: React frontend and settings UI
- `src-tauri/`: Rust backend, commands, and desktop integrations
- `models/whisper/`: local Whisper models directory (ignored by Git)
- `TECHNICAL_SOLUTION.md`: implementation notes and technical plan

## Transcription backends

Voxio currently supports:

- Whisper through `whisper-cli`
- Whisper through the `openai-whisper` Python CLI as a fallback local path
- SenseVoice through `coli`
- OpenAI cloud transcription when `OPENAI_API_KEY` is configured

In local `auto` mode, Voxio routes between Whisper and SenseVoice based on the requested language and the available runtime backend.

## Local models

Whisper model binaries are not tracked in this repository.

Place your local GGML model files under `models/whisper/` or point Voxio at a custom model path with environment variables.

The repository includes a helper script for common model downloads:

```bash
./scripts/install-whisper-model.sh balanced
```

Supported presets:

- `fast` -> `ggml-tiny-q5_1.bin`
- `balanced` -> `ggml-base-q5_1.bin`
- `small` -> `ggml-small.bin`

The script defaults to `models/whisper/` and will not overwrite an existing file unless you pass `--force`.

## Environment variables

Optional overrides:

```bash
export VOXIO_WHISPER_CPP_BIN=/custom/path/to/whisper-cli
export VOXIO_WHISPER_CPP_MODEL=/custom/path/to/ggml-model.bin
export VOXIO_WHISPER_BIN=/custom/path/to/whisper
export VOXIO_COLI_BIN=/custom/path/to/coli
export OPENAI_API_KEY=...
export OPENAI_BASE_URL=https://api.openai.com/v1
```

## Getting started

```bash
npm install
./scripts/install-whisper-model.sh balanced
npm run tauri dev
```

If you only need the web UI during development:

```bash
npm run dev
```

## Development checks

```bash
npm run build
cargo check --manifest-path src-tauri/Cargo.toml
```

## Repository hygiene

- Secrets, local environment files, generated artifacts, and partial model downloads should not be committed.
- A license file is not included yet. Choose one before treating the repository as a fully licensed open source project.

## First run

Use this order for the fastest local setup:

1. Grant microphone, accessibility, and input monitoring access.
2. Install a local backend:
   `whisper-cli` for Whisper or `npm install -g @marswave/coli` for SenseVoice.
3. Download a Whisper model with `./scripts/install-whisper-model.sh balanced`.
4. Open Voxio, keep `Local only + Auto route`, then run a provider test from the setup panel.

## Contributing

See [CONTRIBUTING.md](./CONTRIBUTING.md) for development expectations.

## Security

See [SECURITY.md](./SECURITY.md) for reporting guidance.
