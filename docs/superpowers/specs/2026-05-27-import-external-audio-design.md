# Import External Audio — Design

**Date:** 2026-05-27
**Status:** Approved for planning
**Feature:** 1 of 3 in the "transcription throughput" set (import → parallel chunking → record-while-transcribing)

## Problem

Martin can only transcribe audio it recorded itself. Users have audio
files from other sources — phone voice memos, meeting recordings, voice
recorders — that they cannot bring into Martin. This feature lets a user
pick an existing audio file and have it transcribed through the normal
flow.

## Goals

- Import common audio formats (mp3, m4a, wav, ogg, flac) from disk.
- The imported file lands in the existing "pending recordings" list, so
  the user chooses title/language and triggers transcription exactly as
  with a recording.
- Stay offline and privacy-first: no external binaries, pure-Rust decode.
- Work on slow / low-RAM machines.

## Non-Goals

- Speeding up transcription of long files — that is Feature 2 (parallel
  chunking). Import alone does not change transcription speed.
- Video containers (mp4/mkv) or exotic formats — would require ffmpeg.
- Drag-and-drop entry — deferred; a native file picker covers the need.
- Re-transcribing the same import without re-importing — out of scope;
  the converted copy is deleted on success (see Lifecycle).

## Decisions

| Decision | Choice | Rationale |
|---|---|---|
| Formats | mp3, m4a, wav, ogg, flac | Covers phone/recorder/meeting output |
| Decoder | Symphonia (pure Rust) | No external dependency; offline-safe |
| Conversion point | At import (Approach A) | Reuses entire transcription pipeline untouched |
| Post-import behavior | Lands in pending list | Maximum reuse of existing flow |
| Entry mechanism | Button + native picker | Familiar, explicit; adds tauri-plugin-dialog |
| Memory strategy | Streaming decode → WAV write | Caps RAM to ~one packet; safe on slow machines |
| Converted copy lifecycle | Deleted on successful finalize | Falls out of existing `finish_job` for free |

### Why convert at import (Approach A)

`transcribe_pending_recording` → `run_finalize_pending_file` →
`load_wav_as_mono_f32` reads WAV only (via `hound`). By converting the
imported file to a WAV in `data_dir` at import time, every downstream
stage is reused with zero changes: the transcription job, WAV loading,
pending deletion, and finalize. Symphonia is confined to one module and
never leaks into `transcribe/`. This also keeps Feature 2 simpler —
everything downstream remains uniform WAV.

The rejected alternative (store the original path, decode at transcribe
time) would spread Symphonia into `transcribe/`, couple the WAV-native
and decoded paths, and reference a file outside `data_dir` that could be
moved or deleted by the user.

## Architecture

### Backend — new module `src-tauri/src/audio/import.rs`

The sole boundary where Symphonia exists.

```
pub struct Imported {
    pub wav_path: PathBuf,
    pub duration_secs: f64,
}

pub fn import_audio(source: &Path, dest_dir: &Path) -> Result<Imported, String>
```

Streaming flow (caps memory to one decoded packet at a time):

1. Open `source`, probe the container, select the default track.
2. Create a `hound::WavWriter` for `imported_<timestamp>.wav` in
   `dest_dir`, mono, at the source sample rate (i16 PCM).
3. Loop: decode one packet → downmix its frames to mono (average across
   channels) → write samples to the WAV writer. Repeat until EOF.
4. Finalize the writer. Compute duration via `wav_duration_secs` on the
   written file.
5. Return `Imported { wav_path, duration_secs }`.

Resampling to 16 kHz is **not** done here — it is deferred to
`load_wav_as_mono_f32` at transcription time, exactly as already happens
for 48 kHz recordings. No resample logic is duplicated.

If the source contains no decodable audio / zero frames, return an error
(do not write an empty pending).

### Backend — new Tauri command

```
#[tauri::command]
async fn import_audio_file(
    state: State<'_, AppState>,
    path: String,
) -> Result<PendingRecording, String>
```

1. Validate `path` exists and has a supported extension.
2. Run `import_audio(source, &state.data_dir)` inside
   `tauri::async_runtime::spawn_blocking` (decode is CPU work; keep the
   async runtime free).
