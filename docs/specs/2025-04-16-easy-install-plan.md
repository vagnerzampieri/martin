# Easy Install Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Users install a `.deb`, open the app, and start using — model downloads automatically on first transcription.

**Architecture:** New `model.rs` Rust module handles model existence check and HTTP download with progress events. New `ModelDownload.svelte` component shows a progress overlay. Existing commands (`transcribe_recording`, `start_dictation`) call `ensure_model()` before proceeding. Tauri config updated with `.deb` dependencies.

**Tech Stack:** Rust (reqwest for HTTP), Tauri events, Svelte 5

---

### Task 1: Add reqwest dependency

**Files:**
- Modify: `src-tauri/Cargo.toml`

- [ ] **Step 1: Add reqwest to Cargo.toml**

Add `reqwest` with `blocking` and `rustls-tls` features (no OpenSSL system dep needed):

```toml
reqwest = { version = "0.12", features = ["blocking", "rustls-tls"] }
```

Add after the `libc = "0.2"` line in `[dependencies]`.

- [ ] **Step 2: Verify it compiles**

Run: `cd src-tauri && cargo check`
Expected: compiles with no errors

- [ ] **Step 3: Commit**

```bash
git add src-tauri/Cargo.toml
git commit -m "feat: add reqwest dependency for model download"
```

---

### Task 2: Create model.rs with download and progress events

**Files:**
- Create: `src-tauri/src/model.rs`
- Modify: `src-tauri/src/lib.rs` (add `mod model;`)

- [ ] **Step 1: Write tests for model_path and model_exists**

Create `src-tauri/src/model.rs` with test module:

```rust
use std::path::{Path, PathBuf};

pub fn model_path(data_dir: &Path) -> PathBuf {
    data_dir.join("models").join("ggml-small.bin")
}

pub fn model_exists(data_dir: &Path) -> bool {
    model_path(data_dir).exists()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_path_returns_expected_path() {
        let dir = PathBuf::from("/home/user/.local/share/com.nuuvem.martin");
        let path = model_path(&dir);
        assert_eq!(path, dir.join("models").join("ggml-small.bin"));
    }

    #[test]
    fn model_exists_returns_false_when_missing() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(!model_exists(dir.path()));
    }

    #[test]
    fn model_exists_returns_true_when_present() {
        let dir = tempfile::tempdir().expect("tempdir");
        let models = dir.path().join("models");
        std::fs::create_dir_all(&models).unwrap();
        std::fs::write(models.join("ggml-small.bin"), b"fake").unwrap();
        assert!(model_exists(dir.path()));
    }
}
```

- [ ] **Step 2: Run tests to verify they pass**

Run: `cd src-tauri && cargo test model::tests`
Expected: 3 tests pass

- [ ] **Step 3: Add download_model function**

Add to `src-tauri/src/model.rs` above the tests module:

