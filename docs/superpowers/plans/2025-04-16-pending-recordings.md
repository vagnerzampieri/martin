# Pending Recordings Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Track recordings in the database so they survive app restarts and can be transcribed or deleted from a list on the recorder screen.

**Architecture:** New `pending_recordings` table stores file path + metadata after each recording stops. The recorder screen loads and displays these. Transcribing a pending recording follows the existing Whisper pipeline, then cleans up both the WAV and the DB row. File names use timestamps to avoid overwriting.

**Tech Stack:** Rust (SQLite via rusqlite), Svelte 5 (runes), Tauri 2 commands

**Spec:** `docs/specs/2025-04-16-pending-recordings-design.md`

---

## File Structure

| File | Action | Responsibility |
|------|--------|----------------|
| `src-tauri/src/db/store.rs` | Modify | Add `PendingRecording` struct, `pending_recordings` table, CRUD methods |
| `src-tauri/src/lib.rs` | Modify | Change `stop_recording` return type, update `transcribe_recording` to take `pending_id`, add new commands |
| `src-tauri/src/audio/capture.rs` | Modify | Change `AudioCapture::new` to accept timestamped path |
| `src/lib/Recorder.svelte` | Modify | Load/display pending list, remove standalone transcribe button |
| `src/lib/i18n.js` | Modify | Add i18n keys for pending recordings UI |

---

### Task 1: PendingRecording struct and table migration

**Files:**
- Modify: `src-tauri/src/db/store.rs`

- [ ] **Step 1: Write failing test for pending_recordings table creation**

In the `#[cfg(test)] mod tests` block in `store.rs`, add:

```rust
#[test]
fn save_pending_inserts_and_returns_valid_id() {
    let (store, _temp_file) = create_temp_store();

    let id = store
        .save_pending("/tmp/recording_123.wav", 120.5)
        .expect("Failed to save pending");

    assert!(id > 0);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test save_pending_inserts -v`
Expected: FAIL — `save_pending` method not found

- [ ] **Step 3: Add PendingRecording struct and table creation**

At the top of `store.rs`, after the `Transcription` struct, add:

```rust
#[derive(Debug, Serialize, Clone)]
pub struct PendingRecording {
    pub id: i64,
    pub file_path: String,
    pub duration_secs: f64,
    pub created_at: String,
}
```

In `Store::new`, after the existing `CREATE TABLE IF NOT EXISTS transcriptions` block, add a second `execute_batch`:

```rust
conn.execute_batch(
    "CREATE TABLE IF NOT EXISTS pending_recordings (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        file_path TEXT NOT NULL,
        duration_secs REAL NOT NULL DEFAULT 0.0,
        created_at TEXT NOT NULL DEFAULT (datetime('now', 'localtime'))
    );",
)
.map_err(|e| format!("Failed to create pending_recordings table: {}", e))?;
```

Add the `save_pending` method to `impl Store`:

```rust
pub fn save_pending(&self, file_path: &str, duration_secs: f64) -> Result<i64, String> {
    self.conn
        .execute(
            "INSERT INTO pending_recordings (file_path, duration_secs) VALUES (?1, ?2)",
            params![file_path, duration_secs],
        )
        .map_err(|e| format!("Failed to save pending recording: {}", e))?;

    Ok(self.conn.last_insert_rowid())
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test save_pending_inserts -v`
Expected: PASS

- [ ] **Step 5: Commit**

```
feat: add pending_recordings table and save_pending method
```

---

### Task 2: Remaining store methods (get_pending, list_pending, delete_pending)

**Files:**
- Modify: `src-tauri/src/db/store.rs`

- [ ] **Step 1: Write failing tests**

