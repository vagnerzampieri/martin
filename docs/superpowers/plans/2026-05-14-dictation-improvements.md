# Dictation Improvements Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make live dictation reliable and clear for a long-form academic dictation use case — visual feedback so the user understands system state, automatic formatting, persistent audio, and CPU tuning — without sacrificing transcription precision.

**Architecture:** Backend gains three new shared signals exposed via Tauri events (audio level, session state, stable-vs-provisional text). Two new pure modules (`vad`, `postprocess`) handle silence detection and text normalization with TDD-friendly unit tests. A WAV writer captures dictation audio in real time so it can be reprocessed later. The DB row for each dictation is created at start instead of at stop, allowing periodic auto-save. Frontend renders three explicit states (listening/processing/paused), a VU meter, and visually distinguishes stable from provisional text.

**Tech Stack:** Rust (Tauri 2 backend), Svelte 5 (frontend), `whisper-rs`, `cpal`, `hound`, `rusqlite`.

---

## Phase 0 — Branch setup

### Task 0: Create the feature branch

**Files:** (none — repo metadata only)

- [ ] **Step 1: Confirm the working tree is clean apart from the plan itself**

Run: `git status --short`
Expected: only `?? docs/superpowers/plans/2026-05-14-dictation-improvements.md` (or nothing if the plan was already committed).

- [ ] **Step 2: Create and switch to the branch**

Run: `git checkout -b feat/dictation-improvements`
Expected: `Switched to a new branch 'feat/dictation-improvements'`.

- [ ] **Step 3: Stage and commit the plan file**

```bash
git add docs/superpowers/plans/2026-05-14-dictation-improvements.md
git commit -m "docs(plan): dictation improvements for academic dictation use case"
```

---

## Phase 1 — Foundation Wins (no UX, low risk)

### Task 1: Cargo release profile

**Files:**
- Modify: `src-tauri/Cargo.toml`

- [ ] **Step 1: Append release profile to `Cargo.toml`**

Open `src-tauri/Cargo.toml` and append at the end of the file:

```toml

[profile.release]
opt-level = 3
lto = "fat"
codegen-units = 1
strip = true
```

Note: do NOT add `panic = "abort"`. The transcription workers in `src-tauri/src/lib.rs` (around lines 192–220 and 505–531) and `src-tauri/src/transcribe/job.rs` rely on `std::panic::catch_unwind` to recover from whisper panics and emit a user-visible error. `panic = "abort"` would silently kill the whole process instead.

- [ ] **Step 2: Verify release build still compiles**

Run: `cd src-tauri && cargo build --release --quiet`
Expected: build completes without errors. First build will take noticeably longer than a default release build (~3–5 min vs ~1 min) because of fat LTO.

- [ ] **Step 3: Commit**

```bash
git add src-tauri/Cargo.toml
git commit -m "perf(build): enable LTO and strip in release profile"
```

---

### Task 2: Whisper thread count from physical cores

**Files:**
- Modify: `src-tauri/src/transcribe/whisper.rs:23-29, 54-65, 109-124`

- [ ] **Step 1: Write the failing test**

Add to the existing `#[cfg(test)] mod tests` block in `src-tauri/src/transcribe/whisper.rs`:

```rust
#[test]
fn whisper_thread_count_caps_at_eight_and_floors_at_one() {
    assert_eq!(Transcriber::whisper_thread_count_for(0), 1);
    assert_eq!(Transcriber::whisper_thread_count_for(1), 1);
    assert_eq!(Transcriber::whisper_thread_count_for(4), 4);
    assert_eq!(Transcriber::whisper_thread_count_for(8), 8);
    assert_eq!(Transcriber::whisper_thread_count_for(16), 8);
    assert_eq!(Transcriber::whisper_thread_count_for(64), 8);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd src-tauri && cargo test whisper_thread_count_caps_at_eight_and_floors_at_one`
Expected: FAIL with `no method named whisper_thread_count_for found for struct Transcriber`.

- [ ] **Step 3: Add the helper to `Transcriber`**

In `src-tauri/src/transcribe/whisper.rs`, inside `impl Transcriber { ... }`, add the following helper (place it right after `pub fn new(...)`):

```rust
    /// Pick how many threads whisper should use. Cap at 8 — whisper.cpp's matmul
    /// kernels scale poorly past that, and over-subscription on weak machines
    /// (where dictation lives) actively hurts. Floor at 1 so the caller can pass
    /// `0` from `available_parallelism().get().saturating_sub(...)`.
    pub fn whisper_thread_count_for(physical_cores: usize) -> i32 {
        physical_cores.clamp(1, 8) as i32
    }

    fn default_thread_count() -> i32 {
        let cores = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1);
        Self::whisper_thread_count_for(cores)
    }
```

- [ ] **Step 4: Wire `set_n_threads` into all three transcribe paths**

In `src-tauri/src/transcribe/whisper.rs`, inside `transcribe()` (around line 23), `transcribe_samples()` (around line 54), and `transcribe_with_callbacks()` (around line 109), add this line **immediately after** each `let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });`:

```rust
        params.set_n_threads(Self::default_thread_count());
```

- [ ] **Step 5: Run the test**

Run: `cd src-tauri && cargo test whisper_thread_count_caps_at_eight_and_floors_at_one`
Expected: PASS.

- [ ] **Step 6: Run all whisper tests**

Run: `cd src-tauri && cargo test --lib transcribe::`
Expected: all tests pass.

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/transcribe/whisper.rs
git commit -m "perf(whisper): cap thread count at physical cores (max 8)"
```

---

### Task 3: Fix duplicate inference at rollover

**Files:**
- Modify: `src-tauri/src/dictation.rs:252-292`

- [ ] **Step 1: Read the current rollover block**

Read `src-tauri/src/dictation.rs` lines 248–293 to confirm the current shape.

- [ ] **Step 2: Refactor to reuse the prior transcription**

In `src-tauri/src/dictation.rs`, replace the body of the `while running.load(...)` loop starting at line 248 (`// Convert accumulated audio to mono 16kHz and transcribe the whole thing`) and ending at line 292 (`}`, the closing brace of the rollover `if`) with the following:

```rust
        // Convert accumulated audio to mono 16kHz and transcribe the whole thing
        let mono_16k = convert_to_mono_16k(&accumulated_raw, channels, source_rate);
        last_transcribed_len = accumulated_raw.len();

        let pass_text = match transcriber.transcribe_samples(&mono_16k, language) {
            Ok(text) => text.trim().to_string(),
            Err(e) => {
                eprintln!("Dictation transcription error: {}", e);
                String::new()
            }
        };

        if !pass_text.is_empty() {
            let full_text = if committed_segments.is_empty() {
                pass_text.clone()
            } else {
                format!("{} {}", committed_segments.join(" "), pass_text)
            };
            let _ = app_handle.emit(
                "dictation://segment",
                serde_json::json!({
                    "text": pass_text,
                    "fullText": full_text,
                }),
            );
            if let Ok(mut last) = last_full_text_out.lock() {
                *last = full_text.clone();
            }
        }

        // If buffer exceeds max, commit the text we just produced and start fresh.
        // Reusing `pass_text` avoids a second whisper pass on the same audio.
        if accumulated_raw.len() > max_raw_samples {
            if !pass_text.is_empty() {
                committed_segments.push(pass_text.clone());
                if let Ok(mut sink) = committed_out.lock() {
                    sink.push(pass_text);
                }
            }
            accumulated_raw.clear();
            last_transcribed_len = 0;
        }
```

- [ ] **Step 3: Verify it compiles**

Run: `cd src-tauri && cargo build --quiet`
Expected: success.

- [ ] **Step 4: Run dictation tests**

Run: `cd src-tauri && cargo test --lib dictation`
Expected: existing tests pass (no behavior tests for this loop exist; verification is via the build).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/dictation.rs
git commit -m "fix(dictation): reuse prior transcription at rollover instead of re-running whisper"
```

---

## Phase 2 — VAD (silence detection)

### Task 4: Create `vad` module with RMS computation

**Files:**
- Create: `src-tauri/src/vad.rs`
- Modify: `src-tauri/src/lib.rs:1-7`

- [ ] **Step 1: Register the module**

In `src-tauri/src/lib.rs`, add the `mod vad;` declaration alongside the other module declarations at the top of the file. The block currently reads:

```rust
mod audio;
mod db;
mod dictation;
mod model;
mod summarize;
mod transcribe;
```

Change it to:

```rust
mod audio;
mod db;
mod dictation;
mod model;
mod summarize;
mod transcribe;
mod vad;
```

- [ ] **Step 2: Write the new module with failing tests**

Create `src-tauri/src/vad.rs` with this content:

```rust
//! Voice activity detection helpers. Pure functions only — no I/O, no state.
//! Used by the dictation loop to skip whisper passes during silence and to
//! detect paragraph boundaries.

/// RMS (root mean square) amplitude of a slice of mono samples in [-1.0, 1.0].
/// Returns 0.0 for an empty slice.
pub fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum_sq: f32 = samples.iter().map(|s| s * s).sum();
    (sum_sq / samples.len() as f32).sqrt()
}

/// Returns true when `rms_value` is at or below the silence threshold.
/// Threshold tuned for typical laptop mics in a quiet room.
pub const SILENCE_THRESHOLD: f32 = 0.01;

