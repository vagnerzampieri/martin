# Pending Recordings

## Problem

When the user stops a recording, the WAV file sits on disk with no tracking. If the app closes before transcription, the file becomes orphaned. There is no way to see which recordings exist or manage them.

## Solution

Track recordings in a `pending_recordings` SQLite table. Show them on the recorder screen with options to transcribe or delete. Transcription follows the existing flow; deletion removes the WAV and the DB row.

## Database

New table:

```sql
CREATE TABLE IF NOT EXISTS pending_recordings (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    file_path TEXT NOT NULL,
    duration_secs REAL NOT NULL DEFAULT 0.0,
    created_at TEXT NOT NULL DEFAULT (datetime('now', 'localtime'))
);
```

## File naming

Current: always `recording.wav` (overwrites previous).
New: `recording_<unix_millis>.wav` — supports multiple pending files.

Example: `recording_1713300000123.wav`

## Backend changes

### New struct

```rust
pub struct PendingRecording {
    pub id: i64,
    pub file_path: String,
    pub duration_secs: f64,
    pub created_at: String,
}
```

### Store methods

- `save_pending(file_path, duration_secs) -> Result<i64>`
- `list_pending() -> Result<Vec<PendingRecording>>`
- `delete_pending(id) -> Result<()>`

### Modified commands

**`stop_recording`** — after mixing, calculates WAV duration and inserts into `pending_recordings`. Returns `PendingRecording` instead of `()`.

**`transcribe_recording`** — takes `pending_id` instead of generating its own path. Reads `file_path` from `pending_recordings`, transcribes, saves to `transcriptions`, deletes WAV, deletes pending row.

### New commands

- `list_pending_recordings() -> Vec<PendingRecording>`
- `delete_pending_recording(id)` — deletes WAV file + DB row

### Removed

The standalone "Transcribe" button that assumed a single `recording.wav`.

## Frontend changes

### Recorder.svelte

- On mount: loads pending recordings via `list_pending_recordings`
- After stop: refreshes the pending list
- Shows pending items below the record button with:
  - Date + duration
  - "Transcrever" button (triggers transcription of that specific pending)
  - "x" delete button
- Transcribing state is per-item (track which `id` is transcribing)
- On successful transcription: calls `onTranscribed` callback + removes from list

### Cleanup on load

When `list_pending_recordings` returns, verify each file exists. If the WAV was deleted externally, remove the orphan DB row silently.

## Verification

1. `cargo test` — all existing + new store tests pass
2. Record, stop — pending appears in list
3. Close app, reopen — pending still in list
4. Transcribe from list — moves to history, WAV deleted, pending removed
5. Delete from list — WAV deleted, pending removed
6. Record multiple times — all appear in pending list (no overwrite)