3. `store.save_pending(&wav_path, duration_secs)`.
4. Return the new `PendingRecording`.

Registered in `invoke_handler` alongside the existing commands. Does not
touch `current_job` or `transcriber`, so import never collides with a
running transcription.

### Frontend — `src/lib/Recorder.svelte`

- New **"Import audio"** button beside the recording controls.
- On click: `open()` from `@tauri-apps/plugin-dialog`, filtered to
  `["mp3","m4a","wav","ogg","flac"]`.
- On a chosen path: set an "Importing…" busy state (reuse `appBusy`),
  `invoke("import_audio_file", { path })`, then append the returned
  pending to `pendingRecordings`.
- On error: surface a message; clear the busy state.
- New i18n strings in `src/lib/i18n.js` (`pt` + `en`): button label,
  "Importing…", error messages.

From the pending list onward, the existing
`transcribe_pending_recording` flow handles title, language, progress,
completion, and cleanup — unchanged.

## Data Flow

```
User clicks "Import audio"
  → native dialog (filtered) → file path
  → invoke("import_audio_file", { path })
      → spawn_blocking: Symphonia stream-decode → mono WAV in data_dir
      → save_pending(wav_path, duration)
      → return PendingRecording
  → UI appends to pending list
  → [existing flow] user sets title/language → transcribe_pending_recording
      → run_finalize_pending_file → load_wav_as_mono_f32 (resamples to 16k)
      → finish_job: on success, deletes the WAV + pending row
```

## Lifecycle of the converted WAV

Reuses `finish_job` (`JobKind::PendingFile`) with no new code:

| Outcome | Converted `imported_<ts>.wav` in `data_dir` |
|---|---|
| Transcription complete | Removed automatically |
| Transcription cancelled | Kept — user can re-trigger transcription |
| Transcription error | Kept as partial (same shape as crash recovery) |

The user's **original source file is never touched** — only Martin's
converted copy is created and later removed.

## Error Handling

- Unsupported extension → reject before decoding, clear message.
- Corrupt / undecodable file → Symphonia error propagated as `Err`.
- Zero-frame / silent-container file → error; no empty pending created.
- File missing or unreadable → `Err`.
- Import is independent of `current_job`: it can run while a
  transcription is in progress without conflict.

## Slow-Machine Considerations

- Decode is cheap relative to Whisper; runs off the UI thread via
  `spawn_blocking` with an "Importing…" indicator.
- Streaming decode→write caps memory to ~one packet, removing the
  whole-file-in-RAM ceiling that would risk OOM on low-RAM machines for
  multi-hour files.
- Transcription speed itself is unchanged by this feature; long files on
  weak CPUs remain slow until Feature 2 (parallel chunking).

## Testing (TDD)

**Rust — pure functions (no Symphonia):**
- Downmix stereo → mono (average of channels).
- Downmix multi-channel (>2) → mono.
- Mono WAV write produces the expected sample count and spec.

**Rust — integration with committed fixtures (~0.1 s each):**
- Decode a tiny `.mp3` → expected sample count / duration / mono output.
- Decode a tiny `.flac` and `.ogg` → same assertions.
- Corrupt file → returns `Err`.
- Zero-frame container → returns `Err`.

**Svelte — Vitest (light):**
- Import handler calls `open` then `invoke("import_audio_file")` and
  appends the result to the pending list.
- Error path surfaces a message and clears the busy state.

## New Dependencies

- `symphonia = { version = "0.5", features = ["mp3", "isomp4", "aac", "flac", "ogg", "vorbis", "pcm"] }`
  (m4a = `isomp4` container + `aac` codec).
- `tauri-plugin-dialog` (Rust) + `@tauri-apps/plugin-dialog` (frontend).

## Out of Scope / Follow-ups

- Drag-and-drop import (Tauri 2 file-drop event, no plugin).
- Keeping the converted copy for re-transcription.
- Video / ffmpeg-backed formats.
- Feature 2 (parallel chunking) and Feature 1 (record-while-transcribing)
  are separate specs.
