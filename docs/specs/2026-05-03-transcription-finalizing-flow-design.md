# Transcription Finalizing Flow

## Problem

On slower machines, transcription takes much longer than the user expects. Two concrete failures observed in field testing (Vivi's laptop, 4 cores at ~900 MHz, AVX2 available, 16 GB RAM partially free):

1. **Dictation loses text.** `stop_dictation` reads the `full_text` from the frontend's `$state` and saves that. If the user clicks "Parar ditado" before whisper has emitted the first `dictation://segment` event, `full_text` is empty, the backend returns `"No text was transcribed"`, and the audio is discarded — even though whisper is *still processing* and would have produced text seconds later. The backend already had `committed_segments` in memory but throws them away when the buffer is shorter than `MAX_BUFFER_SECONDS`.

2. **No feedback during long transcriptions.** Both flows (live dictation and "transcribe pending recording") run whisper as a single long-running call. The UI shows nothing — no spinner, no progress, no "still working". The user can't distinguish "stuck" from "slow", may navigate away mid-process or click Stop multiple times, and ends up with the data loss above.

The user can correctly model what's happening if the UI tells them. Today it doesn't.

## Solution

Move the source-of-truth for transcribed text from the frontend to the backend. Replace the implicit "click Stop and we're done" model with an explicit `Finalizing` phase that:

- Saves a transcription row early and updates it as text arrives.
- Streams progress (% via whisper's progress callback) and live text to the UI.
- Locks the navigation menu while a job is finalizing.
- Allows cancel-with-confirmation; never silently loses content.

This applies to **both** flows — dictation and "transcribe pending recording" — under a unified `TranscriptionJob` abstraction.

## State machine

Mirror the same states on backend and frontend.

```
idle  →  recording  →  finalizing  →  complete
                            ↓
                       cancelling  →  idle
```

| State | Meaning | UI |
|---|---|---|
| `idle` | No active job. | Default views. |
| `recording` | Audio is being captured (dictation only). For the pending-file flow this state is skipped — the job goes straight to `finalizing`. | Live dictation view (existing). |
| `finalizing` | Capture stopped (or never happened). Whisper is processing the remaining audio. Backend updates the DB row as text arrives. | `<FinalizingProgress>` component, navigation locked. |
| `cancelling` | User confirmed cancel. Backend aborts whisper, deletes partial DB row + WAV. | Brief spinner, then `idle`. |
| `complete` | Final text saved, transcription row marked `complete`. | Auto-navigate to the transcription view. |

**Rules:**
- Only one `TranscriptionJob` may exist at a time. Attempts to start another return an error.
- The only exits from `finalizing` are `complete` (whisper finished) or `cancelling` (user confirmed).
- During `finalizing`/`cancelling`, the nav buttons (`record`, `dictation`, `history`) are disabled.

## UI components

### `<FinalizingProgress>` — new shared component (`src/lib/`)

Used by both `Dictation.svelte` and `Recorder.svelte` when their state is `finalizing`. Props: `percent: number`, `liveText: string`, `onCancel: () => void`.

Layout:

```
        ╭─────╮
        │ 67% │     Finalizando transcrição…
        ╰─────╯     Pode levar alguns minutos numa máquina lenta.

  "olá hoje queria falar sobre…"

           [ Cancelar ]
```

The circle is an inline SVG with a `stroke-dasharray` animated from 0 to `percent`. Percentage text rendered in the centre.

The Cancel button is secondary-styled; clicking it opens an in-component confirmation modal (`"Cancelar transcrição? O conteúdo será perdido. [Voltar] [Sim, cancelar]"`). Only the Sim button transitions to `cancelling`.

### Navigation lock

A new global Svelte store `appBusy` (a writable boolean). When `true`:
- Each of the three nav buttons in `+page.svelte` gets `disabled` + `cursor: not-allowed` + a `title` tooltip explaining why.
- Visual position and label stay the same; only the interaction is suppressed.

`Dictation` and `Recorder` set `appBusy = true` on entering `finalizing`/`cancelling` and reset to `false` on `complete`/`idle`.

### Transcribe-pending entry point

The "Transcrever" button on each pending recording lives in `Recorder.svelte` (function `transcribePending`). It currently calls `invoke("transcribe_recording", …)` and awaits the result. Rewire it to call the new `transcribe_pending_recording` command, which returns a job `id` immediately and starts emitting events. The component shows `<FinalizingProgress>` from that moment on, listening to the same `transcription://*` events as the dictation flow.

## Backend architecture

### `TranscriptionJob`

Single struct that represents an active transcription, regardless of source.

```rust
struct TranscriptionJob {
    id: i64,                       // row id in `transcriptions`
    kind: JobKind,
    cancel_flag: Arc<AtomicBool>,
    committed_text: String,        // text already consolidated and persisted
}

enum JobKind {
    Dictation,
    PendingFile { wav_path: PathBuf, pending_id: i64 },
}
```

`AppState` gains `current_job: Mutex<Option<TranscriptionJob>>`. Commands that start work check this and reject ("Another transcription is in progress") if it's already `Some`.

The job is created at the moment the user clicks Stop (dictation) or Transcrever (pending). Before that, there's no job — only a `DictationSession` (capture) or a stored WAV.

### Tauri commands

| Command | Purpose | Returns |
|---|---|---|
| `start_dictation(language)` | Existing — starts capture + worker thread. | `()` |
| `stop_dictation()` | **Changed signature** — no `full_text`. Stops capture, transitions to `finalizing`. | `i64` (job id = transcription row id) |
| `transcribe_pending_recording(pending_id, title, language)` | New, replaces `transcribe_recording`. Starts a `PendingFile` job, returns immediately. | `i64` |
| `cancel_job()` | Sets `cancel_flag`, waits for worker thread to exit, cleans up. | `()` |
| `current_job_status()` | For UI re-sync if the page is reloaded mid-job. | `Option<JobStatus>` with state, percent, text. |

The existing `transcribe_recording` command is removed.

### Events (unified namespace)

| Event | Payload | Emitted when |
|---|---|---|
| `transcription://text` | `{ id: i64, text: String }` | New text consolidated; sent on every update. |
| `transcription://progress` | `{ id: i64, percent: u8 }` | Whisper progress callback fires. |
| `transcription://complete` | `{ id: i64, transcription: Transcription }` | Job finished cleanly, row marked `complete`. |
| `transcription://error` | `{ id: i64, error: String }` | Whisper or DB error mid-stream. Row stays `partial`. |
| `transcription://cancelled` | `{ id: i64 }` | Cancel flow finished. Row deleted. |

The legacy `dictation://segment` event is removed (callers replaced).

### Worker thread

Started by `stop_dictation` (for dictation) or `transcribe_pending_recording` (for pending). Runs on `spawn_blocking`. Pseudocode:

```
fn run_finalize(job, audio_source, language, app_handle) {
    let mut consolidated = job.committed_text.clone();

    // Wire progress + abort callbacks into whisper-rs FullParams
    params.set_progress_callback(|p| {
        emit("transcription://progress", { id: job.id, percent: p });
    });
    params.set_abort_callback(|| {
        job.cancel_flag.load(Ordering::Acquire)
    });

    // Run transcription. For dictation, audio_source is the in-memory buffer.
    // For pending, it's the WAV on disk.
    match run_whisper(audio_source, params) {
        Ok(text) => {
            consolidated = merge(consolidated, text);
            store.update_text(job.id, &consolidated, duration);
            emit("transcription://text", { id: job.id, text: consolidated });

            store.mark_complete(job.id);
            if let JobKind::PendingFile { wav_path, pending_id } = job.kind {
                fs::remove_file(&wav_path);
                store.delete_pending(pending_id);
            }
            emit("transcription://complete", { id, transcription: row });
        }
        Err(WhisperError::Aborted) if job.cancel_flag.load() => {
            store.delete(job.id);
            cleanup_files(job.kind);
            emit("transcription://cancelled", { id: job.id });
        }
        Err(e) => {
            // Row stays `partial` with whatever was saved.
            emit("transcription://error", { id: job.id, error: e });
        }
    }

    *app_state.current_job.lock() = None;
}
```

Mid-stream text updates: whisper-rs's segment callback (`set_segment_callback` / `new_segment_callback`) fires per segment as transcription runs. Hook this to update `consolidated`, persist via `store.update_text`, and emit `transcription://text`. This gives us live text in the pending-file flow without needing to chunk the WAV ourselves.

### Cancellation

`whisper_full_abort_callback` (exposed in whisper-rs as `set_abort_callback`) is called by whisper.cpp periodically during inference. The closure reads `job.cancel_flag` (an `Arc<AtomicBool>`) and returns `true` when set, which aborts `state.full()` cleanly. The worker thread then takes the `cancelled` branch above.

`cancel_job()` sets the flag and waits (with a bounded timeout, say 10s) for the worker to finish. If the worker doesn't respond in time, return an error to the UI and let the user retry — never block the UI indefinitely.

## Persistence

### Schema change

Add a `status` column to `transcriptions`:

```sql
ALTER TABLE transcriptions ADD COLUMN status TEXT NOT NULL DEFAULT 'complete';
```

Values: `'complete'`, `'partial'`, `'failed'`.

Run idempotently inside `Store::new`: catch the `"duplicate column name"` error from rusqlite and ignore it. Matches the project's existing migration style (no migration framework).

`Transcription` struct gets a corresponding `pub status: String` field, surfaced to the frontend.

### Save-early, update-often

When a job enters `finalizing`:

1. Backend `INSERT`s a row with `text = ""` (or the committed text already in memory for dictation), `status = 'partial'`, current title, language, `duration_secs = 0`. Records the `id`.
2. Each text update: `UPDATE transcriptions SET text = ?, duration_secs = ? WHERE id = ?` and emit `transcription://text`.
3. On clean finish: `UPDATE transcriptions SET status = 'complete', duration_secs = ? WHERE id = ?`.
4. On cancel: `DELETE FROM transcriptions WHERE id = ?`.
5. On error: leave `status = 'partial'`, emit error, keep the row.

For pending-file jobs: the WAV file and `pending_recordings` row are deleted only after the row is marked `complete`. On error, the WAV stays so the user can retry.

## Recovery on app start

In `Store::new`, after migration:

```sql
UPDATE transcriptions SET status = 'partial' WHERE status NOT IN ('complete', 'failed');
```

Sanity-check pass. Logs the number of rows affected. Doesn't block startup.

The history list shows partial rows with a `⚠ Parcial` badge. Clicking opens the transcription view with whatever text was captured. Standard delete works to remove them.

## Error handling

| Failure | Behaviour |
|---|---|
| Whisper returns `Err` mid-stream | Row stays `partial` with whatever text the segment callback already persisted, `transcription://error` emitted, UI offers "Manter parcial" or "Descartar". |
| Whisper returns empty text after a full run | Row stays at empty `text` with `status = 'partial'`. UI offers same choice. |
| `cancel_job` called while no job is active | Returns OK (idempotent). |
| `start_dictation` / `transcribe_pending_recording` while a job is active | Returns `Err("Another transcription is in progress")`. |
| App killed during `finalizing` | On next start, partial row remains visible in history with badge. WAV remains for pending-file jobs (so user can retry). For dictation, in-memory audio is lost — see Out of scope. |

## Testing

Following CLAUDE.md (Kent Beck TDD).

### Rust unit tests (`#[cfg(test)]`)

- `Store::add_status_column_if_missing` is idempotent — runs twice, no error.
- `Store::save` accepts `status`; `list()` returns it.
- `Store::update_text(id, text, duration)` updates only the targeted row.
- `Store::mark_complete(id)` flips status.
- `Store::reset_partial_on_startup` only touches non-complete rows.
- `TranscriptionJob` state transitions are explicit (Recording → Finalizing → Complete; Finalizing → Cancelling → Idle).
- `cancel_flag` set + run_finalize → returns `Cancelled` variant without panic. (Use a fake transcriber that polls the abort callback in a tight loop.)

### Frontend tests (Vitest)

- `<FinalizingProgress>` renders the percent, the live text, and the cancel button. Cancel triggers the confirmation modal; only the confirming button calls `onCancel`.
- `appBusy` store: setting `true` adds `disabled` to all three nav buttons in `+page.svelte`.
- The dictation/recorder components route between `recording`/`finalizing`/`cancelling` views correctly when receiving the new events.

### Manual integration plan (documented in PR description)

1. Short dictation (10s on the dev machine): full save, complete state.
2. Long dictation (3 min on a slow machine or with `cpufreq-set` capping the CPU): progress visible, eventually saves.
3. Click Stop within ~1s of starting: enters `finalizing` immediately, finishes correctly with a (possibly empty) row marked `complete` — never the old "No text was transcribed" error path.
4. Cancel mid-finalization: confirmation modal appears, confirming aborts whisper, DB row + WAV are gone.
5. Force-quit during `finalizing`: reopen, partial row visible in history with badge, openable.
6. Start dictation while another is finalizing: rejected with clear error.

## Migration of existing user data

- Vivi (and any other beta tester) has a `martin.db` with the old schema and possibly orphan `pending_recordings`. The migration above adds the column without breaking anything. Existing rows default to `'complete'`.
- Existing pending recordings continue to work via the new `transcribe_pending_recording` command (the old command is removed; the frontend's "Transcrever" button is rewired).

## Branch and release

- All work on a separate branch (suggested name: `feat/finalizing-flow`). Confirm name with user before pushing.
- Target: a `v0.2.0` release — bumped from current `0.1.0` because the IPC command surface changes (removed `transcribe_recording`, added two new commands, changed `stop_dictation` signature). Frontend and backend ship together; no need to maintain compatibility shims.
- Release notes will list: dictation save bug fixed, new finalizing UX, partial recovery, navigation lock during processing.

## Out of scope (explicitly deferred)

These are real problems but separate axes; tracking each as a follow-up issue rather than bundling here:

- **Persisting dictation audio to disk** while recording (so a crash mid-dictation doesn't lose the audio). Requires rewriting the dictation capture path.
- **Whisper-rs acceleration features** (`openblas`, `vulkan`, `cuda`). The likely biggest single perf win for Vivi, but orthogonal to UX.
- **User-selectable model size** (tiny/base/small) in the UI. Lets slow machines opt for `tiny` (~75 MB, much faster, lower accuracy).
- **Pending recordings folder relocation** outside `app_data_dir`.
