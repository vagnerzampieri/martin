# Transcription Finalizing Flow — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix the dictation save bug and give both transcription flows (live dictation + transcribe pending recording) a clear, observable finalizing phase with progress, navigation lock, save-as-we-go persistence, and cancel-with-confirmation.

**Architecture:** Backend owns transcription text. A unified `TranscriptionJob` runs whisper on a worker thread, persisting text incrementally to a `transcriptions` row that gains a `status` column. Whisper's segment + progress + abort callbacks drive event emission and cancellation. Frontend listens on a unified `transcription://*` event namespace, transitions through an explicit state machine (recording → finalizing → complete | cancelling), and locks navigation during processing via a small Svelte store.

**Tech Stack:** Rust (whisper-rs callbacks, rusqlite), Tauri 2 events, Svelte 5 runes, no new deps.

**Branch:** `feat/finalizing-flow` — already created. All work commits here. The release tag will be `v0.2.0` (IPC commands change; see Task 28).

**Spec:** `docs/specs/2026-05-03-transcription-finalizing-flow-design.md`

---

## File map

| Path | Action | Responsibility |
|---|---|---|
| `src-tauri/src/db/store.rs` | modify | Schema migration, new persistence methods, `Transcription.status` |
| `src-tauri/src/transcribe/job.rs` | create | `TranscriptionJob`, `JobKind`, `run_finalize` worker |
| `src-tauri/src/transcribe/mod.rs` | modify | Export `job` module |
| `src-tauri/src/transcribe/whisper.rs` | modify | New `transcribe_with_callbacks` |
| `src-tauri/src/dictation.rs` | modify | Stop produces committed text + remaining samples; loop no longer tied to events |
| `src-tauri/src/lib.rs` | modify | New commands, `current_job` in `AppState`, remove old `transcribe_recording` |
| `src-tauri/Cargo.toml` | modify | Version bump |
| `src/lib/appBusy.js` | create | Tiny boolean store for nav lock |
| `src/lib/FinalizingProgress.svelte` | create | Shared finalizing UI with circular progress |
| `src/lib/Dictation.svelte` | modify | State machine + new event protocol |
| `src/lib/Recorder.svelte` | modify | Reroute `transcribePending` to new flow |
| `src/lib/History.svelte` | modify | "Parcial" badge for non-complete rows |
| `src/lib/i18n.js` | modify | New strings |
| `src/routes/+page.svelte` | modify | Bind nav buttons to `appBusy` |
| `src-tauri/tauri.conf.json` | modify | Version bump |
| `package.json` | modify | Version bump |

---

### Task 1: Add `status` column to `transcriptions` (idempotent migration)

**Files:**
- Modify: `src-tauri/src/db/store.rs`

- [ ] **Step 1: Write a test for the migration being idempotent**

Add to the `tests` module in `src-tauri/src/db/store.rs`:

```rust
#[test]
fn new_runs_migration_idempotently() {
    let temp_file = NamedTempFile::new().expect("temp file");
    let path = temp_file.path().to_path_buf();

    // First open creates schema with status column.
    let _ = Store::new(&path).expect("first open");

    // Second open must not error — even though the column already exists.
    let store = Store::new(&path).expect("second open");

    // Sanity: status defaults to 'complete' for new inserts.
    let id = store.save("t", "x", "pt", 1.0).expect("save");
    let row: String = store
        .conn
        .query_row(
            "SELECT status FROM transcriptions WHERE id = ?1",
            rusqlite::params![id],
            |r| r.get(0),
        )
        .expect("query");
    assert_eq!(row, "complete");
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd src-tauri && cargo test --lib db::store::tests::new_runs_migration_idempotently`
Expected: FAIL — no `status` column exists in the schema yet.

- [ ] **Step 3: Add the migration logic to `Store::new`**

Find the `Store::new` function in `src-tauri/src/db/store.rs` and add the migration after the existing `CREATE TABLE` calls, before `Ok(Self { conn })`:

```rust
// Migration: add `status` column if missing. Idempotent — older databases
// (created before this column existed) gain it on next launch.
let migration_result = conn.execute(
    "ALTER TABLE transcriptions ADD COLUMN status TEXT NOT NULL DEFAULT 'complete'",
    [],
);
match migration_result {
    Ok(_) => {}
    Err(rusqlite::Error::SqliteFailure(_, Some(msg)))
        if msg.contains("duplicate column name") => {}
    Err(e) => return Err(format!("Failed to add status column: {}", e)),
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cd src-tauri && cargo test --lib db::store::tests::new_runs_migration_idempotently`
Expected: PASS.

- [ ] **Step 5: Run the full test suite — nothing else should break**

Run: `cd src-tauri && cargo test --lib`
Expected: all existing tests still pass.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/db/store.rs
git commit -m "feat(db): add status column to transcriptions with idempotent migration"
```

---

### Task 2: Surface `status` on the `Transcription` struct

**Files:**
- Modify: `src-tauri/src/db/store.rs`

- [ ] **Step 1: Write a test that `get` and `list` return the status field**

Append to the `tests` module:

```rust
#[test]
fn get_returns_status_field() {
    let (store, _temp_file) = create_temp_store();
    let id = store.save("t", "txt", "pt", 1.0).expect("save");
    let t = store.get(id).expect("get");
    assert_eq!(t.status, "complete");
}

#[test]
fn list_returns_status_field() {
    let (store, _temp_file) = create_temp_store();
    store.save("t1", "x", "pt", 1.0).expect("save");
    let rows = store.list().expect("list");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].status, "complete");
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd src-tauri && cargo test --lib db::store::tests::get_returns_status_field db::store::tests::list_returns_status_field`
Expected: FAIL — `status` field does not exist on `Transcription` yet (compile error).

- [ ] **Step 3: Add the `status` field to `Transcription`**

In `src-tauri/src/db/store.rs`, modify the struct (top of file):

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
}
```

- [ ] **Step 4: Update `get` to read the new column**

Replace the `get` method body's SELECT and row mapping:

```rust
pub fn get(&self, id: i64) -> Result<Transcription, String> {
    self.conn
        .query_row(
            "SELECT id, title, text, language, duration_secs, created_at, summary, status FROM transcriptions WHERE id = ?1",
            params![id],
            |row| {
                Ok(Transcription {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    text: row.get(2)?,
                    language: row.get(3)?,
                    duration_secs: row.get(4)?,
                    created_at: row.get(5)?,
                    summary: row.get(6)?,
                    status: row.get(7)?,
                })
            },
        )
        .map_err(|e| format!("Transcription not found: {}", e))
}
```

- [ ] **Step 5: Update `list` to read the new column**

Replace the `list` method:

```rust
pub fn list(&self) -> Result<Vec<Transcription>, String> {
    let mut stmt = self
        .conn
        .prepare("SELECT id, title, text, language, duration_secs, created_at, summary, status FROM transcriptions ORDER BY created_at DESC")
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
                summary: row.get(6)?,
                status: row.get(7)?,
            })
        })
        .map_err(|e| format!("Failed to query: {}", e))?;

    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("Failed to read row: {}", e))
}
```

- [ ] **Step 6: Run all store tests**

