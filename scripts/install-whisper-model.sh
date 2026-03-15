#!/usr/bin/env sh

set -eu

usage() {
  cat <<'EOF'
Usage:
  ./scripts/install-whisper-model.sh [fast|balanced|small] [--force]

Examples:
  ./scripts/install-whisper-model.sh balanced
  ./scripts/install-whisper-model.sh fast --force

Environment:
  VOXIO_MODEL_DIR
    Override the destination directory. Defaults to <repo>/models/whisper.

  VOXIO_WHISPER_MODEL_BASE_URL
    Override the remote model base URL. Defaults to:
    https://huggingface.co/ggerganov/whisper.cpp/resolve/main
EOF
}

model="balanced"
force="0"

for argument in "$@"; do
  case "$argument" in
    fast|tiny)
      model="fast"
      ;;
    balanced|base)
      model="balanced"
      ;;
    small)
      model="small"
      ;;
    --force)
      force="1"
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown argument: $argument" >&2
      usage >&2
      exit 1
      ;;
  esac
done

case "$model" in
  fast)
    filename="ggml-tiny-q5_1.bin"
    ;;
  balanced)
    filename="ggml-base-q5_1.bin"
    ;;
  small)
    filename="ggml-small.bin"
    ;;
esac

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
destination_dir="${VOXIO_MODEL_DIR:-$repo_root/models/whisper}"
base_url="${VOXIO_WHISPER_MODEL_BASE_URL:-https://huggingface.co/ggerganov/whisper.cpp/resolve/main}"
destination_path="$destination_dir/$filename"
temporary_path="$destination_path.partial"

mkdir -p "$destination_dir"

if [ -f "$destination_path" ] && [ "$force" != "1" ]; then
  echo "Model already exists: $destination_path"
  echo "Use --force to re-download."
  exit 0
fi

download_url="$base_url/$filename"

cleanup() {
  rm -f "$temporary_path"
}

trap cleanup INT TERM EXIT

echo "Downloading $filename"
echo "Source: $download_url"
echo "Destination: $destination_path"

if command -v curl >/dev/null 2>&1; then
  curl -L --fail --progress-bar "$download_url" -o "$temporary_path"
elif command -v wget >/dev/null 2>&1; then
  wget -O "$temporary_path" "$download_url"
else
  echo "Neither curl nor wget is available." >&2
  exit 1
fi

mv "$temporary_path" "$destination_path"
trap - INT TERM EXIT

echo "Installed Whisper model at $destination_path"
