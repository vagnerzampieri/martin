# Transcription Finalizing Flow — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix the dictation save bug and give both transcription flows (live dictation + transcribe pending recording) a clear, observable finalizing phase with progress, navigation lock, save-as-we-go persistence, and cancel-with-confirmation.

**Architecture:** Backend owns transcription text. A unified `TranscriptionJob` runs whisper on a worker thread, persisting text incrementally to a `transcriptions` row that gains a `status` column. Whisper's segment + progress + abort callbacks drive event emission and cancellation. Frontend listens on a unified `transcription://*` event namespace, transitions through an explicit state machine (recording → finalizing → complete | cancelling), and locks navigation during processing via a small Svelte store.

**Tech Stack:** Rust (whisper-rs callbacks, rusqlite), Tauri 2 events, Svelte 5 runes, no new deps.

**Branch:** `feat/finalizing-flow` — already created. All work commits here. The release tag will be `v0.2.0` (IPC commands change; see Tasks 12–14).

**Spec:** `docs/specs/2026-05-03-transcription-finalizing-flow-design.md`

---

## File map

| Path | Action | Responsibility |
|---|---|---|
| `src-tauri/src/db/store.rs` | modify | Schema migration, WAL pragma, `insert_partial`/`update_text`/`mark_complete`/`delete_empty_partials`, `Transcription.status` |
| `src-tauri/src/transcribe/job.rs` | create | `TranscriptionJob`, `JobKind`, `FinalizeOutcome`, `run_finalize_*`, `finish_job`, `ErrorPayload` |
| `src-tauri/src/transcribe/mod.rs` | modify | Export `job` module |
| `src-tauri/src/transcribe/whisper.rs` | modify | New `transcribe_with_callbacks`, expose `load_wav_as_mono_f32` and `wav_duration_secs` |
| `src-tauri/src/dictation.rs` | modify | `JoinHandle` on session, last_full_text accessor, `stop_and_join` |
| `src-tauri/src/lib.rs` | modify | New commands (`stop_dictation` rewritten, `transcribe_pending_recording`, `cancel_job`), `current_job` in `AppState`, remove `transcribe_recording`, `try_state` instead of `state` in workers |
| `src-tauri/Cargo.toml` | modify | Version bump |
| `src/lib/appBusy.js` | create | Tiny boolean store for nav lock |
| `src/lib/FinalizingProgress.svelte` | create | Shared finalizing UI: ring, indeterminate state, jobLabel, accessible cancel modal |
| `src/lib/Dictation.svelte` | modify | State machine + new event protocol |
| `src/lib/Recorder.svelte` | modify | Reroute `transcribePending` to new flow |
| `src/lib/History.svelte` | modify | "Parcial" badge for partial rows |
| `src/lib/i18n.js` | modify | New strings (no `keepPartial`/`discardPartial`) |
| `src/routes/+page.svelte` | modify | `aria-disabled` on Record + Dictate; History remains free |
| `src-tauri/tauri.conf.json` | modify | Version bump |
| `package.json` | modify | Version bump |

---

### Task 1: Add `status` column to `transcriptions` (idempotent migration) + enable WAL

**Files:**
- Modify: `src-tauri/src/db/store.rs`

Status vocabulary is `'complete'` and `'partial'` (no `'failed'` — review found it was reserved but never written).

- [ ] **Step 1: Write tests for the migration**

Two tests: idempotent re-open (covers re-running the migration on a DB that already has the column) and real-upgrade (covers the v0.1.0 → v0.2.0 path on a populated table).

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

#[test]
fn migration_backfills_existing_rows_with_complete_status() {
    // Simulate a v0.1.0 database: create the schema by hand WITHOUT the
    // `status` column, insert rows, then open via Store::new (which runs
    // the migration). All existing rows must end up with `status='complete'`.
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
                summary TEXT
            )",
            [],
        )
        .expect("create v0.1.0 schema");
        conn.execute(
            "INSERT INTO transcriptions (title, text, language, duration_secs) VALUES ('old1', 'a', 'pt', 1.0), ('old2', 'b', 'en', 2.0)",
            [],
        )
        .expect("seed");
    }

    let store = Store::new(&path).expect("upgrade open");
    let rows = store.list().expect("list");
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|r| r.status == "complete"));
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd src-tauri && cargo test --lib db::store::tests::new_runs_migration_idempotently db::store::tests::migration_backfills_existing_rows_with_complete_status`
Expected: FAIL.

- [ ] **Step 3: Add the migration + enable WAL in `Store::new`**

Find the `Store::new` function in `src-tauri/src/db/store.rs`. Right after `Connection::open(...)`, set pragmas — WAL keeps readers from blocking writers, and `synchronous=NORMAL` matches WAL's durability without per-write fsync (per-segment writes during whisper finalize are otherwise an I/O bottleneck — see Task 8).

```rust
conn.pragma_update(None, "journal_mode", "WAL")
    .map_err(|e| format!("Failed to set WAL mode: {}", e))?;
conn.pragma_update(None, "synchronous", "NORMAL")
    .map_err(|e| format!("Failed to set synchronous mode: {}", e))?;
```

Then, after the existing `CREATE TABLE` calls and before `Ok(Self { conn })`, add the migration:

```rust
// Migration: add `status` column if missing. Idempotent — older databases
// (created before this column existed) gain it on next launch with
// existing rows backfilled to 'complete' via the column DEFAULT.
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