Run: `cd src-tauri && cargo test --lib db::store`
Expected: all pass.

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/db/store.rs
git commit -m "feat(db): expose status field on Transcription"
```

---

### Task 3: Add `Store::insert_partial` for save-early

**Files:**
- Modify: `src-tauri/src/db/store.rs`

- [ ] **Step 1: Write the test**

Append to tests:

```rust
#[test]
fn insert_partial_creates_row_with_status_partial() {
    let (store, _temp_file) = create_temp_store();
    let id = store
        .insert_partial("Dictation", "pt")
        .expect("insert_partial");
    let t = store.get(id).expect("get");
    assert_eq!(t.title, "Dictation");
    assert_eq!(t.text, "");
    assert_eq!(t.language, "pt");
    assert_eq!(t.duration_secs, 0.0);
    assert_eq!(t.status, "partial");
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cd src-tauri && cargo test --lib db::store::tests::insert_partial_creates_row_with_status_partial`
Expected: FAIL — `insert_partial` does not exist.

- [ ] **Step 3: Implement `insert_partial`**

Add as a method on `impl Store`, near `save`:

```rust
pub fn insert_partial(&self, title: &str, language: &str) -> Result<i64, String> {
    self.conn
        .execute(
            "INSERT INTO transcriptions (title, text, language, duration_secs, status) VALUES (?1, '', ?2, 0.0, 'partial')",
            params![title, language],
        )
        .map_err(|e| format!("Failed to insert partial transcription: {}", e))?;
    Ok(self.conn.last_insert_rowid())
}
```

- [ ] **Step 4: Run the test**

Run: `cd src-tauri && cargo test --lib db::store::tests::insert_partial_creates_row_with_status_partial`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/db/store.rs
git commit -m "feat(db): add insert_partial for save-early flow"
```

---

### Task 4: Add `Store::update_text` and `Store::mark_complete`

**Files:**
- Modify: `src-tauri/src/db/store.rs`

- [ ] **Step 1: Write the tests**

Append to tests:

```rust
#[test]
fn update_text_overwrites_text_and_duration() {
    let (store, _temp_file) = create_temp_store();
    let id = store.insert_partial("t", "pt").expect("insert");

    store.update_text(id, "first chunk", 5.0).expect("update");
    let row = store.get(id).expect("get");
    assert_eq!(row.text, "first chunk");
    assert_eq!(row.duration_secs, 5.0);
    assert_eq!(row.status, "partial");

    store.update_text(id, "first chunk and more", 12.0).expect("update");
    let row = store.get(id).expect("get");
    assert_eq!(row.text, "first chunk and more");
    assert_eq!(row.duration_secs, 12.0);
}

#[test]
fn update_text_returns_err_for_missing_id() {
    let (store, _temp_file) = create_temp_store();
    let err = store.update_text(999, "x", 1.0).unwrap_err();
    assert!(err.contains("not found"));
}

#[test]
fn mark_complete_flips_status() {
    let (store, _temp_file) = create_temp_store();
    let id = store.insert_partial("t", "pt").expect("insert");
    store.mark_complete(id).expect("mark");
    let row = store.get(id).expect("get");
    assert_eq!(row.status, "complete");
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cd src-tauri && cargo test --lib db::store::tests::update_text db::store::tests::mark_complete`
Expected: FAIL — methods do not exist.

- [ ] **Step 3: Implement the methods**

Add to `impl Store`:

```rust
pub fn update_text(
    &self,
    id: i64,
    text: &str,
    duration_secs: f64,
) -> Result<(), String> {
    let affected = self
        .conn
        .execute(
            "UPDATE transcriptions SET text = ?1, duration_secs = ?2 WHERE id = ?3",
            params![text, duration_secs, id],
        )
        .map_err(|e| format!("Failed to update text: {}", e))?;
    if affected == 0 {
        return Err(format!("Transcription with id {} not found", id));
    }
    Ok(())
}

pub fn mark_complete(&self, id: i64) -> Result<(), String> {
    let affected = self
        .conn
        .execute(
            "UPDATE transcriptions SET status = 'complete' WHERE id = ?1",
            params![id],
        )
        .map_err(|e| format!("Failed to mark complete: {}", e))?;
    if affected == 0 {
        return Err(format!("Transcription with id {} not found", id));
    }
    Ok(())
}
```

- [ ] **Step 4: Run the new tests**

Run: `cd src-tauri && cargo test --lib db::store`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/db/store.rs
git commit -m "feat(db): add update_text and mark_complete for incremental persistence"
```

---

### Task 5: Add `Store::reset_partial_on_startup`

**Files:**
- Modify: `src-tauri/src/db/store.rs`
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: Write the test**

Append to tests:

```rust
#[test]
fn reset_partial_on_startup_only_touches_non_complete() {
    let (store, _temp_file) = create_temp_store();

    // Insert one complete and one partial, plus one with a bogus status.
    let complete_id = store.save("c", "x", "pt", 1.0).expect("save");
    let partial_id = store.insert_partial("p", "pt").expect("partial");
    store
        .conn
        .execute(
            "INSERT INTO transcriptions (title, text, language, duration_secs, status) VALUES ('weird', '', 'pt', 0.0, 'in_progress')",
            [],
        )
        .expect("insert weird");

    let touched = store.reset_partial_on_startup().expect("reset");
    assert_eq!(touched, 1, "only the in_progress row should be normalised");

    assert_eq!(store.get(complete_id).unwrap().status, "complete");
    assert_eq!(store.get(partial_id).unwrap().status, "partial");
    let weird: String = store
        .conn
        .query_row(
            "SELECT status FROM transcriptions WHERE title = 'weird'",
            [],
            |r| r.get(0),
        )
        .expect("query");
    assert_eq!(weird, "partial");
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cd src-tauri && cargo test --lib db::store::tests::reset_partial_on_startup_only_touches_non_complete`
Expected: FAIL.

- [ ] **Step 3: Implement the method**

Add to `impl Store`:

```rust
/// Normalises any non-`complete`, non-`failed` rows to `partial`.
/// Called on startup to recover from crashes mid-job.
/// Returns the number of rows touched.
pub fn reset_partial_on_startup(&self) -> Result<usize, String> {
    let affected = self
        .conn
        .execute(
            "UPDATE transcriptions SET status = 'partial' WHERE status NOT IN ('complete', 'failed', 'partial')",
            [],
        )
        .map_err(|e| format!("Failed to reset partial: {}", e))?;
    Ok(affected)
}
```

- [ ] **Step 4: Wire it into app startup**

In `src-tauri/src/lib.rs`, find the `setup` block (around line 350 — `let data_dir = app...app_data_dir()...`). After `let store = Store::new(&db_path).expect(...)`, add:

```rust
let recovered = store
    .reset_partial_on_startup()
    .expect("Failed to reset partial transcriptions on startup");
if recovered > 0 {
    eprintln!("[startup] reset {} stale transcription(s) to status='partial'", recovered);
}
```

- [ ] **Step 5: Run all tests + a manual smoke**

Run: `cd src-tauri && cargo test --lib db::store`
Expected: all pass.

Run: `cd src-tauri && cargo check`
Expected: compiles cleanly.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/db/store.rs src-tauri/src/lib.rs
git commit -m "feat(db): reset stale partial transcriptions on startup"
```

---

### Task 6: Add `Transcriber::transcribe_with_callbacks`

**Files:**
- Modify: `src-tauri/src/transcribe/whisper.rs`

This wraps `state.full` with progress, segment, and abort callbacks routed back to caller-supplied closures. Used by both job kinds.

- [ ] **Step 1: Read the whisper-rs FullParams API in the existing source**

Read `src-tauri/src/transcribe/whisper.rs:30-45` (existing `transcribe` function). Confirm `FullParams` has `set_progress_callback_safe`, `set_segment_callback_safe_lossy`, and `set_abort_callback_safe`. (whisper-rs 0.14.) If your version exposes only `_unsafe` variants, prefer the `_safe` ones. The patterns here use the `_safe` variants.

- [ ] **Step 2: Write the test**

Append to the `tests` mod in `src-tauri/src/transcribe/whisper.rs`:

```rust
#[test]
fn transcribe_with_callbacks_signature_compiles() {
    // Compile-time-only test: ensures the public signature is shaped correctly.
    fn _sig_check(t: &Transcriber, samples: &[f32]) {
        let _ = t.transcribe_with_callbacks(
            samples,
            "pt",
            |_p: i32| {},
            |_seg: &str| {},
            || false,
        );
    }
    let _ = _sig_check;
}
```

This guards the API shape; we cover behavior in integration paths because actually running whisper requires the model file.

- [ ] **Step 3: Run to verify the test fails to compile**

Run: `cd src-tauri && cargo test --lib transcribe::whisper::tests::transcribe_with_callbacks_signature_compiles`
Expected: FAIL — method does not exist.

- [ ] **Step 4: Implement the method**

Add to `impl Transcriber` in `src-tauri/src/transcribe/whisper.rs`, after `transcribe_samples`:

```rust
/// Transcribe samples while emitting progress, per-segment text, and
/// observing an abort flag. Caller closures must be `Send + 'static`.
///
/// - `on_progress` is called with whisper's internal progress (0-100).
/// - `on_segment` is called with each new segment text as it is produced.
/// - `should_abort` is polled by whisper periodically; returning true
///   aborts inference cleanly.
pub fn transcribe_with_callbacks<P, S, A>(
    &self,
    samples: &[f32],
    language: &str,
    on_progress: P,
    on_segment: S,
    should_abort: A,
) -> Result<String, String>
where
    P: FnMut(i32) + Send + 'static,
    S: FnMut(&str) + Send + 'static,
    A: FnMut() -> bool + Send + 'static,
{
    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    params.set_language(Some(language));
    params.set_print_special(false);
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);
    params.set_no_context(true);

    params.set_progress_callback_safe(on_progress);
    params.set_segment_callback_safe_lossy(on_segment);
    params.set_abort_callback_safe(should_abort);

    let mut state = self
        .ctx
        .create_state()
        .map_err(|e| format!("Failed to create state: {}", e))?;

    state
        .full(params, samples)
        .map_err(|e| format!("Transcription failed: {}", e))?;

    let num_segments = state
        .full_n_segments()
        .map_err(|e| format!("Failed to get segments: {}", e))?;

    let segments: Vec<String> = (0..num_segments)
        .map(|i| state.full_get_segment_text(i))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("Failed to extract segment text: {}", e))?;

    Ok(segments.join(" ").trim().to_string())
}
```

- [ ] **Step 5: Run the test**

Run: `cd src-tauri && cargo test --lib transcribe::whisper::tests::transcribe_with_callbacks_signature_compiles`
Expected: PASS (compile-time only).

Run: `cd src-tauri && cargo check`
Expected: compiles.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/transcribe/whisper.rs
git commit -m "feat(whisper): add transcribe_with_callbacks for progress/segment/abort"
```

---

### Task 7: Create `transcribe::job` module — types and state

**Files:**
- Create: `src-tauri/src/transcribe/job.rs`
- Modify: `src-tauri/src/transcribe/mod.rs`

- [ ] **Step 1: Read `mod.rs` to find the export pattern**

