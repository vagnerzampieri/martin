mod audio;
mod db;
mod dictation;
mod model;
mod postprocess;
mod summarize;
mod transcribe;
mod vad;

use std::path::PathBuf;
use std::sync::Mutex;

use tauri::{Emitter, Manager, State};

use audio::capture::{finalize_recording, AudioCapture};
use db::store::{PendingRecording, Store, Transcription};
use dictation::DictationSession;
use summarize::{build_prompt, call_claude_cli, is_claude_cli_available};
use transcribe::job::TranscriptionJob;
use transcribe::whisper::Transcriber;

/// Wrapper to allow AudioCapture (which contains cpal::Stream, a !Send type)
/// to be stored in Tauri managed state. Access is serialized through a Mutex.
struct SendableCapture(Option<AudioCapture>);

// SAFETY: AudioCapture is only accessed through a Mutex, serializing all access.
// cpal::Stream is !Send only as a precaution on some platforms; our usage is safe
// because we never move it across threads without synchronization.
unsafe impl Send for SendableCapture {}
unsafe impl Sync for SendableCapture {}

pub struct AppState {
    capture: Mutex<SendableCapture>,
    dictation: Mutex<Option<DictationSession>>,
    store: std::sync::Arc<Mutex<Store>>,
    transcriber: Mutex<Option<Transcriber>>,
    data_dir: PathBuf,
    current_job: Mutex<Option<TranscriptionJob>>,
}

#[tauri::command]
fn start_recording(state: State<'_, AppState>) -> Result<(), String> {
    let mut guard = state.capture.lock().map_err(|e| e.to_string())?;
    if guard.0.is_some() {
        return Err("Recording already in progress".to_string());
    }

    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| format!("System clock error: {}", e))?
        .as_millis();
    let audio_path = state.data_dir.join(format!("recording_{}.wav", timestamp));

    let mut capture = AudioCapture::new(audio_path);
    capture.start()?;
    guard.0 = Some(capture);

    Ok(())
}

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

    let duration_secs = transcribe::whisper::wav_duration_secs(&output_path)?;
    let file_path = output_path.to_string_lossy().to_string();

    let store = state.store.lock().map_err(|e| e.to_string())?;
    let id = store.save_pending(&file_path, duration_secs)?;
    store.get_pending(id)
}

fn get_or_create_transcriber(
    cached: Option<Transcriber>,
    model_path: &std::path::Path,
) -> Result<Transcriber, String> {
    match cached {
        Some(t) => Ok(t),
        None => Transcriber::new(model_path),
    }
}

fn clear_current_job_and_emit_error(app: &tauri::AppHandle, job_id: i64, error: String) {
    if let Some(s) = app.try_state::<AppState>() {
        let _ = s.current_job.lock().map(|mut g| *g = None);
    }
    let _ = app.emit(
        "transcription://error",
        crate::transcribe::job::ErrorPayload { id: job_id, error },
    );
}

#[tauri::command]
async fn transcribe_pending_recording(
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
    pending_id: i64,
    title: String,
    language: String,
) -> Result<i64, String> {
    use crate::transcribe::job::{
        finish_job, run_finalize_pending_file, ErrorPayload, JobKind, TranscriptionJob,
    };
    use std::panic::AssertUnwindSafe;

    {
        let job_guard = state.current_job.lock().map_err(|e| e.to_string())?;
        if job_guard.is_some() {
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

    eprintln!(
        "[martin] transcribe_pending_recording pending_id={} duration={:.1}s",
        pending_id, pending.duration_secs
    );

    let data_dir = state.data_dir.clone();
    let app_for_model = app_handle.clone();
    tauri::async_runtime::spawn_blocking(move || model::ensure_model(&data_dir, &app_for_model))
        .await
        .map_err(|e| format!("Model check failed: {}", e))??;

    let mut job_guard = state.current_job.lock().map_err(|e| e.to_string())?;
    if job_guard.is_some() {
        return Err("Another transcription is in progress".to_string());
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
                        ErrorPayload {
                            id: job_id,
                            error: e,
                        },
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

        if let Some(s) = app.try_state::<AppState>() {
            let _ = s.transcriber.lock().map(|mut g| *g = Some(transcriber));
            let _ = s.current_job.lock().map(|mut g| *g = None);
        }

        finish_job(job, outcome, store_for_worker, app);
    });

    Ok(id)
}

#[tauri::command]
fn list_transcriptions(state: State<'_, AppState>) -> Result<Vec<Transcription>, String> {
    let store = state.store.lock().map_err(|e| e.to_string())?;
    store.list()
}

#[tauri::command]
fn get_transcription(state: State<'_, AppState>, id: i64) -> Result<Transcription, String> {
    let store = state.store.lock().map_err(|e| e.to_string())?;
    store.get(id)
}

#[tauri::command]
fn delete_transcription(state: State<'_, AppState>, id: i64) -> Result<(), String> {
    let store = state.store.lock().map_err(|e| e.to_string())?;
    store.delete(id)
}

#[tauri::command]
fn check_claude_cli() -> bool {
    is_claude_cli_available()
}

#[tauri::command]
async fn summarize_transcription(state: State<'_, AppState>, id: i64) -> Result<String, String> {
    let text = {
        let store = state.store.lock().map_err(|e| e.to_string())?;
        let transcription = store.get(id)?;
        transcription.text
    };

    let prompt = build_prompt(&text);

    let summary = tauri::async_runtime::spawn_blocking(move || call_claude_cli(&prompt))
        .await
        .map_err(|e| format!("Summary task failed: {}", e))??;

    let store = state.store.lock().map_err(|e| e.to_string())?;
    store.save_summary(id, &summary)?;

    Ok(summary)
}

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
        pending.file_path
    };

    // Delete file first, then DB row — avoids orphaned files if DB delete succeeds but file delete fails
    if let Err(e) = std::fs::remove_file(&file_path) {
        eprintln!("Warning: failed to delete recording file: {}", e);
    }

    let store = state.store.lock().map_err(|e| e.to_string())?;
    store.delete_pending(id)?;

    Ok(())
}

