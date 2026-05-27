# Import External Audio — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let a user pick an existing audio file (mp3/m4a/wav/ogg/flac) and transcribe it through Martin's existing pending-recordings flow.

**Architecture:** A new `audio/import.rs` module uses Symphonia (pure Rust) to stream-decode the chosen file into a mono WAV in `data_dir`, then a new `import_audio_file` Tauri command saves it as a pending recording. Everything downstream (transcription, WAV loading, cleanup) is reused unchanged. The frontend adds an "Import audio" button + native file picker to `Recorder.svelte`.

**Tech Stack:** Rust, Tauri 2, Symphonia 0.5, hound, tauri-plugin-dialog, Svelte 5.

**Spec:** `docs/superpowers/specs/2026-05-27-import-external-audio-design.md`

---

## File Structure

- Create: `src-tauri/src/audio/import.rs` — Symphonia decode → mono WAV (the only place Symphonia is used).
- Modify: `src-tauri/src/audio/mod.rs` — register the `import` module.
- Modify: `src-tauri/Cargo.toml` — add `symphonia`, `tauri-plugin-dialog`.
- Modify: `src-tauri/src/lib.rs` — add `import_audio_file` command, register dialog plugin + command.
- Modify: `src-tauri/capabilities/default.json` — allow `dialog:allow-open`.
- Modify: `package.json` — add `@tauri-apps/plugin-dialog`.
- Modify: `src/lib/i18n.js` — new strings (pt + en).
- Modify: `src/lib/Recorder.svelte` — import button + handler + busy state.