Read `src-tauri/src/transcribe/mod.rs`. It will be one or two lines exporting `whisper`. Add `job` next to it.

- [ ] **Step 2: Add the module declaration**

Replace contents of `src-tauri/src/transcribe/mod.rs` with:

```rust
pub mod job;
pub mod whisper;
```

- [ ] **Step 3: Create `src-tauri/src/transcribe/job.rs` with the type skeleton**

```rust
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub enum JobKind {
    Dictation,
    PendingFile { wav_path: PathBuf, pending_id: i64 },
}

pub struct TranscriptionJob {
    pub id: i64,
    pub kind: JobKind,
    pub cancel_flag: Arc<AtomicBool>,
    pub committed_text: String,
}

impl TranscriptionJob {
    pub fn new(id: i64, kind: JobKind) -> Self {
        Self {
            id,
            kind,
            cancel_flag: Arc::new(AtomicBool::new(false)),
            committed_text: String::new(),
        }
    }

    pub fn cancel(&self) {
        self.cancel_flag.store(true, std::sync::atomic::Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel_flag.load(std::sync::atomic::Ordering::Acquire)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_initializes_with_empty_text_and_unset_flag() {
        let job = TranscriptionJob::new(7, JobKind::Dictation);
        assert_eq!(job.id, 7);
        assert_eq!(job.committed_text, "");
        assert!(!job.is_cancelled());
    }

    #[test]
    fn cancel_sets_the_flag() {
        let job = TranscriptionJob::new(1, JobKind::Dictation);
        assert!(!job.is_cancelled());
        job.cancel();
        assert!(job.is_cancelled());
    }

    #[test]
    fn pending_file_carries_path_and_id() {
        let kind = JobKind::PendingFile {
            wav_path: PathBuf::from("/tmp/foo.wav"),
            pending_id: 42,
        };
        let job = TranscriptionJob::new(10, kind);
        match &job.kind {
            JobKind::PendingFile { wav_path, pending_id } => {
                assert_eq!(wav_path, &PathBuf::from("/tmp/foo.wav"));
                assert_eq!(*pending_id, 42);
            }
            _ => panic!("wrong kind"),
        }
    }
}
```

- [ ] **Step 4: Run the tests**

Run: `cd src-tauri && cargo test --lib transcribe::job`
Expected: 3 tests pass.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/transcribe/mod.rs src-tauri/src/transcribe/job.rs
git commit -m "feat(transcribe): add TranscriptionJob and JobKind"
```

---

### Task 8: Implement `run_finalize` worker (dictation kind)

**Files:**
- Modify: `src-tauri/src/transcribe/job.rs`
- Modify: `src-tauri/src/db/store.rs` (expose `delete` and `delete_pending` are already pub)

The worker drives whisper, persists incrementally, and emits events. Splitting by kind keeps the function readable.

- [ ] **Step 1: Add helper struct and the dictation worker**

Append to `src-tauri/src/transcribe/job.rs`:

```rust
use std::sync::Mutex;

use serde::Serialize;
use serde_json::json;
use tauri::{AppHandle, Emitter};

use crate::db::store::Store;
use crate::transcribe::whisper::Transcriber;

/// Final outcome of a finalize run. Used by the orchestrator to decide
/// whether to emit `complete`, `cancelled`, or `error`.
#[derive(Debug)]
pub enum FinalizeOutcome {
    Complete { final_text: String, duration_secs: f64 },
    Cancelled,
    Error(String),
}

#[derive(Clone, Serialize)]
struct ProgressPayload {
    id: i64,
    percent: u8,
}

#[derive(Clone, Serialize)]
struct TextPayload {
    id: i64,
    text: String,
}

#[derive(Clone, Serialize)]
struct ErrorPayload {
    id: i64,
    error: String,
}

#[derive(Clone, Serialize)]
struct CancelledPayload {
    id: i64,
}

/// Runs whisper on `samples`, updating the row in `store` and emitting
/// `transcription://*` events. Returns the outcome so the orchestrator
/// can choose the terminal action (mark_complete, delete, leave partial).
pub fn run_finalize_dictation(
    job: &TranscriptionJob,
    transcriber: &Transcriber,
    samples: Vec<f32>,
    duration_secs: f64,
    language: String,
    store: Arc<Mutex<Store>>,
    app_handle: AppHandle,
) -> FinalizeOutcome {
    let id = job.id;
    let cancel_flag = job.cancel_flag.clone();
    let committed_prefix = job.committed_text.clone();

    // Accumulator shared with the segment callback.
    let accumulated = Arc::new(Mutex::new(committed_prefix.clone()));
    let acc_for_callback = accumulated.clone();
    let store_for_callback = store.clone();
    let app_for_callback = app_handle.clone();

    let on_progress = {
        let app = app_handle.clone();
        move |percent: i32| {
            let p = percent.clamp(0, 100) as u8;
            let _ = app.emit("transcription://progress", ProgressPayload { id, percent: p });
        }
    };

    let on_segment = move |seg: &str| {
        let trimmed = seg.trim();
        if trimmed.is_empty() {
            return;
        }
        let new_text = {
            let mut acc = acc_for_callback.lock().expect("acc lock");
            if acc.is_empty() {
                acc.push_str(trimmed);
            } else {
                acc.push(' ');
                acc.push_str(trimmed);
            }
            acc.clone()
        };

        if let Ok(s) = store_for_callback.lock() {
            let _ = s.update_text(id, &new_text, duration_secs);
        }
        let _ = app_for_callback.emit(
            "transcription://text",
            TextPayload { id, text: new_text },
        );
    };

    let abort_flag = cancel_flag.clone();
    let on_abort = move || abort_flag.load(std::sync::atomic::Ordering::Acquire);

    let result = transcriber.transcribe_with_callbacks(
        &samples,
        &language,
        on_progress,
        on_segment,
        on_abort,
    );

    if cancel_flag.load(std::sync::atomic::Ordering::Acquire) {
        return FinalizeOutcome::Cancelled;
    }

    match result {
        Ok(_) => {
            let final_text = accumulated.lock().expect("acc lock").clone();
            // Final flush in case segment callback missed anything.
            if let Ok(s) = store.lock() {
                let _ = s.update_text(id, &final_text, duration_secs);
            }
            FinalizeOutcome::Complete { final_text, duration_secs }
        }
        Err(e) => FinalizeOutcome::Error(e),
    }
}
```

- [ ] **Step 2: Verify it compiles**

Run: `cd src-tauri && cargo check`
Expected: compiles.

If you get an `unused_variable` warning for `json` (unused import), remove it. The `json!` macro is needed only if we used it; in this code we used typed payloads — drop the `serde_json::json` import if unused.

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/transcribe/job.rs
git commit -m "feat(job): run_finalize_dictation worker with persistence + events"
```

---

### Task 9: Add `run_finalize_pending` worker (file kind)

**Files:**
- Modify: `src-tauri/src/transcribe/job.rs`

The pending-file worker reuses the same callback logic but loads samples from the WAV first.

- [ ] **Step 1: Add the function**

Append to `src-tauri/src/transcribe/job.rs`:

```rust
use std::path::Path;

/// Loads the WAV at `wav_path` then runs the same finalize loop as the
/// dictation worker. On success, the caller is responsible for deleting
/// the WAV and the matching `pending_recordings` row.
pub fn run_finalize_pending_file(
    job: &TranscriptionJob,
    transcriber: &Transcriber,
    wav_path: &Path,
    language: String,
    store: Arc<Mutex<Store>>,
    app_handle: AppHandle,
) -> FinalizeOutcome {
    let samples = match crate::transcribe::whisper::Transcriber::load_wav_as_mono_f32_pub(wav_path) {
        Ok(s) => s,
        Err(e) => return FinalizeOutcome::Error(format!("Failed to read WAV: {}", e)),
    };

    let duration_secs = match crate::transcribe::whisper::wav_duration_secs(wav_path) {
        Ok(d) => d,
        Err(e) => return FinalizeOutcome::Error(format!("Failed to read WAV duration: {}", e)),
    };

    run_finalize_dictation(
        job,
        transcriber,
        samples,
        duration_secs,
        language,
        store,
        app_handle,
    )
}
```

- [ ] **Step 2: Expose the WAV loader and duration helper publicly**

In `src-tauri/src/transcribe/whisper.rs`:

(a) Rename or alias the private `load_wav_as_mono_f32` for public use. Add:

```rust
pub fn load_wav_as_mono_f32_pub(path: &std::path::Path) -> Result<Vec<f32>, String> {
    Self::load_wav_as_mono_f32(path)
}
```

inside `impl Transcriber`.

(b) The existing `wav_duration_secs` helper currently lives in `src-tauri/src/lib.rs`. Move it into `src-tauri/src/transcribe/whisper.rs` as a free function `pub fn wav_duration_secs(...)` and reference it from `lib.rs` as `transcribe::whisper::wav_duration_secs`. After moving, delete the original from `lib.rs`.

- [ ] **Step 3: Compile**