#[tauri::command]
async fn start_dictation(
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
    language: String,
) -> Result<(), String> {
    {
        let guard = state.dictation.lock().map_err(|e| e.to_string())?;
        if guard.as_ref().is_some_and(|d| d.is_running()) {
            return Err("Dictation already in progress".to_string());
        }
    }

    let data_dir = state.data_dir.clone();
    let app_for_model = app_handle.clone();
    tauri::async_runtime::spawn_blocking(move || model::ensure_model(&data_dir, &app_for_model))
        .await
        .map_err(|e| format!("Model check failed: {}", e))??;

    let mut session = DictationSession::new();
    session.start()?;

    let buffer = session.buffer();
    let running = session.running_flag();
    let source_rate = session.source_rate();
    let channels = session.channels();
    let committed_out = session.committed();
    let last_full_text_out = session.last_full_text();

    eprintln!(
        "[martin] start_dictation source_rate={}Hz channels={} language={}",
        source_rate, channels, language
    );
    let final_audio_out = session.final_audio();

    let model_path = model::model_path(&state.data_dir);
    let cached = state.transcriber.lock().map_err(|e| e.to_string())?.take();

    let transcriber = get_or_create_transcriber(cached, &model_path)?;

    let state_for_loop = session.state();
    let app_for_loop = app_handle.clone();
    let language_owned = language.clone();
    let worker = std::thread::spawn(move || {
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

        if let Some(s) = app_for_loop.try_state::<AppState>() {
            let _ = s.transcriber.lock().map(|mut g| *g = Some(transcriber));
        }
    });

    let level_running = session.running_flag();
    let level_peak = session.last_peak_bits();
    let level_state = session.state();
    let level_app = app_handle.clone();
    let level_worker = std::thread::spawn(move || {
        dictation::run_level_emitter(level_running, level_peak, level_state, level_app);
    });

    session.set_worker(worker);
    session.set_level_worker(level_worker);

    *state.dictation.lock().map_err(|e| e.to_string())? = Some(session);

    Ok(())
}

