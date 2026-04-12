# Martin — Meeting Transcriber Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build an open-source, Linux-first desktop app that records meeting audio (system + mic), transcribes it locally using Whisper, and saves transcriptions to a local SQLite database.

**Architecture:** Tauri 2.0 app with a Rust backend handling audio capture (cpal) and transcription (whisper-rs), and a Svelte frontend for the UI. Audio is captured to a temporary WAV file, transcribed after the user stops recording, then the audio file is deleted. Transcriptions are stored in a local SQLite database.

**Tech Stack:** Tauri 2.0, Rust, Svelte, cpal (audio), whisper-rs (transcription), rusqlite (SQLite), hound (WAV encoding)

---

## Prerequisites — System Dependencies

Before starting, install the required system dev libraries. This only needs to happen once.

```bash
# Ubuntu/Debian
sudo apt install -y \
  libwebkit2gtk-4.1-dev \
  libgtk-3-dev \
  libasound2-dev \
  libpulse-dev \
  build-essential \
  curl \
  wget \
  file \
  libssl-dev \
  libayatana-appindicator3-dev \
  librsvg2-dev

# Install Tauri CLI
cargo install tauri-cli --version "^2"
```

---

## File Structure

```
martin/
├── src-tauri/
│   ├── Cargo.toml              # Rust dependencies
│   ├── tauri.conf.json         # Tauri config
│   ├── src/
│   │   ├── main.rs             # Tauri entry point
│   │   ├── lib.rs              # Module declarations
│   │   ├── audio/
│   │   │   ├── mod.rs          # Audio module exports
│   │   │   ├── capture.rs      # Audio capture (cpal, system + mic)
│   │   │   └── wav_writer.rs   # Write PCM samples to WAV file
│   │   ├── transcribe/
│   │   │   ├── mod.rs          # Transcribe module exports
│   │   │   └── whisper.rs      # Whisper.cpp integration
│   │   └── db/
│   │       ├── mod.rs          # DB module exports
│   │       └── store.rs        # SQLite storage for transcriptions
│   └── models/                 # Whisper models downloaded here
├── src/
│   ├── App.svelte              # Root Svelte component
│   ├── main.js                 # Svelte entry point
│   ├── lib/
│   │   ├── Recorder.svelte     # Start/stop recording UI
│   │   ├── TranscriptionView.svelte  # Display single transcription
│   │   └── History.svelte      # List past transcriptions
│   └── styles/
│       └── global.css          # Base styles
├── package.json                # Frontend dependencies
├── vite.config.js              # Vite config for Svelte
├── LICENSE                     # GPLv3
└── README.md                   # Project docs
```

---

## Task 1: Scaffold the Tauri + Svelte Project

**Files:**
- Create: entire project scaffold via `cargo tauri init`
- Create: `src/App.svelte`
- Create: `src/main.js`
- Create: `package.json`
- Create: `vite.config.js`

- [ ] **Step 1: Initialize the Tauri project with Svelte**

```bash
cd /home/nuuvem/Projects/study/martin
npm create tauri-app@latest . -- --template svelte --manager npm --yes
```

If the CLI asks questions interactively, use these values:
- Project name: `martin`
- Frontend: `Svelte`
- Package manager: `npm`

- [ ] **Step 2: Install frontend dependencies**

```bash
cd /home/nuuvem/Projects/study/martin
npm install
```

- [ ] **Step 3: Verify the scaffold builds**

```bash
cd /home/nuuvem/Projects/study/martin
cargo tauri build --debug 2>&1 | tail -5
```

Expected: build completes successfully, produces a binary.

If `cargo tauri` is not found, run `cargo install tauri-cli --version "^2"` first.

- [ ] **Step 4: Verify the app launches**

```bash
cd /home/nuuvem/Projects/study/martin
cargo tauri dev &
sleep 5
# A window should appear with the default Tauri + Svelte template
kill %1
```

Expected: a desktop window opens with default Svelte content.

- [ ] **Step 5: Initialize git and commit**

```bash
cd /home/nuuvem/Projects/study/martin
git init
echo "target/\nnode_modules/\ndist/\nsrc-tauri/models/" > .gitignore
git add .
git commit -m "feat: scaffold Tauri 2.0 + Svelte project"
```

---

## Task 2: Add Rust Dependencies

**Files:**
- Modify: `src-tauri/Cargo.toml`

- [ ] **Step 1: Add audio and transcription dependencies to Cargo.toml**

Add these dependencies to the `[dependencies]` section of `src-tauri/Cargo.toml`:

```toml
cpal = "0.15"
hound = "3.5"
whisper-rs = "0.14"
rusqlite = { version = "0.31", features = ["bundled"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
chrono = { version = "0.4", features = ["serde"] }
tempfile = "3"
```

Note: check the latest versions of `whisper-rs` and `cpal` on crates.io before adding. The versions above may need updating.

- [ ] **Step 2: Verify dependencies compile**

```bash
cd /home/nuuvem/Projects/study/martin/src-tauri
cargo check 2>&1 | tail -10
```

Expected: compiles with no errors (warnings are ok). `whisper-rs` will download and build whisper.cpp from source — this takes a few minutes the first time.