Run: `cd src-tauri && cargo check`
Expected: compiles.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/transcribe/job.rs src-tauri/src/transcribe/whisper.rs src-tauri/src/lib.rs
git commit -m "feat(job): pending-file worker reuses dictation finalize path"
```

---

### Task 10: Add `complete_job` and `cancel_job_cleanup` orchestration helpers

**Files:**
- Modify: `src-tauri/src/transcribe/job.rs`

After a worker returns a `FinalizeOutcome`, the calling Tauri command must perform terminal cleanup (mark complete, delete pending file, emit complete/cancelled/error). Centralise this so dictation and pending share the same logic.

- [ ] **Step 1: Append helpers to `job.rs`**

```rust
#[derive(Clone, Serialize)]
struct CompletePayload {
    id: i64,
    transcription: crate::db::store::Transcription,
}

/// Apply the terminal effect of a finalize run to the DB and the filesystem,
/// then emit the matching event.
pub fn finish_job(
    job: TranscriptionJob,
    outcome: FinalizeOutcome,
    store: Arc<Mutex<Store>>,
    app_handle: AppHandle,
) {
    let id = job.id;
    match outcome {
        FinalizeOutcome::Complete { final_text, duration_secs } => {
            if let Ok(s) = store.lock() {
                let _ = s.update_text(id, &final_text, duration_secs);
                let _ = s.mark_complete(id);

                if let JobKind::PendingFile { wav_path, pending_id } = &job.kind {
                    if let Err(e) = std::fs::remove_file(wav_path) {
                        eprintln!("[finish_job] failed to remove WAV {:?}: {}", wav_path, e);
                    }
                    let _ = s.delete_pending(*pending_id);
                }

                if let Ok(t) = s.get(id) {
                    let _ = app_handle.emit(
                        "transcription://complete",
                        CompletePayload { id, transcription: t },
                    );
                }
            }
        }
        FinalizeOutcome::Cancelled => {
            if let Ok(s) = store.lock() {
                let _ = s.delete(id);
                if let JobKind::PendingFile { wav_path, .. } = &job.kind {
                    let _ = std::fs::remove_file(wav_path);
                    // Note: we do NOT delete the pending_recordings row here —
                    // the user cancelled the transcription, but the recording
                    // itself remains a valid pending recording until they
                    // explicitly delete it.
                }
            }
            let _ = app_handle.emit("transcription://cancelled", CancelledPayload { id });
        }
        FinalizeOutcome::Error(err) => {
            // Row stays partial with whatever the segment callback persisted.
            let _ = app_handle.emit(
                "transcription://error",
                ErrorPayload { id, error: err },
            );
        }
    }
}
```

Note on the cancelled branch: for dictation there is no `pending_recordings` row at all (audio is in memory), so deleting one is a no-op. For pending-file, the WAV existed before the user clicked Transcrever; cancelling the transcription should leave the recording in place so they can retry, which is why we do not delete the pending row here. This matches the spec.

- [ ] **Step 2: Compile**

Run: `cd src-tauri && cargo check`
Expected: compiles. Warnings about unused imports — clean any up.

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/transcribe/job.rs
git commit -m "feat(job): finish_job handler for terminal effects + events"
```

---

### Task 11: Wire `current_job` into `AppState`

**Files:**
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: Add the field to `AppState`**

Edit `src-tauri/src/lib.rs`:

```rust
use transcribe::job::TranscriptionJob;

pub struct AppState {
    capture: Mutex<SendableCapture>,
    dictation: Mutex<Option<DictationSession>>,
    store: std::sync::Arc<Mutex<Store>>,
    transcriber: Mutex<Option<Transcriber>>,
    data_dir: PathBuf,
    current_job: Mutex<Option<TranscriptionJob>>,
}
```

Note that `store` becomes `Arc<Mutex<Store>>` so worker threads can hold a reference.

- [ ] **Step 2: Update the setup block where `AppState` is constructed**

In the existing `setup` closure (around line 350), wrap the store in an Arc:

```rust
let store = Store::new(&db_path).expect("Failed to create store");
let recovered = store
    .reset_partial_on_startup()
    .expect("Failed to reset partial transcriptions on startup");
if recovered > 0 {
    eprintln!("[startup] reset {} stale transcription(s) to status='partial'", recovered);
}

app.manage(AppState {
    capture: Mutex::new(SendableCapture(None)),
    dictation: Mutex::new(None),
    store: std::sync::Arc::new(Mutex::new(store)),
    transcriber: Mutex::new(None),
    data_dir,
    current_job: Mutex::new(None),
});
```

- [ ] **Step 3: Update every existing `state.store.lock()` call**

The existing code does `state.store.lock()`. With `store` now an `Arc<Mutex<Store>>`, the call still works — `Arc<Mutex<T>>` derefs to `Mutex<T>`. Verify by running:

Run: `cd src-tauri && cargo check`
Expected: compiles. If any call breaks, replace with `state.store.lock()` (no `.deref()` needed; `Arc` auto-derefs).

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/lib.rs
git commit -m "feat(state): add current_job to AppState; share Store via Arc"
```

---

### Task 12: Replace `stop_dictation` with the new finalize-launching version

**Files:**
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/src/dictation.rs`

The new `stop_dictation`:
1. Stops audio capture and drains the buffer + committed segments.
2. Inserts a partial transcription row.
3. Stores a `TranscriptionJob` in `current_job`.
4. Spawns a worker thread that runs `run_finalize_dictation` then `finish_job`.
5. Returns the new transcription `id`.

The `run_transcription_loop` in `dictation.rs` keeps emitting live text during recording (existing behavior) but is no longer responsible for the final emit — the new worker takes over after stop.

- [ ] **Step 1: Expose committed segments + buffer drain on `DictationSession`**

Edit `src-tauri/src/dictation.rs`. Add to `DictationSession`:

```rust
pub fn drain_buffer(&self) -> Vec<f32> {
    let mut buf = self.audio_buffer.lock().expect("buffer lock");
    let drained = buf.drain(..).collect();
    drained
}
```

(Existing `stop` already sets `running=false` and drops the stream; the worker still running will exit on next loop check.)

The `run_transcription_loop` returns `Vec<String>` of committed segments. Today this return value is dropped on the floor (the spawned task discards it). We need to capture it.

Refactor the spawn in `start_dictation` (around line 250 in `lib.rs`) to send the committed segments into a shared `Mutex<Option<Vec<String>>>` that lives on the `DictationSession`.

In `src-tauri/src/dictation.rs`:

```rust
pub struct DictationSession {
    stream: Option<Stream>,
    audio_buffer: Arc<Mutex<Vec<f32>>>,
    running: Arc<AtomicBool>,
    source_rate: u32,
    channels: u16,
    committed: Arc<Mutex<Vec<String>>>,
}
```

Initialize `committed: Arc::new(Mutex::new(Vec::new()))` in `new()`.

Add:

```rust
pub fn committed(&self) -> Arc<Mutex<Vec<String>>> {
    self.committed.clone()
}
```

Modify `run_transcription_loop` to take an extra parameter `committed_out: Arc<Mutex<Vec<String>>>` and replace its current `Vec::new()` declaration with a `let mut committed_segments = ...`. At the points where the loop pushes to `committed_segments`, also clone into `committed_out`. Specifically, find the two `committed_segments.push(text)` calls (in the rollover case and in the final-block case) and add right after each:

```rust
if let Ok(mut sink) = committed_out.lock() {
    sink.push(text.clone());
}
```

The function still returns `committed_segments` for backward compat in case anyone reads it; but the new path reads `committed_out`.

- [ ] **Step 2: Update `start_dictation` in `lib.rs` to pass `committed_out`**

Find the spawn in `start_dictation`. Replace:

```rust
let segments = dictation::run_transcription_loop(
    buffer,
    running,
    &transcriber,
    &language,
    source_rate,
    channels,
    handle,
);
```

with:

```rust
let segments = dictation::run_transcription_loop(
    buffer,
    running,
    committed_out,
    &transcriber,
    &language,
    source_rate,
    channels,
    handle,
);
```

and add (just before the spawn) a `let committed_out = session_ref.committed();` — where `session_ref` is the `DictationSession` you can clone the Arc from before the session is moved into the AppState mutex. This requires fetching the committed handle from the just-built `DictationSession` *before* `*state.dictation.lock() = Some(session);`. Reorder the existing code:

```rust
let mut session = DictationSession::new();
session.start()?;

let buffer = session.buffer();
let running = session.running_flag();
let source_rate = session.source_rate();
let channels = session.channels();
let committed_out = session.committed();   // <-- NEW

*state.dictation.lock().map_err(|e| e.to_string())? = Some(session);

// (existing lines that build the spawn, now passing committed_out)
```

- [ ] **Step 3: Replace the body of `stop_dictation`**

Delete the existing `stop_dictation` async function entirely and replace with:

```rust
#[tauri::command]
async fn stop_dictation(
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
    title: String,
    language: String,
    duration_secs: f64,
) -> Result<i64, String> {
    use crate::transcribe::job::{
        finish_job, run_finalize_dictation, JobKind, TranscriptionJob,
    };

    // Reject if another job is already finalizing.
    {
        let guard = state.current_job.lock().map_err(|e| e.to_string())?;
        if guard.is_some() {
            return Err("Another transcription is in progress".to_string());
        }
    }

    // Stop capture and pull audio + committed text.
    let (samples, committed) = {
        let mut guard = state.dictation.lock().map_err(|e| e.to_string())?;
        let session = guard.as_mut().ok_or("No dictation in progress")?;
        session.stop();
        let samples = session.drain_buffer();
        let committed = session
            .committed()
            .lock()
            .map_err(|e| e.to_string())?
            .join(" ")
            .trim()
            .to_string();
        // Take the session out so the next start_dictation works fresh.
        *guard = None;
        (samples, committed)
    };

    // Insert partial row up front.
    let id = {
        let store = state.store.lock().map_err(|e| e.to_string())?;
        let new_id = store.insert_partial(&title, &language)?;
        if !committed.is_empty() {
            store.update_text(new_id, &committed, duration_secs)?;
        }
        new_id
    };

    // Build the job and store it.
    let mut job = TranscriptionJob::new(id, JobKind::Dictation);
    job.committed_text = committed;
    let cancel_flag = job.cancel_flag.clone();
    let job_id = job.id;

    *state.current_job.lock().map_err(|e| e.to_string())? = Some(TranscriptionJob {
        id: job.id,
        kind: job.kind.clone(),
        cancel_flag: cancel_flag.clone(),
        committed_text: job.committed_text.clone(),
    });

    // Spawn the worker.
    let store = state.store.clone();
    let app = app_handle.clone();
    let transcriber_arc = state.transcriber.lock().map_err(|e| e.to_string())?.take();
    let model_path = crate::model::model_path(&state.data_dir);

    tauri::async_runtime::spawn_blocking(move || {
        let transcriber = match transcriber_arc {
            Some(t) => t,
            None => match Transcriber::new(&model_path) {
                Ok(t) => t,
                Err(e) => {
                    let _ = app.emit(
                        "transcription://error",
                        serde_json::json!({ "id": job_id, "error": e }),
                    );
                    return;
                }
            },
        };

        let outcome = run_finalize_dictation(
            &job,
            &transcriber,
            samples,
            duration_secs,
            language,
            store.clone(),
            app.clone(),
        );

        finish_job(job, outcome, store, app.clone());

        // Restore transcriber to cache and clear current_job.
        let app_state = app.state::<AppState>();
        let _ = app_state
            .transcriber
            .lock()
            .map(|mut g| *g = Some(transcriber));
        let _ = app_state.current_job.lock().map(|mut g| *g = None);
    });

    Ok(id)
}
```

The TranscriptionJob is built twice because `Clone` isn't free — simplest to derive `Clone` on it. Add to `src-tauri/src/transcribe/job.rs`:

```rust
#[derive(Clone)]
pub struct TranscriptionJob {
    // … existing fields
}
```

(`Arc<AtomicBool>` is `Clone`; `String` is `Clone`; `JobKind` already derives `Clone`; `i64` is `Copy`. So the derive is sound.)

Then in the command, replace the manual rebuild with `state.current_job.lock()...? = Some(job.clone());`.

- [ ] **Step 4: Compile**

Run: `cd src-tauri && cargo check`
Expected: compiles.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/dictation.rs src-tauri/src/transcribe/job.rs src-tauri/src/lib.rs
git commit -m "feat(dictation): stop_dictation now launches a finalize job"
```

---

### Task 13: Replace `transcribe_recording` with `transcribe_pending_recording`

**Files:**
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: Delete `transcribe_recording`**

Remove the entire `transcribe_recording` async function from `src-tauri/src/lib.rs` (it lives around line 109-149). Also remove its registration in `tauri::generate_handler!` (the macro at the bottom of the setup).

- [ ] **Step 2: Add the replacement**

Add this command to `src-tauri/src/lib.rs`:

```rust
#[tauri::command]
async fn transcribe_pending_recording(
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
    pending_id: i64,
    title: String,
    language: String,
) -> Result<i64, String> {
    use crate::transcribe::job::{
        finish_job, run_finalize_pending_file, JobKind, TranscriptionJob,
    };

    {
        let guard = state.current_job.lock().map_err(|e| e.to_string())?;
        if guard.is_some() {
            return Err("Another transcription is in progress".to_string());
        }
    }

    let pending = {
        let store = state.store.lock().map_err(|e| e.to_string())?;
        store.get_pending(pending_id)?
    };

    let wav_path = std::path::PathBuf::from(&pending.file_path);
    if !wav_path.exists() {
        let store = state.store.lock().map_err(|e| e.to_string())?;
        let _ = store.delete_pending(pending_id);
        return Err("Recording file not found. It may have been deleted.".to_string());
    }

    let id = {
        let store = state.store.lock().map_err(|e| e.to_string())?;
        store.insert_partial(&title, &language)?
    };

    let job = TranscriptionJob::new(
        id,
        JobKind::PendingFile {
            wav_path: wav_path.clone(),
            pending_id,
        },
    );

    *state.current_job.lock().map_err(|e| e.to_string())? = Some(job.clone());

    let store = state.store.clone();
    let app = app_handle.clone();
    let transcriber_arc = state.transcriber.lock().map_err(|e| e.to_string())?.take();
    let model_path = crate::model::model_path(&state.data_dir);
    let language_owned = language.clone();
    let job_id = id;

    tauri::async_runtime::spawn_blocking(move || {
        let transcriber = match transcriber_arc {
            Some(t) => t,
            None => match Transcriber::new(&model_path) {
                Ok(t) => t,
                Err(e) => {
                    let _ = app.emit(
                        "transcription://error",
                        serde_json::json!({ "id": job_id, "error": e }),
                    );
                    return;
                }
            },
        };

        let outcome = run_finalize_pending_file(
            &job,
            &transcriber,
            &wav_path,
            language_owned,
            store.clone(),
            app.clone(),
        );

        finish_job(job, outcome, store, app.clone());

        let app_state = app.state::<AppState>();
        let _ = app_state
            .transcriber
            .lock()
            .map(|mut g| *g = Some(transcriber));
        let _ = app_state.current_job.lock().map(|mut g| *g = None);
    });

    Ok(id)
}
```

- [ ] **Step 3: Register the new command**

In `tauri::generate_handler!`, replace `transcribe_recording` with `transcribe_pending_recording`.

- [ ] **Step 4: Compile**

Run: `cd src-tauri && cargo check`
Expected: compiles cleanly.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/lib.rs
git commit -m "feat(transcribe): replace transcribe_recording with async pending flow"
```

---

### Task 14: Add `cancel_job` and `current_job_status` commands

**Files:**
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: Add `cancel_job`**

```rust
#[tauri::command]
fn cancel_job(state: State<'_, AppState>) -> Result<(), String> {
    let guard = state.current_job.lock().map_err(|e| e.to_string())?;
    if let Some(job) = guard.as_ref() {
        job.cancel();
    }
    Ok(())
}
```

The actual cleanup (delete row, delete WAV) happens in the worker thread when `transcribe_with_callbacks` aborts and `finish_job` runs the `Cancelled` arm. The worker also clears `current_job` itself.

- [ ] **Step 2: Add `current_job_status`**

```rust
use serde::Serialize;

#[derive(Clone, Serialize)]
struct JobStatus {
    id: i64,
    kind: String,
    cancelling: bool,
}

#[tauri::command]
fn current_job_status(state: State<'_, AppState>) -> Result<Option<JobStatus>, String> {
    let guard = state.current_job.lock().map_err(|e| e.to_string())?;
    Ok(guard.as_ref().map(|job| JobStatus {
        id: job.id,
        kind: match &job.kind {
            transcribe::job::JobKind::Dictation => "dictation".to_string(),
            transcribe::job::JobKind::PendingFile { .. } => "pending_file".to_string(),
        },
        cancelling: job.is_cancelled(),
    }))
}
```

- [ ] **Step 3: Register both commands in `tauri::generate_handler!`**

- [ ] **Step 4: Compile**

Run: `cd src-tauri && cargo check`
Expected: compiles.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/lib.rs
git commit -m "feat(commands): cancel_job and current_job_status"
```

---

### Task 15: Run full backend test suite as a checkpoint

- [ ] **Step 1: Run everything**

Run: `cd src-tauri && cargo test --lib`
Expected: all green. If any old test fails (e.g., one that asserted the old `transcribe_recording` shape), update or remove it — but document why in the commit message.

- [ ] **Step 2: Run clippy**

Run: `cd src-tauri && cargo clippy -- -D warnings`
Expected: no warnings. Fix anything clippy flags.

- [ ] **Step 3: Run rustfmt**

Run: `cd src-tauri && cargo fmt`

- [ ] **Step 4: Commit any formatting / lint fixes**

```bash
git add -u
git commit -m "chore: cargo fmt + clippy fixes for finalizing flow"
```

---

### Task 16: Add i18n strings

**Files:**
- Modify: `src/lib/i18n.js`

- [ ] **Step 1: Add the new keys to both `pt` and `en`**

Append inside the `pt` block (before the closing brace):

```js
    finalizing: "Finalizando transcrição…",
    finalizingHint: "Pode levar alguns minutos numa máquina lenta.",
    cancelTranscription: "Cancelar",
    cancelConfirmTitle: "Cancelar transcrição?",
    cancelConfirmBody: "O conteúdo será perdido.",
    cancelConfirmYes: "Sim, cancelar",
    cancelConfirmNo: "Voltar",
    navLockedTooltip: "Aguarde a transcrição terminar",
    partialBadge: "Parcial",
    keepPartial: "Manter o que foi transcrito",
    discardPartial: "Descartar",
