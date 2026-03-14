# Contributing to Voxio

Thanks for contributing.

## Before you start

- Use Node.js 20+ and a recent Rust toolchain.
- Install JavaScript dependencies with `npm install`.
- Install desktop prerequisites required by Tauri for your platform.
- On macOS, some runtime features also depend on system permissions such as microphone, accessibility, and input monitoring.

## Development workflow

1. Create a feature branch from `main`.
2. Keep changes focused and easy to review.
3. Update docs when behavior or setup changes.
4. Run the checks below before opening a pull request.

## Checks

```bash
npm run build
cargo check --manifest-path src-tauri/Cargo.toml
```

## Project structure

- `src/`: React UI and frontend state.
- `src-tauri/`: Rust backend, commands, and platform integration.
- `models/whisper/`: bundled local Whisper models used by the desktop app.

## Pull requests

- Describe the user-facing change.
- Mention platform assumptions or limitations.
- Include screenshots or recordings for UI changes when relevant.
- Call out any follow-up work that is intentionally left out.
