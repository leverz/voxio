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
- Placeholder Rust modules for hotkey, audio, ASR, and text injection

What is not implemented yet:

- Continuous microphone capture and VAD
- Whisper-based speech recognition
- Accessibility-native text injection
- Rich native permission requests and onboarding

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
