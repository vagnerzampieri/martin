mod audio;
mod db;
mod dictation;
mod model;
mod summarize;
mod transcribe;

use std::path::PathBuf;
use std::sync::Mutex;

use tauri::{Manager, State};

use audio::capture::{finalize_recording, AudioCapture};
use db::store::{PendingRecording, Store, Transcription};
use dictation::DictationSession;
use summarize::{build_prompt, call_claude_cli, is_claude_cli_available};
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
    store: Mutex<Store>,
    transcriber: Mutex<Option<Transcriber>>,
    data_dir: PathBuf,
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

    let duration_secs = wav_duration_secs(&output_path)?;
    let file_path = output_path.to_string_lossy().to_string();

    let store = state.store.lock().map_err(|e| e.to_string())?;
    let id = store.save_pending(&file_path, duration_secs)?;
    store.get_pending(id)
}

fn wav_duration_secs(path: &std::path::Path) -> Result<f64, String> {
    let reader = hound::WavReader::open(path).map_err(|e| format!("Failed to read WAV: {}", e))?;
    let spec = reader.spec();
    Ok(reader.duration() as f64 / spec.sample_rate as f64)
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
    let model_path = tauri::async_runtime::spawn_blocking(move || {
        model::ensure_model(&data_dir, &app)
    })
    .await
    .map_err(|e| format!("Model check failed: {}", e))??;

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
    let app = app_handle.clone();
    tauri::async_runtime::spawn_blocking(move || {
        model::ensure_model(&data_dir, &app)
    })
    .await
    .map_err(|e| format!("Model check failed: {}", e))??;

    let mut session = DictationSession::new();
    session.start()?;

    let buffer = session.buffer();
    let running = session.running_flag();
    let source_rate = session.source_rate();
    let channels = session.channels();

    *state.dictation.lock().map_err(|e| e.to_string())? = Some(session);

    let model_path = model::model_path(&state.data_dir);
    let cached = state.transcriber.lock().map_err(|e| e.to_string())?.take();

    tauri::async_runtime::spawn(async move {
        let handle = app_handle.clone();
        let result = tauri::async_runtime::spawn_blocking(move || {
            let transcriber = match get_or_create_transcriber(cached, &model_path) {
                Ok(t) => t,
                Err(e) => {
                    eprintln!("Failed to create transcriber: {}", e);
                    return (None, Vec::new());
                }
            };

            let segments = dictation::run_transcription_loop(
                buffer,
                running,
                &transcriber,
                &language,
                source_rate,
                channels,
                handle,
            );

            (Some(transcriber), segments)
        })
        .await;

        if let Ok((Some(transcriber), _)) = result {
            let app_state = app_handle.state::<AppState>();
            let _ = app_state
                .transcriber
                .lock()
                .map(|mut guard| *guard = Some(transcriber));
        }
    });

    Ok(())
}

#[tauri::command]
async fn stop_dictation(
    state: State<'_, AppState>,
    title: String,
    full_text: String,
    language: String,
    duration_secs: f64,
) -> Result<Transcription, String> {
    {
        let mut guard = state.dictation.lock().map_err(|e| e.to_string())?;
        if let Some(ref mut session) = *guard {
            session.stop();
        } else {
            return Err("No dictation in progress".to_string());
        }
    }

    // Brief wait for transcription thread to process remaining audio
    tauri::async_runtime::spawn(async {
        std::thread::sleep(std::time::Duration::from_millis(500));
    })
    .await
    .map_err(|e| format!("Wait failed: {}", e))?;

    {
        let mut guard = state.dictation.lock().map_err(|e| e.to_string())?;
        *guard = None;
    }

    if full_text.trim().is_empty() {
        return Err("No text was transcribed".to_string());
    }

    let store = state.store.lock().map_err(|e| e.to_string())?;
    let id = store.save(&title, &full_text, &language, duration_secs)?;
    store.get(id)
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
    tauri::async_runtime::spawn_blocking(move || {
        model::download_model(&data_dir, &app_handle)
    })
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
                store: Mutex::new(store),
                transcriber: Mutex::new(None),
                data_dir,
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