pub fn is_silent(rms_value: f32) -> bool {
    rms_value <= SILENCE_THRESHOLD
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rms_of_empty_slice_is_zero() {
        assert_eq!(rms(&[]), 0.0);
    }

    #[test]
    fn rms_of_silence_is_zero() {
        let samples = vec![0.0_f32; 1000];
        assert_eq!(rms(&samples), 0.0);
    }

    #[test]
    fn rms_of_dc_signal_equals_its_amplitude() {
        let samples = vec![0.5_f32; 100];
        assert!((rms(&samples) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn rms_of_sign_alternating_signal_equals_amplitude() {
        let samples: Vec<f32> = (0..1000).map(|i| if i % 2 == 0 { 0.3 } else { -0.3 }).collect();
        assert!((rms(&samples) - 0.3).abs() < 1e-6);
    }

    #[test]
    fn is_silent_true_at_and_below_threshold() {
        assert!(is_silent(0.0));
        assert!(is_silent(SILENCE_THRESHOLD));
        assert!(is_silent(SILENCE_THRESHOLD - 0.001));
    }

    #[test]
    fn is_silent_false_above_threshold() {
        assert!(!is_silent(SILENCE_THRESHOLD + 0.001));
        assert!(!is_silent(0.5));
    }
}
```

- [ ] **Step 3: Run the new tests**

Run: `cd src-tauri && cargo test --lib vad::`
Expected: 6 tests pass.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/lib.rs src-tauri/src/vad.rs
git commit -m "feat(vad): add pure RMS-based silence detection helpers"
```

---

### Task 5: Shared session-state atomic

**Files:**
- Modify: `src-tauri/src/dictation.rs`

- [ ] **Step 1: Add session-state types at the top of `dictation.rs`**

In `src-tauri/src/dictation.rs`, replace the existing `use` block at the top of the file (lines 1–9) with:

```rust
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, Stream};
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use tauri::Emitter;

use crate::transcribe::whisper::Transcriber;
```

Then, **immediately after** the `use` statements, add:

```rust
/// Externally visible dictation state. Encoded as `u8` so it can live in an
/// `AtomicU8` shared between the capture, transcription, and level-poller threads.
/// Values are stable — the frontend depends on them.
pub const STATE_LISTENING: u8 = 0;
pub const STATE_PROCESSING: u8 = 1;
pub const STATE_PAUSED: u8 = 2;

pub fn state_label(state: u8) -> &'static str {
    match state {
        STATE_PROCESSING => "processing",
        STATE_PAUSED => "paused",
        _ => "listening",
    }
}
```

- [ ] **Step 2: Add fields to `DictationSession`**

In `src-tauri/src/dictation.rs`, find the `pub struct DictationSession` definition (around line 16) and replace it with:

```rust
pub struct DictationSession {
    stream: Option<Stream>,
    audio_buffer: Arc<Mutex<Vec<f32>>>,
    running: Arc<AtomicBool>,
    source_rate: u32,
    channels: u16,
    committed: Arc<Mutex<Vec<String>>>,
    last_full_text: Arc<Mutex<String>>,
    final_audio: Arc<Mutex<Vec<f32>>>,
    /// Last-known peak amplitude of the audio callback, as f32 bits in u32.
    /// Updated by the cpal callback, read by the level-poller thread.
    last_peak_bits: Arc<AtomicU32>,
    /// Current session state (see `STATE_*` constants).
    state: Arc<AtomicU8>,
    worker: Option<JoinHandle<()>>,
    level_worker: Option<JoinHandle<()>>,
}
```

- [ ] **Step 3: Update `DictationSession::new`**

Find `impl DictationSession { ... pub fn new() -> Self { ... } }` (around line 36) and replace `new()` with:

```rust
    pub fn new() -> Self {
        Self {
            stream: None,
            audio_buffer: Arc::new(Mutex::new(Vec::new())),
            running: Arc::new(AtomicBool::new(false)),
            source_rate: WHISPER_SAMPLE_RATE,
            channels: 1,
            committed: Arc::new(Mutex::new(Vec::new())),
            last_full_text: Arc::new(Mutex::new(String::new())),
            final_audio: Arc::new(Mutex::new(Vec::new())),
            last_peak_bits: Arc::new(AtomicU32::new(0)),
            state: Arc::new(AtomicU8::new(STATE_LISTENING)),
            worker: None,
            level_worker: None,
        }
    }
```

- [ ] **Step 4: Add accessors and worker setter**

In the same `impl DictationSession` block, **replace** the existing `pub fn set_worker(...)` and `pub fn stop_and_join(...)` methods with:

```rust
    pub fn last_peak_bits(&self) -> Arc<AtomicU32> {
        self.last_peak_bits.clone()
    }

    pub fn state(&self) -> Arc<AtomicU8> {
        self.state.clone()
    }

    pub fn set_worker(&mut self, handle: JoinHandle<()>) {
        self.worker = Some(handle);
    }

    pub fn set_level_worker(&mut self, handle: JoinHandle<()>) {
        self.level_worker = Some(handle);
    }

    /// Stops the audio stream, signals workers, and joins them.
    pub fn stop_and_join(&mut self) {
        self.running.store(false, Ordering::Release);
        self.stream.take();
        if let Some(handle) = self.worker.take() {
            let _ = handle.join();
        }
        if let Some(handle) = self.level_worker.take() {
            let _ = handle.join();
        }
    }
```

- [ ] **Step 5: Update the cpal callback to record peak amplitude**

In `src-tauri/src/dictation.rs`, find `pub fn start(&mut self) -> Result<(), String> { ... }` and the two `device.build_input_stream` blocks. Replace the whole `let stream = match sample_format { ... };` expression (currently lines 79–105) with:

```rust
        let peak_for_i16 = self.last_peak_bits.clone();
        let peak_for_f32 = self.last_peak_bits.clone();
        let stream = match sample_format {
            SampleFormat::I16 => device
                .build_input_stream(
                    &config.into(),
                    move |data: &[i16], _| {
                        let mut peak: f32 = 0.0;
                        if let Ok(mut buf) = buffer.lock() {
                            for &s in data {
                                let f = s as f32 / i16::MAX as f32;
                                let a = f.abs();
                                if a > peak {
                                    peak = a;
                                }
                                buf.push(f);
                            }
                        }
                        peak_for_i16.store(peak.to_bits(), Ordering::Relaxed);
                    },
                    |err| eprintln!("Dictation stream error: {}", err),
                    None,
                )
                .map_err(|e| format!("Failed to build input stream: {}", e))?,
            SampleFormat::F32 => device
                .build_input_stream(
                    &config.into(),
                    move |data: &[f32], _| {
                        let mut peak: f32 = 0.0;
                        if let Ok(mut buf) = buffer.lock() {
                            for &s in data {
                                let a = s.abs();
                                if a > peak {
                                    peak = a;
                                }
                                buf.push(s);
                            }
                        }
                        peak_for_f32.store(peak.to_bits(), Ordering::Relaxed);
                    },
                    |err| eprintln!("Dictation stream error: {}", err),
                    None,
                )
                .map_err(|e| format!("Failed to build input stream: {}", e))?,
            format => return Err(format!("Unsupported sample format: {:?}", format)),
        };
```

- [ ] **Step 6: Verify build**

Run: `cd src-tauri && cargo build --quiet`
Expected: success.

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/dictation.rs
git commit -m "feat(dictation): add atomic peak-level and state shared across threads"
```

---

### Task 6: Silence gate + state transitions in the transcription loop

**Files:**
- Modify: `src-tauri/src/dictation.rs` (the `run_transcription_loop` function and its callers)

- [ ] **Step 1: Update `run_transcription_loop` signature and body**

In `src-tauri/src/dictation.rs`, replace the entire `run_transcription_loop` function (starting at the doc comment around line 204 and ending at the closing brace around line 306) with:

```rust
/// Runs the transcription loop on a blocking thread.
/// Re-transcribes the entire accumulated audio buffer each cycle for
/// maximum Whisper accuracy. Skips whisper passes during silence and
/// updates the shared session state (listening/processing/paused).
/// At rollover, commits the current text as a segment and starts a fresh buffer.
///
/// On exit, hands off the post-rollover raw audio into `final_audio_out`
/// so the finalize worker can re-transcribe it with progress callbacks.
#[allow(clippy::too_many_arguments)]
pub fn run_transcription_loop(
    buffer: Arc<Mutex<Vec<f32>>>,
    running: Arc<AtomicBool>,
    committed_out: Arc<Mutex<Vec<String>>>,
    last_full_text_out: Arc<Mutex<String>>,
    final_audio_out: Arc<Mutex<Vec<f32>>>,
    state: Arc<AtomicU8>,
    transcriber: &Transcriber,
    language: &str,
    source_rate: u32,
    channels: u16,
    app_handle: tauri::AppHandle,
) {
    let mut committed_segments: Vec<String> = Vec::new();
    let mut accumulated_raw: Vec<f32> = Vec::new();
    let mut last_transcribed_len: usize = 0;
    let mut consecutive_silent_polls: u32 = 0;

    let raw_samples_per_second = source_rate as usize * channels as usize;
    let min_raw_samples = raw_samples_per_second * MIN_SECONDS_TO_TRANSCRIBE;
    let max_raw_samples = raw_samples_per_second * MAX_BUFFER_SECONDS;

    while running.load(Ordering::Acquire) {
        std::thread::sleep(std::time::Duration::from_millis(POLL_INTERVAL_MS));

        // Drain new samples from the shared buffer
        let new_chunk_start = accumulated_raw.len();
        if let Ok(mut buf) = buffer.lock() {
            accumulated_raw.extend(buf.drain(..));
        }
        let new_chunk_end = accumulated_raw.len();

        // Compute RMS over the NEW chunk only. We need the new-chunk view to
        // decide whether speech is happening right now, independent of how much
        // total audio we have accumulated.
        let new_chunk_rms = if new_chunk_end > new_chunk_start {
            crate::vad::rms(&accumulated_raw[new_chunk_start..new_chunk_end])
        } else {
            0.0
        };

        let chunk_is_silent = crate::vad::is_silent(new_chunk_rms);
        if chunk_is_silent {
            consecutive_silent_polls = consecutive_silent_polls.saturating_add(1);
        } else {
            consecutive_silent_polls = 0;
        }

        // PAUSED after 2 consecutive silent polls (~1s). Switch back to
        // LISTENING the moment speech resumes. PROCESSING is set during
        // the whisper pass below.
        if chunk_is_silent && consecutive_silent_polls >= 2 {
            state.store(STATE_PAUSED, Ordering::Release);
        } else if !chunk_is_silent
            && state.load(Ordering::Acquire) == STATE_PAUSED
        {
            state.store(STATE_LISTENING, Ordering::Release);
        }

        // Only transcribe if we have enough audio AND there is new audio since last run
        if accumulated_raw.len() < min_raw_samples || accumulated_raw.len() == last_transcribed_len
        {
            continue;
        }

        // Silence gate: if the freshly added chunk is silent AND we already
        // produced text recently, don't waste CPU re-transcribing.
        if chunk_is_silent && last_transcribed_len > 0 {
            continue;
        }

        let mono_16k = convert_to_mono_16k(&accumulated_raw, channels, source_rate);
        last_transcribed_len = accumulated_raw.len();

        state.store(STATE_PROCESSING, Ordering::Release);
        let pass_text = match transcriber.transcribe_samples(&mono_16k, language) {
            Ok(text) => text.trim().to_string(),
            Err(e) => {
                eprintln!("Dictation transcription error: {}", e);
                String::new()
            }
        };
        // Back to LISTENING unless the silence detector has already flipped us
        // to PAUSED in a subsequent (unlikely, this is the same thread) tick.
        let _ = state.compare_exchange(
            STATE_PROCESSING,
            STATE_LISTENING,
            Ordering::AcqRel,
            Ordering::Acquire,
        );

        if !pass_text.is_empty() {
            let full_text = if committed_segments.is_empty() {
                pass_text.clone()
            } else {
                format!("{} {}", committed_segments.join(" "), pass_text)
            };
            let _ = app_handle.emit(
                "dictation://segment",
                serde_json::json!({
                    "stableText": committed_segments.join(" "),
                    "provisionalText": pass_text,
                    "fullText": full_text,
                }),
            );
            if let Ok(mut last) = last_full_text_out.lock() {
                *last = full_text.clone();
            }
        }

        // If buffer exceeds max, commit the text we just produced and start fresh.
        if accumulated_raw.len() > max_raw_samples {
            if !pass_text.is_empty() {
                committed_segments.push(pass_text.clone());
                if let Ok(mut sink) = committed_out.lock() {
                    sink.push(pass_text);
                }
            }
            accumulated_raw.clear();
            last_transcribed_len = 0;
        }
    }

    // Final drain of any audio captured between the last poll and the stop signal.
    if let Ok(mut buf) = buffer.lock() {
        accumulated_raw.extend(buf.drain(..));
    }

    if let Ok(mut sink) = final_audio_out.lock() {
        *sink = accumulated_raw;
    }
}
```

- [ ] **Step 2: Update the caller in `lib.rs`**

In `src-tauri/src/lib.rs`, find the `start_dictation` function and the `std::thread::spawn(move || { dictation::run_transcription_loop(...) })` block (around lines 343–360). Replace the inner `dictation::run_transcription_loop(...)` call with:

```rust
        dictation::run_transcription_loop(
            buffer,
            running,
            committed_out,
            last_full_text_out,
            final_audio_out,
            state_for_loop,
            &transcriber,
            &language_owned,
            source_rate,
            channels,
            app_for_loop.clone(),
        );
```

Then, **before** the `let app_for_loop = app_handle.clone();` line (around line 341), add:

```rust
    let state_for_loop = session.state();
```

- [ ] **Step 3: Verify build**

Run: `cd src-tauri && cargo build --quiet`
Expected: success.

- [ ] **Step 4: Run tests**

Run: `cd src-tauri && cargo test --lib`
Expected: all tests pass.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/dictation.rs src-tauri/src/lib.rs
git commit -m "feat(dictation): skip whisper on silence and track session state"
```

---

### Task 7: Level-and-state emitter thread

**Files:**
- Modify: `src-tauri/src/dictation.rs`
- Modify: `src-tauri/src/lib.rs:343-365`

- [ ] **Step 1: Add the level-poller function to `dictation.rs`**

In `src-tauri/src/dictation.rs`, **append** at the very bottom of the file (after `run_transcription_loop`):

```rust
const LEVEL_EMIT_INTERVAL_MS: u64 = 100;

/// Emits `dictation://level` (audio peak amplitude, 0.0–1.0) every
/// LEVEL_EMIT_INTERVAL_MS and `dictation://state` whenever the shared
/// state atomic changes. Runs on its own thread so UI updates stay
/// responsive even while whisper is busy on the transcription thread.
pub fn run_level_emitter(
    running: Arc<AtomicBool>,
    last_peak_bits: Arc<AtomicU32>,
    state: Arc<AtomicU8>,
    app_handle: tauri::AppHandle,
) {
    let mut last_emitted_state: u8 = u8::MAX;

    while running.load(Ordering::Acquire) {
        std::thread::sleep(std::time::Duration::from_millis(LEVEL_EMIT_INTERVAL_MS));

        let peak = f32::from_bits(last_peak_bits.load(Ordering::Relaxed));
        // Reset peak so the next interval reflects only fresh audio.
        last_peak_bits.store(0u32, Ordering::Relaxed);

        let _ = app_handle.emit(
            "dictation://level",
            serde_json::json!({ "peak": peak }),
        );

        let current = state.load(Ordering::Acquire);
        if current != last_emitted_state {
            last_emitted_state = current;
            let _ = app_handle.emit(
                "dictation://state",
                serde_json::json!({ "state": state_label(current) }),
            );
        }
    }
}
```

- [ ] **Step 2: Spawn the level emitter from `start_dictation`**

In `src-tauri/src/lib.rs`, inside `start_dictation`, **after** the `let worker = std::thread::spawn(move || { ... });` block and **before** `session.set_worker(worker);`, add:

```rust
    let level_running = session.running_flag();
    let level_peak = session.last_peak_bits();
    let level_state = session.state();
    let level_app = app_handle.clone();
    let level_worker = std::thread::spawn(move || {
        dictation::run_level_emitter(level_running, level_peak, level_state, level_app);
    });
```

Then, **after** the existing `session.set_worker(worker);` line, add:

```rust
    session.set_level_worker(level_worker);
```

- [ ] **Step 3: Verify build**

Run: `cd src-tauri && cargo build --quiet`
Expected: success.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/dictation.rs src-tauri/src/lib.rs
git commit -m "feat(dictation): emit level and state events on a dedicated thread"
```

---

## Phase 3 — Text post-processing

### Task 8: Create `postprocess` module with capitalization and spacing

**Files:**
- Create: `src-tauri/src/postprocess.rs`
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: Register the module**

In `src-tauri/src/lib.rs`, add `mod postprocess;` next to the other module declarations (alongside `mod vad;` from Task 4).

- [ ] **Step 2: Write the new module with tests**

Create `src-tauri/src/postprocess.rs` with this content:

```rust
//! Pure text post-processing for dictation output. Applied to the full assembled
//! text on every emission so behavior is consistent regardless of chunking.

/// Collapse runs of inline whitespace (spaces/tabs) into a single space, but
/// preserve newlines exactly. Trims trailing whitespace from each line.
pub fn collapse_whitespace(text: &str) -> String {
    text.lines()
        .map(|line| {
            let trimmed_end = line.trim_end();
            let mut out = String::with_capacity(trimmed_end.len());
            let mut prev_space = false;
            for ch in trimmed_end.chars() {
                if ch == ' ' || ch == '\t' {
                    if !prev_space {
                        out.push(' ');
                    }
                    prev_space = true;
                } else {
                    out.push(ch);
                    prev_space = false;
                }
            }
            out
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Ensure a single space after `,;:` and `.!?` when followed by a letter/digit.
/// Fixes whisper outputs like `texto,palavra` → `texto, palavra`.
pub fn fix_punctuation_spacing(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        out.push(ch);
        if matches!(ch, ',' | ';' | ':' | '.' | '!' | '?') {
            if let Some(&next) = chars.peek() {
                if next.is_alphanumeric() {
                    out.push(' ');
                }
            }
        }
    }
    out
}

/// Capitalize the first alphabetic character of each sentence. A sentence
/// boundary is start-of-string, double newline, or `.!?` followed by whitespace.
pub fn capitalize_sentences(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut should_capitalize_next = true;
    for ch in text.chars() {
        if should_capitalize_next && ch.is_alphabetic() {
            for upper in ch.to_uppercase() {
                out.push(upper);
            }
            should_capitalize_next = false;
        } else {
            out.push(ch);
            if matches!(ch, '.' | '!' | '?') {
                should_capitalize_next = true;
            } else if ch == '\n' {
                // Paragraph break (\n\n) implies new sentence; single \n keeps the flag
                // as it was so we don't over-capitalize wrapped lines.
                should_capitalize_next = true;
            } else if !ch.is_whitespace() {
                should_capitalize_next = false;
            }
        }
    }
    out
}

/// Apply all normalization passes. Order matters:
///   1. Punctuation spacing fixes attached punctuation
///   2. Whitespace collapsing removes extra spaces introduced by step 1
///   3. Capitalization comes last so it sees clean sentence boundaries
pub fn normalize(text: &str) -> String {
    let s = fix_punctuation_spacing(text);
    let s = collapse_whitespace(&s);
    capitalize_sentences(&s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collapse_whitespace_squashes_inline_runs() {
        assert_eq!(collapse_whitespace("a    b   c"), "a b c");
    }

    #[test]
    fn collapse_whitespace_preserves_newlines() {
        assert_eq!(collapse_whitespace("a\n\nb"), "a\n\nb");
        assert_eq!(collapse_whitespace("a  \n\n  b"), "a\n\n b");
    }

    #[test]
    fn collapse_whitespace_trims_trailing_spaces_per_line() {
        assert_eq!(collapse_whitespace("hello   \nworld   "), "hello\nworld");
    }

    #[test]
    fn fix_punctuation_inserts_space_when_glued_to_letter() {
        assert_eq!(fix_punctuation_spacing("texto,palavra"), "texto, palavra");
        assert_eq!(fix_punctuation_spacing("oi.tudo bem"), "oi. tudo bem");
        assert_eq!(fix_punctuation_spacing("a!b?c"), "a! b? c");
    }

    #[test]
    fn fix_punctuation_leaves_existing_spaces_alone() {
        assert_eq!(fix_punctuation_spacing("texto, palavra"), "texto, palavra");
        assert_eq!(fix_punctuation_spacing("fim."), "fim.");
    }

    #[test]
    fn capitalize_first_letter_of_text() {
        assert_eq!(capitalize_sentences("olá mundo"), "Olá mundo");
    }

    #[test]
    fn capitalize_after_period_and_space() {
        assert_eq!(
            capitalize_sentences("primeiro. segundo. terceiro."),
            "Primeiro. Segundo. Terceiro."
        );
    }

    #[test]
    fn capitalize_after_paragraph_break() {
        assert_eq!(
            capitalize_sentences("primeiro paragrafo.\n\nsegundo paragrafo."),
            "Primeiro paragrafo.\n\nSegundo paragrafo."
        );
    }

    #[test]
    fn capitalize_leaves_already_uppercase_intact() {
        assert_eq!(capitalize_sentences("Olá. Tudo bem?"), "Olá. Tudo bem?");
    }

    #[test]
    fn normalize_runs_all_passes() {
        let input = "olá   mundo,como vai?tudo bem.   obrigado";
        let expected = "Olá mundo, como vai? Tudo bem. Obrigado";
        assert_eq!(normalize(input), expected);
    }

    #[test]
    fn normalize_preserves_paragraph_breaks() {
        let input = "primeiro paragrafo.\n\nsegundo paragrafo";
        let expected = "Primeiro paragrafo.\n\nSegundo paragrafo";
        assert_eq!(normalize(input), expected);
    }

    #[test]
    fn normalize_is_idempotent() {
        let input = "Olá mundo. Tudo bem?\n\nNovo paragrafo.";
        assert_eq!(normalize(input), normalize(&normalize(input)));
    }
}
```

- [ ] **Step 3: Run the tests**

Run: `cd src-tauri && cargo test --lib postprocess::`
Expected: all 11 tests pass.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/lib.rs src-tauri/src/postprocess.rs
git commit -m "feat(postprocess): add whitespace, punctuation, and capitalization passes"
```

---

### Task 9: Voice command replacement

**Files:**
- Modify: `src-tauri/src/postprocess.rs`

- [ ] **Step 1: Add the failing tests**

In `src-tauri/src/postprocess.rs`, add these tests to the existing `mod tests`:

```rust
    #[test]
    fn voice_commands_replace_paragraph_command() {
        assert_eq!(
            replace_voice_commands("primeiro novo parágrafo segundo"),
            "primeiro\n\nsegundo"
        );
    }

    #[test]
    fn voice_commands_replace_newline_command() {
        assert_eq!(
            replace_voice_commands("linha um nova linha linha dois"),
            "linha um\nlinha dois"
        );
    }

    #[test]
    fn voice_commands_replace_punctuation_commands() {
        assert_eq!(
            replace_voice_commands("texto vírgula mais texto ponto final"),
            "texto, mais texto."
        );
        assert_eq!(
            replace_voice_commands("isso ponto de interrogação"),
            "isso?"
        );
        assert_eq!(
            replace_voice_commands("uau ponto de exclamação"),
            "uau!"
        );
    }

    #[test]
    fn voice_commands_replace_quote_commands() {
        assert_eq!(
            replace_voice_commands("ele disse abre aspas oi fecha aspas"),
            "ele disse \"oi\""
        );
    }

    #[test]
    fn voice_commands_are_case_insensitive() {
        assert_eq!(
            replace_voice_commands("Novo Parágrafo segundo"),
            "\n\nsegundo"
        );
        assert_eq!(
            replace_voice_commands("texto VÍRGULA mais"),
            "texto, mais"
        );
    }

    #[test]
    fn voice_commands_ignore_substring_inside_word() {
        // "vírgula" inside a longer word should NOT match
        assert_eq!(
            replace_voice_commands("avírgulab"),
            "avírgulab"
        );
    }

    #[test]
    fn normalize_applies_voice_commands_first() {
        let input = "olá vírgula tudo bem ponto de interrogação";
        let expected = "Olá, tudo bem?";
        assert_eq!(normalize(input), expected);
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd src-tauri && cargo test --lib postprocess::voice_commands`
Expected: FAIL with `cannot find function replace_voice_commands in this scope`.

- [ ] **Step 3: Implement `replace_voice_commands`**

In `src-tauri/src/postprocess.rs`, add this function **before** the existing `pub fn normalize(...)`:

```rust
/// Replace spoken formatting commands with their punctuation/whitespace equivalents.
/// Matches are case-insensitive and respect word boundaries — `avírgulab` is left alone.
/// Order matters: longest phrases must be tried first so `ponto de interrogação`
/// is not eaten by `ponto final`.
pub fn replace_voice_commands(text: &str) -> String {
    // Sorted longest-first to prevent shorter phrases from cannibalizing longer ones.
    const COMMANDS: &[(&str, &str)] = &[
        ("ponto de interrogação", "?"),
        ("ponto de exclamação", "!"),
        ("novo parágrafo", "\n\n"),
        ("nova linha", "\n"),
        ("ponto final", "."),
        ("abre aspas", "\""),
        ("fecha aspas", "\""),
        ("vírgula", ","),
    ];

    let mut result = String::with_capacity(text.len());
    let lower: Vec<char> = text.to_lowercase().chars().collect();
    let original: Vec<char> = text.chars().collect();
    let mut i = 0;

    while i < original.len() {
        let mut matched = false;
        for &(phrase, replacement) in COMMANDS {
            let phrase_chars: Vec<char> = phrase.chars().collect();
            if i + phrase_chars.len() > original.len() {
                continue;
            }
            if !lower[i..i + phrase_chars.len()]
                .iter()
                .zip(phrase_chars.iter())
                .all(|(a, b)| a == b)
            {
                continue;
            }
            // Word boundaries: char before must be non-alphabetic (or start),
            // char after must be non-alphabetic (or end).
            let before_ok = i == 0 || !original[i - 1].is_alphabetic();
            let after_idx = i + phrase_chars.len();
            let after_ok = after_idx == original.len() || !original[after_idx].is_alphabetic();
            if !before_ok || !after_ok {
                continue;
            }

            // Eat one leading space already in `result` if this replacement starts a new line
            // or is a punctuation that should be glued to the previous word.
            if matches!(replacement, "\n" | "\n\n" | "." | "," | "?" | "!" | "\"")
                && result.ends_with(' ')
            {
                result.pop();
            }
            result.push_str(replacement);
            i = after_idx;
            // Eat one trailing space so "vírgula mais" doesn't become ",  mais"
            if i < original.len() && original[i] == ' ' {
                if matches!(replacement, "\n" | "\n\n") {
                    i += 1;
                }
            }
            matched = true;
            break;
        }

        if !matched {
            result.push(original[i]);
            i += 1;
        }
    }

    result
}
```

- [ ] **Step 4: Wire `replace_voice_commands` into `normalize`**

In `src-tauri/src/postprocess.rs`, replace the existing `pub fn normalize(text: &str) -> String { ... }` with:

```rust
/// Apply all normalization passes. Order matters:
///   1. Voice command substitutions (must run first so we capitalize correctly later)
///   2. Punctuation spacing fixes
///   3. Whitespace collapsing
///   4. Capitalization
pub fn normalize(text: &str) -> String {
    let s = replace_voice_commands(text);
    let s = fix_punctuation_spacing(&s);
    let s = collapse_whitespace(&s);
    capitalize_sentences(&s)
}
```

- [ ] **Step 5: Run tests**

Run: `cd src-tauri && cargo test --lib postprocess::`
Expected: all postprocess tests pass.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/postprocess.rs
git commit -m "feat(postprocess): replace spoken formatting commands with punctuation"
```

---

### Task 10: Apply normalization in the dictation loop

**Files:**
- Modify: `src-tauri/src/dictation.rs` (inside `run_transcription_loop`)

- [ ] **Step 1: Apply `postprocess::normalize` to emitted text**

In `src-tauri/src/dictation.rs`, inside `run_transcription_loop`, locate the `if !pass_text.is_empty() { let full_text = ...; let _ = app_handle.emit(...); ... }` block. Replace it with:

```rust
        if !pass_text.is_empty() {
            let stable_text = committed_segments.join(" ");
            let raw_full = if stable_text.is_empty() {
                pass_text.clone()
            } else {
                format!("{} {}", stable_text, pass_text)
            };
            let full_text = crate::postprocess::normalize(&raw_full);
            let stable_normalized = crate::postprocess::normalize(&stable_text);
            let provisional_normalized = crate::postprocess::normalize(&pass_text);

            let _ = app_handle.emit(
                "dictation://segment",
                serde_json::json!({
                    "stableText": stable_normalized,
                    "provisionalText": provisional_normalized,
                    "fullText": full_text,
                }),
            );
            if let Ok(mut last) = last_full_text_out.lock() {
                *last = full_text.clone();
            }
        }
```

- [ ] **Step 2: Apply normalization on the stop-time fast path**

In `src-tauri/src/lib.rs`, inside `stop_dictation`, find the fast-path branch beginning with `if !last_full.trim().is_empty()` (around line 452). Replace `let final_text = last_full.trim().to_string();` with:

```rust
        let final_text = crate::postprocess::normalize(last_full.trim());
```

- [ ] **Step 3: Apply normalization in the finalize worker outcome**

In `src-tauri/src/transcribe/job.rs`, inside `run_finalize_dictation`, find the `Ok(_) => { let final_text = accumulated.lock()...; ...; FinalizeOutcome::Complete { final_text, ... } }` arm (around lines 222–232). Replace it with:

```rust
        Ok(_) => {
            let raw_final = accumulated
                .lock()
                .map(|a| a.clone())
                .unwrap_or(committed_prefix);
            let final_text = crate::postprocess::normalize(&raw_final);
            eprintln!("[finalize id={}] complete: {} chars", id, final_text.len());
            FinalizeOutcome::Complete {
                final_text,
                duration_secs,
            }
        }
```

- [ ] **Step 4: Verify build and run tests**

Run: `cd src-tauri && cargo build --quiet && cargo test --lib`
Expected: build succeeds, all tests pass.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/dictation.rs src-tauri/src/lib.rs src-tauri/src/transcribe/job.rs
git commit -m "feat(dictation): normalize whisper output with postprocess pipeline"
```

---

## Phase 4 — Paragraph by pause

### Task 11: Insert paragraph break on long pause

**Files:**
- Modify: `src-tauri/src/dictation.rs`

- [ ] **Step 1: Track long-pause boundaries in the loop**

In `src-tauri/src/dictation.rs`, add this constant near the other constants at the top of the file (around line 10–14):

```rust
const PARAGRAPH_PAUSE_POLLS: u32 = 5; // ~2.5s of silence (5 × 500ms poll)
```

- [ ] **Step 2: Add a paragraph-break flag and rollover-on-pause behavior**

In `run_transcription_loop`, after the line `let mut consecutive_silent_polls: u32 = 0;`, add:

```rust
    let mut pending_paragraph_break: bool = false;
```

Then, **inside** the `if chunk_is_silent && consecutive_silent_polls >= 2 { state.store(STATE_PAUSED, ...); }` block, extend it to also flag the paragraph break and roll over the buffer:

Replace the existing two-branch state update block (the `if chunk_is_silent && consecutive_silent_polls >= 2 { ... } else if !chunk_is_silent && state.load(...) == STATE_PAUSED { ... }`) with:

```rust
        if chunk_is_silent && consecutive_silent_polls >= 2 {
            state.store(STATE_PAUSED, Ordering::Release);

            // A pause this long is a paragraph boundary. Commit the current
            // pass text (if any) as a segment, clear the buffer, and queue a
            // paragraph break for the NEXT non-empty emission so the break
            // appears between paragraphs, not before an empty next chunk.
            if consecutive_silent_polls == PARAGRAPH_PAUSE_POLLS && !accumulated_raw.is_empty() {
                let mono_16k = convert_to_mono_16k(&accumulated_raw, channels, source_rate);
                if let Ok(text) = transcriber.transcribe_samples(&mono_16k, language) {
                    let text = text.trim().to_string();
                    if !text.is_empty() {
                        committed_segments.push(text.clone());
                        if let Ok(mut sink) = committed_out.lock() {
                            sink.push(text);
                        }
                        pending_paragraph_break = true;
                    }
                }
                accumulated_raw.clear();
                last_transcribed_len = 0;
            }
        } else if !chunk_is_silent && state.load(Ordering::Acquire) == STATE_PAUSED {
            state.store(STATE_LISTENING, Ordering::Release);
        }
```

- [ ] **Step 3: Use the flag when assembling stableText**

Still in `run_transcription_loop`, replace the existing `if !pass_text.is_empty() { ... }` block (from Task 10) with:

```rust
        if !pass_text.is_empty() {
            let separator = if pending_paragraph_break { "\n\n" } else { " " };
            let stable_text = committed_segments.join(separator);
            let raw_full = if stable_text.is_empty() {
                pass_text.clone()
            } else {
                format!("{}{}{}", stable_text, separator, pass_text)
            };
            // Reset the flag — it has been consumed by this emission.
            pending_paragraph_break = false;
            let full_text = crate::postprocess::normalize(&raw_full);
            let stable_normalized = crate::postprocess::normalize(&stable_text);
            let provisional_normalized = crate::postprocess::normalize(&pass_text);

            let _ = app_handle.emit(
                "dictation://segment",
                serde_json::json!({
                    "stableText": stable_normalized,
                    "provisionalText": provisional_normalized,
                    "fullText": full_text,
                }),
            );
            if let Ok(mut last) = last_full_text_out.lock() {
                *last = full_text.clone();
            }
        }
```

- [ ] **Step 4: Update the size-based rollover to use the same `\n\n` style when there is already committed paragraph history**

Still in `run_transcription_loop`, find the `if accumulated_raw.len() > max_raw_samples { ... }` block. Replace it with:

```rust
        if accumulated_raw.len() > max_raw_samples {
            if !pass_text.is_empty() {
                committed_segments.push(pass_text.clone());
                if let Ok(mut sink) = committed_out.lock() {
                    sink.push(pass_text);
                }
            }
            accumulated_raw.clear();
            last_transcribed_len = 0;
        }
```

(No change needed if it already matches — leave as-is.)

- [ ] **Step 5: Verify build and tests**

Run: `cd src-tauri && cargo build --quiet && cargo test --lib`
Expected: build succeeds, all tests pass.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/dictation.rs
git commit -m "feat(dictation): insert paragraph break on pauses longer than 2.5s"
```

---

## Phase 5 — WAV preservation + auto-save

### Task 12: Add `audio_path` column to transcriptions

**Files:**
- Modify: `src-tauri/src/db/store.rs`

- [ ] **Step 1: Write a failing test for the migration**

In `src-tauri/src/db/store.rs`, add to the existing `mod tests` block:

```rust
    #[test]
    fn migration_adds_audio_path_column_with_null_default() {
        let temp_file = NamedTempFile::new().expect("temp file");
        let path = temp_file.path().to_path_buf();

        {
            let conn = rusqlite::Connection::open(&path).expect("open raw");
            conn.execute(
                "CREATE TABLE transcriptions (
                    id INTEGER PRIMARY KEY,
                    title TEXT NOT NULL,
                    text TEXT NOT NULL,
                    language TEXT NOT NULL,
                    duration_secs REAL NOT NULL,
                    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                    summary TEXT,
                    status TEXT NOT NULL DEFAULT 'complete'
                )",
                [],
            )
            .expect("create pre-audio_path schema");
            conn.execute(
                "INSERT INTO transcriptions (title, text, language, duration_secs) VALUES ('old', 'a', 'pt', 1.0)",
                [],
            )
            .expect("seed");
        }

        let store = Store::new(&path).expect("upgrade open");
        let row: Option<String> = store
            .conn
            .query_row(
                "SELECT audio_path FROM transcriptions LIMIT 1",
                [],
                |r| r.get(0),
            )
            .expect("query");
        assert!(row.is_none());
    }

    #[test]
    fn set_audio_path_persists_and_get_returns_it() {
        let (store, _temp_file) = create_temp_store();
        let id = store.insert_partial("t", "pt").expect("insert");
        store.set_audio_path(id, "/tmp/dictation_42.wav").expect("set");
        let row = store.get(id).expect("get");
        assert_eq!(row.audio_path.as_deref(), Some("/tmp/dictation_42.wav"));
    }
```

- [ ] **Step 2: Run tests to verify failure**

Run: `cd src-tauri && cargo test --lib db::store::tests::migration_adds_audio_path_column_with_null_default db::store::tests::set_audio_path_persists_and_get_returns_it`
Expected: FAIL — `no such column: audio_path` and `no method named set_audio_path`.

- [ ] **Step 3: Extend the `Transcription` struct**

In `src-tauri/src/db/store.rs`, replace the existing `pub struct Transcription { ... }` with:

```rust
#[derive(Debug, Serialize, Clone)]
pub struct Transcription {
    pub id: i64,
    pub title: String,
    pub text: String,
    pub language: String,
    pub duration_secs: f64,
    pub created_at: String,
    pub summary: Option<String>,
    pub status: String,
    pub audio_path: Option<String>,
}
```

- [ ] **Step 4: Add the migration in `Store::new`**

In `src-tauri/src/db/store.rs`, inside `Store::new` (around line 65), **after** the existing `match migration_result { ... }` block, add a second migration:

```rust
        let audio_path_migration = conn.execute(
            "ALTER TABLE transcriptions ADD COLUMN audio_path TEXT",
            [],
        );
        match audio_path_migration {
            Ok(_) => {}
            Err(rusqlite::Error::SqliteFailure(_, Some(msg)))
                if msg.contains("duplicate column name") => {}
            Err(e) => return Err(format!("Failed to add audio_path column: {}", e)),
        }
```

- [ ] **Step 5: Update `get` and `list` to read the new column**

In `src-tauri/src/db/store.rs`, locate the SQL strings in `list` and `get`. Change both `SELECT id, title, text, language, duration_secs, created_at, summary, status FROM transcriptions` to:

```sql
SELECT id, title, text, language, duration_secs, created_at, summary, status, audio_path FROM transcriptions
```

Then, in both row-mapping closures, add `audio_path: row.get(8)?,` after the existing `status: row.get(7)?,` line.

- [ ] **Step 6: Add `set_audio_path`**

In `src-tauri/src/db/store.rs`, **after** the existing `pub fn mark_complete(...)` method (around line 134), add:

```rust
    pub fn set_audio_path(&self, id: i64, audio_path: &str) -> Result<(), String> {
        let affected = self
            .conn
            .execute(
                "UPDATE transcriptions SET audio_path = ?1 WHERE id = ?2",
                params![audio_path, id],
            )
            .map_err(|e| format!("Failed to set audio_path: {}", e))?;
        if affected == 0 {
            return Err(format!("Transcription with id {} not found", id));
        }
        Ok(())
    }
```

- [ ] **Step 7: Run all DB tests**

Run: `cd src-tauri && cargo test --lib db::store`
Expected: all tests pass, including the two new ones.

- [ ] **Step 8: Commit**

```bash
git add src-tauri/src/db/store.rs
git commit -m "feat(db): add nullable audio_path column to transcriptions"
```

---

### Task 13: Persist WAV during dictation

**Files:**
- Modify: `src-tauri/src/dictation.rs`
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: Add WAV writer to `DictationSession`**

In `src-tauri/src/dictation.rs`, **append** to the existing `use` block:

```rust
use crate::audio::wav_writer::AudioWavWriter;
use std::path::PathBuf;
```

Then, in the `pub struct DictationSession { ... }` declaration, add two new fields at the end:

```rust
    wav_writer: Option<AudioWavWriter>,
    audio_path: Option<PathBuf>,
```

Update `DictationSession::new()` to initialize them:

```rust
            wav_writer: None,
            audio_path: None,
```

- [ ] **Step 2: Open the WAV at the start of `DictationSession::start`**

In `src-tauri/src/dictation.rs`, replace the current `pub fn start(&mut self) -> Result<(), String>` signature with one that accepts an output path:

```rust
    pub fn start(&mut self, audio_path: PathBuf) -> Result<(), String> {
```

Then, immediately after the existing `let sample_format = config.sample_format();` line and **before** `let buffer = self.audio_buffer.clone();`, add:

```rust
        let writer = AudioWavWriter::new(&audio_path, self.source_rate, self.channels)?;
        let writer_handle = writer.writer_handle();
        self.wav_writer = Some(writer);
        self.audio_path = Some(audio_path);
```

- [ ] **Step 3: Write samples to disk inside the cpal callbacks**

In `src-tauri/src/dictation.rs`, replace the `let stream = match sample_format { ... };` block (the one you edited in Task 5) with:

```rust
        let peak_for_i16 = self.last_peak_bits.clone();
        let peak_for_f32 = self.last_peak_bits.clone();
        let writer_i16 = writer_handle.clone();
        let writer_f32 = writer_handle.clone();
        let stream = match sample_format {
            SampleFormat::I16 => device
                .build_input_stream(
                    &config.into(),
                    move |data: &[i16], _| {
                        let mut peak: f32 = 0.0;
                        if let Ok(mut buf) = buffer.lock() {
                            for &s in data {
                                let f = s as f32 / i16::MAX as f32;
                                let a = f.abs();
                                if a > peak {
                                    peak = a;
                                }
                                buf.push(f);
                            }
                        }
                        peak_for_i16.store(peak.to_bits(), Ordering::Relaxed);
                        if let Ok(mut guard) = writer_i16.lock() {
                            if let Some(ref mut w) = *guard {
                                for &s in data {
                                    let _ = w.write_sample(s);
                                }
                            }
                        }
                    },
                    |err| eprintln!("Dictation stream error: {}", err),
                    None,
                )
                .map_err(|e| format!("Failed to build input stream: {}", e))?,
            SampleFormat::F32 => device
                .build_input_stream(
                    &config.into(),
                    move |data: &[f32], _| {
                        let mut peak: f32 = 0.0;
                        if let Ok(mut buf) = buffer.lock() {
                            for &s in data {
                                let a = s.abs();
                                if a > peak {
                                    peak = a;
                                }
                                buf.push(s);
                            }
                        }
                        peak_for_f32.store(peak.to_bits(), Ordering::Relaxed);
                        if let Ok(mut guard) = writer_f32.lock() {
                            if let Some(ref mut w) = *guard {
                                for &s in data {
                                    let _ = w.write_sample((s * i16::MAX as f32) as i16);
                                }
                            }
                        }
                    },
                    |err| eprintln!("Dictation stream error: {}", err),
                    None,
                )
                .map_err(|e| format!("Failed to build input stream: {}", e))?,
            format => return Err(format!("Unsupported sample format: {:?}", format)),
        };
```

- [ ] **Step 4: Finalize the WAV in `stop_and_join`**

In `src-tauri/src/dictation.rs`, replace `pub fn stop_and_join(&mut self)` with:

```rust
    pub fn stop_and_join(&mut self) {
        self.running.store(false, Ordering::Release);
        self.stream.take();
        if let Some(handle) = self.worker.take() {
            let _ = handle.join();
        }
        if let Some(handle) = self.level_worker.take() {
            let _ = handle.join();
        }
        if let Some(writer) = self.wav_writer.take() {
            if let Err(e) = writer.finalize() {
                eprintln!("[dictation] failed to finalize WAV: {}", e);
            }
        }
    }

    pub fn audio_path(&self) -> Option<PathBuf> {
        self.audio_path.clone()
    }
```

- [ ] **Step 5: Update the caller in `start_dictation`**

In `src-tauri/src/lib.rs`, inside `start_dictation`, find `session.start()?;` and replace it with:

```rust
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| format!("System clock error: {}", e))?
        .as_millis();
    let audio_path = state.data_dir.join(format!("dictation_{}.wav", timestamp));
    session.start(audio_path)?;
```

- [ ] **Step 6: Verify build**

Run: `cd src-tauri && cargo build --quiet`
Expected: success.

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/dictation.rs src-tauri/src/lib.rs
git commit -m "feat(dictation): persist raw mic audio as WAV during the session"
```

---

### Task 14: Create DB row at start; persist text periodically; finalize at stop

**Files:**
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/src/dictation.rs`

- [ ] **Step 1: Add a partial-row id and persistence timing to `run_transcription_loop`**

In `src-tauri/src/dictation.rs`, change the signature of `run_transcription_loop`:

```rust
#[allow(clippy::too_many_arguments)]
pub fn run_transcription_loop(
    buffer: Arc<Mutex<Vec<f32>>>,
    running: Arc<AtomicBool>,
    committed_out: Arc<Mutex<Vec<String>>>,
    last_full_text_out: Arc<Mutex<String>>,
    final_audio_out: Arc<Mutex<Vec<f32>>>,
    state: Arc<AtomicU8>,
    transcriber: &Transcriber,
    language: &str,
    source_rate: u32,
    channels: u16,
    partial_id: i64,
    store: std::sync::Arc<Mutex<crate::db::store::Store>>,
    app_handle: tauri::AppHandle,
) {
```

After `let mut last_transcribed_len: usize = 0;`, add:

```rust
    let mut last_persist = std::time::Instant::now();
    const PERSIST_INTERVAL_MS: u128 = 5000;
    let started_at = std::time::Instant::now();
```

Inside the `if !pass_text.is_empty() { ... }` block (the one that emits `dictation://segment`), **after** `if let Ok(mut last) = last_full_text_out.lock() { *last = full_text.clone(); }`, add:

```rust
            if last_persist.elapsed().as_millis() >= PERSIST_INTERVAL_MS {
                last_persist = std::time::Instant::now();
                if let Ok(s) = store.lock() {
                    let elapsed_secs = started_at.elapsed().as_secs_f64();
                    let _ = s.update_text(partial_id, &full_text, elapsed_secs);
                }
            }
```

- [ ] **Step 2: Pass the id and store from `start_dictation`**

In `src-tauri/src/lib.rs`, inside `start_dictation`, **before** the `let app_for_loop = app_handle.clone();` line, replace the existing chunk that previously read:

```rust
    let state_for_loop = session.state();
    let app_for_loop = app_handle.clone();
    let language_owned = language.clone();
```

with:

```rust
    let state_for_loop = session.state();
    let store_for_loop = state.store.clone();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| format!("System clock error: {}", e))?
        .as_secs();
    let title = format!("Dictation {}", now);
    let partial_id = {
        let store = state.store.lock().map_err(|e| e.to_string())?;
        let id = store.insert_partial(&title, &language)?;
        if let Some(path) = session.audio_path() {
            let _ = store.set_audio_path(id, &path.to_string_lossy());
        }
        id
    };
    let app_for_loop = app_handle.clone();
    let language_owned = language.clone();
```

Then, inside `std::thread::spawn(move || { dictation::run_transcription_loop(...) })`, update the call to include the new arguments. Replace the call with:

```rust
        dictation::run_transcription_loop(
            buffer,
            running,
            committed_out,
            last_full_text_out,
            final_audio_out,
            state_for_loop,
            &transcriber,
            &language_owned,
            source_rate,
            channels,
            partial_id,
            store_for_loop,
            app_for_loop.clone(),
        );
```

- [ ] **Step 3: Return the partial id from `start_dictation` so the frontend can hold it**

In `src-tauri/src/lib.rs`, change `start_dictation`'s signature return type from `Result<(), String>` to `Result<i64, String>` and replace its final `Ok(())` with `Ok(partial_id)`.

- [ ] **Step 4: Reuse the same id in `stop_dictation`**

In `src-tauri/src/lib.rs`, change `stop_dictation`'s signature to accept the id and skip the duplicate insert. Replace:

```rust
async fn stop_dictation(
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
    title: String,
    language: String,
    duration_secs: f64,
) -> Result<i64, String> {
```

with:

```rust
async fn stop_dictation(
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
    partial_id: i64,
    title: String,
    language: String,
    duration_secs: f64,
) -> Result<i64, String> {
```

Then, inside the function body, find the block:

```rust
    let id = {
        let store = state.store.lock().map_err(|e| e.to_string())?;
        let new_id = store.insert_partial(&title, &language)?;
        if !partial_text.is_empty() {
            store.update_text(new_id, &partial_text, duration_secs)?;
        }
        new_id
    };
```

Replace it with:

```rust
    let id = partial_id;
    {
        let store = state.store.lock().map_err(|e| e.to_string())?;
        store.update_title(id, &title)?;
        if !partial_text.is_empty() {
            store.update_text(id, &partial_text, duration_secs)?;
        }
    }
```

- [ ] **Step 5: Add `update_title` to `Store`**

In `src-tauri/src/db/store.rs`, **after** the existing `set_audio_path` method, add:

```rust
    pub fn update_title(&self, id: i64, title: &str) -> Result<(), String> {
        let affected = self
            .conn
            .execute(
                "UPDATE transcriptions SET title = ?1 WHERE id = ?2",
                params![title, id],
            )
            .map_err(|e| format!("Failed to update title: {}", e))?;
        if affected == 0 {
            return Err(format!("Transcription with id {} not found", id));
        }
        Ok(())
    }
```

Also add a test in the `mod tests` block:

```rust
    #[test]
    fn update_title_changes_title() {
        let (store, _temp_file) = create_temp_store();
        let id = store.insert_partial("Old Title", "pt").expect("insert");
        store.update_title(id, "New Title").expect("update");
        let row = store.get(id).expect("get");
        assert_eq!(row.title, "New Title");
    }
```

- [ ] **Step 6: Update the frontend `Dictation.svelte` to capture and forward `partial_id`**

In `src/lib/Dictation.svelte`, replace the `startDictation` function with:

```javascript
    let partialId = $state(null);

    async function startDictation() {
        try {
            error = "";
            liveText = "";
            percent = 0;
            partialId = await invoke("start_dictation", { language: locale });
            phase = "recording";
            elapsed = 0;
            timer = setInterval(() => { elapsed += 1; }, 1000);
        } catch (e) {
            error = e;
        }
    }
```

And update `stopDictation` to pass `partialId`:

```javascript
    async function stopDictation() {
        try {
            clearInterval(timer);
            timer = null;
            const now = new Date().toLocaleString(
                locale === "pt" ? "pt-BR" : "en-US",
            );
            recordedDurationLabel = `${t("recordedDuration")} ${formatTime(elapsed)}`;
            phase = "finalizing";
            appBusy.set(true);
            await invoke("stop_dictation", {
                partialId,
                title: `${t("dictation")} ${now}`,
                language: locale,
                durationSecs: elapsed,
            });
        } catch (e) {
            error = e;
            phase = "idle";
            appBusy.set(false);
            recordedDurationLabel = "";
        }
    }
```

- [ ] **Step 7: Verify build and tests**

Run: `cd src-tauri && cargo build --quiet && cargo test --lib && npm run check`
Expected: backend builds, tests pass, `svelte-check` passes.

- [ ] **Step 8: Commit**

```bash
git add src-tauri/src/lib.rs src-tauri/src/dictation.rs src-tauri/src/db/store.rs src/lib/Dictation.svelte
git commit -m "feat(dictation): create row at start, auto-save every 5s, finalize at stop"
```

---

## Phase 6 — Frontend UX

### Task 15: VU meter component

**Files:**
- Create: `src/lib/VuMeter.svelte`

- [ ] **Step 1: Create the VU meter component**

Create `src/lib/VuMeter.svelte` with this content:

```svelte
<script>
    let { peak = 0 } = $props();

    // Visual: 16 segments, light up proportionally to peak.
    // Smooth decay so the bar doesn't flicker.
    const SEGMENTS = 16;
    let displayed = $state(0);
    let raf = null;

    $effect(() => {
        const target = Math.min(1, Math.max(0, peak));
        if (raf) cancelAnimationFrame(raf);
        const animate = () => {
            const diff = target - displayed;
            // Rise quickly, decay slowly.
            const step = diff > 0 ? diff * 0.6 : diff * 0.15;
            displayed = Math.max(0, displayed + step);
            if (Math.abs(target - displayed) > 0.005) {
                raf = requestAnimationFrame(animate);
            }
        };
        animate();
    });

    let activeCount = $derived(Math.round(displayed * SEGMENTS));
</script>

<div class="vu" aria-label="Audio input level">
    {#each Array(SEGMENTS) as _, i}
        <span
            class="seg"
            class:on={i < activeCount}
            class:hot={i >= SEGMENTS * 0.75}
        ></span>
    {/each}
</div>

<style>
    .vu {
        display: flex;
        gap: 2px;
        align-items: stretch;
        height: 14px;
        width: 100%;
        max-width: 240px;
    }
    .seg {
        flex: 1;
        background: var(--border);
        border-radius: 2px;
        opacity: 0.5;
        transition: background 80ms linear, opacity 80ms linear;
    }
    .seg.on {
        opacity: 1;
        background: var(--info);
    }
    .seg.on.hot {
        background: var(--accent);
    }
</style>
```

- [ ] **Step 2: Verify svelte-check**

Run: `npm run check`
Expected: no new errors.

- [ ] **Step 3: Commit**

```bash
git add src/lib/VuMeter.svelte
git commit -m "feat(ui): add VU meter component"
```

---

### Task 16: State indicator + provisional/stable rendering in Dictation.svelte

**Files:**
- Modify: `src/lib/Dictation.svelte`
- Modify: `src/lib/i18n.js`

- [ ] **Step 1: Add i18n strings**

Open `src/lib/i18n.js`. Inside the `pt` translations object, add:

```javascript
        stateListening: "Ouvindo",
        stateProcessing: "Processando",
        statePaused: "Pausa detectada",
        provisionalHint: "Texto cinza ainda pode ser revisado; texto preto já está confirmado.",
```

Inside the `en` translations object, add:

```javascript
        stateListening: "Listening",
        stateProcessing: "Processing",
        statePaused: "Paused",
        provisionalHint: "Gray text may still be refined; black text is confirmed.",
```

- [ ] **Step 2: Wire new event subscriptions and stable/provisional state**

In `src/lib/Dictation.svelte`, replace the `<script>` block from `let { onTranscribed } = $props();` down to (but not including) `onMount(async () => {` with:

```javascript
    let { onTranscribed } = $props();

    /** @type {"idle" | "recording" | "finalizing" | "cancelling"} */
    let phase = $state("idle");
    let error = $state("");
    let stableText = $state("");
    let provisionalText = $state("");
    let liveText = $state(""); // legacy single-string view used during finalize
    let elapsed = $state(0);
    let percent = $state(0);
    let recordedDurationLabel = $state("");
    let peak = $state(0);
    /** @type {"listening" | "processing" | "paused"} */
    let dictationState = $state("listening");
    let partialId = $state(null);
    /** @type {ReturnType<typeof setInterval> | null} */
    let timer = null;

    /** @type {Array<() => void>} */
    let unlisteners = [];

    function isFinalizing() {
        return phase === "finalizing" || phase === "cancelling";
    }

    function stateClass(s) {
        if (s === "processing") return "state-processing";
        if (s === "paused") return "state-paused";
        return "state-listening";
    }

    function stateLabel(s) {
        if (s === "processing") return t("stateProcessing");
        if (s === "paused") return t("statePaused");
        return t("stateListening");
    }
```

- [ ] **Step 3: Register the new event listeners**

In `src/lib/Dictation.svelte`, replace the entire `onMount(async () => { unlisteners.push(...); });` block with:

```javascript
    onMount(async () => {
        unlisteners.push(
            await listen("dictation://segment", (event) => {
                if (phase !== "recording") return;
                stableText = event.payload.stableText ?? "";
                provisionalText = event.payload.provisionalText ?? "";
                liveText = event.payload.fullText ?? "";
            }),
            await listen("dictation://level", (event) => {
                if (phase !== "recording") return;
                peak = event.payload.peak ?? 0;
            }),
            await listen("dictation://state", (event) => {
                if (phase !== "recording") return;
                dictationState = event.payload.state ?? "listening";
            }),
            await listen("transcription://text", (event) => {
                if (!isFinalizing()) return;
                const incoming = event.payload.text ?? "";
                if (incoming.length > liveText.length) {
                    liveText = incoming;
                }
            }),
            await listen("transcription://progress", (event) => {
                if (!isFinalizing()) return;
                percent = event.payload.percent;
            }),
            await listen("transcription://complete", (event) => {
                if (!isFinalizing()) return;
                percent = 100;
                setTimeout(() => {
                    phase = "idle";
                    appBusy.set(false);
                    stableText = "";
                    provisionalText = "";
                    liveText = "";
                    percent = 0;
                    peak = 0;
                    dictationState = "listening";
                    recordedDurationLabel = "";
                }, 250);
                onTranscribed?.(event.payload.transcription);
            }),
            await listen("transcription://cancelled", (_event) => {
                if (!isFinalizing()) return;
                phase = "idle";
                appBusy.set(false);
                stableText = "";
                provisionalText = "";
                liveText = "";
                percent = 0;
                peak = 0;
                dictationState = "listening";
                recordedDurationLabel = "";
            }),
            await listen("transcription://error", (event) => {
                if (!isFinalizing()) return;
                error = event.payload.error;
                phase = "idle";
                appBusy.set(false);
                recordedDurationLabel = "";
            }),
        );
    });
```

- [ ] **Step 4: Replace the recording-phase UI**

In `src/lib/Dictation.svelte`, replace the `{#if phase === "recording"} ... {/if}` block (currently lines ~138–148) with:

```svelte
    {#if phase === "recording"}
        <div class="status {stateClass(dictationState)}">
            <span class="dot"></span>
            {stateLabel(dictationState)} · {formatTime(elapsed)}
        </div>
        <VuMeter {peak} />
        <button class="btn-stop" onclick={stopDictation}>
            {t("stopDictation")}
        </button>
        {#if stableText || provisionalText}
            <div class="live-text">
                <pre><span class="stable">{stableText}</span>{#if stableText && provisionalText}{" "}{/if}<span class="provisional">{provisionalText}</span></pre>
                <small class="hint">{t("provisionalHint")}</small>
            </div>
        {/if}
    {:else if phase === "finalizing" || phase === "cancelling"}
```

- [ ] **Step 5: Import VuMeter**

In `src/lib/Dictation.svelte`, add to the import block at the top:

```javascript
    import VuMeter from "./VuMeter.svelte";
```

- [ ] **Step 6: Update the styles**

In `src/lib/Dictation.svelte`, **append** to the existing `<style>` block:

```css
    .state-listening .dot {
        background: var(--info);
        animation: pulse 1.4s infinite;
    }
    .state-processing .dot {
        background: var(--accent);
        animation: pulse 0.7s infinite;
    }
    .state-paused .dot {
        background: var(--border);
        animation: none;
        opacity: 0.6;
    }

    .live-text pre .stable {
        color: var(--text);
    }
    .live-text pre .provisional {
        color: var(--muted, #888);
        font-style: italic;
    }
    .live-text .hint {
        display: block;
        margin-top: 8px;
        font-size: 0.8rem;
        color: var(--muted, #888);
    }
```

- [ ] **Step 7: Verify svelte-check**

Run: `npm run check`
Expected: no errors.

- [ ] **Step 8: Manual verification**

Run: `npm run tauri dev` (in one terminal). Once the app launches:
1. Start a dictation. Confirm the state indicator shows "Ouvindo" (listening) with a blue pulse, and the VU meter reacts to your voice.
2. Pause silently for >3 seconds. Confirm the indicator switches to "Pausa detectada" (paused) and the VU meter calms.
3. Resume speaking. Confirm the indicator returns to "Ouvindo", and that a paragraph break (\n\n) appears in the transcript between the two utterances.
4. Speak "novo parágrafo" out loud mid-sentence. Confirm the output shows a paragraph break.
5. Stop dictation. Confirm the transcript saves and the WAV file exists at `~/.local/share/com.nuuvem.martin/dictation_*.wav`.

- [ ] **Step 9: Commit**

```bash
git add src/lib/Dictation.svelte src/lib/i18n.js
git commit -m "feat(ui): three-state indicator, VU meter, and provisional/stable text rendering"
```

---

## Phase 7 — Documentation

### Task 17: Update README to reflect new dictation behavior

**Files:**
- Modify: `README.md`

- [ ] **Step 1: Update the Dictation mode feature bullets**

In `README.md`, replace the entire `### Dictation mode` block under `## Features` (currently 4 bullets) with:

```markdown
### Dictation mode
- **Real-time transcription** — speaks and sees text appear live as you talk
- **Mic-only capture** — uses microphone directly, no system audio needed
- **Three-state indicator** — Listening / Processing / Paused, so you know what the app is doing
- **VU meter** — visual confirmation that the microphone is picking up your voice
- **Provisional vs stable text** — recently transcribed text shown in gray italic (may still be refined), confirmed text in normal style
- **Automatic paragraphs** — long pauses (>2.5s) become paragraph breaks in the output
- **Voice formatting commands** — say "novo parágrafo", "nova linha", "ponto final", "vírgula", "ponto de interrogação", "ponto de exclamação", "abre aspas", "fecha aspas" to insert formatting
- **Smart capitalization** — sentences are capitalized automatically and punctuation spacing is normalized
- **Silence-aware** — skips whisper passes during silence to save CPU
- **Continuous auto-save** — partial transcript is persisted every 5 seconds; nothing is lost if the app crashes
- **Audio preserved** — raw mic audio is saved as a WAV alongside the transcript so you can reprocess later if needed
```

- [ ] **Step 2: Rewrite the "Dictation mode" subsection under "How audio capture works"**

In `README.md`, replace the entire `### Dictation mode` block under `## How audio capture works` (currently four bullet points describing the loop) with:

```markdown
### Dictation mode

Captures microphone audio via cpal and transcribes in real time:

- Audio streams into a shared buffer at the device's native sample rate
- A second thread emits the audio level (`dictation://level`) and session state (`dictation://state`, one of `listening`/`processing`/`paused`) for UI feedback
- Every ~500ms, the transcription loop drains new samples, measures RMS, and skips the whisper pass if the chunk is silent
- When there is enough new audio, the full accumulated buffer is converted to mono 16kHz and sent to Whisper for re-transcription (this is what keeps the output accurate — Whisper self-corrects with more context)
- The output passes through a text-normalization pipeline (voice command substitution → punctuation spacing → whitespace collapse → sentence capitalization) before being emitted
- A pause longer than ~2.5s commits the current text as a segment and inserts a paragraph break (`\n\n`) before the next segment
- When the buffer exceeds 120 seconds, the current segment is committed (reusing the last transcription) and a new buffer starts
- The mic audio is written to a WAV file in real time, so the audio is available for reprocessing after the session ends
- The partial transcription row is created on `Start Dictation` and updated every ~5s during the session; on `Stop Dictation` the same row is finalized
```

- [ ] **Step 3: Update the Architecture tree**

In `README.md`, replace the existing fenced ` ```\nsrc-tauri/src/\n... ` block under `## Architecture` with:

```
src-tauri/src/
├── lib.rs              # Tauri commands, app state
├── audio/
│   ├── capture.rs      # Mic (cpal) + system audio (pw-record)
│   ├── mix.rs          # WAV mixing (mic + system → single file)
│   └── wav_writer.rs   # Thread-safe WAV writer (used by recorder and dictation)
├── db/
│   └── store.rs        # SQLite CRUD for transcriptions + pending recordings
├── dictation.rs        # Real-time mic capture + transcription loop + level/state emitter
├── model.rs            # Auto-download Whisper model with progress events
├── postprocess.rs      # Pure text normalization: voice commands, spacing, capitalization
├── summarize.rs        # Claude CLI integration for AI summaries
├── vad.rs              # Pure RMS-based silence detection helpers
└── transcribe/
    ├── whisper.rs      # Whisper transcription + WAV loading + resampling
    └── job.rs          # Finalize worker + cancel/progress orchestration

src/
├── lib/
│   ├── i18n.js         # Locale detection + translations (pt/en)
│   ├── format.js       # Shared date/duration formatting
│   ├── appBusy.js      # Cross-tab "app busy" store
│   ├── Recorder.svelte # Recording controls + pending recordings list
│   ├── Dictation.svelte # Real-time dictation with state/level/provisional UI
│   ├── VuMeter.svelte  # Audio-level meter component
│   ├── FinalizingProgress.svelte # Progress overlay for finalize phase
│   ├── ModelDownload.svelte # Model download progress overlay
│   ├── History.svelte  # Transcription list
│   └── TranscriptionView.svelte  # View + copy + summarize
└── routes/
    └── +page.svelte    # Main page (three tabs: Record / Dictation / History)
```

- [ ] **Step 4: Verify the README still renders cleanly**

Run: `head -200 README.md | grep -c "^##"`
Expected: at least 6 (Screenshots, Features, Requirements, Install, Usage, How audio capture works are top-level headings).

- [ ] **Step 5: Commit**

```bash
git add README.md
git commit -m "docs(readme): document new dictation UX, audio preservation, and post-processing"
```

---

## Self-Review Checklist

After implementation, verify all 12 scope items from the brainstorm plus the housekeeping tasks:

1. ✅ Provisional vs stable text — Tasks 6, 10, 16
2. ✅ Explicit states (Listening/Processing/Paused) — Tasks 5, 6, 7, 16
3. ✅ VU meter — Tasks 5, 7, 15, 16
4. ✅ Paragraph by pause — Task 11
5. ✅ Capitalization & spacing — Tasks 8, 10
6. ✅ Voice commands — Tasks 9, 10
7. ✅ Continuous auto-save — Task 14
8. ✅ Preserved WAV — Tasks 12, 13
9. ✅ Cargo release profile — Task 1
10. ✅ `set_n_threads` — Task 2
11. ✅ VAD silence gate — Tasks 4, 6
12. ✅ Rollover bug — Task 3
13. ✅ Branch isolation — Task 0
14. ✅ README updated — Task 17