- [ ] **Step 3: Commit**

```bash
cd /home/nuuvem/Projects/study/martin
git add src-tauri/Cargo.toml src-tauri/Cargo.lock
git commit -m "feat: add audio, transcription, and storage dependencies"
```

---

## Task 3: Audio Capture Module

**Files:**
- Create: `src-tauri/src/audio/mod.rs`
- Create: `src-tauri/src/audio/capture.rs`
- Create: `src-tauri/src/audio/wav_writer.rs`
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: Create the audio module structure**

Create `src-tauri/src/audio/mod.rs`:

```rust
pub mod capture;
pub mod wav_writer;
```

- [ ] **Step 2: Implement WAV writer**

Create `src-tauri/src/audio/wav_writer.rs`:

```rust
use hound::{WavSpec, WavWriter};
use std::path::Path;
use std::sync::{Arc, Mutex};

pub struct AudioWavWriter {
    writer: Arc<Mutex<Option<WavWriter<std::io::BufWriter<std::fs::File>>>>>,
    spec: WavSpec,
}

impl AudioWavWriter {
    pub fn new(path: &Path, sample_rate: u32, channels: u16) -> Result<Self, String> {
        let spec = WavSpec {
            channels,
            sample_rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };

        let writer = WavWriter::create(path, spec)
            .map_err(|e| format!("Failed to create WAV file: {}", e))?;

        Ok(Self {
            writer: Arc::new(Mutex::new(Some(writer))),
            spec,
        })
    }

    pub fn writer_handle(&self) -> Arc<Mutex<Option<WavWriter<std::io::BufWriter<std::fs::File>>>>> {
        self.writer.clone()
    }

    pub fn spec(&self) -> WavSpec {
        self.spec
    }

    pub fn finalize(&self) -> Result<(), String> {
        let mut guard = self.writer.lock().map_err(|e| e.to_string())?;
        if let Some(writer) = guard.take() {
            writer.finalize().map_err(|e| format!("Failed to finalize WAV: {}", e))?;
        }
        Ok(())
    }
}
```

- [ ] **Step 3: Implement audio capture**

Create `src-tauri/src/audio/capture.rs`:

```rust
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, SampleFormat, Stream, StreamConfig};
use hound::WavWriter;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use super::wav_writer::AudioWavWriter;

pub struct AudioCapture {
    output_path: PathBuf,
    streams: Vec<Stream>,
    wav_writer: Option<AudioWavWriter>,
}

impl AudioCapture {
    pub fn new(output_path: PathBuf) -> Self {
        Self {
            output_path,
            streams: Vec::new(),
            wav_writer: None,
        }
    }

    pub fn start(&mut self) -> Result<(), String> {
        let host = cpal::default_host();

        // Get the default input device (microphone)
        let input_device = host
            .default_input_device()
            .ok_or("No input device available")?;

        let config = input_device
            .default_input_config()
            .map_err(|e| format!("Failed to get input config: {}", e))?;

        let sample_rate = config.sample_rate().0;
        let channels = config.channels();

        let wav_writer = AudioWavWriter::new(&self.output_path, sample_rate, channels)?;
        let writer_handle = wav_writer.writer_handle();

        let stream = match config.sample_format() {
            SampleFormat::I16 => self.build_stream_i16(&input_device, &config.into(), writer_handle),
            SampleFormat::F32 => self.build_stream_f32(&input_device, &config.into(), writer_handle),
            format => Err(format!("Unsupported sample format: {:?}", format)),
        }?;

        stream.play().map_err(|e| format!("Failed to play stream: {}", e))?;
        self.streams.push(stream);
        self.wav_writer = Some(wav_writer);

        Ok(())
    }

    pub fn stop(&mut self) -> Result<PathBuf, String> {
        // Drop streams to stop recording
        self.streams.clear();

        // Finalize the WAV file
        if let Some(ref writer) = self.wav_writer {
            writer.finalize()?;
        }
        self.wav_writer = None;

        Ok(self.output_path.clone())
    }

    fn build_stream_i16(
        &self,
        device: &Device,
        config: &StreamConfig,
        writer: Arc<Mutex<Option<WavWriter<std::io::BufWriter<std::fs::File>>>>>,
    ) -> Result<Stream, String> {
        let stream = device
            .build_input_stream(
                config,
                move |data: &[i16], _| {
                    if let Ok(mut guard) = writer.lock() {
                        if let Some(ref mut w) = *guard {
                            for &sample in data {
                                let _ = w.write_sample(sample);
                            }
                        }
                    }
                },
                |err| eprintln!("Audio stream error: {}", err),
                None,
            )
            .map_err(|e| format!("Failed to build input stream: {}", e))?;

        Ok(stream)
    }

    fn build_stream_f32(
        &self,
        device: &Device,
        config: &StreamConfig,
        writer: Arc<Mutex<Option<WavWriter<std::io::BufWriter<std::fs::File>>>>>,
    ) -> Result<Stream, String> {
        let stream = device
            .build_input_stream(
                config,
                move |data: &[f32], _| {
                    if let Ok(mut guard) = writer.lock() {
                        if let Some(ref mut w) = *guard {
                            for &sample in data {
                                let sample_i16 = (sample * i16::MAX as f32) as i16;
                                let _ = w.write_sample(sample_i16);
                            }
                        }
                    }
                },
                |err| eprintln!("Audio stream error: {}", err),
                None,
            )
            .map_err(|e| format!("Failed to build input stream: {}", e))?;

        Ok(stream)
    }
}
```