#[tauri::command]
async fn stop_dictation(
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
    title: String,
    language: String,
    duration_secs: f64,
) -> Result<i64, String> {
    use crate::transcribe::job::{finish_job, run_finalize_dictation, JobKind, TranscriptionJob};
    use std::panic::AssertUnwindSafe;

    let mut dictation_guard = state.dictation.lock().map_err(|e| e.to_string())?;
    let session = dictation_guard.as_mut().ok_or("No dictation in progress")?;

    let mut job_guard = state.current_job.lock().map_err(|e| e.to_string())?;
    if job_guard.is_some() {
        return Err("Another transcription is in progress".to_string());
    }

    // Capture device rate + channels BEFORE join+drop so we can
    // convert the raw audio to mono 16kHz (whisper's required format)
    // after the session is taken.
    let source_rate = session.source_rate();
    let channels = session.channels();

    session.stop_and_join();

    // After join: raw audio (device rate, multi-channel) is in
    // final_audio; convert to mono 16k before handing it to whisper.
    // Without the conversion, whisper interprets device-rate stereo
    // samples as 16kHz mono and hallucinates garbage.
    let raw_samples = session.take_final_audio();
    let samples = dictation::convert_to_mono_16k(&raw_samples, channels, source_rate);
    eprintln!(
        "[martin] stop_dictation raw={} samples mono16k={} samples (~{:.1}s)",
        raw_samples.len(),
        samples.len(),
        samples.len() as f64 / 16000.0
    );

    // last_full_text: best-effort transcription up to the last live poll
    // (covers all audio so far, including post-rollover). Used as the
    // text shown in the partial row for crash recovery.
    let last_full = session
        .last_full_text()
        .lock()
        .map(|s| s.clone())
        .unwrap_or_default();

    // Worker seed: ONLY committed (rolled-over) segments. The audio
    // handed off in `samples` is post-rollover, so the finalize whisper
    // pass will produce text for that. Seeding `acc` with `last_full`
    // would double-count the post-rollover portion.
    let worker_seed = session
        .committed()
        .lock()
        .map(|c| c.join(" ").trim().to_string())
        .unwrap_or_default();

    *dictation_guard = None;
    drop(dictation_guard);

    let partial_text = if !last_full.trim().is_empty() {
        last_full.trim().to_string()
    } else {
        worker_seed.clone()
    };

    let id = {
        let store = state.store.lock().map_err(|e| e.to_string())?;
        let new_id = store.insert_partial(&title, &language)?;
        if !partial_text.is_empty() {
            store.update_text(new_id, &partial_text, duration_secs)?;
        }
        new_id
    };

    // Fast path: if the live transcription loop already produced text
    // for this audio, skip the finalize whisper pass entirely. The live
    // loop and finalize use identical params on identical audio — running
    // whisper a second time is duplicated work that turns into minutes
    // of wait on slow machines and risks OOM (whisper error -6 is
    // typically encode failure under memory pressure).
    if !last_full.trim().is_empty() {
        eprintln!(
            "[martin] stop_dictation fast path: live text complete ({} chars), skipping finalize pass",
            last_full.trim().len()
        );
        let final_text = last_full.trim().to_string();
        let transcription = {
            let store = state.store.lock().map_err(|e| e.to_string())?;
            store.update_text(id, &final_text, duration_secs)?;
            store.mark_complete(id)?;
            store.get(id)?
        };

        // current_job was never set on the fast path — drop the guard
        // so it is released cleanly.
        drop(job_guard);

        #[derive(Clone, serde::Serialize)]
        struct CompletePayload {
            id: i64,
            transcription: db::store::Transcription,
        }
        let _ = app_handle.emit(
            "transcription://complete",
            CompletePayload { id, transcription },
        );
        return Ok(id);
    }

    let mut job = TranscriptionJob::new(id, JobKind::Dictation);
    job.committed_text = worker_seed;
    let job_id = job.id;

    *job_guard = Some(job.clone());
    drop(job_guard);

    let store_for_worker = state.store.clone();
    let app = app_handle.clone();
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
                if let Some(s) = app.try_state::<AppState>() {
                    let _ = s.transcriber.lock().map(|mut g| *g = Some(transcriber));
                }
                return;
            }
        };

        if let Some(s) = app.try_state::<AppState>() {
            let _ = s.transcriber.lock().map(|mut g| *g = Some(transcriber));
            let _ = s.current_job.lock().map(|mut g| *g = None);
        }

        finish_job(job, outcome, store_for_worker, app);
    });

    Ok(id)
}

#[tauri::command]
fn cancel_job(state: State<'_, AppState>) -> Result<(), String> {
    let guard = state.current_job.lock().map_err(|e| e.to_string())?;
    if let Some(job) = guard.as_ref() {
        job.cancel();
    }
    Ok(())
}

#[tauri::command]
fn check_model_exists(state: State<'_, AppState>) -> bool {
    model::model_exists(&state.data_dir)
}

#[tauri::command]
async fn download_whisper_model(
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    let data_dir = state.data_dir.clone();
    tauri::async_runtime::spawn_blocking(move || model::download_model(&data_dir, &app_handle))
        .await
        .map_err(|e| format!("Download task failed: {}", e))??;
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Capture whisper.cpp / ggml stderr noise. With no `log_backend` /
    // `tracing_backend` feature enabled on whisper-rs, this effectively
    // silences whisper's per-token decode prints — the model load lines
    // and the long `whisper_full_with_state: id = N, decoder = 0...`
    // streams that flooded the dev console.
    whisper_rs::install_logging_hooks();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let data_dir = app
                .path()
                .app_data_dir()
                .expect("Failed to get app data dir");
            std::fs::create_dir_all(&data_dir).expect("Failed to create data dir");

            let db_path = data_dir.join("martin.db");

            let store = Store::new(&db_path).expect("Failed to initialize database");

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

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            start_recording,
            stop_recording,
            transcribe_pending_recording,
            cancel_job,
            list_transcriptions,
            get_transcription,
            delete_transcription,
            check_claude_cli,
            summarize_transcription,
            list_pending_recordings,
            delete_pending_recording,
            start_dictation,
            stop_dictation,
            check_model_exists,
            download_whisper_model,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
