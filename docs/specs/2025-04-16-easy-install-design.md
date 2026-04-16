# Easy Install for End Users

**Date:** 2025-04-16
**Goal:** Users install a `.deb`, open the app, and start using it — no manual model download, no compilation.

## 1. Auto-download Whisper model

### Behavior

When the app needs to transcribe (Recording or Dictation) and `ggml-small.bin` is not found in `~/.local/share/com.nuuvem.martin/models/`, it downloads the model from HuggingFace before proceeding.

This check happens at the moment of transcription, not at app startup. The app opens instantly; the download screen only appears when the user first tries to transcribe or dictate.

### Backend (Rust)

New module `src-tauri/src/model.rs`:

- `model_path() -> PathBuf` — returns `~/.local/share/com.nuuvem.martin/models/ggml-small.bin`
- `model_exists() -> bool` — checks if the file exists
- `download_model(app_handle) -> Result<PathBuf, String>` — downloads from `https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin` to a `.part` temp file, renames on completion. Emits progress events during download.
- `ensure_model(app_handle) -> Result<PathBuf, String>` — if exists, returns path; if not, calls `download_model`

Progress events emitted to frontend via Tauri event `model://download-progress`:
```json
{ "percent": 42, "downloaded_mb": 196.0, "total_mb": 466.0 }
```

Completion event: `model://download-complete`.
Error event: `model://download-error` with `{ "message": "..." }`.

### Frontend (Svelte)

New component `src/lib/ModelDownload.svelte`:

- Listens to `model://download-progress`, `model://download-complete`, `model://download-error`
- Shows a progress bar with percentage and MB downloaded/total
- On error: shows error message with a "Try again" button
- Centered overlay that blocks interaction until download finishes

### Integration

Before transcription starts (in `start_dictation` and the transcribe pending recording command), call `ensure_model()`. If the model needs downloading, the frontend shows the download overlay. Transcription proceeds only after download completes.

### Tauri commands

- `check_model_exists() -> bool` — frontend can check upfront to show UI hints
- `download_model()` — triggers download manually (used by "Try again")

### Edge cases

- **Download interrupted**: `.part` file stays on disk. Next attempt re-downloads (does not resume — simplicity over optimization for a one-time 466MB download).
- **Disk full**: download fails, error event emitted, user sees message.
- **No internet**: download fails immediately, user sees message.
- **Concurrent calls**: `ensure_model` should be idempotent — if download is already in progress, second call waits for it (use a Mutex or flag).

## 2. Declare system dependencies in `.deb`

### tauri.conf.json changes

Add to the `bundle` section:

```json
"linux": {
  "deb": {
    "depends": [
      "libwebkit2gtk-4.1-0",
      "libgtk-3-0",
      "libasound2",
      "libpulse0",
      "pipewire"
    ]
  }
}
```

This ensures `sudo apt install ./martin_0.1.0_amd64.deb` resolves all system dependencies automatically.

### Build and publish

Manual process:
1. `cargo tauri build`
2. `.deb` appears in `src-tauri/target/release/bundle/deb/`
3. Upload to GitHub Releases

## Out of scope

- GitHub Actions CI/CD
- AppImage builds
- Model selection UI (always `small`)
- Settings/preferences screen
- Partial download resume