Testing note: automated tests live as `#[cfg(test)]` modules inside the Rust files (matching the repo's existing pattern, e.g. `whisper.rs`). The frontend has no Vitest setup; frontend behavior is verified manually (Task 9) — standing up Vitest is out of scope for this feature.

---

## Task 1: Add dependencies and register the dialog plugin

**Files:**
- Modify: `src-tauri/Cargo.toml`
- Modify: `src-tauri/src/lib.rs:639-641` (Tauri builder)
- Modify: `src-tauri/capabilities/default.json`
- Modify: `package.json`

- [ ] **Step 1: Add Rust dependencies**

In `src-tauri/Cargo.toml`, under `[dependencies]`, add:

```toml
symphonia = { version = "0.5", features = ["mp3", "isomp4", "aac", "flac", "ogg", "vorbis", "pcm", "wav"] }
tauri-plugin-dialog = "2"
```

(`isomp4` + `aac` cover m4a; `pcm` + `wav` cover wav and let the test fixtures decode.)

- [ ] **Step 2: Register the dialog plugin**

In `src-tauri/src/lib.rs`, add the plugin to the builder chain right after the existing opener plugin (around line 640):

```rust
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
```

- [ ] **Step 3: Grant the open-file permission**

In `src-tauri/capabilities/default.json`, add `"dialog:allow-open"` to the `permissions` array:

```json
  "permissions": [
    "core:default",
    "opener:default",
    "dialog:allow-open"
  ]
```

- [ ] **Step 4: Add the frontend dialog dependency**

Run: `npm install @tauri-apps/plugin-dialog@^2`
Expected: `package.json` gains `"@tauri-apps/plugin-dialog": "^2"` and install succeeds.

- [ ] **Step 5: Verify the project still builds**

Run: `cd src-tauri && cargo build`
Expected: compiles successfully (new crates downloaded, no errors).

- [ ] **Step 6: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/src/lib.rs src-tauri/capabilities/default.json package.json package-lock.json
git commit -m "build: add symphonia and tauri-plugin-dialog for audio import"
```

---

## Task 2: Pure helper — downmix interleaved samples to mono

**Files:**
- Create: `src-tauri/src/audio/import.rs`
- Modify: `src-tauri/src/audio/mod.rs`

- [ ] **Step 1: Register the module**

In `src-tauri/src/audio/mod.rs`, add:

```rust
pub mod capture;
pub mod import;
pub mod mix;
pub mod wav_writer;
```

- [ ] **Step 2: Write the failing test**

Create `src-tauri/src/audio/import.rs` with only the test module and a stub:

```rust
fn downmix_to_mono(interleaved: &[f32], channels: usize) -> Vec<f32> {
    unimplemented!()
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPSILON: f32 = 0.0001;

    fn assert_close(a: f32, b: f32) {
        assert!((a - b).abs() < EPSILON, "expected {b}, got {a}");
    }

    #[test]
    fn downmix_stereo_averages_channels() {
        // frames: (1.0,3.0) -> 2.0 ; (-2.0,4.0) -> 1.0
        let out = downmix_to_mono(&[1.0, 3.0, -2.0, 4.0], 2);
        assert_eq!(out.len(), 2);
        assert_close(out[0], 2.0);
        assert_close(out[1], 1.0);
    }

    #[test]
    fn downmix_quad_averages_four_channels() {
        let out = downmix_to_mono(&[1.0, 1.0, 1.0, 1.0], 4);
        assert_eq!(out.len(), 1);
        assert_close(out[0], 1.0);
    }

    #[test]
    fn downmix_mono_is_passthrough() {
        let out = downmix_to_mono(&[0.5, -0.5, 0.25], 1);
        assert_eq!(out, vec![0.5, -0.5, 0.25]);
    }
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cd src-tauri && cargo test --lib audio::import::tests::downmix`
Expected: FAIL (panics on `unimplemented!()`).

- [ ] **Step 4: Implement `downmix_to_mono`**

Replace the stub with:

```rust
fn downmix_to_mono(interleaved: &[f32], channels: usize) -> Vec<f32> {
    if channels <= 1 {
        return interleaved.to_vec();
    }
    interleaved
        .chunks(channels)
        .map(|frame| frame.iter().sum::<f32>() / frame.len() as f32)
        .collect()
}
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cd src-tauri && cargo test --lib audio::import::tests::downmix`
Expected: PASS (3 tests).

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/audio/mod.rs src-tauri/src/audio/import.rs
git commit -m "feat(import): add downmix-to-mono helper"
```

---

## Task 3: Pure helper — convert f32 sample to clamped i16

**Files:**
- Modify: `src-tauri/src/audio/import.rs`

- [ ] **Step 1: Write the failing test**

Add the stub above the test module:

```rust
fn f32_to_i16(sample: f32) -> i16 {
    unimplemented!()
}
```

Add these tests inside the existing `mod tests`:

```rust
    #[test]
    fn f32_to_i16_maps_full_scale() {
        assert_eq!(f32_to_i16(1.0), i16::MAX);
        assert_eq!(f32_to_i16(-1.0), -i16::MAX);
        assert_eq!(f32_to_i16(0.0), 0);
    }

    #[test]
    fn f32_to_i16_clamps_out_of_range() {
        assert_eq!(f32_to_i16(2.0), i16::MAX);
        assert_eq!(f32_to_i16(-2.0), -i16::MAX);
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd src-tauri && cargo test --lib audio::import::tests::f32_to_i16`
Expected: FAIL (panics on `unimplemented!()`).

- [ ] **Step 3: Implement `f32_to_i16`**

```rust
fn f32_to_i16(sample: f32) -> i16 {
    (sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd src-tauri && cargo test --lib audio::import::tests::f32_to_i16`
Expected: PASS (2 tests).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/audio/import.rs
git commit -m "feat(import): add clamped f32-to-i16 conversion"
```

---

## Task 4: `import_audio` — stream-decode any supported file into a mono WAV

**Files:**
- Modify: `src-tauri/src/audio/import.rs`

- [ ] **Step 1: Write the failing test**

Add this test to `mod tests`. It generates a stereo 16 kHz WAV in a temp dir, then imports it — exercising the full Symphonia probe → decode → downmix → write → duration pipeline (Symphonia decodes WAV via the `wav`/`pcm` features).

```rust
    fn write_stereo_wav(path: &std::path::Path, sample_rate: u32, frames: usize) {
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(path, spec).unwrap();
        for _ in 0..frames {
            w.write_sample(1000_i16).unwrap(); // L
            w.write_sample(3000_i16).unwrap(); // R
        }
        w.finalize().unwrap();
    }

    #[test]
    fn import_stereo_wav_produces_mono_wav_with_matching_duration() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("source.wav");
        write_stereo_wav(&src, 16000, 16000); // 1.0s of stereo audio

        let imported = import_audio(&src, dir.path()).expect("import failed");

        assert!(imported.wav_path.exists());
        assert!((imported.duration_secs - 1.0).abs() < 0.05);

        let reader = hound::WavReader::open(&imported.wav_path).unwrap();
        assert_eq!(reader.spec().channels, 1, "output must be mono");
        assert_eq!(reader.spec().sample_rate, 16000);
        assert_eq!(reader.duration(), 16000, "one mono frame per source frame");
    }

    #[test]
    fn import_missing_file_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("does_not_exist.wav");
        assert!(import_audio(&src, dir.path()).is_err());
    }
