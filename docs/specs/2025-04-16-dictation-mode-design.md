# Dictation Mode — Real-time Transcription

## Problem

The current flow (record → stop → transcribe) is batch-only and doesn't support real-time dictation. Users who need to see text appearing as they speak have no option today.

## Solution

A new "Dictation" mode that captures mic audio and transcribes it in ~5-second chunks using a sliding window approach. Text segments appear in the UI as they are transcribed. When the user stops, the full text is saved as a transcription in the history.

## Approach

Sliding window with a dedicated transcription thread. Every ~5 seconds of accumulated audio is sent to Whisper with ~1 second of overlap from the previous chunk for context continuity. Results are emitted as Tauri events to the frontend.

## Navigation

New "Dictation" tab in the main navigation: Gravar / **Ditado** / Histórico.

Dictation is a separate flow from recording meetings:
- Recording = mic + system → stop → transcribe later
- Dictation = mic only → text appears live → stop → saves to history

## Backend

### New module: `src-tauri/src/dictation.rs`

```rust
pub struct DictationSession {
    stream: Option<cpal::Stream>,
    audio_buffer: Arc<Mutex<Vec<f32>>>,
    running: Arc<AtomicBool>,
}
```

The audio callback from cpal writes samples (converted to f32, mono, resampled to 16kHz) into the shared `audio_buffer`.

### Transcription loop

Runs on `spawn_blocking`. Pseudocode:

```
overlap_samples = []  // ~1s of previous chunk's end
all_segments = []

while running:
    sleep until buffer has >= 5s of audio (80,000 samples at 16kHz)
    
    chunk = overlap_samples + drain(audio_buffer)
    overlap_samples = chunk[last 16,000 samples]  // 1s overlap
    
    text = transcriber.transcribe_samples(chunk, language)
    all_segments.push(text)
    emit("dictation://segment", { text, full_text: all_segments.join("\n") })

// After loop ends, return all_segments joined
```

### Tauri commands

- `start_dictation(language: String)` — starts mic stream + transcription thread, returns immediately
- `stop_dictation(title: String)` — sets running=false, waits for thread to finish, saves full text to `transcriptions` table, returns `Transcription`

### Whisper — new method

Add `transcribe_samples(&self, samples: &[f32], language: &str) -> Result<String, String>` to `Transcriber`. Same as `transcribe()` but skips WAV loading/resampling since the audio is already mono f32 at 16kHz.

### AppState changes

Add `dictation: Mutex<Option<DictationSession>>` to `AppState`.

The Transcriber is already cached in `AppState.transcriber`. The dictation thread will take it from the mutex at start and return it at stop, same pattern as `transcribe_recording`.

## Frontend

### New component: `src/lib/Dictation.svelte`

- "Iniciar Ditado" / "Parar Ditado" button
- Text area that grows as segments arrive (scrolls to bottom automatically)
- Elapsed time display (reuse `formatTime` from Recorder)
- Listens to `dictation://segment` Tauri event
- On stop: calls `stop_dictation`, navigates to TranscriptionView with result

### i18n keys

```
pt: { dictation, startDictation, stopDictation, dictating }
en: { dictation, startDictation, stopDictation, dictating }
```

### +page.svelte

Add third tab and Dictation component to the view switching logic.

## Audio pipeline

Dictation captures mic only (no system audio, no pw-record). The cpal callback:

1. Receives i16 or f32 samples from the mic
2. Converts to f32 if needed
3. Converts stereo to mono if needed
4. Resamples to 16kHz if needed
5. Pushes to the shared buffer

Resampling in the callback should be simple (linear interpolation, same as existing `Transcriber::resample`). The buffer accumulates ready-to-use 16kHz mono f32 samples.

## Data flow

```
[cpal mic callback] --f32 16kHz mono--> [Arc<Mutex<Vec<f32>>>]
                                              |
                                    [transcription thread]
                                              |
                                    every ~5s: drain buffer
                                              |
                                    [Whisper full() on chunk]
                                              |
                                    [Tauri event: dictation://segment]
                                              |
                                    [Dictation.svelte: append text]
```

## Error handling

- If Whisper fails on a chunk, log the error and skip (don't crash the session)
- If the mic disconnects, stop the session and show error
- The `write_error` AtomicBool pattern from AudioCapture is reused

## Files to create/modify

| File | Action |
|------|--------|
| `src-tauri/src/dictation.rs` | Create — DictationSession, start/stop logic |
| `src-tauri/src/transcribe/whisper.rs` | Modify — add `transcribe_samples` method |
| `src-tauri/src/lib.rs` | Modify — add dictation state, commands, register handler |
| `src/lib/Dictation.svelte` | Create — dictation UI |
| `src/lib/i18n.js` | Modify — add dictation keys |
| `src/routes/+page.svelte` | Modify — add dictation tab |

## Verification

1. `cargo test` — existing tests pass + new `transcribe_samples` test
2. Start dictation, speak for 10-20 seconds — text appears in chunks
3. Stop — full text saved to history, visible in transcription view
4. Start dictation, close app — no crash, no orphaned threads
5. Start dictation without mic — shows error