Note: SQLite does not support adding a CHECK constraint via ALTER TABLE, so the column accepts any string. The code only writes `'complete'` and `'partial'`; future tightening would require a table rebuild and is deferred.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cd src-tauri && cargo test --lib db::store::tests::new_runs_migration db::store::tests::migration_backfills`
Expected: PASS.

- [ ] **Step 5: Run the full test suite**

Run: `cd src-tauri && cargo test --lib`
Expected: all existing tests still pass.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/db/store.rs
git commit -m "feat(db): add status column with idempotent migration and enable WAL"
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

### Task 5: Add `Store::delete_empty_partials` (startup sweep)

**Files:**
- Modify: `src-tauri/src/db/store.rs`
- Modify: `src-tauri/src/lib.rs`

The earlier draft of this task added a generic `reset_partial_on_startup` that normalised any non-`complete`/`failed`/`partial` rows. Review found this was speculative — the code never produces a third status, so the function had no real callers. The `'failed'` value is also dropped from the schema vocabulary for the same reason (the Error arm of `finish_job` leaves the row at `'partial'`).

What we DO need: when the app is force-killed between `insert_partial` and the first segment callback, History gets a row with `text=''` and `duration_secs=0.0` named "Dictation 03/05/2026 14:32:11" — visually indistinguishable from a real transcription. Sweep those on startup.

- [ ] **Step 1: Write the test**

```rust
#[test]
fn delete_empty_partials_removes_only_empty_partials() {
    let (store, _temp_file) = create_temp_store();

    let kept_complete = store.save("c", "x", "pt", 1.0).expect("save");
    let kept_partial_with_text = store.insert_partial("p1", "pt").expect("insert");
    store.update_text(kept_partial_with_text, "some text", 5.0).expect("update");
    let removed_id = store.insert_partial("ghost", "pt").expect("insert");

    let removed = store.delete_empty_partials().expect("sweep");
    assert_eq!(removed, 1, "only the empty partial should be deleted");

    assert!(store.get(kept_complete).is_ok());
    assert!(store.get(kept_partial_with_text).is_ok());
    assert!(store.get(removed_id).is_err());
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cd src-tauri && cargo test --lib db::store::tests::delete_empty_partials`
Expected: FAIL.

- [ ] **Step 3: Implement the method**

```rust
/// Removes partial rows that have no text and no duration — these can only
/// come from a force-kill that happened before the first segment callback
/// fired. Returns the number of rows deleted.
pub fn delete_empty_partials(&self) -> Result<usize, String> {
    let affected = self
        .conn
        .execute(
            "DELETE FROM transcriptions WHERE status = 'partial' AND text = '' AND duration_secs = 0.0",
            [],
        )
        .map_err(|e| format!("Failed to sweep empty partials: {}", e))?;
    Ok(affected)
}
```

- [ ] **Step 4: Wire into app startup**

In `src-tauri/src/lib.rs`'s setup closure, after `Store::new`:

```rust
let swept = store
    .delete_empty_partials()
    .expect("Failed to sweep empty partials on startup");
if swept > 0 {
    eprintln!("[startup] swept {} empty partial transcription(s)", swept);
}
```

- [ ] **Step 5: Run all tests**

Run: `cd src-tauri && cargo test --lib db::store`
Expected: all pass.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/db/store.rs src-tauri/src/lib.rs
git commit -m "feat(db): sweep empty partial transcriptions on startup"
```

---

### Task 6: Add `Transcriber::transcribe_with_callbacks`

**Files:**
- Modify: `src-tauri/src/transcribe/whisper.rs`

This wraps `state.full` with progress, segment, and abort callbacks routed back to caller-supplied closures. Used by both job kinds.

- [ ] **Step 1: Verify the whisper-rs FullParams callback API**

The existing `transcribe` in `whisper.rs` does not use callbacks, so you cannot confirm the API there. Instead verify against the docs for the version pinned in `Cargo.lock`:

Run: `cd src-tauri && cargo doc --no-deps -p whisper-rs --open` (or browse `https://docs.rs/whisper-rs/<version>/whisper_rs/struct.FullParams.html`).

Confirm `FullParams` exposes `set_progress_callback_safe(FnMut(i32) + Send + 'static)`, `set_segment_callback_safe_lossy(FnMut(&str) + Send + 'static)`, and `set_abort_callback_safe(FnMut() -> bool + Send + 'static)`. If the pinned version exposes only `_unsafe` variants, stop and either upgrade `whisper-rs` or raise the issue — wrapping unsafe trampolines is out of scope for this task.

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

#[derive(Clone)]
pub struct TranscriptionJob {
    pub id: i64,
    pub kind: JobKind,
    pub cancel_flag: Arc<AtomicBool>,
    pub committed_text: String,
}

// Cloning a `TranscriptionJob` clones the `Arc<AtomicBool>` — both copies
// observe the same cancel flag. This is what lets `cancel_job` (which
// holds the copy stored in `current_job`) signal the worker thread (which
// owns the original).

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
pub struct ErrorPayload {
    pub id: i64,
    pub error: String,
}

#[derive(Clone, Serialize)]
struct CancelledPayload {
    id: i64,
}

use std::time::{Duration, Instant};

/// Runs whisper on `samples`, updating the row in `store` and emitting
/// `transcription://*` events. Returns the outcome so the orchestrator
/// can choose the terminal action (mark_complete, delete, leave partial).
///
/// Persistence is debounced: SQLite writes happen at most every
/// `PERSIST_DEBOUNCE_MS` to avoid fsync storms during fast segment cadence.
/// The Tauri text event is emitted on every segment for live UI feedback.
/// `finish_job`'s Complete arm performs the final authoritative write, so
/// no data is lost if the last debounced write was skipped.
pub fn run_finalize_dictation(
    job: &TranscriptionJob,
    transcriber: &Transcriber,
    samples: Vec<f32>,
    duration_secs: f64,
    language: String,
    store: Arc<Mutex<Store>>,
    app_handle: AppHandle,
) -> FinalizeOutcome {
    const PERSIST_DEBOUNCE_MS: u64 = 1000;

    let id = job.id;
    let cancel_flag = job.cancel_flag.clone();
    let committed_prefix = job.committed_text.clone();

    // Accumulator shared with the segment callback.
    let accumulated = Arc::new(Mutex::new(committed_prefix.clone()));
    let acc_for_callback = accumulated.clone();
    let store_for_callback = store.clone();
    let app_for_callback = app_handle.clone();
    let last_persist = Arc::new(Mutex::new(Instant::now()));
    let last_persist_for_callback = last_persist.clone();

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
        // Lock guards may be poisoned if a previous callback panicked. Take
        // the inner value via `into_inner` semantics by `unwrap_or_else`,
        // but for our case we just skip the segment on poison rather than
        // propagate panic into whisper's inference thread.
        let new_text = match acc_for_callback.lock() {
            Ok(mut acc) => {
                if acc.is_empty() {
                    acc.push_str(trimmed);
                } else {
                    acc.push(' ');
                    acc.push_str(trimmed);
                }
                acc.clone()
            }
            Err(_) => return,
        };

        // Debounced persistence: write at most once per PERSIST_DEBOUNCE_MS.
        // The final authoritative write happens in finish_job's Complete arm.
        let should_persist = match last_persist_for_callback.lock() {
            Ok(mut last) => {
                if last.elapsed() >= Duration::from_millis(PERSIST_DEBOUNCE_MS) {
                    *last = Instant::now();
                    true
                } else {
                    false
                }
            }
            Err(_) => false,
        };

        if should_persist {
            if let Ok(s) = store_for_callback.lock() {
                let _ = s.update_text(id, &new_text, duration_secs);
            }
        }

        // Emit on every segment — UI feedback should be immediate even when
        // SQLite writes are debounced.
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

    // Trust whisper's return value: an aborted inference returns Err. Do
    // NOT post-hoc check the cancel flag against an Ok result — that would
    // turn a successful-but-late-cancel into a phantom cancellation
    // (deleting work the user actually got back).
    match result {
        Ok(_) => {
            let final_text = accumulated
                .lock()
                .map(|a| a.clone())
                .unwrap_or(committed_prefix);
            FinalizeOutcome::Complete { final_text, duration_secs }
        }
        Err(_) if cancel_flag.load(std::sync::atomic::Ordering::Acquire) => {
            FinalizeOutcome::Cancelled
        }
        Err(e) => FinalizeOutcome::Error(e),
    }
}
```

Notes on the rewrite vs. the original sketch:

1. **Debounced persistence (1s)** — segment callbacks fire on whisper's inference thread; per-segment fsync against SQLite was a P2 contention risk. The Tauri text event still fires on every segment for live UI feedback.
2. **No `.expect("acc lock")`** — a poisoned mutex (from a prior callback panic) skips the segment instead of propagating panic into whisper's inference thread.
3. **No redundant final flush** — `finish_job`'s Complete arm calls `update_text` then `mark_complete` as the single terminal write.
4. **Cancel signal trusted via whisper's return** — `Ok(_)` is always a successful completion; only `Err` with `cancel_flag` set is a cancellation. This closes the "cancel-at-completion-boundary" race surfaced by the adversarial review.

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
    let samples = match crate::transcribe::whisper::Transcriber::load_wav_as_mono_f32(wav_path) {
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

(a) Make `load_wav_as_mono_f32` public — change `fn load_wav_as_mono_f32(...)` to `pub fn load_wav_as_mono_f32(...)` on the existing definition (around line 82). Do not add a wrapper; per CLAUDE.md, "Avoid empty abstractions."

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
/// then emit the matching event. The caller is responsible for clearing
/// `current_job` from `AppState` BEFORE calling `finish_job` so that any
/// frontend handler reacting to the emitted terminal event can immediately
/// initiate a new job without seeing a stale `current_job` reservation.
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
                // Cancel = discard everything. The transcription row, if
                // any, is removed. For pending-file kind we leave BOTH the
                // WAV and the pending_recordings row untouched so the user
                // can simply click Transcrever again.
                let _ = s.delete(id);
            }
            let _ = app_handle.emit("transcription://cancelled", CancelledPayload { id });
        }
        FinalizeOutcome::Error(err) => {
            // Row stays partial (status='partial') with whatever the
            // debounced segment callback persisted. The user sees it in
            // History via the partial badge — same shape as crash recovery.
            let _ = app_handle.emit(
                "transcription://error",
                ErrorPayload { id, error: err },
            );
        }
    }
}
```

Notes on the cancelled and error branches:
- **Cancel = discard all** (matches the modal copy "O conteúdo será descartado"). Earlier drafts had `keepPartial`/`discardPartial` strings implying a preserve-partial choice; those are dropped (Task 16). Partial state is reserved for unintentional crashes, not deliberate cancellation.
- **Pending-file cancel** preserves both the WAV and the pending_recordings row — the original asymmetric "delete WAV but keep row" behavior produced a phantom-retry that always failed.
- **Empty-partial sweep on startup** (added in Task 5b below) deletes any partial row with empty text and zero duration — these can only come from a force-kill that happened before the first segment callback fired, and they are visually indistinguishable from real transcriptions in History.

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

In the existing `setup` closure (around line 350), wrap the store in an Arc. The `delete_empty_partials` call belongs here too — Task 5 wires it in.

```rust
let store = Store::new(&db_path).expect("Failed to create store");
let swept = store
    .delete_empty_partials()
    .expect("Failed to sweep empty partials on startup");
if swept > 0 {
    eprintln!("[startup] swept {} empty partial transcription(s)", swept);
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

Also note: `state.store.clone()` now produces a new `Arc` reference (not a cloned `Store`). This is what enables worker threads to hold a shared reference — Tasks 12 and 13 rely on it.

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

- [ ] **Step 1: Add JoinHandle + last_full_text to DictationSession; expose committed segments and buffer drain**

The original draft of this step had a fatal race: the live `run_transcription_loop` may still be mid-`transcribe_samples` when `stop_dictation` runs. Without a join, `drain_buffer` returns *less* than what was captured (samples are stuck inside the live loop's stack frame), and the live loop's transcriber is still in use when `stop_dictation` calls `take()` on the cache — leading to two whisper contexts loaded simultaneously. Both surfaced as P0s in review.

Fix: **own a `std::thread::JoinHandle<()>` on `DictationSession`**, switch the dictation loop from `tauri::async_runtime::spawn_blocking` to `std::thread::spawn`, and join it in `stop_dictation` before doing anything else. By the time we read `committed`/`drain_buffer`, the live loop has fully exited and put the cached transcriber back.

Also add `last_full_text` so `stop_dictation` can seed the finalize worker with the most recent live-emitted transcription as the committed prefix — this turns the short-dictation case (under one rollover) from "re-transcribe everything from scratch" into "re-transcribe the un-committed tail."

In `src-tauri/src/dictation.rs`:

```rust
use std::thread::JoinHandle;

pub struct DictationSession {
    stream: Option<Stream>,
    audio_buffer: Arc<Mutex<Vec<f32>>>,
    running: Arc<AtomicBool>,
    source_rate: u32,
    channels: u16,
    committed: Arc<Mutex<Vec<String>>>,
    last_full_text: Arc<Mutex<String>>,
    worker: Option<JoinHandle<()>>,
}
```

Initialize the new fields in `new()`:

```rust
committed: Arc::new(Mutex::new(Vec::new())),
last_full_text: Arc::new(Mutex::new(String::new())),
worker: None,
```

Add accessors and the buffer drain:

```rust
pub fn committed(&self) -> Arc<Mutex<Vec<String>>> {
    self.committed.clone()
}

pub fn last_full_text(&self) -> Arc<Mutex<String>> {
    self.last_full_text.clone()
}

pub fn set_worker(&mut self, handle: JoinHandle<()>) {
    self.worker = Some(handle);
}

/// Stops the audio stream, signals the worker, and joins it.
/// Returns only after the worker has fully exited.
pub fn stop_and_join(&mut self) {
    self.running.store(false, std::sync::atomic::Ordering::Release);
    self.stream.take();  // drops the stream, stopping capture
    if let Some(handle) = self.worker.take() {
        let _ = handle.join();
    }
}

pub fn drain_buffer(&self) -> Vec<f32> {
    self.audio_buffer
        .lock()
        .map(|mut buf| buf.drain(..).collect())
        .unwrap_or_default()
}
```

Modify `run_transcription_loop` to take two extra parameters: `committed_out: Arc<Mutex<Vec<String>>>` and `last_full_text_out: Arc<Mutex<String>>`. After every `committed_segments.push(text)` (rollover and final-block cases), also push into `committed_out`. After every emit of `dictation://segment` with `fullText`, also overwrite `last_full_text_out`:

```rust
if let Ok(mut sink) = committed_out.lock() {
    sink.push(text.clone());
}
// After emitting the dictation://segment event:
if let Ok(mut last) = last_full_text_out.lock() {
    *last = full_text.clone();
}
```

Note: this preserves the existing `dictation://segment` emit (which Task 20's frontend uses for live-during-recording feedback) while exposing the same `fullText` to the backend for the prefix.

- [ ] **Step 2: Update `start_dictation` in `lib.rs` to use `std::thread::spawn` and store the JoinHandle**

The existing code uses `tauri::async_runtime::spawn` (or `spawn_blocking`) for the live transcription loop, which gives a JoinHandle that can only be awaited from async context — useless from synchronous `stop_dictation`. Switch to `std::thread::spawn`, capture the handle, and store it on the session so `stop_and_join` can wait deterministically.

Reorder so accessors are pulled BEFORE the session moves into AppState:

```rust
let mut session = DictationSession::new();
session.start()?;

let buffer = session.buffer();
let running = session.running_flag();
let source_rate = session.source_rate();
let channels = session.channels();
let committed_out = session.committed();
let last_full_text_out = session.last_full_text();

// Take the transcriber out of the cache — the live loop owns it until
// it exits. Stop_dictation will only `.take()` from the cache AFTER
// joining, by which point the live loop has put it back.
let transcriber = state
    .transcriber
    .lock()
    .map_err(|e| e.to_string())?
    .take()
    .ok_or("Transcriber not initialised")?;

let app_handle_for_loop = app_handle.clone();
let language_owned = language.clone();

let worker = std::thread::spawn(move || {
    dictation::run_transcription_loop(
        buffer,
        running,
        committed_out,
        last_full_text_out,
        &transcriber,
        &language_owned,
        source_rate,
        channels,
        app_handle_for_loop.clone(),
    );

    // Restore the transcriber to the cache for the finalize worker.
    let app_state = app_handle_for_loop.try_state::<AppState>();
    if let Some(s) = app_state {
        let _ = s.transcriber.lock().map(|mut g| *g = Some(transcriber));
    }
});

session.set_worker(worker);

*state.dictation.lock().map_err(|e| e.to_string())? = Some(session);
```

Two notes on this rewrite:
- The transcriber is restored to the cache *inside the worker thread* on exit. This guarantees `stop_dictation` (after joining the worker) sees the transcriber back in the cache, avoiding the dual-load OOM scenario.
- `try_state` instead of `state` so a window-close during shutdown does not panic.

- [ ] **Step 3: Replace the body of `stop_dictation`**

Delete the existing `stop_dictation` async function entirely and replace with the version below. Compared to earlier drafts:
- The `current_job` reservation is **atomic** — held across the entire setup so two concurrent stops cannot both pass the gate.
- The live worker is **joined before** reading `committed`/`drain_buffer`, eliminating the audio-loss race (P0 from review).
- `last_full_text` seeds the worker as `committed_text` so short dictations don't re-transcribe everything from scratch (P1 from review).
- `catch_unwind` wraps the worker body so a panic still clears `current_job` and emits `transcription://error`.
- `current_job` is cleared **before** `finish_job` emits its terminal event so a fast frontend handler can immediately start a new job.

```rust
use std::panic::AssertUnwindSafe;

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

    // Hold the dictation lock across the entire reservation so two concurrent
    // stop_dictation calls cannot both pass the "no job in progress" check.
    let mut dictation_guard = state.dictation.lock().map_err(|e| e.to_string())?;
    let session = dictation_guard
        .as_mut()
        .ok_or("No dictation in progress")?;

    // Atomic reservation: hold current_job lock across the check-and-set.
    let mut job_guard = state.current_job.lock().map_err(|e| e.to_string())?;
    if job_guard.is_some() {
        return Err("Another transcription is in progress".to_string());
    }

    // Join the live transcription loop. After this returns, the live worker
    // has put the cached transcriber back and the audio buffer is fully
    // drained into the loop's local accumulator (none stuck in flight).
    session.stop_and_join();

    let samples = session.drain_buffer();
    let last_full = session
        .last_full_text()
        .lock()
        .map(|s| s.clone())
        .unwrap_or_default();
    // Use the live loop's most recent fullText as the committed prefix.
    // Fallback to joined committed segments for the rollover case.
    let committed_prefix = if !last_full.trim().is_empty() {
        last_full.trim().to_string()
    } else {
        session
            .committed()
            .lock()
            .map(|c| c.join(" ").trim().to_string())
            .unwrap_or_default()
    };

    // Take the session out so the next start_dictation works fresh.
    *dictation_guard = None;
    drop(dictation_guard);

    // Insert partial row up front. If the user force-kills before the first
    // segment callback fires, this row will be swept on next launch by
    // delete_empty_partials (Task 5).
    let id = {
        let store = state.store.lock().map_err(|e| e.to_string())?;
        let new_id = store.insert_partial(&title, &language)?;
        if !committed_prefix.is_empty() {
            store.update_text(new_id, &committed_prefix, duration_secs)?;
        }
        new_id
    };

    let mut job = TranscriptionJob::new(id, JobKind::Dictation);
    job.committed_text = committed_prefix;
    let job_id = job.id;

    *job_guard = Some(job.clone());
    drop(job_guard);

    let store_for_worker = state.store.clone();
    let app = app_handle.clone();
    // After stop_and_join, the cache holds the live worker's transcriber.
    let transcriber_taken = state.transcriber.lock().map_err(|e| e.to_string())?.take();
    let model_path = crate::model::model_path(&state.data_dir);

    std::thread::spawn(move || {
        let transcriber = match transcriber_taken {
            Some(t) => t,
            None => match Transcriber::new(&model_path) {
                Ok(t) => t,
                Err(e) => {
                    clear_current_job_and_emit_error(&app, job_id, e);
                    return;
                }
            },
        };

        // Wrap the finalize body in catch_unwind so a panic still leaves
        // the system recoverable (current_job cleared, transcriber not lost
        // forever, error event fired). Frontend will see error and unblock.
        let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
            let outcome = run_finalize_dictation(
                &job,
                &transcriber,
                samples,
                duration_secs,
                language,
                store_for_worker.clone(),
                app.clone(),
            );
            (job, outcome)
        }));

        let (job, outcome) = match result {
            Ok(t) => t,
            Err(_panic) => {
                clear_current_job_and_emit_error(
                    &app,
                    job_id,
                    "Transcription worker panicked".to_string(),
                );
                // Restore transcriber so subsequent jobs work.
                if let Some(s) = app.try_state::<AppState>() {
                    let _ = s.transcriber.lock().map(|mut g| *g = Some(transcriber));
                }
                return;
            }
        };

        // Clear current_job and restore transcriber BEFORE finish_job emits
        // the terminal event. Otherwise a fast frontend handler reacting to
        // complete/cancelled may issue a new command and see a stale
        // "Another transcription is in progress" rejection.
        if let Some(s) = app.try_state::<AppState>() {
            let _ = s.transcriber.lock().map(|mut g| *g = Some(transcriber));
            let _ = s.current_job.lock().map(|mut g| *g = None);
        }

        finish_job(job, outcome, store_for_worker, app);
    });

    Ok(id)
}

fn clear_current_job_and_emit_error(
    app: &tauri::AppHandle,
    job_id: i64,
    error: String,
) {
    if let Some(s) = app.try_state::<AppState>() {
        let _ = s.current_job.lock().map(|mut g| *g = None);
    }
    let _ = app.emit(
        "transcription://error",
        crate::transcribe::job::ErrorPayload {
            id: job_id,
            error,
        },
    );
}
```

`ErrorPayload` is the typed struct defined in Task 8; expose it as `pub` in `job.rs` and re-import here so we don't have two payload shapes for the same event. (Earlier drafts used `serde_json::json!({...})` for the early-error path and the typed struct elsewhere — review flagged this as a divergence risk.)

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

Mirrors `stop_dictation`'s shape: atomic `current_job` reservation, `catch_unwind` around the worker, `current_job` cleared before `finish_job` emits, typed `ErrorPayload`, `try_state` instead of `state`.

Add this command to `src-tauri/src/lib.rs`:

```rust
use std::panic::AssertUnwindSafe;

#[tauri::command]
async fn transcribe_pending_recording(
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
    pending_id: i64,
    title: String,
    language: String,
) -> Result<i64, String> {
    use crate::transcribe::job::{
        finish_job, run_finalize_pending_file, ErrorPayload, JobKind,
        TranscriptionJob,
    };

    // Atomic reservation: hold current_job lock across the check-and-set.
    let mut job_guard = state.current_job.lock().map_err(|e| e.to_string())?;
    if job_guard.is_some() {
        return Err("Another transcription is in progress".to_string());
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

    *job_guard = Some(job.clone());
    drop(job_guard);

    let store_for_worker = state.store.clone();
    let app = app_handle.clone();
    let transcriber_taken = state.transcriber.lock().map_err(|e| e.to_string())?.take();
    let model_path = crate::model::model_path(&state.data_dir);
    let language_owned = language.clone();
    let job_id = id;
    let wav_for_worker = wav_path.clone();

    std::thread::spawn(move || {
        let transcriber = match transcriber_taken {
            Some(t) => t,
            None => match Transcriber::new(&model_path) {
                Ok(t) => t,
                Err(e) => {
                    if let Some(s) = app.try_state::<AppState>() {
                        let _ = s.current_job.lock().map(|mut g| *g = None);
                    }
                    let _ = app.emit(
                        "transcription://error",
                        ErrorPayload { id: job_id, error: e },
                    );
                    return;
                }
            },
        };

        let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
            let outcome = run_finalize_pending_file(
                &job,
                &transcriber,
                &wav_for_worker,
                language_owned,
                store_for_worker.clone(),
                app.clone(),
            );
            (job, outcome)
        }));

        let (job, outcome) = match result {
            Ok(t) => t,
            Err(_panic) => {
                if let Some(s) = app.try_state::<AppState>() {
                    let _ = s.transcriber.lock().map(|mut g| *g = Some(transcriber));
                    let _ = s.current_job.lock().map(|mut g| *g = None);
                }
                let _ = app.emit(
                    "transcription://error",
                    ErrorPayload {
                        id: job_id,
                        error: "Transcription worker panicked".to_string(),
                    },
                );
                return;
            }
        };

        // Clear current_job and restore transcriber before finish_job emits.
        if let Some(s) = app.try_state::<AppState>() {
            let _ = s.transcriber.lock().map(|mut g| *g = Some(transcriber));
            let _ = s.current_job.lock().map(|mut g| *g = None);
        }

        finish_job(job, outcome, store_for_worker, app);
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

### Task 14: Add `cancel_job` command

**Files:**
- Modify: `src-tauri/src/lib.rs`

The earlier draft also added `current_job_status` for "UI re-sync if the page is reloaded mid-job," but no frontend code calls it. Drop it as dead IPC surface; reintroduce when there is a real caller.

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

The actual cleanup (delete row, leave pending WAV+row alone) happens in the worker thread once `transcribe_with_callbacks` returns `Err` from the abort flag and `finish_job` runs the `Cancelled` arm. The worker also clears `current_job` itself before `finish_job` emits — see Task 12 Step 3.

- [ ] **Step 2: Register the command in `tauri::generate_handler!`**

- [ ] **Step 3: Compile**

Run: `cd src-tauri && cargo check`
Expected: compiles.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/lib.rs
git commit -m "feat(commands): cancel_job"
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

Cancel semantic is **discard everything** (matches the modal copy), so no `keepPartial`/`discardPartial` strings — they were dead code in earlier drafts. We also add `loadingModel` and `recordedDuration` for the indeterminate-progress state and the orientation context shown above the progress ring.

Append inside the `pt` block (before the closing brace):

```js
    finalizing: "Finalizando transcrição…",
    finalizingHint: "Pode levar alguns minutos numa máquina lenta.",
    loadingModel: "Carregando modelo…",
    recordedDuration: "Gravação:",
    cancelTranscription: "Cancelar",
    cancelConfirmTitle: "Cancelar transcrição?",
    cancelConfirmBody: "O conteúdo será descartado.",
    cancelConfirmYes: "Sim, cancelar",
    cancelConfirmNo: "Voltar",
    navLockedTooltip: "Aguarde a transcrição terminar",
    partialBadge: "Parcial",
```

Append inside the `en` block (before the closing brace):

```js
    finalizing: "Finalizing transcription…",
    finalizingHint: "May take a few minutes on a slow machine.",
    loadingModel: "Loading model…",
    recordedDuration: "Recorded:",
    cancelTranscription: "Cancel",
    cancelConfirmTitle: "Cancel transcription?",
    cancelConfirmBody: "The content will be discarded.",
    cancelConfirmYes: "Yes, cancel",
    cancelConfirmNo: "Back",
    navLockedTooltip: "Wait for the transcription to finish",
    partialBadge: "Partial",
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

### Task 18: Wire `appBusy` into the nav buttons (Record + Dictate only)

**Files:**
- Modify: `src/routes/+page.svelte`

History stays navigable during finalize. It is read-only; locking it forces the user to sit and stare at a progress ring during the multi-minute window when they most want to verify the app is alive. The single-job invariant is enforced by the backend, not the nav lock.

We use `aria-disabled` instead of the `disabled` attribute. Disabled HTML buttons drop out of the tab order and suppress hover events, which makes the tooltip unreliable in webview environments. With `aria-disabled` the element stays focusable, screen readers announce the disabled state, and CSS `cursor: not-allowed` still works.

- [ ] **Step 1: Import the store**

```js
import { appBusy } from "../lib/appBusy.js";
```

- [ ] **Step 2: Apply `aria-disabled` and click guards to Record + Dictate**

```svelte
<button
    class:active={currentView === "recorder"}
    onclick={() => $appBusy ? null : showRecorder()}
    aria-disabled={$appBusy}
    title={$appBusy ? t("navLockedTooltip") : ""}
>
    {t("record")}
</button>
<button
    class:active={currentView === "dictation"}
    onclick={() => $appBusy ? null : showDictation()}
    aria-disabled={$appBusy}
    title={$appBusy ? t("navLockedTooltip") : ""}
>
    {t("dictation")}
</button>
<button
    class:active={currentView === "history"}
    onclick={showHistory}
>
    {t("history")}
</button>
```

- [ ] **Step 3: Add `aria-disabled` styling**

```css
nav button[aria-disabled="true"] {
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
git commit -m "feat(ui): lock Record and Dictate during finalize; keep History free"
```

---

### Task 19: Build `<FinalizingProgress>` component

**Files:**
- Create: `src/lib/FinalizingProgress.svelte`

- [ ] **Step 1: Create the file**

```svelte
<script>
    import { t } from "./i18n.js";
    import { tick } from "svelte";

    let {
        percent = 0,
        liveText = "",
        cancelling = false,
        jobLabel = "",
        onCancel,
    } = $props();
    let confirming = $state(false);
    let liveTextEl;
    let dialogEl;
    let lastFocused = null;

    const SIZE = 96;
    const STROKE = 8;
    const RADIUS = (SIZE - STROKE) / 2;
    const CIRCUMFERENCE = 2 * Math.PI * RADIUS;

    let dashOffset = $derived(
        CIRCUMFERENCE * (1 - Math.max(0, Math.min(100, percent)) / 100),
    );

    // Whisper's progress callback only starts firing once inference begins.
    // Before that — model load, audio decode, first segment — `percent`
    // sits at 0. Show an indeterminate label so the user does not read
    // a frozen 0% as "stuck."
    let isIndeterminate = $derived(percent === 0 && !cancelling);

    // Auto-scroll the live-text pane to the latest segment as it grows.
    $effect(() => {
        if (liveText && liveTextEl) {
            liveTextEl.scrollTop = liveTextEl.scrollHeight;
        }
    });

    async function requestCancel() {
        if (cancelling) return;
        lastFocused = document.activeElement;
        confirming = true;
        await tick();
        // Focus the safe default ("Voltar"), trap with Escape and Tab.
        dialogEl?.querySelector(".btn-secondary")?.focus();
    }

    function dismissCancel() {
        confirming = false;
        lastFocused?.focus?.();
    }

    function confirmCancel() {
        confirming = false;
        lastFocused?.focus?.();
        onCancel?.();
    }

    function handleDialogKey(e) {
        if (!confirming) return;
        if (e.key === "Escape") {
            e.preventDefault();
            dismissCancel();
            return;
        }
        if (e.key === "Tab") {
            const focusables = dialogEl?.querySelectorAll("button");
            if (!focusables || focusables.length === 0) return;
            const first = focusables[0];
            const last = focusables[focusables.length - 1];
            if (e.shiftKey && document.activeElement === first) {
                e.preventDefault();
                last.focus();
            } else if (!e.shiftKey && document.activeElement === last) {
                e.preventDefault();
                first.focus();
            }
        }
    }
</script>

<svelte:window on:keydown={handleDialogKey} />

<div class="finalizing">
    {#if jobLabel}
        <span class="job-label">{jobLabel}</span>
    {/if}

    <div class="ring-wrap" class:indeterminate={isIndeterminate}>
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
                stroke-dasharray={isIndeterminate ? "20 200" : CIRCUMFERENCE}
                stroke-dashoffset={isIndeterminate ? 0 : dashOffset}
                stroke-linecap="round"
                transform="rotate(-90 {SIZE / 2} {SIZE / 2})"
                style="transition: stroke-dashoffset 200ms linear;"
            />
        </svg>
        {#if !isIndeterminate}
            <span class="percent">{Math.round(percent)}%</span>
        {/if}
    </div>

    <div class="status">
        <strong>{t("finalizing")}</strong>
        <span class="hint">
            {isIndeterminate ? t("loadingModel") : t("finalizingHint")}
        </span>
    </div>

    {#if liveText}
        <div class="live-text">
            <pre bind:this={liveTextEl}>{liveText}</pre>
        </div>
    {/if}

    <button
        class="btn-cancel"
        onclick={requestCancel}
        disabled={cancelling}
        aria-disabled={cancelling}
    >
        {t("cancelTranscription")}
    </button>
</div>

{#if confirming}
    <div class="modal-backdrop">
        <div
            class="modal"
            role="dialog"
            aria-modal="true"
            aria-labelledby="cancel-title"
            aria-describedby="cancel-body"
            bind:this={dialogEl}
        >
            <h3 id="cancel-title">{t("cancelConfirmTitle")}</h3>
            <p id="cancel-body">{t("cancelConfirmBody")}</p>
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

    .job-label {
        font-size: 0.9rem;
        color: var(--text-muted);
    }

    .ring-wrap {
        position: relative;
        width: 96px;
        height: 96px;
    }

    .ring-wrap.indeterminate svg {
        animation: ring-spin 1.4s linear infinite;
    }

    @keyframes ring-spin {
        from { transform: rotate(0deg); }
        to { transform: rotate(360deg); }
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

    .btn-cancel:hover:not([disabled]) {
        color: var(--accent);
        border-color: var(--accent);
    }

    .btn-cancel[disabled] {
        opacity: 0.5;
        cursor: wait;
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

Component contract:
- **`percent` 0–100**: drives the ring fill. While `percent === 0` and not cancelling, the component renders an indeterminate spinning arc + "Loading model…" hint instead of a frozen 0%.
- **`liveText`**: optional progressive transcript. Auto-scrolls to the bottom as it grows.
- **`cancelling`**: when true, the cancel button is disabled (and visibly waiting). Set this from the parent when `phase === "cancelling"`.
- **`jobLabel`**: optional context line shown above the ring (e.g. "Recorded: 2m 14s" or the prefilled title) so the user remembers what they're waiting on.
- **`onCancel`**: fires after the user confirms in the modal.
- **Modal a11y**: `role="dialog"`, `aria-modal`, `aria-labelledby`, `aria-describedby`, focus trap on Tab/Shift-Tab, Escape dismisses, focus returns to the cancel button on close.

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

No id-filter on the event listeners: the backend enforces a single-job-at-a-time invariant (atomic `current_job` reservation in Tasks 12/13), so the frontend can correlate events to "the current job" via `phase`. This also closes the listener-race window where progress events arrived before the awaited invoke returned the new id.

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
    let recordedDurationLabel = $state("");
    let timer = null;

    let unlisteners = [];

    function isFinalizing() {
        return phase === "finalizing" || phase === "cancelling";
    }

    onMount(async () => {
        unlisteners.push(
            // Live-during-recording feedback. The Rust dictation loop keeps
            // emitting `dictation://segment` while `phase === "recording"`;
            // once we transition to `finalizing`, the new
            // `transcription://text` events take over.
            await listen("dictation://segment", (event) => {
                if (phase !== "recording") return;
                liveText = event.payload.fullText;
            }),
            await listen("transcription://text", (event) => {
                if (!isFinalizing()) return;
                liveText = event.payload.text;
            }),
            await listen("transcription://progress", (event) => {
                if (!isFinalizing()) return;
                percent = event.payload.percent;
            }),
            await listen("transcription://complete", (event) => {
                if (!isFinalizing()) return;
                percent = 100;
                // Brief moment showing 100% before disappearing.
                setTimeout(() => {
                    phase = "idle";
                    appBusy.set(false);
                    liveText = "";
                    percent = 0;
                    recordedDurationLabel = "";
                }, 250);
                onTranscribed?.(event.payload.transcription);
            }),
            await listen("transcription://cancelled", (event) => {
                if (!isFinalizing()) return;
                phase = "idle";
                appBusy.set(false);
                liveText = "";
                percent = 0;
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
            const now = new Date().toLocaleString(
                locale === "pt" ? "pt-BR" : "en-US",
            );
            // Bridge: show the recording duration above the ring so the
            // user keeps context for what they were dictating.
            recordedDurationLabel = `${t("recordedDuration")} ${formatTime(elapsed)}`;
            phase = "finalizing";
            appBusy.set(true);
            await invoke("stop_dictation", {
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
        <FinalizingProgress
            {percent}
            {liveText}
            cancelling={phase === "cancelling"}
            jobLabel={recordedDurationLabel}
            onCancel={requestCancel}
        />
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

- [ ] **Step 3: Verify the `dictation://segment` emit is unchanged in Rust**

The Step 1 onMount block already includes the `dictation://segment` listener (live-during-recording feedback). Confirm the Rust side still emits it with payload shape `{ fullText, ... }`:

Run: `grep -rn "dictation://" src-tauri/`
Expected: emit in `src-tauri/src/dictation.rs` is preserved (Task 12 changes the stop path but keeps the loop's mid-recording emit).

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

The id-filter on listeners is dropped — backend enforces single-job-at-a-time, so `phase` alone is enough to correlate events. We track the `startedFromPendingId` separately so we can remove the right pending-recording row from the list on completion.

Replace `let transcribingId = $state(null);` with:

```js
let liveText = $state("");
let percent = $state(0);
let phase = $state("idle"); // "idle" | "finalizing" | "cancelling"
let pendingDurationLabel = $state("");
let startedFromPendingId = null;
let unlisteners = [];

function isFinalizing() {
    return phase === "finalizing" || phase === "cancelling";
}
```

Then add to `onMount`:

```js
unlisteners.push(
    await listen("transcription://text", (event) => {
        if (!isFinalizing()) return;
        liveText = event.payload.text;
    }),
    await listen("transcription://progress", (event) => {
        if (!isFinalizing()) return;
        percent = event.payload.percent;
    }),
    await listen("transcription://complete", (event) => {
        if (!isFinalizing()) return;
        const t_ = event.payload.transcription;
        if (startedFromPendingId !== null) {
            pendingRecordings = pendingRecordings.filter(
                (p) => p.id !== startedFromPendingId,
            );
        }
        percent = 100;
        setTimeout(() => {
            phase = "idle";
            appBusy.set(false);
            liveText = "";
            percent = 0;
            pendingDurationLabel = "";
            startedFromPendingId = null;
        }, 250);
        onTranscribed?.(t_);
    }),
    await listen("transcription://cancelled", (event) => {
        if (!isFinalizing()) return;
        phase = "idle";
        appBusy.set(false);
        liveText = "";
        percent = 0;
        pendingDurationLabel = "";
        startedFromPendingId = null;
    }),
    await listen("transcription://error", (event) => {
        if (!isFinalizing()) return;
        error = event.payload.error;
        phase = "idle";
        appBusy.set(false);
        pendingDurationLabel = "";
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
        const pending = pendingRecordings.find((p) => p.id === id);
        startedFromPendingId = id;
        if (pending && typeof pending.duration_secs === "number") {
            pendingDurationLabel = `${t("recordedDuration")} ${formatDuration(pending.duration_secs)}`;
        }
        phase = "finalizing";
        appBusy.set(true);
        liveText = "";
        percent = 0;
        const now = new Date().toLocaleString(
            locale === "pt" ? "pt-BR" : "en-US",
        );
        await invoke("transcribe_pending_recording", {
            pendingId: id,
            title: `${t("meetingTitle")} ${now}`,
            language: locale,
        });
    } catch (e) {
        error = e;
        phase = "idle";
        appBusy.set(false);
        startedFromPendingId = null;
        pendingDurationLabel = "";
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

function formatDuration(secs) {
    const m = Math.floor(secs / 60).toString().padStart(2, "0");
    const s = Math.floor(secs % 60).toString().padStart(2, "0");
    return `${m}:${s}`;
}
```

- [ ] **Step 4: Add finalizing UI to the markup**

In the markup block, gate the pending-list block behind `!isFinalizing()`:

```svelte
{#if phase === "finalizing" || phase === "cancelling"}
    <FinalizingProgress
        {percent}
        {liveText}
        cancelling={phase === "cancelling"}
        jobLabel={pendingDurationLabel}
        onCancel={requestCancel}
    />
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

The badge is purely informational — clicking it does nothing different from clicking the row, and there is no recovery action attached. Drop the warning glyph (which implies actionability) and use a darker amber for WCAG AA contrast on the tinted background.

- [ ] **Step 1: Render the badge in the title block**

In the `<li>` markup, inside the `<span class="title">` block, after the existing `{#if item.summary}` block, add:

```svelte
{#if item.status === "partial"}
    <span class="partial-badge">{t("partialBadge")}</span>
{/if}
```

- [ ] **Step 2: Add styling for the badge**

`#7d5a00` on `rgba(255,193,7,0.18)` over the surface variable yields ~5:1 contrast, satisfying WCAG AA at small font sizes.

```css
.partial-badge {
    display: inline-block;
    background: rgba(255, 193, 7, 0.18);
    color: #7d5a00;
    font-size: 0.7rem;
    font-weight: 600;
    padding: 2px 8px;
    border-radius: 10px;
    margin-left: 6px;
    vertical-align: middle;
}

@media (prefers-color-scheme: dark) {
    .partial-badge {
        color: #ffd54f;
    }
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

### Task 24: Smoke test, lint, and PR

This collapses what was Tasks 24–27. For a single-maintainer privacy-first desktop tool, a separate committed test-report file plus a separate release-notes file is more ceremony than the project earns. The PR description carries the same information without polluting the repo.

- [ ] **Step 1: Re-run all checks**

```bash
cd src-tauri && cargo fmt && cargo clippy -- -D warnings && cargo test --lib
cd .. && npm run check
```

Expected: all green.

- [ ] **Step 2: Run `cargo tauri dev` and walk the scenarios**

Open the app and run through the scenarios below. Note observed wall-clock times in the PR body.

| # | Scenario | Pass criteria |
|---|---|---|
| A | Short dictation, ~10s | FinalizingProgress shows, % advances, row saves with `status='complete'` |
| B | Long dictation, ~60s | live-text updates during recording; stop → ring; nav: Record+Dictate locked, History remains clickable; saves complete |
| C | Stop within 1s | no error path; row may have empty text but ends `status='complete'` (or no row if whisper produced nothing — startup sweep would catch a stranded partial) |
| D | Cancel mid-finalize | confirmation modal opens, focus on "Voltar"; Sim cancelar → cancelled event arrives, row deleted, returns to idle |
| E | Pending-file: record 15s → Transcrever | FinalizingProgress immediate; on complete: pending row deleted, WAV deleted, transcription in history |
| F | Pending-file cancel | row remains, WAV remains, click Transcrever again → finalize succeeds |
| G | Start a job while one is running | Record + Dictate aria-disabled; clicking does nothing; History still navigable; programmatic invoke returns "Another transcription is in progress" |
| H | Force-kill mid-finalize (`pkill -9 martin`) | reopen → row with the partial badge in History; if the kill happened before any segment fired, no ghost row (swept by `delete_empty_partials`) |
| I | Worker panic / model file missing | error event shows; nav unlocks; subsequent jobs work |

DB inspection helper (read-only):
```bash
sqlite3 ~/.local/share/com.nuuvem.martin/martin.db \
  "SELECT id, status, substr(text,1,40), duration_secs FROM transcriptions ORDER BY id DESC LIMIT 5"
```

- [ ] **Step 3: Push and open the PR**

```bash
git push -u origin feat/finalizing-flow

gh pr create --title "feat: transcription finalizing flow (v0.2.0)" --body "$(cat <<'EOF'
## Summary
- Fixes the dictation save bug where text was lost on slow machines.
- Adds an explicit finalizing phase with circular progress + live text + cancel.
- Locks Record/Dictate during finalize; History stays navigable.
- Surfaces force-kill recovery as `'partial'` rows in History; sweeps empty partials on startup.
- Bumps to v0.2.0.

## Spec / Plan
- docs/specs/2026-05-03-transcription-finalizing-flow-design.md
- docs/specs/2026-05-03-transcription-finalizing-flow-plan.md

## Schema migration
- `transcriptions` gains a `status` column. ALTER TABLE on launch; existing rows backfill to `'complete'` via the column DEFAULT. SQLite WAL + `synchronous=NORMAL` enabled.

## Internal architecture
- New module `src-tauri/src/transcribe/job.rs`: `TranscriptionJob`, `JobKind`, `FinalizeOutcome`, `run_finalize_dictation`, `run_finalize_pending_file`, `finish_job`.
- `DictationSession` now owns a `JoinHandle` for its transcription loop; `stop_dictation` joins it before draining the audio buffer.
- New events on `transcription://*`: `text`, `progress`, `complete`, `cancelled`, `error`.
- IPC: removed `transcribe_recording`; added `transcribe_pending_recording`, `cancel_job`. `stop_dictation` no longer accepts `full_text`.

## Smoke test results
| Scenario | Result | Notes |
|---|---|---|
| A short dictation | … | … |
| … | … | … |

## Known limitations carried forward
- Dictation audio is in RAM during recording; a crash before Stop loses it. Tracked separately.
- CPU-only whisper on slower machines can take minutes to finalize. The new progress UI makes this visible; an OpenBLAS/Vulkan build is a follow-up.
EOF
)"
```

---

## Self-review checklist (run after writing the plan, before execution)

The plan author has already run this once. The executing engineer should re-verify:

- [ ] Every spec section maps to at least one task. (See File map above.)
- [ ] No "TBD" or "implement later" lines in any step.
- [ ] Type and method names used in later tasks match what was defined earlier (`Transcription.status`, `Store::insert_partial`, `Store::update_text`, `Store::mark_complete`, `Store::delete_empty_partials`, `Transcriber::transcribe_with_callbacks`, `TranscriptionJob`, `JobKind`, `FinalizeOutcome`, `ErrorPayload`, `run_finalize_dictation`, `run_finalize_pending_file`, `finish_job`, `cancel_job`, `transcribe_pending_recording`, `appBusy`, `FinalizingProgress`).
- [ ] Frontend listens to all five `transcription://*` events that the backend emits, plus `dictation://segment` for live-during-recording feedback.
- [ ] Cancel cleanup: dictation deletes DB row; pending-file deletes DB row only and leaves WAV + `pending_recordings` row intact so retry works.
- [ ] Migration is idempotent and backfills existing rows to `'complete'` (Task 1 has tests for both).
- [ ] `current_job` is cleared and `transcriber` is restored to the cache **before** `finish_job` emits its terminal event.
- [ ] Worker bodies are wrapped in `catch_unwind`; on panic, `current_job` is cleared and `transcription://error` is emitted with the typed `ErrorPayload`.
- [ ] DictationSession's `JoinHandle` is joined before the audio buffer is drained.
- [ ] `state.transcriber.lock().take()` only happens AFTER joining the live dictation worker (so the cache holds the transcriber, no double-load).
- [ ] No frontend listener filters by job id — single-job invariant is enforced backend-side.
- [ ] No `keepPartial`/`discardPartial`/`current_job_status` references anywhere (dropped during review).
- [ ] No `'failed'` status value referenced anywhere — vocabulary is `'complete'` and `'partial'` only.