```

- [ ] **Step 2: Add the public API stub + imports**

At the top of `src-tauri/src/audio/import.rs`, add the imports and stub (above the helpers):

```rust
use std::fs::File;
use std::io::BufWriter;
use std::path::{Path, PathBuf};

use hound::{SampleFormat, WavSpec, WavWriter};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

use crate::transcribe::whisper::wav_duration_secs;

pub struct Imported {
    pub wav_path: PathBuf,
    pub duration_secs: f64,
}

pub fn import_audio(_source: &Path, _dest_dir: &Path) -> Result<Imported, String> {
    unimplemented!()
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cd src-tauri && cargo test --lib audio::import::tests::import`
Expected: FAIL (panics on `unimplemented!()`).

- [ ] **Step 4: Implement `import_audio`**

Replace the `import_audio` stub with:

```rust
pub fn import_audio(source: &Path, dest_dir: &Path) -> Result<Imported, String> {
    let file = File::open(source).map_err(|e| format!("Failed to open file: {}", e))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    if let Some(ext) = source.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }

    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|e| format!("Unsupported or corrupt audio: {}", e))?;
    let mut format = probed.format;

    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or("No decodable audio track")?;
    let track_id = track.id;
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| format!("No decoder available: {}", e))?;

    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| format!("System clock error: {}", e))?
        .as_millis();
    let wav_path = dest_dir.join(format!("imported_{}.wav", timestamp));

    let mut writer: Option<WavWriter<BufWriter<File>>> = None;
    let mut samples_written: u64 = 0;

    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(SymphoniaError::IoError(e))
                if e.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break;
            }
            Err(SymphoniaError::ResetRequired) => break,
            Err(e) => return Err(format!("Error reading audio: {}", e)),
        };

        if packet.track_id() != track_id {
            continue;
        }

        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            Err(SymphoniaError::DecodeError(_)) => continue, // skip a bad packet
            Err(e) => return Err(format!("Decode error: {}", e)),
        };

        let spec = *decoded.spec();
        let channels = spec.channels.count();

        if writer.is_none() {
            let wav_spec = WavSpec {
                channels: 1,
                sample_rate: spec.rate,
                bits_per_sample: 16,
                sample_format: SampleFormat::Int,
            };
            writer = Some(
                WavWriter::create(&wav_path, wav_spec)
                    .map_err(|e| format!("Failed to create WAV: {}", e))?,
            );
        }

        // One SampleBuffer per packet keeps memory bounded to a single packet.
        let mut sbuf = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
        sbuf.copy_interleaved_ref(decoded);
        let mono = downmix_to_mono(sbuf.samples(), channels);

        let w = writer.as_mut().expect("writer initialized above");
        for s in mono {
            w.write_sample(f32_to_i16(s))
                .map_err(|e| format!("Failed to write sample: {}", e))?;
            samples_written += 1;
        }
    }

    let writer = writer.ok_or("Audio file contained no decodable frames")?;
    writer
        .finalize()
        .map_err(|e| format!("Failed to finalize WAV: {}", e))?;

    if samples_written == 0 {
        let _ = std::fs::remove_file(&wav_path);
        return Err("Audio file contained no samples".to_string());
    }

    let duration_secs = wav_duration_secs(&wav_path)?;
    Ok(Imported {
        wav_path,
        duration_secs,
    })
}
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cd src-tauri && cargo test --lib audio::import`
Expected: PASS (all import + helper tests).

- [ ] **Step 6: Lint and format**

Run: `cd src-tauri && cargo fmt && cargo clippy --all-targets -- -D warnings`
Expected: no warnings, no diff complaints.

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/audio/import.rs
git commit -m "feat(import): stream-decode audio files to mono WAV via symphonia"
```