```

Append inside the `en` block (before the closing brace):

```js
    finalizing: "Finalizing transcription…",
    finalizingHint: "May take a few minutes on a slow machine.",
    cancelTranscription: "Cancel",
    cancelConfirmTitle: "Cancel transcription?",
    cancelConfirmBody: "The content will be lost.",
    cancelConfirmYes: "Yes, cancel",
    cancelConfirmNo: "Back",
    navLockedTooltip: "Wait for the transcription to finish",
    partialBadge: "Partial",
    keepPartial: "Keep what was transcribed",
    discardPartial: "Discard",
```

- [ ] **Step 2: Run check**

Run: `npm run check`
Expected: clean.

- [ ] **Step 3: Commit**

```bash
git add src/lib/i18n.js
git commit -m "feat(i18n): strings for finalizing flow"
```

---

### Task 17: Add `appBusy` store

**Files:**
- Create: `src/lib/appBusy.js`

- [ ] **Step 1: Create the file**

```js
import { writable } from "svelte/store";

/**
 * Global flag — when true, navigation is locked because a transcription
 * job is finalizing or cancelling. Components running long jobs set
 * this to true on entry and false on exit.
 */
export const appBusy = writable(false);
```

- [ ] **Step 2: Commit**

```bash
git add src/lib/appBusy.js
git commit -m "feat: appBusy store for global navigation lock"
```

---

### Task 18: Wire `appBusy` into the nav buttons

**Files:**
- Modify: `src/routes/+page.svelte`

- [ ] **Step 1: Import the store and bind disabled state**

Edit `src/routes/+page.svelte`. Add the import near the other imports:

```js
import { appBusy } from "../lib/appBusy.js";
```

- [ ] **Step 2: Apply `disabled` and tooltip to the three nav buttons**

Replace the three `<button>` elements inside `<nav>` with:

```svelte
<button
    class:active={currentView === "recorder"}
    onclick={showRecorder}
    disabled={$appBusy}
    title={$appBusy ? t("navLockedTooltip") : ""}
>
    {t("record")}
</button>
<button
    class:active={currentView === "dictation"}
    onclick={showDictation}
    disabled={$appBusy}
    title={$appBusy ? t("navLockedTooltip") : ""}
>
    {t("dictation")}
</button>
<button
    class:active={currentView === "history"}
    onclick={showHistory}
    disabled={$appBusy}
    title={$appBusy ? t("navLockedTooltip") : ""}
>
    {t("history")}
</button>
```

- [ ] **Step 3: Add `disabled` styling**

In the `<style>` block at the bottom of `+page.svelte`, after the existing `nav button` rules, add:

```css
nav button:disabled {
    opacity: 0.4;
    cursor: not-allowed;
}
```

- [ ] **Step 4: Run check**

Run: `npm run check`
Expected: clean.

- [ ] **Step 5: Commit**

```bash
git add src/routes/+page.svelte
git commit -m "feat(ui): nav buttons disable while a job is finalizing"
```

---

### Task 19: Build `<FinalizingProgress>` component

**Files:**
- Create: `src/lib/FinalizingProgress.svelte`

- [ ] **Step 1: Create the file**

```svelte
<script>
    import { t } from "./i18n.js";

    let { percent = 0, liveText = "", onCancel } = $props();
    let confirming = $state(false);

    const SIZE = 96;
    const STROKE = 8;
    const RADIUS = (SIZE - STROKE) / 2;
    const CIRCUMFERENCE = 2 * Math.PI * RADIUS;

    let dashOffset = $derived(CIRCUMFERENCE * (1 - Math.max(0, Math.min(100, percent)) / 100));

    function requestCancel() {
        confirming = true;
    }

    function dismissCancel() {
        confirming = false;
    }

    function confirmCancel() {
        confirming = false;
        onCancel?.();
    }
</script>