```rust
#[test]
fn get_pending_retrieves_saved_record() {
    let (store, _temp_file) = create_temp_store();

    let id = store
        .save_pending("/tmp/rec.wav", 60.0)
        .expect("Failed to save");

    let pending = store.get_pending(id).expect("Failed to get");

    assert_eq!(pending.id, id);
    assert_eq!(pending.file_path, "/tmp/rec.wav");
    assert_eq!(pending.duration_secs, 60.0);
    assert!(!pending.created_at.is_empty());
}

#[test]
fn list_pending_returns_records_ordered_by_created_at_desc() {
    let (store, _temp_file) = create_temp_store();

    store.conn.execute(
        "INSERT INTO pending_recordings (file_path, duration_secs, created_at) VALUES (?1, ?2, ?3)",
        params!["/tmp/first.wav", 10.0, "2025-01-01 10:00:00"],
    ).expect("Failed to insert");
    store.conn.execute(
        "INSERT INTO pending_recordings (file_path, duration_secs, created_at) VALUES (?1, ?2, ?3)",
        params!["/tmp/second.wav", 20.0, "2025-01-01 11:00:00"],
    ).expect("Failed to insert");

    let records = store.list_pending().expect("Failed to list");

    assert_eq!(records.len(), 2);
    assert_eq!(records[0].file_path, "/tmp/second.wav");
    assert_eq!(records[1].file_path, "/tmp/first.wav");
}

#[test]
fn list_pending_returns_empty_when_no_records() {
    let (store, _temp_file) = create_temp_store();

    let records = store.list_pending().expect("Failed to list");

    assert!(records.is_empty());
}

#[test]
fn delete_pending_removes_record() {
    let (store, _temp_file) = create_temp_store();

    let id = store
        .save_pending("/tmp/rec.wav", 30.0)
        .expect("Failed to save");

    store.delete_pending(id).expect("Failed to delete");

    let result = store.get_pending(id);
    assert!(result.is_err());
}

#[test]
fn delete_pending_returns_error_for_nonexistent_id() {
    let (store, _temp_file) = create_temp_store();

    let result = store.delete_pending(999);

    assert!(result.is_err());
    assert!(result.unwrap_err().contains("not found"));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test pending -v`
Expected: FAIL — methods not found

- [ ] **Step 3: Implement get_pending, list_pending, delete_pending**

Add to `impl Store`:

```rust
pub fn get_pending(&self, id: i64) -> Result<PendingRecording, String> {
    self.conn
        .query_row(
            "SELECT id, file_path, duration_secs, created_at FROM pending_recordings WHERE id = ?1",
            params![id],
            |row| {
                Ok(PendingRecording {
                    id: row.get(0)?,
                    file_path: row.get(1)?,
                    duration_secs: row.get(2)?,
                    created_at: row.get(3)?,
                })
            },
        )
        .map_err(|e| format!("Pending recording not found: {}", e))
}

pub fn list_pending(&self) -> Result<Vec<PendingRecording>, String> {
    let mut stmt = self
        .conn
        .prepare(
            "SELECT id, file_path, duration_secs, created_at FROM pending_recordings ORDER BY created_at DESC",
        )
        .map_err(|e| format!("Failed to prepare query: {}", e))?;

    let rows = stmt
        .query_map([], |row| {
            Ok(PendingRecording {
                id: row.get(0)?,
                file_path: row.get(1)?,
                duration_secs: row.get(2)?,
                created_at: row.get(3)?,
            })
        })
        .map_err(|e| format!("Failed to query: {}", e))?;

    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("Failed to read row: {}", e))
}

pub fn delete_pending(&self, id: i64) -> Result<(), String> {
    let affected = self
        .conn
        .execute(
            "DELETE FROM pending_recordings WHERE id = ?1",
            params![id],
        )
        .map_err(|e| format!("Failed to delete pending recording: {}", e))?;

    if affected == 0 {
        return Err(format!("Pending recording with id {} not found", id));
    }
    Ok(())
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test pending -v`
Expected: all 6 pending tests PASS

- [ ] **Step 5: Commit**

```
feat: add list, get, delete methods for pending recordings
```

---

### Task 3: Timestamped file names in AudioCapture

**Files:**
- Modify: `src-tauri/src/audio/capture.rs`
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: Change AudioCapture::new to take the full output path**

`AudioCapture::new` already takes a `PathBuf` for the output. The change is in `lib.rs` where the path is constructed. In `start_recording`, replace:

```rust
let audio_path = state.audio_path();
```

with:

```rust
let timestamp = std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH)
    .unwrap()
    .as_millis();
let audio_path = state.data_dir.join(format!("recording_{}.wav", timestamp));
```

Remove the `audio_path()` helper from `impl AppState` since it's no longer used with a fixed name.