---

## Task 5: `import_audio_file` Tauri command

**Files:**
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: Add the command**

In `src-tauri/src/lib.rs`, add this command near the other recording commands (e.g. after `stop_recording`, around line 79). Note `PendingRecording` and `Store` are already imported at the top of the file.

```rust
const SUPPORTED_IMPORT_EXTENSIONS: [&str; 5] = ["mp3", "m4a", "wav", "ogg", "flac"];

#[tauri::command]
async fn import_audio_file(
    state: State<'_, AppState>,
    path: String,
) -> Result<PendingRecording, String> {
    let source = PathBuf::from(&path);

    if !source.exists() {
        return Err("File not found".to_string());
    }

    let ext = source
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_default();
    if !SUPPORTED_IMPORT_EXTENSIONS.contains(&ext.as_str()) {
        return Err(format!("Unsupported file type: .{}", ext));
    }

    let dest_dir = state.data_dir.clone();
    let imported = tauri::async_runtime::spawn_blocking(move || {
        audio::import::import_audio(&source, &dest_dir)
    })
    .await
    .map_err(|e| format!("Import task failed: {}", e))??;

    let file_path = imported.wav_path.to_string_lossy().to_string();
    let store = state.store.lock().map_err(|e| e.to_string())?;
    let id = store.save_pending(&file_path, imported.duration_secs)?;
    store.get_pending(id)
}
```

- [ ] **Step 2: Register the command in the handler**

In `src-tauri/src/lib.rs`, add `import_audio_file,` to the `tauri::generate_handler!` list (around line 671, next to `stop_recording`):

```rust
            start_recording,
            stop_recording,
            import_audio_file,
```

- [ ] **Step 3: Build to verify it compiles**

Run: `cd src-tauri && cargo build`
Expected: compiles successfully.

- [ ] **Step 4: Lint and format**

Run: `cd src-tauri && cargo fmt && cargo clippy --all-targets -- -D warnings`
Expected: no warnings.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/lib.rs
git commit -m "feat(import): add import_audio_file command"
```

---

## Task 6: i18n strings

**Files:**
- Modify: `src/lib/i18n.js`

- [ ] **Step 1: Add Portuguese strings**

In `src/lib/i18n.js`, inside the `pt` object (after `provisionalHint`, line 51), add:

```javascript
    importAudio: "Importar áudio",
    importing: "Importando…",
    importError: "Falha ao importar áudio",
    audioFiles: "Arquivos de áudio",
```

- [ ] **Step 2: Add English strings**

Inside the `en` object (after `provisionalHint`, line 102), add:

```javascript
    importAudio: "Import audio",
    importing: "Importing…",
    importError: "Failed to import audio",
    audioFiles: "Audio files",
```

- [ ] **Step 3: Type-check**

Run: `npm run check`
Expected: no new errors.

- [ ] **Step 4: Commit**

```bash
git add src/lib/i18n.js
git commit -m "feat(import): add i18n strings for audio import"
```

---

## Task 7: Import button + handler in Recorder.svelte

**Files:**
- Modify: `src/lib/Recorder.svelte`

- [ ] **Step 1: Import the dialog `open` API**

In the `<script>` block of `src/lib/Recorder.svelte`, add after the existing `@tauri-apps/api` imports (line 3):

```javascript
    import { open } from "@tauri-apps/plugin-dialog";
```

- [ ] **Step 2: Add the importing state**

After `let processing = $state(false);` (line 13), add:

```javascript
    let importing = $state(false);
```

- [ ] **Step 3: Add the import handler**

After the `stopRecording` function (line 126), add:

```javascript
    async function importAudio() {
        try {
            error = "";
            const selected = await open({
                multiple: false,
                filters: [
                    {
                        name: t("audioFiles"),
                        extensions: ["mp3", "m4a", "wav", "ogg", "flac"],
                    },
                ],
            });
            if (typeof selected !== "string") return; // user cancelled
            importing = true;
            const pending = await invoke("import_audio_file", {
                path: selected,
            });
            pendingRecordings = [pending, ...pendingRecordings];
        } catch (e) {
            error = `${t("importError")}: ${e}`;
        } finally {
            importing = false;
        }
    }