<div class="finalizing">
    <div class="ring-wrap">
        <svg width={SIZE} height={SIZE} viewBox="0 0 {SIZE} {SIZE}">
            <circle
                cx={SIZE / 2}
                cy={SIZE / 2}
                r={RADIUS}
                fill="none"
                stroke="var(--border)"
                stroke-width={STROKE}
            />
            <circle
                cx={SIZE / 2}
                cy={SIZE / 2}
                r={RADIUS}
                fill="none"
                stroke="var(--info)"
                stroke-width={STROKE}
                stroke-dasharray={CIRCUMFERENCE}
                stroke-dashoffset={dashOffset}
                stroke-linecap="round"
                transform="rotate(-90 {SIZE / 2} {SIZE / 2})"
                style="transition: stroke-dashoffset 200ms linear;"
            />
        </svg>
        <span class="percent">{Math.round(percent)}%</span>
    </div>

    <div class="status">
        <strong>{t("finalizing")}</strong>
        <span class="hint">{t("finalizingHint")}</span>
    </div>

    {#if liveText}
        <div class="live-text"><pre>{liveText}</pre></div>
    {/if}

    <button class="btn-cancel" onclick={requestCancel}>
        {t("cancelTranscription")}
    </button>
</div>

{#if confirming}
    <div class="modal-backdrop" role="dialog">
        <div class="modal">
            <h3>{t("cancelConfirmTitle")}</h3>
            <p>{t("cancelConfirmBody")}</p>
            <div class="modal-actions">
                <button class="btn-secondary" onclick={dismissCancel}>
                    {t("cancelConfirmNo")}
                </button>
                <button class="btn-danger" onclick={confirmCancel}>
                    {t("cancelConfirmYes")}
                </button>
            </div>
        </div>
    </div>
{/if}

<style>
    .finalizing {
        display: flex;
        flex-direction: column;
        align-items: center;
        gap: 16px;
        padding: 32px;
    }

    .ring-wrap {
        position: relative;
        width: 96px;
        height: 96px;
    }

    .percent {
        position: absolute;
        inset: 0;
        display: flex;
        align-items: center;
        justify-content: center;
        font-size: 1.1rem;
        font-weight: 600;
    }

    .status {
        display: flex;
        flex-direction: column;
        align-items: center;
        gap: 4px;
        text-align: center;
    }

    .hint {
        font-size: 0.85rem;
        color: var(--text-muted);
    }

    .live-text {
        width: 100%;
        max-width: 600px;
    }

    .live-text pre {
        background: var(--surface);
        border: 1px solid var(--border);
        border-radius: var(--radius);
        padding: 16px;
        white-space: pre-wrap;
        word-wrap: break-word;
        font-family: inherit;
        font-size: 0.95rem;
        line-height: 1.6;
        max-height: 40vh;
        overflow-y: auto;
    }

    .btn-cancel {
        background: transparent;
        color: var(--text-muted);
        border: 1px solid var(--border);
        padding: 8px 20px;
        font-size: 0.9rem;
    }

    .btn-cancel:hover {
        color: var(--accent);
        border-color: var(--accent);
    }

    .modal-backdrop {
        position: fixed;
        inset: 0;
        background: rgba(0, 0, 0, 0.55);
        display: flex;
        align-items: center;
        justify-content: center;
        z-index: 100;
    }

    .modal {
        background: var(--surface);
        border-radius: var(--radius);
        padding: 24px;
        max-width: 360px;
        text-align: center;
    }

    .modal h3 {
        margin-bottom: 8px;
    }

    .modal p {
        color: var(--text-muted);
        margin-bottom: 20px;
    }

    .modal-actions {
        display: flex;
        justify-content: center;
        gap: 12px;
    }

    .btn-secondary {
        background: var(--primary);
        color: white;
        padding: 8px 16px;
    }

    .btn-danger {
        background: var(--accent);
        color: white;
        padding: 8px 16px;
    }
</style>
```

- [ ] **Step 2: Run check**

Run: `npm run check`
Expected: clean.

- [ ] **Step 3: Commit**

```bash
git add src/lib/FinalizingProgress.svelte
git commit -m "feat(ui): FinalizingProgress component with circular progress + cancel modal"
```

---

### Task 20: Refactor `Dictation.svelte` to use the new event protocol

**Files:**
- Modify: `src/lib/Dictation.svelte`

- [ ] **Step 1: Replace the entire `<script>` block**

```svelte
<script>
    import { invoke } from "@tauri-apps/api/core";
    import { listen } from "@tauri-apps/api/event";
    import { onMount, onDestroy } from "svelte";
    import { t, locale } from "./i18n.js";
    import { appBusy } from "./appBusy.js";
    import FinalizingProgress from "./FinalizingProgress.svelte";

    let { onTranscribed } = $props();

    /** "idle" | "recording" | "finalizing" | "cancelling" */
    let phase = $state("idle");
    let error = $state("");
    let liveText = $state("");
    let elapsed = $state(0);
    let percent = $state(0);
    let timer = null;
    let activeJobId = null;

    let unlisteners = [];

    onMount(async () => {
        unlisteners.push(
            await listen("transcription://text", (event) => {
                if (event.payload.id !== activeJobId) return;
                liveText = event.payload.text;
            }),
            await listen("transcription://progress", (event) => {
                if (event.payload.id !== activeJobId) return;
                percent = event.payload.percent;
            }),
            await listen("transcription://complete", (event) => {
                if (event.payload.id !== activeJobId) return;
                phase = "idle";
                appBusy.set(false);
                liveText = "";
                percent = 0;
                onTranscribed?.(event.payload.transcription);
                activeJobId = null;
            }),
            await listen("transcription://cancelled", (event) => {
                if (event.payload.id !== activeJobId) return;
                phase = "idle";
                appBusy.set(false);
                liveText = "";
                percent = 0;
                activeJobId = null;
            }),
            await listen("transcription://error", (event) => {
                if (event.payload.id !== activeJobId) return;
                error = event.payload.error;
                phase = "idle";
                appBusy.set(false);
                activeJobId = null;
            }),
        );
    });

    onDestroy(() => {
        if (timer) clearInterval(timer);
        for (const u of unlisteners) u();
    });

    async function startDictation() {
        try {
            error = "";
            liveText = "";
            percent = 0;
            await invoke("start_dictation", { language: locale });
            phase = "recording";
            elapsed = 0;
            timer = setInterval(() => { elapsed += 1; }, 1000);
        } catch (e) {
            error = e;
        }
    }

    async function stopDictation() {
        try {
            clearInterval(timer);
            timer = null;
            const now = new Date().toLocaleString("pt-BR");
            phase = "finalizing";
            appBusy.set(true);
            const id = await invoke("stop_dictation", {
                title: `${t("dictation")} ${now}`,
                language: locale,
                durationSecs: elapsed,
            });
            activeJobId = id;
        } catch (e) {
            error = e;
            phase = "idle";
            appBusy.set(false);
        }
    }

    async function requestCancel() {
        try {
            phase = "cancelling";
            await invoke("cancel_job");
        } catch (e) {
            error = e;
        }
    }

    function formatTime(secs) {
        const m = Math.floor(secs / 60).toString().padStart(2, "0");
        const s = (secs % 60).toString().padStart(2, "0");
        return `${m}:${s}`;
    }
</script>
```

- [ ] **Step 2: Replace the markup**

```svelte
<div class="dictation">
    {#if phase === "recording"}
        <div class="status dictating">
            <span class="dot"></span>
            {t("dictating")} {formatTime(elapsed)}
        </div>
        <button class="btn-stop" onclick={stopDictation}>
            {t("stopDictation")}
        </button>
        {#if liveText}
            <div class="live-text"><pre>{liveText}</pre></div>
        {/if}
    {:else if phase === "finalizing" || phase === "cancelling"}
        <FinalizingProgress {percent} {liveText} onCancel={requestCancel} />
    {:else}
        <button class="btn-start" onclick={startDictation}>
            {t("startDictation")}
        </button>
    {/if}

    {#if error}
        <div class="error">{error}</div>
    {/if}
</div>
```

The existing `<style>` block can stay as-is. Remove the now-unused `.live-text` styles only if they conflict (they shouldn't — the new component owns its own `.live-text`).

- [ ] **Step 3: Re-add the `dictation://segment` listener for live-recording feedback**

The old `dictation://segment` event is still emitted by the Rust dictation loop during recording (Task 12 only changed the stop path). The new `<script>` we just wrote dropped the listener for it, so the live-text box would no longer update during recording. Add the listener back, scoped to the `recording` phase only — the new `transcription://text` events take over once we transition to `finalizing`.

Confirm there are no other callers first:

Run: `grep -rn "dictation://" src/ src-tauri/`
Expected: only the emit in `src-tauri/src/dictation.rs`. Then add the listener:

Add inside `onMount`:

```js
unlisteners.push(
    await listen("dictation://segment", (event) => {
        if (phase !== "recording") return;
        liveText = event.payload.fullText;
    }),
);
```

This keeps backward-compat with the existing live-during-recording behavior.

- [ ] **Step 4: Run check**

Run: `npm run check`
Expected: clean.

- [ ] **Step 5: Commit**

```bash
git add src/lib/Dictation.svelte
git commit -m "feat(ui): Dictation uses transcription://* events + FinalizingProgress"
```

---

### Task 21: Refactor `Recorder.svelte` to use the new pending flow

**Files:**
- Modify: `src/lib/Recorder.svelte`

- [ ] **Step 1: Add imports**

In `src/lib/Recorder.svelte`'s `<script>`, after existing imports:

```js
import { listen } from "@tauri-apps/api/event";
import { appBusy } from "./appBusy.js";
import FinalizingProgress from "./FinalizingProgress.svelte";
```

- [ ] **Step 2: Add finalizing state**

Replace `let transcribingId = $state(null);` with:

```js
let transcribingId = $state(null);
let liveText = $state("");
let percent = $state(0);
let phase = $state("idle"); // "idle" | "finalizing" | "cancelling"
let unlisteners = [];
```

Add a tracking variable above `onMount` (next to the other `let` declarations):

```js
let startedFromPendingId = null;
```

Then add to `onMount`:

```js
unlisteners.push(
    await listen("transcription://text", (event) => {
        if (event.payload.id !== transcribingId) return;
        liveText = event.payload.text;
    }),
    await listen("transcription://progress", (event) => {
        if (event.payload.id !== transcribingId) return;
        percent = event.payload.percent;
    }),
    await listen("transcription://complete", (event) => {
        if (event.payload.id !== transcribingId) return;
        const t_ = event.payload.transcription;
        if (startedFromPendingId !== null) {
            pendingRecordings = pendingRecordings.filter((p) => p.id !== startedFromPendingId);
        }
        phase = "idle";
        appBusy.set(false);
        liveText = "";
        percent = 0;
        transcribingId = null;
        startedFromPendingId = null;
        onTranscribed?.(t_);
    }),
    await listen("transcription://cancelled", (event) => {
        if (event.payload.id !== transcribingId) return;
        phase = "idle";
        appBusy.set(false);
        liveText = "";
        percent = 0;
        transcribingId = null;
        startedFromPendingId = null;
    }),
    await listen("transcription://error", (event) => {
        if (event.payload.id !== transcribingId) return;
        error = event.payload.error;
        phase = "idle";
        appBusy.set(false);
        transcribingId = null;
        startedFromPendingId = null;
    }),
);
```

Add to `onDestroy`:

```js
for (const u of unlisteners) u();
```

- [ ] **Step 3: Rewrite `transcribePending`**

```js
async function transcribePending(id) {
    try {
        error = "";
        startedFromPendingId = id;
        phase = "finalizing";
        appBusy.set(true);
        liveText = "";
        percent = 0;
        const now = new Date().toLocaleString("pt-BR");
        const newId = await invoke("transcribe_pending_recording", {
            pendingId: id,
            title: `${t("meetingTitle")} ${now}`,
            language: locale,
        });
        transcribingId = newId;
    } catch (e) {
        error = e;
        phase = "idle";
        appBusy.set(false);
        startedFromPendingId = null;
    }
}

async function requestCancel() {
    try {
        phase = "cancelling";
        await invoke("cancel_job");
    } catch (e) {
        error = e;
    }
}
```

- [ ] **Step 4: Add finalizing UI to the markup**

In the markup block, replace `{#if !recording && !processing && pendingRecordings.length > 0}` with `{#if phase === "finalizing" || phase === "cancelling"}` first branch, then `{:else if !recording && !processing && pendingRecordings.length > 0}`:

```svelte
{#if phase === "finalizing" || phase === "cancelling"}
    <FinalizingProgress {percent} {liveText} onCancel={requestCancel} />
{:else if !recording && !processing && pendingRecordings.length > 0}
    <!-- existing pending list block stays here -->
{/if}
```

Also hide the start button when finalizing:

Replace:

```svelte
{#if recording}
   ...
{:else if processing}
   ...
{:else}
    <button class="btn-start" onclick={startRecording}>
        {t("startRecording")}
    </button>
{/if}
```

with:

```svelte
{#if recording}
   ...
{:else if processing}
   ...
{:else if phase === "idle"}
    <button class="btn-start" onclick={startRecording}>
        {t("startRecording")}
    </button>
{/if}
```

- [ ] **Step 5: Run check**

Run: `npm run check`
Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add src/lib/Recorder.svelte
git commit -m "feat(ui): Recorder uses transcribe_pending_recording + FinalizingProgress"
```

---

### Task 22: Show "Parcial" badge in History

**Files:**
- Modify: `src/lib/History.svelte`

- [ ] **Step 1: Render the badge in the title block**

In the `<li>` markup, inside the `<span class="title">` block, after the existing `{#if item.summary}` block, add:

```svelte
{#if item.status === "partial"}
    <span class="partial-badge" title={t("partialBadge")}>⚠ {t("partialBadge")}</span>
{/if}
```

- [ ] **Step 2: Add styling for the badge**

In the `<style>` block:

```css
.partial-badge {
    display: inline-block;
    background: rgba(255, 193, 7, 0.18);
    color: #ffb300;
    font-size: 0.7rem;
    font-weight: 600;
    padding: 2px 8px;
    border-radius: 10px;
    margin-left: 6px;
    vertical-align: middle;
}
```

- [ ] **Step 3: Run check**

Run: `npm run check`
Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add src/lib/History.svelte
git commit -m "feat(ui): partial badge in history list"
```

---

### Task 23: Bump version to 0.2.0

**Files:**
- Modify: `package.json`
- Modify: `src-tauri/Cargo.toml`
- Modify: `src-tauri/tauri.conf.json`

- [ ] **Step 1: Update `package.json`**

Change `"version": "0.1.0"` to `"version": "0.2.0"`.

- [ ] **Step 2: Update `src-tauri/Cargo.toml`**

Change `version = "0.1.0"` to `version = "0.2.0"`.

- [ ] **Step 3: Update `src-tauri/tauri.conf.json`**

Change `"version": "0.1.0"` to `"version": "0.2.0"`.

- [ ] **Step 4: Run a sanity build**

Run: `cd src-tauri && cargo check`
Run: `npm run check`
Expected: both clean.

- [ ] **Step 5: Commit**

```bash
git add package.json src-tauri/Cargo.toml src-tauri/tauri.conf.json
git commit -m "chore: bump version to 0.2.0"
```

---

### Task 24: Manual integration testing

**Files:** none modified — this task produces the test report committed in Task 25.

Open a notes file `/tmp/martin-test-report.md` while doing this. Record observed times and pass/fail.

- [ ] **Step 1: Build the app in dev mode**

Run: `cargo tauri dev`
Wait until the window opens.

- [ ] **Step 2: Test scenario A — short dictation (10s)**

1. Click "Iniciar Ditado".
2. Speak 10 seconds of clear Portuguese ("um, dois, três, teste do martin").
3. Click "Parar Ditado".
4. Observe: `<FinalizingProgress>` appears, % advances, `liveText` shows the transcription.
5. Wait for completion (≤ 60s on this dev machine).
6. Confirm: navigates to `TranscriptionView` with the text. Status not visible yet — confirm DB row has `status = 'complete'` via:
   ```bash
   python3 -c "import sqlite3; con = sqlite3.connect('/home/nuuvem/.local/share/com.nuuvem.martin/martin.db'); print(list(con.execute('SELECT id, status, length(text) FROM transcriptions ORDER BY id DESC LIMIT 1')))"
   ```

Record: PASS / FAIL + observed time.

- [ ] **Step 3: Test scenario B — long dictation (60s)**

Same flow with 60 seconds of speech. Confirm:
- During recording, the live-text box updates with at least one segment.
- Stop button → finalizing screen.
- Progress %  monotonically advances.
- Eventually saves to history with `status = 'complete'`.
- Nav buttons are visibly disabled during finalizing.

Record: PASS / FAIL + observed time.

- [ ] **Step 4: Test scenario C — Stop within 1 second**

Start dictation, immediately stop. Confirm:
- No `"No text was transcribed"` error path.
- Finalizing screen shows briefly, then transitions to history.
- Row exists in `transcriptions` (possibly with empty text but `status = 'complete'`).

Record: PASS / FAIL.

- [ ] **Step 5: Test scenario D — cancel mid-finalization**

Dictate 30s, stop, click Cancelar before finalizing finishes, confirm in modal. Verify:
- `transcription://cancelled` event arrives.
- DB has no new row for this job.
- Live-text disappears, returns to idle.

Record: PASS / FAIL.

- [ ] **Step 6: Test scenario E — pending file flow**

1. Click "Gravar".
2. Record 15s of audio. Stop.
3. The pending recording appears below.
4. Click "Transcrever".
5. Observe: `<FinalizingProgress>` shows up immediately, % advances.
6. Wait for complete. Confirm:
   - Pending row deleted (`SELECT * FROM pending_recordings`).
   - WAV file deleted from `~/.local/share/com.nuuvem.martin/`.
   - Transcription appears in history.

Record: PASS / FAIL.

- [ ] **Step 7: Test scenario F — start while one is finalizing**

While a job is finalizing, try to click "Gravar" or "Ditado" in the nav. Verify both are disabled with the tooltip.

Then try invoking `start_dictation` from the JS console (DevTools): expected error "Another transcription is in progress". (Skip this if DevTools is awkward to access in the dev build; the nav guard is enough proof.)

Record: PASS / FAIL.

- [ ] **Step 8: Test scenario G — force kill during finalizing**

1. Start a long dictation (~30s).
2. Stop. Wait for `<FinalizingProgress>` to show ~20%.
3. Force-kill the app: `pkill -9 martin` from a terminal.
4. Reopen the app.
5. Observe in History: a row appears with the partial badge.

Record: PASS / FAIL.

---

### Task 25: Commit the test report

**Files:**
- Create: `docs/specs/2026-05-03-transcription-finalizing-flow-test-report.md`

- [ ] **Step 1: Write the report**

Use the structure from Task 24 — list each scenario with PASS/FAIL, observed timing, and any anomalies.

- [ ] **Step 2: Commit**

```bash
git add docs/specs/2026-05-03-transcription-finalizing-flow-test-report.md
git commit -m "docs: manual test report for finalizing flow v0.2.0"
```

---

### Task 26: Update release notes

**Files:**
- Create: `docs/release-notes-v0.2.0.md`

- [ ] **Step 1: Draft the notes**

```md
# v0.2.0 — Finalizing flow

## Fixed
- Dictation no longer loses text when Stop is pressed before whisper produces its first segment. The backend now owns the transcription text end-to-end.

## Added
- Explicit "finalizing" phase for both flows (live dictation + transcribe pending recording), with a circular progress indicator and live text.
- Navigation menu locks while a transcription is finalizing, so users can't navigate into half-rendered states.
- Cancel-with-confirmation: stops the running whisper inference cleanly and removes the in-progress row.
- Partial recovery: if the app is killed mid-finalization, the partial text is kept and surfaced in History with a "Parcial" badge.

## Changed
- IPC surface: removed `transcribe_recording`; added `transcribe_pending_recording`, `cancel_job`, `current_job_status`. `stop_dictation` no longer accepts `full_text`.
- `transcriptions` schema gains a `status` column (`complete` / `partial` / `failed`). Existing rows are migrated to `complete` automatically on first launch.

## Known limitations
- Dictation audio is held in RAM while recording; a crash during recording (before Stop) loses the audio. Tracked separately.
- Whisper is CPU-only in this build. On slower machines (≤ 4 cores at low frequency) finalizing a long recording can take several minutes — the new progress UI makes this visible. A whisper-rs build with OpenBLAS / Vulkan acceleration is tracked as a follow-up.
```

- [ ] **Step 2: Commit**

```bash
git add docs/release-notes-v0.2.0.md
git commit -m "docs: release notes for v0.2.0"
```

---

### Task 27: Final review pass and pre-PR cleanup

- [ ] **Step 1: Re-run everything**

```bash
cd src-tauri && cargo fmt && cargo clippy -- -D warnings && cargo test --lib
cd .. && npm run check
```

Expected: all clean.

- [ ] **Step 2: Inspect the diff**

Run: `git log --oneline main..HEAD`
Expected: a clean sequence of focused commits, no fixup-style noise.

Run: `git diff main..HEAD --stat`
Expected: file changes match the file map at the top of this plan.

- [ ] **Step 3: Open the PR**

Push the branch:

```bash
git push -u origin feat/finalizing-flow
```

Open PR via `gh`:

```bash
gh pr create --title "feat: transcription finalizing flow (v0.2.0)" --body "$(cat <<'EOF'
## Summary
- Fixes the dictation save bug where text was lost on slow machines.
- Adds an explicit finalizing phase with progress + live text + cancel.
- Locks navigation while a job is finalizing.
- Persists partial state and surfaces it via a "Parcial" badge in History.
- Bumps to v0.2.0; IPC commands change.

## Spec
docs/specs/2026-05-03-transcription-finalizing-flow-design.md

## Plan
docs/specs/2026-05-03-transcription-finalizing-flow-plan.md

## Test plan
docs/specs/2026-05-03-transcription-finalizing-flow-test-report.md

## Migration
- The `transcriptions` table gains a `status` column. Migration is idempotent on launch.
EOF
)"
```

---

## Self-review checklist (run after writing the plan, before execution)

The plan author has already run this once. The executing engineer should re-verify:

- [ ] Every spec section maps to at least one task. (See File map above.)
- [ ] No "TBD" or "implement later" lines in any step.
- [ ] Type and method names used in later tasks match what was defined earlier (`Transcription.status`, `Store::insert_partial`, `Store::update_text`, `Store::mark_complete`, `Store::reset_partial_on_startup`, `Transcriber::transcribe_with_callbacks`, `TranscriptionJob`, `JobKind`, `FinalizeOutcome`, `run_finalize_dictation`, `run_finalize_pending_file`, `finish_job`, `cancel_job`, `current_job_status`, `transcribe_pending_recording`, `appBusy`, `FinalizingProgress`).
- [ ] Frontend listens to all five `transcription://*` events that the backend emits.
- [ ] Cancel cleanup matches spec: dictation deletes DB row; pending-file deletes DB row + WAV but keeps `pending_recordings` row.
- [ ] Migration is idempotent (Task 1 test asserts this).
