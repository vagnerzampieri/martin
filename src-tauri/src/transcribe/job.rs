#![allow(dead_code)]

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
        self.cancel_flag
            .store(true, std::sync::atomic::Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel_flag.load(std::sync::atomic::Ordering::Acquire)
    }
}

use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::db::store::Store;
use crate::transcribe::whisper::Transcriber;

/// Final outcome of a finalize run. Used by the orchestrator to decide
/// whether to emit `complete`, `cancelled`, or `error`.
#[derive(Debug)]
pub enum FinalizeOutcome {
    Complete {
        final_text: String,
        duration_secs: f64,
    },
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
            let _ = app.emit(
                "transcription://progress",
                ProgressPayload { id, percent: p },
            );
        }
    };

    let on_segment = move |seg: &str| {
        let trimmed = seg.trim();
        if trimmed.is_empty() {
            return;
        }
        // Lock guards may be poisoned if a previous callback panicked.
        // Skip the segment on poison rather than propagate panic into
        // whisper's inference thread.
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
        let _ = app_for_callback.emit("transcription://text", TextPayload { id, text: new_text });
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
            FinalizeOutcome::Complete {
                final_text,
                duration_secs,
            }
        }
        Err(_) if cancel_flag.load(std::sync::atomic::Ordering::Acquire) => {
            FinalizeOutcome::Cancelled
        }
        Err(e) => FinalizeOutcome::Error(e),
    }
}

/// Loads the WAV at `wav_path` then runs the same finalize loop as the
/// dictation worker. On success, the caller is responsible for deleting
/// the WAV and the matching `pending_recordings` row.
pub fn run_finalize_pending_file(
    job: &TranscriptionJob,
    transcriber: &Transcriber,
    wav_path: &std::path::Path,
    language: String,
    store: Arc<Mutex<Store>>,
    app_handle: AppHandle,
) -> FinalizeOutcome {
    let samples = match Transcriber::load_wav_as_mono_f32(wav_path) {
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
        FinalizeOutcome::Complete {
            final_text,
            duration_secs,
        } => {
            if let Ok(s) = store.lock() {
                let _ = s.update_text(id, &final_text, duration_secs);
                let _ = s.mark_complete(id);

                if let JobKind::PendingFile {
                    wav_path,
                    pending_id,
                } = &job.kind
                {
                    if let Err(e) = std::fs::remove_file(wav_path) {
                        eprintln!("[finish_job] failed to remove WAV {:?}: {}", wav_path, e);
                    }
                    let _ = s.delete_pending(*pending_id);
                }

                if let Ok(t) = s.get(id) {
                    let _ = app_handle.emit(
                        "transcription://complete",
                        CompletePayload {
                            id,
                            transcription: t,
                        },
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
            let _ = app_handle.emit("transcription://error", ErrorPayload { id, error: err });
        }
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
            JobKind::PendingFile {
                wav_path,
                pending_id,
            } => {
                assert_eq!(wav_path, &PathBuf::from("/tmp/foo.wav"));
                assert_eq!(*pending_id, 42);
            }
            _ => panic!("wrong kind"),
        }
    }
}