```

- [ ] **Step 4: Render the button and importing state**

In the template, replace the idle branch (lines 196-199):

```svelte
    {:else if phase === "idle"}
        <button class="btn-start" onclick={startRecording}>
            {t("startRecording")}
        </button>
    {/if}
```

with:

```svelte
    {:else if phase === "idle"}
        {#if importing}
            <div class="status processing">{t("importing")}</div>
        {:else}
            <button class="btn-start" onclick={startRecording}>
                {t("startRecording")}
            </button>
            <button class="btn-import" onclick={importAudio}>
                {t("importAudio")}
            </button>
        {/if}
    {/if}
```

- [ ] **Step 5: Add the button style**

In the `<style>` block, after the `.btn-stop` rule (line 286), add:

```css
    .btn-import {
        background: transparent;
        color: var(--text-muted);
        border: 1px solid var(--text-muted);
        font-size: 1rem;
        padding: 10px 24px;
    }
```

- [ ] **Step 6: Type-check**

Run: `npm run check`
Expected: no new errors.

- [ ] **Step 7: Commit**

```bash
git add src/lib/Recorder.svelte
git commit -m "feat(import): add import button and file picker to recorder"
```

---

## Task 8: Full build verification

**Files:** none (verification only)

- [ ] **Step 1: Run the full Rust test suite**

Run: `cd src-tauri && cargo test`
Expected: all tests pass, including the new `audio::import` tests.

- [ ] **Step 2: Run clippy across the workspace**

Run: `cd src-tauri && cargo clippy --all-targets -- -D warnings`
Expected: no warnings.

- [ ] **Step 3: Frontend type-check**

Run: `npm run check`
Expected: no errors.

---

## Task 9: Manual verification (golden path + encoded formats)

The frontend has no automated tests; verify in the running app. Encoded-format decode (mp3/m4a/ogg/flac) is covered here since generating encoded fixtures requires external tooling.

- [ ] **Step 1: Launch the app**

Run: `cargo tauri dev`

- [ ] **Step 2: Import golden path**

Click **Import audio**, choose an **mp3** file. Expected: an "Importing…" indicator appears, then a new entry shows in "Pending recordings" with a sensible duration.

- [ ] **Step 3: Transcribe the imported file**

Click **Transcribe** on the imported pending entry. Expected: transcription runs through the normal flow and completes; the transcript appears in History.

- [ ] **Step 4: Verify cleanup**

After completion, confirm the pending entry disappeared. (The converted `imported_<ts>.wav` in the app data dir is deleted by `finish_job` — same as a recording.)

- [ ] **Step 5: Format matrix**

Repeat Steps 2-3 with one file each of **m4a**, **ogg**, **flac**, and **wav**. Expected: each imports and transcribes successfully.

- [ ] **Step 6: Error path**

Try importing a non-audio file renamed to `.mp3` (corrupt). Expected: a "Failed to import audio" error message appears and the app stays usable.

- [ ] **Step 7: Concurrency sanity**

Start transcribing a long imported file, and while it runs, import another file. Expected: the second import succeeds and lands in pending without disturbing the running transcription.

---

## Self-Review Notes

- **Spec coverage:** formats (Task 1 features), Symphonia decode (Task 4), convert-at-import to mono WAV in data_dir (Task 4), pending-list flow (Task 5), native picker button (Tasks 1,7), streaming decode for slow machines (Task 4 — one SampleBuffer per packet), lifecycle/cleanup reused (verified Task 9 Step 4), error handling (Task 5 validation + Task 4 decode errors + Task 9 Step 6). All covered.
- **Divergences from spec, intentional:** (1) automated tests use an in-test generated WAV instead of committed mp3/flac/ogg fixtures (avoids committing binaries / requiring ffmpeg); encoded formats verified in the Task 9 manual matrix. (2) No Svelte/Vitest test — the repo has no Vitest setup; frontend verified manually. Both keep scope tight per YAGNI.
- **Type consistency:** `Imported { wav_path, duration_secs }`, `import_audio(&Path, &Path)`, `import_audio_file(path: String) -> PendingRecording`, and the `invoke("import_audio_file", { path })` call all line up.