- [ ] **Step 4: Register the audio module in lib.rs**

Modify `src-tauri/src/lib.rs` to add:

```rust
mod audio;
```

- [ ] **Step 5: Verify it compiles**

```bash
cd /home/nuuvem/Projects/study/martin/src-tauri
cargo check 2>&1 | tail -10
```

Expected: compiles with no errors.

- [ ] **Step 6: Commit**

```bash
cd /home/nuuvem/Projects/study/martin
git add src-tauri/src/audio/
git add src-tauri/src/lib.rs
git commit -m "feat: add audio capture module with WAV recording"
```

---

## Task 4: Whisper Transcription Module

**Files:**
- Create: `src-tauri/src/transcribe/mod.rs`
- Create: `src-tauri/src/transcribe/whisper.rs`
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: Create the transcribe module structure**

Create `src-tauri/src/transcribe/mod.rs`:

```rust
pub mod whisper;
```

- [ ] **Step 2: Implement the whisper transcription**

Create `src-tauri/src/transcribe/whisper.rs`:

```rust
use std::path::Path;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

pub struct Transcriber {
    ctx: WhisperContext,
}

impl Transcriber {
    pub fn new(model_path: &Path) -> Result<Self, String> {
        let ctx = WhisperContext::new_with_params(
            model_path.to_str().ok_or("Invalid model path")?,
            WhisperContextParameters::default(),
        )
        .map_err(|e| format!("Failed to load Whisper model: {}", e))?;

        Ok(Self { ctx })
    }

    pub fn transcribe(&self, audio_path: &Path, language: &str) -> Result<String, String> {
        let samples = self.load_wav_as_mono_f32(audio_path)?;

        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_language(Some(language));
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(true);

        let mut state = self
            .ctx
            .create_state()
            .map_err(|e| format!("Failed to create state: {}", e))?;

        state
            .full(params, &samples)
            .map_err(|e| format!("Transcription failed: {}", e))?;

        let num_segments = state
            .full_n_segments()
            .map_err(|e| format!("Failed to get segments: {}", e))?;

        let mut text = String::new();
        for i in 0..num_segments {
            if let Ok(segment) = state.full_get_segment_text(i) {
                text.push_str(&segment);
                text.push('\n');
            }
        }

        Ok(text.trim().to_string())
    }

    fn load_wav_as_mono_f32(&self, path: &Path) -> Result<Vec<f32>, String> {
        let mut reader =
            hound::WavReader::open(path).map_err(|e| format!("Failed to open WAV: {}", e))?;

        let spec = reader.spec();
        let samples: Vec<f32> = match spec.sample_format {
            hound::SampleFormat::Int => reader
                .samples::<i16>()
                .filter_map(|s| s.ok())
                .map(|s| s as f32 / i16::MAX as f32)
                .collect(),
            hound::SampleFormat::Float => {
                reader.samples::<f32>().filter_map(|s| s.ok()).collect()
            }
        };

        // Convert to mono if stereo
        if spec.channels == 2 {
            Ok(samples
                .chunks(2)
                .map(|chunk| (chunk[0] + chunk.get(1).copied().unwrap_or(0.0)) / 2.0)
                .collect())
        } else {
            Ok(samples)
        }
    }
}
```

- [ ] **Step 3: Register the transcribe module in lib.rs**

Add to `src-tauri/src/lib.rs`:

```rust
mod transcribe;
```

- [ ] **Step 4: Verify it compiles**

```bash
cd /home/nuuvem/Projects/study/martin/src-tauri
cargo check 2>&1 | tail -10
```

Expected: compiles with no errors.

- [ ] **Step 5: Commit**

```bash
cd /home/nuuvem/Projects/study/martin
git add src-tauri/src/transcribe/
git add src-tauri/src/lib.rs
git commit -m "feat: add Whisper transcription module"
```

---

## Task 5: SQLite Storage Module

**Files:**
- Create: `src-tauri/src/db/mod.rs`
- Create: `src-tauri/src/db/store.rs`
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: Create the db module structure**

Create `src-tauri/src/db/mod.rs`:

```rust
pub mod store;
```

- [ ] **Step 2: Implement the storage layer**

Create `src-tauri/src/db/store.rs`:

```rust
use rusqlite::{params, Connection};
use serde::Serialize;
use std::path::Path;

#[derive(Debug, Serialize, Clone)]
pub struct Transcription {
    pub id: i64,
    pub title: String,
    pub text: String,
    pub language: String,
    pub duration_secs: f64,
    pub created_at: String,
}

pub struct Store {
    conn: Connection,
}

impl Store {
    pub fn new(db_path: &Path) -> Result<Self, String> {
        let conn =
            Connection::open(db_path).map_err(|e| format!("Failed to open database: {}", e))?;

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS transcriptions (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                title TEXT NOT NULL,
                text TEXT NOT NULL,
                language TEXT NOT NULL DEFAULT 'pt',
                duration_secs REAL NOT NULL DEFAULT 0.0,
                created_at TEXT NOT NULL DEFAULT (datetime('now', 'localtime'))
            );",
        )
        .map_err(|e| format!("Failed to create table: {}", e))?;

        Ok(Self { conn })
    }

    pub fn save(&self, title: &str, text: &str, language: &str, duration_secs: f64) -> Result<i64, String> {
        self.conn
            .execute(
                "INSERT INTO transcriptions (title, text, language, duration_secs) VALUES (?1, ?2, ?3, ?4)",
                params![title, text, language, duration_secs],
            )
            .map_err(|e| format!("Failed to save transcription: {}", e))?;

        Ok(self.conn.last_insert_rowid())
    }

    pub fn list(&self) -> Result<Vec<Transcription>, String> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, title, text, language, duration_secs, created_at FROM transcriptions ORDER BY created_at DESC")
            .map_err(|e| format!("Failed to prepare query: {}", e))?;

        let rows = stmt
            .query_map([], |row| {
                Ok(Transcription {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    text: row.get(2)?,
                    language: row.get(3)?,
                    duration_secs: row.get(4)?,
                    created_at: row.get(5)?,
                })
            })
            .map_err(|e| format!("Failed to query: {}", e))?;

        let mut transcriptions = Vec::new();
        for row in rows {
            transcriptions.push(row.map_err(|e| format!("Failed to read row: {}", e))?);
        }

        Ok(transcriptions)
    }

    pub fn get(&self, id: i64) -> Result<Transcription, String> {
        self.conn
            .query_row(
                "SELECT id, title, text, language, duration_secs, created_at FROM transcriptions WHERE id = ?1",
                params![id],
                |row| {
                    Ok(Transcription {
                        id: row.get(0)?,
                        title: row.get(1)?,
                        text: row.get(2)?,
                        language: row.get(3)?,
                        duration_secs: row.get(4)?,
                        created_at: row.get(5)?,
                    })
                },
            )
            .map_err(|e| format!("Transcription not found: {}", e))
    }

    pub fn delete(&self, id: i64) -> Result<(), String> {
        self.conn
            .execute("DELETE FROM transcriptions WHERE id = ?1", params![id])
            .map_err(|e| format!("Failed to delete: {}", e))?;
        Ok(())
    }
}
```

- [ ] **Step 3: Register the db module in lib.rs**

Add to `src-tauri/src/lib.rs`:

```rust
mod db;
```

- [ ] **Step 4: Verify it compiles**

```bash
cd /home/nuuvem/Projects/study/martin/src-tauri
cargo check 2>&1 | tail -10
```

Expected: compiles with no errors.

- [ ] **Step 5: Commit**

```bash
cd /home/nuuvem/Projects/study/martin
git add src-tauri/src/db/
git add src-tauri/src/lib.rs
git commit -m "feat: add SQLite storage module for transcriptions"
```

---

## Task 6: Tauri Commands (Bridge Rust to Frontend)

**Files:**
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/src/main.rs`

- [ ] **Step 1: Implement Tauri commands in lib.rs**

Replace the contents of `src-tauri/src/lib.rs` with:

```rust
mod audio;
mod db;
mod transcribe;

use audio::capture::AudioCapture;
use db::store::{Store, Transcription};
use transcribe::whisper::Transcriber;

use std::path::PathBuf;
use std::sync::Mutex;
use tauri::{AppHandle, Manager, State};

pub struct AppState {
    capture: Mutex<Option<AudioCapture>>,
    store: Store,
    model_path: PathBuf,
    data_dir: PathBuf,
}

#[tauri::command]
fn start_recording(state: State<'_, AppState>) -> Result<(), String> {
    let audio_path = state.data_dir.join("recording.wav");
    let mut capture = AudioCapture::new(audio_path);
    capture.start()?;

    let mut guard = state.capture.lock().map_err(|e| e.to_string())?;
    *guard = Some(capture);

    Ok(())
}

#[tauri::command]
fn stop_recording(state: State<'_, AppState>) -> Result<(), String> {
    let mut guard = state.capture.lock().map_err(|e| e.to_string())?;
    if let Some(mut capture) = guard.take() {
        capture.stop()?;
    }
    Ok(())
}

#[tauri::command]
fn transcribe_recording(state: State<'_, AppState>, title: String, language: String) -> Result<Transcription, String> {
    let audio_path = state.data_dir.join("recording.wav");

    if !audio_path.exists() {
        return Err("No recording found. Record a meeting first.".to_string());
    }

    let transcriber = Transcriber::new(&state.model_path)?;
    let text = transcriber.transcribe(&audio_path, &language)?;

    // Get audio duration from the WAV file
    let reader = hound::WavReader::open(&audio_path)
        .map_err(|e| format!("Failed to read WAV: {}", e))?;
    let spec = reader.spec();
    let duration_secs = reader.duration() as f64 / spec.sample_rate as f64;

    let id = state.store.save(&title, &text, &language, duration_secs)?;

    // Delete the audio file after transcription
    let _ = std::fs::remove_file(&audio_path);

    state.store.get(id)
}

