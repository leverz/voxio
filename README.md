# Voxio

Voxio is an open-source desktop voice typing tool. This repository currently contains the `v0.1` application shell based on `Tauri 2 + React + TypeScript`, with Rust-side state management and placeholder modules for hotkey registration, audio capture, ASR, and text injection.

## Status

What is implemented now:

- Desktop app shell and settings UI
- Dictation state machine skeleton
- Tauri commands and frontend event bridge
- Global shortcut registration through the Tauri shortcut plugin
- Clipboard-based text injection with simulated paste
- Local config persistence
- Local audio capture and offline transcription path

What is not implemented yet:

- Continuous microphone capture and VAD
- Streaming or persistent low-latency ASR session
- Accessibility-native text injection
- Rich native permission requests and onboarding

## ASR provider order

The app currently tries providers in this order:

1. `whisper-cpp` via `/opt/homebrew/bin/whisper-cli`
2. `openai-whisper` Python CLI as a fallback when no local GGML model is configured

The repository currently includes a lightweight local model at:

- `models/whisper/ggml-tiny-q5_1.bin`

You can override the `whisper-cpp` paths with:

```bash
export VOXIO_WHISPER_CPP_BIN=/custom/path/to/whisper-cli
export VOXIO_WHISPER_CPP_MODEL=/custom/path/to/ggml-model.bin
```

## Development

Install dependencies:

```bash
npm install
```

Run the frontend shell:

```bash
npm run dev
```

Run the desktop app:

```bash
npm run tauri dev
```

## Environment notes

The current machine now has `Rust/Cargo` installed and the Rust workspace passes `cargo check`.

The remaining native prerequisite reported by `tauri info` is full `Xcode`, which is still missing on this machine. `Xcode Command Line Tools` are present, but a full desktop app run may still be blocked until `Xcode` is installed.
