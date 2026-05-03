mod audio;
mod db;
mod dictation;
mod model;
mod summarize;
mod transcribe;

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
async fn transcribe_recording(
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
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

    let data_dir = state.data_dir.clone();
    let app = app_handle.clone();
    let model_path =
        tauri::async_runtime::spawn_blocking(move || model::ensure_model(&data_dir, &app))
            .await
            .map_err(|e| format!("Model check failed: {}", e))??;

    let cached = state.transcriber.lock().map_err(|e| e.to_string())?.take();
    let audio = audio_path.clone();
    let lang = language.clone();

    let (text, duration_secs, transcriber) = tauri::async_runtime::spawn_blocking(
        move || -> Result<(String, f64, Transcriber), String> {
            let transcriber = get_or_create_transcriber(cached, &model_path)?;
            let text = transcriber.transcribe(&audio, &lang)?;
            let duration_secs = transcribe::whisper::wav_duration_secs(&audio)?;
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

    let model_path = model::model_path(&state.data_dir);
    let cached = state.transcriber.lock().map_err(|e| e.to_string())?.take();

    let transcriber = get_or_create_transcriber(cached, &model_path)?;

    let app_for_loop = app_handle.clone();
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
            app_for_loop.clone(),
        );

        if let Some(s) = app_for_loop.try_state::<AppState>() {
            let _ = s.transcriber.lock().map(|mut g| *g = Some(transcriber));
        }
    });

    session.set_worker(worker);

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

    session.stop_and_join();

    let samples = session.drain_buffer();
    let last_full = session
        .last_full_text()
        .lock()
        .map(|s| s.clone())
        .unwrap_or_default();
    let committed_prefix = if !last_full.trim().is_empty() {
        last_full.trim().to_string()
    } else {
        session
            .committed()
            .lock()
            .map(|c| c.join(" ").trim().to_string())
            .unwrap_or_default()
    };

    *dictation_guard = None;
    drop(dictation_guard);

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
            transcribe_recording,
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