#[tauri::command]
fn list_transcriptions(state: State<'_, AppState>) -> Result<Vec<Transcription>, String> {
    state.store.list()
}

#[tauri::command]
fn get_transcription(state: State<'_, AppState>, id: i64) -> Result<Transcription, String> {
    state.store.get(id)
}

#[tauri::command]
fn delete_transcription(state: State<'_, AppState>, id: i64) -> Result<(), String> {
    state.store.delete(id)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let data_dir = app
                .path()
                .app_data_dir()
                .expect("Failed to get app data dir");
            std::fs::create_dir_all(&data_dir).expect("Failed to create data dir");

            let db_path = data_dir.join("martin.db");
            let model_path = data_dir.join("models").join("ggml-small.bin");

            let store = Store::new(&db_path).expect("Failed to initialize database");

            app.manage(AppState {
                capture: Mutex::new(None),
                store,
                model_path,
                data_dir,
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            start_recording,
            stop_recording,
            transcribe_recording,
            list_transcriptions,
            get_transcription,
            delete_transcription,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
```

- [ ] **Step 2: Ensure main.rs calls lib::run()**

`src-tauri/src/main.rs` should contain:

```rust
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    martin_lib::run();
}
```

Note: replace `martin_lib` with whatever the crate name is in `Cargo.toml` (with hyphens replaced by underscores). Check the `[package] name` field — if it's `martin`, then use `martin::run()`. If it's `martin-app`, use `martin_app::run()`.

- [ ] **Step 3: Verify it compiles**

```bash
cd /home/nuuvem/Projects/study/martin/src-tauri
cargo check 2>&1 | tail -10
```

Expected: compiles with no errors.

- [ ] **Step 4: Commit**

```bash
cd /home/nuuvem/Projects/study/martin
git add src-tauri/src/lib.rs src-tauri/src/main.rs
git commit -m "feat: add Tauri commands bridging Rust backend to frontend"
```

---

## Task 7: Download Whisper Model Script

**Files:**
- Create: `scripts/download-model.sh`

- [ ] **Step 1: Create model download script**

Create `scripts/download-model.sh`:

```bash
#!/bin/bash
set -euo pipefail

MODEL="${1:-small}"
MODELS_DIR="${2:-$HOME/.local/share/com.martin.app/models}"

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
```

- [ ] **Step 2: Make it executable**

```bash
chmod +x /home/nuuvem/Projects/study/martin/scripts/download-model.sh
```

- [ ] **Step 3: Download the small model for testing**

```bash
cd /home/nuuvem/Projects/study/martin
./scripts/download-model.sh small
```

Expected: downloads `ggml-small.bin` (~466MB) to the models directory.

- [ ] **Step 4: Commit**

```bash
cd /home/nuuvem/Projects/study/martin
git add scripts/download-model.sh
git commit -m "feat: add Whisper model download script"
```

---

## Task 8: Svelte Frontend — Recorder Component

**Files:**
- Create: `src/lib/Recorder.svelte`
- Modify: `src/App.svelte`
- Create: `src/styles/global.css`

- [ ] **Step 1: Create global styles**

Create `src/styles/global.css`:

```css
:root {
    --bg: #1a1a2e;
    --surface: #16213e;
    --primary: #0f3460;
    --accent: #e94560;
    --text: #eee;
    --text-muted: #999;
    --border: #2a2a4a;
    --success: #4ade80;
    --radius: 8px;
}

* {
    margin: 0;
    padding: 0;
    box-sizing: border-box;
}

body {
    font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
    background: var(--bg);
    color: var(--text);
    min-height: 100vh;
}

button {
    cursor: pointer;
    border: none;
    border-radius: var(--radius);
    padding: 12px 24px;
    font-size: 1rem;
    font-weight: 600;
    transition: opacity 0.2s;
}

button:hover {
    opacity: 0.9;
}

button:disabled {
    opacity: 0.5;
    cursor: not-allowed;
}
```

- [ ] **Step 2: Create the Recorder component**

Create `src/lib/Recorder.svelte`:

```svelte
<script>
    import { invoke } from "@tauri-apps/api/core";

    let recording = false;
    let transcribing = false;
    let error = "";
    let elapsed = 0;
    let timer = null;

    async function startRecording() {
        try {
            error = "";
            await invoke("start_recording");
            recording = true;
            elapsed = 0;
            timer = setInterval(() => { elapsed += 1; }, 1000);
        } catch (e) {
            error = e;
        }
    }

    async function stopRecording() {
        try {
            clearInterval(timer);
            timer = null;
            await invoke("stop_recording");
            recording = false;
        } catch (e) {
            error = e;
        }
    }

    async function transcribe() {
        try {
            error = "";
            transcribing = true;
            const now = new Date().toLocaleString("pt-BR");
            const result = await invoke("transcribe_recording", {
                title: `Reuniao ${now}`,
                language: "pt",
            });
            transcribing = false;
            dispatch("transcribed", result);
        } catch (e) {
            error = e;
            transcribing = false;
        }
    }

    import { createEventDispatcher } from "svelte";
    const dispatch = createEventDispatcher();

    function formatTime(secs) {
        const m = Math.floor(secs / 60).toString().padStart(2, "0");
        const s = (secs % 60).toString().padStart(2, "0");
        return `${m}:${s}`;
    }
</script>

<div class="recorder">
    {#if recording}
        <div class="status recording">
            <span class="dot"></span>
            Gravando... {formatTime(elapsed)}
        </div>
        <button class="btn-stop" on:click={stopRecording}>
            Parar Gravacao
        </button>
    {:else if transcribing}
        <div class="status processing">
            Transcrevendo... isso pode levar alguns minutos.
        </div>
    {:else}
        <button class="btn-start" on:click={startRecording}>
            Iniciar Gravacao
        </button>
        <button class="btn-transcribe" on:click={transcribe}>
            Transcrever
        </button>
    {/if}

    {#if error}
        <div class="error">{error}</div>
    {/if}
</div>

<style>
    .recorder {
        display: flex;
        flex-direction: column;
        align-items: center;
        gap: 16px;
        padding: 32px;
    }

    .status {
        font-size: 1.2rem;
        display: flex;
        align-items: center;
        gap: 8px;
    }

    .recording .dot {
        width: 12px;
        height: 12px;
        background: var(--accent);
        border-radius: 50%;
        animation: pulse 1s infinite;
    }

    @keyframes pulse {
        0%, 100% { opacity: 1; }
        50% { opacity: 0.3; }
    }

    .btn-start {
        background: var(--accent);
        color: white;
        font-size: 1.3rem;
        padding: 16px 48px;
    }

    .btn-stop {
        background: var(--primary);
        color: white;
        font-size: 1.3rem;
        padding: 16px 48px;
    }

    .btn-transcribe {
        background: var(--success);
        color: #1a1a2e;
        padding: 12px 32px;
    }

    .processing {
        color: var(--text-muted);
    }

    .error {
        color: var(--accent);
        background: rgba(233, 69, 96, 0.1);
        padding: 12px 16px;
        border-radius: var(--radius);
        max-width: 400px;
        text-align: center;
    }
</style>
```

- [ ] **Step 3: Create the App component**

Replace `src/App.svelte` with:

```svelte
<script>
    import "./styles/global.css";
    import Recorder from "./lib/Recorder.svelte";

    let currentView = "recorder";
    let lastTranscription = null;

    function onTranscribed(event) {
        lastTranscription = event.detail;
        currentView = "result";
    }

    function backToRecorder() {
        currentView = "recorder";
        lastTranscription = null;
    }
</script>

<main>
    <h1>Martin</h1>
    <p class="subtitle">Transcritor de reunioes</p>

    {#if currentView === "recorder"}
        <Recorder on:transcribed={onTranscribed} />
    {:else if currentView === "result" && lastTranscription}
        <div class="result">
            <h2>{lastTranscription.title}</h2>
            <pre class="transcript">{lastTranscription.text}</pre>
            <button class="btn-back" on:click={backToRecorder}>
                Nova Gravacao
            </button>
        </div>
    {/if}
</main>

<style>
    main {
        max-width: 700px;
        margin: 0 auto;
        padding: 40px 20px;
        text-align: center;
    }

    h1 {
        font-size: 2.5rem;
        margin-bottom: 4px;
    }

    .subtitle {
        color: var(--text-muted);
        margin-bottom: 40px;
    }

    .result {
        text-align: left;
        padding: 20px;
    }

    .result h2 {
        margin-bottom: 16px;
    }

    .transcript {
        background: var(--surface);
        border: 1px solid var(--border);
        border-radius: var(--radius);
        padding: 20px;
        white-space: pre-wrap;
        word-wrap: break-word;
        font-family: inherit;
        font-size: 0.95rem;
        line-height: 1.6;
        max-height: 60vh;
        overflow-y: auto;
        margin-bottom: 20px;
    }

    .btn-back {
        background: var(--primary);
        color: white;
    }
</style>
```

- [ ] **Step 4: Verify frontend builds**

```bash
cd /home/nuuvem/Projects/study/martin
npm run build 2>&1 | tail -5
```

Expected: builds with no errors.

- [ ] **Step 5: Commit**

```bash
cd /home/nuuvem/Projects/study/martin
git add src/lib/Recorder.svelte src/App.svelte src/styles/global.css
git commit -m "feat: add recorder UI with start/stop and transcription trigger"
```

---

## Task 9: Svelte Frontend — History Component

**Files:**
- Create: `src/lib/History.svelte`
- Create: `src/lib/TranscriptionView.svelte`
- Modify: `src/App.svelte`

- [ ] **Step 1: Create the History component**

Create `src/lib/History.svelte`:

```svelte
<script>
    import { invoke } from "@tauri-apps/api/core";
    import { onMount, createEventDispatcher } from "svelte";

    const dispatch = createEventDispatcher();
    let transcriptions = [];
    let loading = true;

    onMount(async () => {
        try {
            transcriptions = await invoke("list_transcriptions");
        } catch (e) {
            console.error(e);
        }
        loading = false;
    });

    function select(t) {
        dispatch("select", t);
    }

    async function remove(id) {
        await invoke("delete_transcription", { id });
        transcriptions = transcriptions.filter((t) => t.id !== id);
    }

    function formatDate(dateStr) {
        return new Date(dateStr).toLocaleString("pt-BR");
    }

    function formatDuration(secs) {
        const m = Math.floor(secs / 60);
        const s = Math.round(secs % 60);
        return `${m}min ${s}s`;
    }
</script>

<div class="history">
    <h2>Historico</h2>

    {#if loading}
        <p class="muted">Carregando...</p>
    {:else if transcriptions.length === 0}
        <p class="muted">Nenhuma transcricao ainda.</p>
    {:else}
        <ul>
            {#each transcriptions as t}
                <li>
                    <button class="item" on:click={() => select(t)}>
                        <span class="title">{t.title}</span>
                        <span class="meta">
                            {formatDate(t.created_at)} · {formatDuration(t.duration_secs)}
                        </span>
                    </button>
                    <button class="delete" on:click|stopPropagation={() => remove(t.id)}>
                        ×
                    </button>
                </li>
            {/each}
        </ul>
    {/if}
</div>

<style>
    .history {
        text-align: left;
        padding: 20px;
    }

    h2 {
        margin-bottom: 16px;
    }

    .muted {
        color: var(--text-muted);
    }

    ul {
        list-style: none;
    }

    li {
        display: flex;
        align-items: center;
        border: 1px solid var(--border);
        border-radius: var(--radius);
        margin-bottom: 8px;
    }

    .item {
        flex: 1;
        background: var(--surface);
        color: var(--text);
        text-align: left;
        padding: 12px 16px;
        border-radius: var(--radius) 0 0 var(--radius);
        display: flex;
        flex-direction: column;
        gap: 4px;
    }

    .item:hover {
        background: var(--primary);
    }

    .title {
        font-weight: 600;
    }

    .meta {
        font-size: 0.85rem;
        color: var(--text-muted);
    }

    .delete {
        background: transparent;
        color: var(--accent);
        padding: 12px 16px;
        font-size: 1.2rem;
        border-radius: 0 var(--radius) var(--radius) 0;
    }

    .delete:hover {
        background: rgba(233, 69, 96, 0.2);
    }
</style>
```

- [ ] **Step 2: Create the TranscriptionView component**

Create `src/lib/TranscriptionView.svelte`:

```svelte
<script>
    import { createEventDispatcher } from "svelte";
    const dispatch = createEventDispatcher();

    export let transcription;

    function back() {
        dispatch("back");
    }

    function copyToClipboard() {
        navigator.clipboard.writeText(transcription.text);
    }
</script>

<div class="view">
    <div class="header">
        <button class="btn-back" on:click={back}>← Voltar</button>
        <button class="btn-copy" on:click={copyToClipboard}>Copiar texto</button>
    </div>

    <h2>{transcription.title}</h2>
    <p class="meta">{transcription.created_at} · {transcription.language}</p>

    <pre class="transcript">{transcription.text}</pre>
</div>

<style>
    .view {
        text-align: left;
        padding: 20px;
    }

    .header {
        display: flex;
        justify-content: space-between;
        margin-bottom: 16px;
    }

    .btn-back {
        background: var(--primary);
        color: white;
        padding: 8px 16px;
    }

    .btn-copy {
        background: var(--surface);
        color: var(--text);
        border: 1px solid var(--border);
        padding: 8px 16px;
    }

    h2 {
        margin-bottom: 4px;
    }

    .meta {
        color: var(--text-muted);
        font-size: 0.85rem;
        margin-bottom: 16px;
    }

    .transcript {
        background: var(--surface);
        border: 1px solid var(--border);
        border-radius: var(--radius);
        padding: 20px;
        white-space: pre-wrap;
        word-wrap: break-word;
        font-family: inherit;
        font-size: 0.95rem;
        line-height: 1.6;
        max-height: 60vh;
        overflow-y: auto;
    }
</style>
```

- [ ] **Step 3: Update App.svelte with navigation**

Replace `src/App.svelte` with:

```svelte
<script>
    import "./styles/global.css";
    import Recorder from "./lib/Recorder.svelte";
    import History from "./lib/History.svelte";
    import TranscriptionView from "./lib/TranscriptionView.svelte";

    let currentView = "recorder";
    let selectedTranscription = null;

    function onTranscribed(event) {
        selectedTranscription = event.detail;
        currentView = "view";
    }

    function onSelect(event) {
        selectedTranscription = event.detail;
        currentView = "view";
    }

    function showRecorder() {
        currentView = "recorder";
        selectedTranscription = null;
    }

    function showHistory() {
        currentView = "history";
        selectedTranscription = null;
    }
</script>

<main>
    <header>
        <h1>Martin</h1>
        <nav>
            <button class:active={currentView === "recorder"} on:click={showRecorder}>
                Gravar
            </button>
            <button class:active={currentView === "history"} on:click={showHistory}>
                Historico
            </button>
        </nav>
    </header>

    {#if currentView === "recorder"}
        <Recorder on:transcribed={onTranscribed} />
    {:else if currentView === "history"}
        <History on:select={onSelect} />
    {:else if currentView === "view" && selectedTranscription}
        <TranscriptionView
            transcription={selectedTranscription}
            on:back={showHistory}
        />
    {/if}
</main>

<style>
    main {
        max-width: 700px;
        margin: 0 auto;
        padding: 20px;
    }

    header {
        text-align: center;
        margin-bottom: 32px;
    }

    h1 {
        font-size: 2rem;
        margin-bottom: 12px;
    }

    nav {
        display: flex;
        justify-content: center;
        gap: 8px;
    }

    nav button {
        background: var(--surface);
        color: var(--text-muted);
        border: 1px solid var(--border);
        padding: 8px 20px;
    }

    nav button.active {
        background: var(--primary);
        color: var(--text);
        border-color: var(--primary);
    }
</style>
```

- [ ] **Step 4: Verify frontend builds**

```bash
cd /home/nuuvem/Projects/study/martin
npm run build 2>&1 | tail -5
```

Expected: builds with no errors.

- [ ] **Step 5: Commit**

```bash
cd /home/nuuvem/Projects/study/martin
git add src/lib/History.svelte src/lib/TranscriptionView.svelte src/App.svelte
git commit -m "feat: add history list and transcription view components"
```

---

## Task 10: Full Build and Manual Test

**Files:** None (testing only)

- [ ] **Step 1: Ensure the Whisper model is downloaded**

```bash
./scripts/download-model.sh small
```

- [ ] **Step 2: Build the full app**

```bash
cd /home/nuuvem/Projects/study/martin
cargo tauri build --debug 2>&1 | tail -20
```

Expected: builds successfully, outputs a binary path.

- [ ] **Step 3: Launch and test manually**

```bash
cargo tauri dev
```

Manual test checklist:
1. App window opens with "Martin" title and two nav buttons
2. Click "Iniciar Gravacao" — recording starts, timer counts up
3. Speak for ~10 seconds
4. Click "Parar Gravacao" — recording stops
5. Click "Transcrever" — transcription runs (takes 1-3 minutes on CPU)
6. Transcription text appears on screen
7. Click "Historico" — the transcription shows in the list
8. Click on the transcription — full text is displayed
9. Click "Copiar texto" — text is copied to clipboard

- [ ] **Step 4: Commit any fixes needed**

```bash
cd /home/nuuvem/Projects/study/martin
git add -A
git commit -m "fix: adjustments from manual testing"
```

---

## Task 11: Add LICENSE and README

**Files:**
- Create: `LICENSE`
- Create: `README.md`

- [ ] **Step 1: Add GPLv3 license**

```bash
cd /home/nuuvem/Projects/study/martin
wget -qO LICENSE https://www.gnu.org/licenses/gpl-3.0.txt
```

- [ ] **Step 2: Create README.md**

Create `README.md`:

```markdown
# Martin

Meeting transcriber for Linux. Record your meetings, transcribe locally with Whisper, keep your data on your machine.

## Features

- Record system audio and microphone
- Local transcription using Whisper (no internet needed)
- Portuguese and English support
- Transcription history with search
- Audio files deleted after transcription (privacy first)

## Install

### Prerequisites

- Rust 1.70+
- Node.js 18+
- System dependencies:

```bash
# Ubuntu/Debian
sudo apt install -y libwebkit2gtk-4.1-dev libgtk-3-dev libasound2-dev libpulse-dev build-essential libssl-dev libayatana-appindicator3-dev librsvg2-dev
```

### Build

```bash
git clone https://github.com/YOUR_USER/martin.git
cd martin
npm install
cargo install tauri-cli --version "^2"
./scripts/download-model.sh small
cargo tauri build
```

The binary will be in `src-tauri/target/release/martin`.

## Usage

1. Open Martin
2. Click **Iniciar Gravacao** before your meeting
3. Have your meeting
4. Click **Parar Gravacao** when done
5. Click **Transcrever** — wait for local transcription
6. Done. Text is saved, audio is deleted.

## Whisper Models

| Model | Size | Quality | Speed |
|-------|------|---------|-------|
| tiny | 75MB | Usable | Fast |
| base | 142MB | Good | Fast |
| small | 466MB | Very good | Moderate |
| medium | 1.5GB | Excellent | Slow |

Download with: `./scripts/download-model.sh <model>`

## License

GPLv3
```

- [ ] **Step 3: Commit**

```bash
cd /home/nuuvem/Projects/study/martin
git add LICENSE README.md
git commit -m "docs: add GPLv3 license and README"
```

---

## Summary

| Task | What it does |
|------|--------------|
| 1 | Scaffold Tauri + Svelte project |
| 2 | Add Rust dependencies |
| 3 | Audio capture module (cpal + WAV) |
| 4 | Whisper transcription module |
| 5 | SQLite storage module |
| 6 | Tauri commands (bridge Rust ↔ Svelte) |
| 7 | Whisper model download script |
| 8 | Recorder UI component |
| 9 | History and transcription view UI |
| 10 | Full build and manual test |
| 11 | LICENSE and README |