```rust
use serde::Serialize;
use tauri::{AppHandle, Emitter};

const MODEL_URL: &str =
    "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin";

#[derive(Clone, Serialize)]
pub struct DownloadProgress {
    pub percent: u8,
    pub downloaded_mb: f64,
    pub total_mb: f64,
}

/// Downloads the Whisper model to data_dir/models/ggml-small.bin.
/// Emits `model://download-progress` events during download.
/// Uses a .part temp file and renames on completion to avoid partial files.
pub fn download_model(data_dir: &Path, app_handle: &AppHandle) -> Result<PathBuf, String> {
    let dest = model_path(data_dir);
    let models_dir = dest.parent().unwrap();
    std::fs::create_dir_all(models_dir)
        .map_err(|e| format!("Failed to create models directory: {}", e))?;

    let part_path = dest.with_extension("bin.part");

    let response = reqwest::blocking::get(MODEL_URL)
        .map_err(|e| format!("Download failed: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("Download failed with status: {}", response.status()));
    }

    let total_bytes = response.content_length().unwrap_or(0);
    let total_mb = total_bytes as f64 / 1_048_576.0;

    let mut file = std::fs::File::create(&part_path)
        .map_err(|e| format!("Failed to create file: {}", e))?;

    let mut downloaded: u64 = 0;
    let mut last_percent: u8 = 0;
    let mut reader = std::io::BufReader::new(response);

    loop {
        use std::io::Read;
        let mut buf = [0u8; 65536];
        let n = reader.read(&mut buf).map_err(|e| format!("Read error: {}", e))?;
        if n == 0 {
            break;
        }

        use std::io::Write;
        file.write_all(&buf[..n])
            .map_err(|e| format!("Write error: {}", e))?;

        downloaded += n as u64;

        let percent = if total_bytes > 0 {
            ((downloaded as f64 / total_bytes as f64) * 100.0) as u8
        } else {
            0
        };

        if percent != last_percent {
            last_percent = percent;
            let _ = app_handle.emit("model://download-progress", DownloadProgress {
                percent,
                downloaded_mb: downloaded as f64 / 1_048_576.0,
                total_mb,
            });
        }
    }

    drop(file);
    std::fs::rename(&part_path, &dest)
        .map_err(|e| format!("Failed to rename downloaded file: {}", e))?;

    let _ = app_handle.emit("model://download-complete", ());

    Ok(dest)
}

/// Returns the model path if it exists, or downloads it first.
pub fn ensure_model(data_dir: &Path, app_handle: &AppHandle) -> Result<PathBuf, String> {
    if model_exists(data_dir) {
        return Ok(model_path(data_dir));
    }
    download_model(data_dir, app_handle)
}
```

- [ ] **Step 4: Add `mod model` to lib.rs**

In `src-tauri/src/lib.rs`, add `mod model;` after the existing module declarations (after `mod transcribe;` line area — keep them alphabetical):

```rust
mod audio;
mod db;
mod dictation;
mod model;
mod summarize;
mod transcribe;
```

- [ ] **Step 5: Verify it compiles**

Run: `cd src-tauri && cargo check`
Expected: compiles with no errors

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/model.rs src-tauri/src/lib.rs
git commit -m "feat: add model download module with progress events"
```

---

### Task 3: Add Tauri commands for model check and download

**Files:**
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: Add check_model_exists and download_model commands**

In `src-tauri/src/lib.rs`, add the import at the top with the other use statements:

```rust
use model::{ensure_model, model_exists};
```

Add two new Tauri commands before the `run()` function:

```rust
#[tauri::command]
fn check_model_exists(state: State<'_, AppState>) -> bool {
    model::model_exists(&state.data_dir)
}

#[tauri::command]
async fn download_whisper_model(
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    let data_dir = state.data_dir.clone();
    tauri::async_runtime::spawn_blocking(move || {
        model::download_model(&data_dir, &app_handle)
    })
    .await
    .map_err(|e| format!("Download task failed: {}", e))??;
    Ok(())
}
```

- [ ] **Step 2: Register the new commands in the invoke_handler**

In the `.invoke_handler(tauri::generate_handler![...])` call, add the two new commands:

```rust
check_model_exists,
download_whisper_model,
```

- [ ] **Step 3: Wire ensure_model into transcribe_recording**

In `transcribe_recording`, after getting the `pending` and before the model/transcriber section, add an `ensure_model` call. Replace the existing block that creates the transcriber:

```rust
let model_path = state.model_path.clone();
```

With:

```rust
let data_dir = state.data_dir.clone();
let app = app_handle.clone();
let model_path = tauri::async_runtime::spawn_blocking(move || {
    ensure_model(&data_dir, &app)
})
.await
.map_err(|e| format!("Model check failed: {}", e))??;
```

The `transcribe_recording` function signature needs to receive `app_handle`:

```rust
#[tauri::command]
async fn transcribe_recording(
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
    pending_id: i64,
    title: String,
    language: String,
) -> Result<Transcription, String> {
```

- [ ] **Step 4: Wire ensure_model into start_dictation**

In `start_dictation`, after the guard check and before creating the `DictationSession`, add:

```rust
let data_dir = state.data_dir.clone();
let app = app_handle.clone();
let _model_path = tauri::async_runtime::spawn_blocking(move || {
    ensure_model(&data_dir, &app)
})
.await
.map_err(|e| format!("Model check failed: {}", e))??;
```

- [ ] **Step 5: Verify it compiles**

Run: `cd src-tauri && cargo check`
Expected: compiles with no errors

- [ ] **Step 6: Run all tests**

Run: `cd src-tauri && cargo test`
Expected: all tests pass

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/lib.rs
git commit -m "feat: wire model auto-download into transcription commands"
```

---

### Task 4: Create ModelDownload.svelte component

**Files:**
- Create: `src/lib/ModelDownload.svelte`
- Modify: `src/lib/i18n.js`

- [ ] **Step 1: Add i18n keys**

In `src/lib/i18n.js`, add these keys to both `pt` and `en` objects:

In `pt` (after `dictating`):

```javascript
    downloadingModel: "Baixando modelo de transcrição...",
    downloadProgress: "Baixado",
    downloadError: "Falha no download",
    downloadRetry: "Tentar novamente",
    downloadOf: "de",
```

In `en` (after `dictating`):

```javascript
    downloadingModel: "Downloading transcription model...",
    downloadProgress: "Downloaded",
    downloadError: "Download failed",
    downloadRetry: "Try again",
    downloadOf: "of",
```

- [ ] **Step 2: Create ModelDownload.svelte**

Create `src/lib/ModelDownload.svelte`:

```svelte
<script>
    import { invoke } from "@tauri-apps/api/core";
    import { listen } from "@tauri-apps/api/event";
    import { onMount, onDestroy } from "svelte";
    import { t } from "./i18n.js";

    let { onComplete, onError } = $props();

    let percent = $state(0);
    let downloadedMb = $state(0);
    let totalMb = $state(0);
    let error = $state("");
    let downloading = $state(false);
    let unlistenProgress = null;
    let unlistenComplete = null;
    let unlistenError = null;

    onMount(async () => {
        unlistenProgress = await listen("model://download-progress", (event) => {
            percent = event.payload.percent;
            downloadedMb = event.payload.downloaded_mb;
            totalMb = event.payload.total_mb;
        });
        unlistenComplete = await listen("model://download-complete", () => {
            onComplete?.();
        });
        unlistenError = await listen("model://download-error", (event) => {
            error = event.payload.message;
        });
        startDownload();
    });

    onDestroy(() => {
        if (unlistenProgress) unlistenProgress();
        if (unlistenComplete) unlistenComplete();
        if (unlistenError) unlistenError();
    });

    async function startDownload() {
        try {
            error = "";
            downloading = true;
            await invoke("download_whisper_model");
        } catch (e) {
            error = String(e);
            downloading = false;
            onError?.(String(e));
        }
    }
</script>

<div class="overlay">
    <div class="card">
        <h2>{t("downloadingModel")}</h2>

        {#if error}
            <div class="error">{t("downloadError")}: {error}</div>
            <button class="btn-retry" onclick={startDownload}>
                {t("downloadRetry")}
            </button>
        {:else}
            <div class="progress-bar">
                <div class="progress-fill" style="width: {percent}%"></div>
            </div>
            <div class="progress-text">
                {t("downloadProgress")} {downloadedMb.toFixed(0)} {t("downloadOf")} {totalMb.toFixed(0)} MB ({percent}%)
            </div>
        {/if}
    </div>
</div>

<style>
    .overlay {
        position: fixed;
        inset: 0;
        background: rgba(0, 0, 0, 0.8);
        display: flex;
        align-items: center;
        justify-content: center;
        z-index: 100;
    }

    .card {
        background: var(--surface);
        border: 1px solid var(--border);
        border-radius: var(--radius);
        padding: 32px 40px;
        max-width: 420px;
        width: 90%;
        text-align: center;
    }

    h2 {
        font-size: 1.1rem;
        margin-bottom: 20px;
        color: var(--text);
    }

    .progress-bar {
        width: 100%;
        height: 8px;
        background: var(--border);
        border-radius: 4px;
        overflow: hidden;
        margin-bottom: 12px;
    }

    .progress-fill {
        height: 100%;
        background: var(--info);
        border-radius: 4px;
        transition: width 0.3s ease;
    }

    .progress-text {
        font-size: 0.85rem;
        color: var(--text-muted);
    }

    .error {
        color: var(--accent);
        background: rgba(233, 69, 96, 0.1);
        padding: 12px;
        border-radius: var(--radius);
        margin-bottom: 16px;
        font-size: 0.9rem;
    }

    .btn-retry {
        background: var(--info);
        color: white;
        padding: 10px 24px;
    }
</style>
```

- [ ] **Step 3: Verify no syntax errors**

Run: `npm run check`
Expected: no errors

- [ ] **Step 4: Commit**

```bash
git add src/lib/ModelDownload.svelte src/lib/i18n.js
git commit -m "feat: add ModelDownload component with progress overlay"
```

---

### Task 5: Integrate ModelDownload into +page.svelte

**Files:**
- Modify: `src/routes/+page.svelte`

- [ ] **Step 1: Add model check state and import**

In `src/routes/+page.svelte`, add the import and model state:

```svelte
<script>
    import "../styles/global.css";
    import { invoke } from "@tauri-apps/api/core";
    import { onMount } from "svelte";
    import { t } from "../lib/i18n.js";
    import Recorder from "../lib/Recorder.svelte";
    import Dictation from "../lib/Dictation.svelte";
    import History from "../lib/History.svelte";
    import TranscriptionView from "../lib/TranscriptionView.svelte";
    import ModelDownload from "../lib/ModelDownload.svelte";

    let currentView = $state("recorder");
    let selectedTranscription = $state(null);
    let modelReady = $state(true);
    let checkingModel = $state(true);

    onMount(async () => {
        try {
            modelReady = await invoke("check_model_exists");
        } catch {
            modelReady = false;
        }
        checkingModel = false;
    });

    function onModelDownloaded() {
        modelReady = true;
    }

    function showTranscription(transcription) {
        selectedTranscription = transcription;
        currentView = "view";
    }

    function showRecorder() {
        currentView = "recorder";
        selectedTranscription = null;
    }

    function showDictation() {
        currentView = "dictation";
        selectedTranscription = null;
    }

    function showHistory() {
        currentView = "history";
        selectedTranscription = null;
    }
</script>
```

- [ ] **Step 2: Add the ModelDownload overlay to the template**

Add the overlay right after the opening `<main>` tag and before `<header>`:

```svelte
<main>
    {#if !checkingModel && !modelReady}
        <ModelDownload onComplete={onModelDownloaded} />
    {/if}

    <header>
```

The rest of the template stays unchanged.

- [ ] **Step 3: Verify it compiles**

Run: `npm run check`
Expected: no errors

- [ ] **Step 4: Commit**

```bash
git add src/routes/+page.svelte
git commit -m "feat: show model download overlay on first launch"
```

---

### Task 6: Configure .deb system dependencies

**Files:**
- Modify: `src-tauri/tauri.conf.json`

- [ ] **Step 1: Add Linux deb depends**

In `src-tauri/tauri.conf.json`, replace the `"bundle"` section:

```json
"bundle": {
    "active": true,
    "targets": "all",
    "icon": [
        "icons/32x32.png",
        "icons/128x128.png",
        "icons/128x128@2x.png",
        "icons/icon.icns",
        "icons/icon.ico"
    ],
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
}
```

- [ ] **Step 2: Verify it compiles**

Run: `cd src-tauri && cargo check`
Expected: compiles with no errors

- [ ] **Step 3: Commit**

```bash
git add src-tauri/tauri.conf.json
git commit -m "feat: declare system dependencies in .deb package config"
```

---

### Task 7: Update README install instructions

**Files:**
- Modify: `README.md`

- [ ] **Step 1: Update the Install section**

Replace the current Install section with:

```markdown
## Install

### From .deb (recommended)

Download the latest `.deb` from [GitHub Releases](https://github.com/YOUR_USER/martin/releases) and install:

```bash
sudo apt install ./martin_0.1.0_amd64.deb
```

On first use, the app will automatically download the Whisper model (~466MB). Internet required only for this one-time download.

### From source

```bash
git clone https://github.com/YOUR_USER/martin.git
cd martin
npm install
cargo install tauri-cli --version "^2"
cargo tauri build
```

The binary will be in `src-tauri/target/release/martin`.
```

- [ ] **Step 2: Remove the manual model download step from the install section**

The `./scripts/download-model.sh small` line should not appear in install instructions anymore since the model downloads automatically. The script still exists for development use.

- [ ] **Step 3: Update the Architecture section**

Add `model.rs` to the architecture tree:

```
├── model.rs            # Auto-download Whisper model with progress events
```

And add `ModelDownload.svelte`:

```
│   ├── ModelDownload.svelte # Model download progress overlay
```

- [ ] **Step 4: Commit**

```bash
git add README.md
git commit -m "docs: update README with .deb install and auto-download"
```
