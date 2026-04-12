#!/bin/bash
set -euo pipefail

MODEL="${1:-small}"
MODELS_DIR="${2:-$HOME/.local/share/com.nuuvem.martin/models}"

VALID_MODELS="tiny base small medium"
if ! echo "$VALID_MODELS" | grep -qw "$MODEL"; then
    echo "Invalid model: $MODEL"
    echo "Valid models: $VALID_MODELS"
    exit 1
fi

mkdir -p "$MODELS_DIR"

URL="https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-${MODEL}.bin"
DEST="$MODELS_DIR/ggml-${MODEL}.bin"

if [ -f "$DEST" ]; then
    echo "Model already exists: $DEST"
    exit 0
fi

echo "Downloading ggml-${MODEL}.bin..."
wget -q --show-progress -O "$DEST" "$URL"
echo "Model saved to: $DEST"
