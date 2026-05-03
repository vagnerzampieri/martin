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