- [ ] **Step 2: Store the audio_path in AppState for stop_recording to access**

Add a field to `AppState`:

```rust
pub struct AppState {
    capture: Mutex<SendableCapture>,
    recording_path: Mutex<Option<PathBuf>>,
    store: Mutex<Store>,
    transcriber: Mutex<Option<Transcriber>>,
    model_path: PathBuf,
    data_dir: PathBuf,
}
```

Initialize it as `recording_path: Mutex::new(None)` in `run()`.

In `start_recording`, after creating the capture, store the path:

```rust
*state.recording_path.lock().map_err(|e| e.to_string())? = Some(audio_path);
```

- [ ] **Step 3: Run tests to verify nothing broke**

Run: `cargo test`
Expected: all 29 tests PASS (no behavioral change yet)

- [ ] **Step 4: Commit**

```
refactor: use timestamped file names for recordings
```

---

### Task 4: Update stop_recording to return PendingRecording

**Files:**
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/src/db/store.rs` (import)

- [ ] **Step 1: Change stop_recording to save pending and return it**

```rust
#[tauri::command]
async fn stop_recording(state: State<'_, AppState>) -> Result<PendingRecording, String> {
    let stop_result = {
        let mut guard = state.capture.lock().map_err(|e| e.to_string())?;
        let mut capture = guard.0.take().ok_or("No active recording to stop")?;
        capture.stop_streams()?
    };

    let output_path = tauri::async_runtime::spawn_blocking(move || finalize_recording(stop_result))
        .await
        .map_err(|e| format!("Recording finalization failed: {}", e))??;

    let duration_secs = wav_duration_secs(&output_path)?;
    let file_path = output_path.to_string_lossy().to_string();

    let store = state.store.lock().map_err(|e| e.to_string())?;
    let id = store.save_pending(&file_path, duration_secs)?;
    store.get_pending(id)
}
```

Update the import at the top of `lib.rs`:

```rust
use db::store::{PendingRecording, Store, Transcription};
```

- [ ] **Step 2: Run tests**

Run: `cargo test`
Expected: all tests PASS

- [ ] **Step 3: Commit**

```
feat: stop_recording saves pending recording to database
```

---

### Task 5: Update transcribe_recording to work with pending_id

**Files:**
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: Change transcribe_recording signature and implementation**

Replace the current `transcribe_recording` with:

```rust
#[tauri::command]
async fn transcribe_recording(
    state: State<'_, AppState>,
    pending_id: i64,
    title: String,
    language: String,
) -> Result<Transcription, String> {
    let pending = {
        let store = state.store.lock().map_err(|e| e.to_string())?;
        store.get_pending(pending_id)?
    };

    let audio_path = std::path::PathBuf::from(&pending.file_path);
    if !audio_path.exists() {
        let store = state.store.lock().map_err(|e| e.to_string())?;
        let _ = store.delete_pending(pending_id);
        return Err("Recording file not found. It may have been deleted.".to_string());
    }

    let model_path = state.model_path.clone();
    let cached = state.transcriber.lock().map_err(|e| e.to_string())?.take();
    let audio = audio_path.clone();
    let lang = language.clone();

    let (text, duration_secs, transcriber) = tauri::async_runtime::spawn_blocking(
        move || -> Result<(String, f64, Transcriber), String> {
            let transcriber = get_or_create_transcriber(cached, &model_path)?;
            let text = transcriber.transcribe(&audio, &lang)?;
            let duration_secs = wav_duration_secs(&audio)?;
            Ok((text, duration_secs, transcriber))
        },
    )
    .await
    .map_err(|e| format!("Transcription task failed: {}", e))??;

    *state.transcriber.lock().map_err(|e| e.to_string())? = Some(transcriber);

    let store = state.store.lock().map_err(|e| e.to_string())?;
    let id = store.save(&title, &text, &language, duration_secs)?;

    if let Err(e) = std::fs::remove_file(&audio_path) {
        eprintln!("Warning: failed to delete recording file: {}", e);
    }

    store.delete_pending(pending_id)?;

    store.get(id)
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test`
Expected: all tests PASS

- [ ] **Step 3: Commit**

```
feat: transcribe_recording takes pending_id instead of fixed path
```

---

### Task 6: Add new Tauri commands (list_pending, delete_pending)

**Files:**
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: Add list and delete commands**

```rust
#[tauri::command]
fn list_pending_recordings(state: State<'_, AppState>) -> Result<Vec<PendingRecording>, String> {
    let store = state.store.lock().map_err(|e| e.to_string())?;
    store.list_pending()
}

#[tauri::command]
fn delete_pending_recording(state: State<'_, AppState>, id: i64) -> Result<(), String> {
    let file_path = {
        let store = state.store.lock().map_err(|e| e.to_string())?;
        let pending = store.get_pending(id)?;
        store.delete_pending(id)?;
        pending.file_path
    };

    if let Err(e) = std::fs::remove_file(&file_path) {
        eprintln!("Warning: failed to delete recording file: {}", e);
    }

    Ok(())
}
```

- [ ] **Step 2: Register commands in invoke_handler**

Add `list_pending_recordings` and `delete_pending_recording` to the `generate_handler!` macro:

```rust
.invoke_handler(tauri::generate_handler![
    start_recording,
    stop_recording,
    transcribe_recording,
    list_transcriptions,
    get_transcription,
    delete_transcription,
    check_claude_cli,
    summarize_transcription,
    list_pending_recordings,
    delete_pending_recording,
])
```

- [ ] **Step 3: Remove unused audio_path helper**

Delete the `impl AppState` block with `fn audio_path` if it's no longer used anywhere.

- [ ] **Step 4: Run tests and clippy**

Run: `cargo test && cargo clippy`
Expected: all tests PASS, no new warnings

- [ ] **Step 5: Commit**

```
feat: add list_pending_recordings and delete_pending_recording commands
```

---

### Task 7: Add i18n keys

**Files:**
- Modify: `src/lib/i18n.js`

- [ ] **Step 1: Add keys to both locales**

In `pt`:

```javascript
pendingRecordings: "Gravações pendentes",
noPending: "Nenhuma gravação pendente",
deleteRecording: "Excluir gravação",
```

In `en`:

```javascript
pendingRecordings: "Pending recordings",
noPending: "No pending recordings",
deleteRecording: "Delete recording",
```

- [ ] **Step 2: Commit**

```
feat: add i18n keys for pending recordings
```

---

### Task 8: Update Recorder.svelte with pending recordings list

**Files:**
- Modify: `src/lib/Recorder.svelte`

- [ ] **Step 1: Rewrite Recorder.svelte**

Replace the full `<script>` section:

```javascript
<script>
    import { invoke } from "@tauri-apps/api/core";
    import { onMount } from "svelte";
    import { t } from "./i18n.js";

    let { onTranscribed } = $props();

    let recording = $state(false);
    let processing = $state(false);
    let error = $state("");
    let elapsed = $state(0);
    let timer = null;

    let pendingRecordings = $state([]);
    let transcribingId = $state(null);

    onMount(loadPending);

    async function loadPending() {
        try {
            pendingRecordings = await invoke("list_pending_recordings");
        } catch (e) {
            console.error("Failed to load pending recordings:", e);
        }
    }

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
            recording = false;
            processing = true;
            const pending = await invoke("stop_recording");
            pendingRecordings = [pending, ...pendingRecordings];
        } catch (e) {
            error = e;
        } finally {
            processing = false;
        }
    }

    async function transcribePending(id) {
        try {
            error = "";
            transcribingId = id;
            const now = new Date().toLocaleString("pt-BR");
            const result = await invoke("transcribe_recording", {
                pendingId: id,
                title: `${t("meetingTitle")} ${now}`,
                language: "pt",
            });
            pendingRecordings = pendingRecordings.filter((p) => p.id !== id);
            onTranscribed?.(result);
        } catch (e) {
            error = e;
        } finally {
            transcribingId = null;
        }
    }

    async function deletePending(id) {
        try {
            await invoke("delete_pending_recording", { id });
            pendingRecordings = pendingRecordings.filter((p) => p.id !== id);
        } catch (e) {
            error = e;
        }
    }

    function formatTime(secs) {
        const m = Math.floor(secs / 60).toString().padStart(2, "0");
        const s = (secs % 60).toString().padStart(2, "0");
        return `${m}:${s}`;
    }

    function formatDuration(secs) {
        const m = Math.floor(secs / 60);
        const s = Math.round(secs % 60);
        return `${m}min ${s}s`;
    }

    function formatDate(dateStr) {
        return new Date(dateStr).toLocaleString("pt-BR");
    }
</script>
```

- [ ] **Step 2: Replace the template**

```svelte
<div class="recorder">
    {#if recording}
        <div class="status recording">
            <span class="dot"></span>
            {t("recording")} {formatTime(elapsed)}
        </div>
        <button class="btn-stop" onclick={stopRecording}>
            {t("stopRecording")}
        </button>
    {:else if processing}
        <div class="status processing">
            {t("processingAudio")}
        </div>
    {:else}
        <button class="btn-start" onclick={startRecording}>
            {t("startRecording")}
        </button>
    {/if}

    {#if error}
        <div class="error">{error}</div>
    {/if}

    {#if !recording && !processing && pendingRecordings.length > 0}
        <div class="pending">
            <h3>{t("pendingRecordings")}</h3>
            <ul>
                {#each pendingRecordings as pending}
                    <li>
                        <span class="pending-info">
                            {t("meetingTitle")} {formatDate(pending.created_at)} · {formatDuration(pending.duration_secs)}
                        </span>
                        <div class="pending-actions">
                            <button
                                class="btn-transcribe"
                                disabled={transcribingId === pending.id}
                                onclick={() => transcribePending(pending.id)}
                            >
                                {transcribingId === pending.id ? t("transcribing") : t("transcribe")}
                            </button>
                            <button
                                class="btn-delete"
                                disabled={transcribingId === pending.id}
                                onclick={() => deletePending(pending.id)}
                            >
                                ×
                            </button>
                        </div>
                    </li>
                {/each}
            </ul>
        </div>
    {/if}
</div>
```

- [ ] **Step 3: Update the styles**

Keep all existing styles and add:

```css
.pending {
    width: 100%;
    max-width: 500px;
    margin-top: 16px;
}

.pending h3 {
    font-size: 0.95rem;
    color: var(--text-muted);
    margin-bottom: 8px;
}

.pending ul {
    list-style: none;
}

.pending li {
    display: flex;
    align-items: center;
    justify-content: space-between;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    padding: 10px 12px;
    margin-bottom: 6px;
    background: var(--surface);
}

.pending-info {
    font-size: 0.9rem;
    color: var(--text);
}

.pending-actions {
    display: flex;
    gap: 6px;
    flex-shrink: 0;
}

.pending-actions .btn-transcribe {
    font-size: 0.8rem;
    padding: 6px 14px;
}

.pending-actions .btn-delete {
    background: transparent;
    color: var(--accent);
    padding: 6px 10px;
    font-size: 1.1rem;
}

.pending-actions .btn-delete:hover {
    background: rgba(233, 69, 96, 0.2);
}

.btn-transcribe:disabled {
    opacity: 0.5;
    cursor: not-allowed;
}
```

Remove the `.btn-transcribe` existing block (the old standalone one) since it's redefined in `.pending-actions`. Also remove the `{:else if transcribing}` template block since transcribing state is now per-item.

- [ ] **Step 4: Run cargo fmt, npm run check**

Run: `cargo fmt && npm run check`

- [ ] **Step 5: Commit**

```
feat: show pending recordings list on recorder screen
```

---

### Task 9: Final verification

- [ ] **Step 1: Run all Rust tests**

Run: `cargo test`
Expected: all tests PASS (29 existing + 6 new pending tests = 35)

- [ ] **Step 2: Run clippy and fmt**

Run: `cargo clippy && cargo fmt --check`
Expected: no new warnings, formatting clean

- [ ] **Step 3: Manual testing**

1. `cargo tauri dev`
2. Record for ~10 seconds, stop — pending appears in list
3. Record again — second pending appears (different file name)
4. Close app, reopen — both pendings still visible
5. Click "Transcrever" on one — transcription appears in history, pending removed
6. Click "×" on the other — pending removed, WAV deleted
7. Verify no orphan WAV files remain in data dir

- [ ] **Step 4: Commit any final adjustments**
