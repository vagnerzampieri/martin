mod audio;
mod db;
mod transcribe;

use audio::capture::AudioCapture;
use db::store::{Store, Transcription};
use transcribe::whisper::Transcriber;

use std::path::PathBuf;
use std::sync::Mutex;
use tauri::{Manager, State};

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
    store: Mutex<Store>,
    model_path: PathBuf,
    data_dir: PathBuf,
}

#[tauri::command]
fn start_recording(state: State<'_, AppState>) -> Result<(), String> {
    let audio_path = state.data_dir.join("recording.wav");
    let mut capture = AudioCapture::new(audio_path);
    capture.start()?;

    let mut guard = state.capture.lock().map_err(|e| e.to_string())?;
    guard.0 = Some(capture);

    Ok(())
}

#[tauri::command]
fn stop_recording(state: State<'_, AppState>) -> Result<(), String> {
    let mut guard = state.capture.lock().map_err(|e| e.to_string())?;
    if let Some(mut capture) = guard.0.take() {
        capture.stop()?;
    }
    Ok(())
}

#[tauri::command]
async fn transcribe_recording(state: State<'_, AppState>, title: String, language: String) -> Result<Transcription, String> {
    let audio_path = state.data_dir.join("recording.wav");
    let model_path = state.model_path.clone();

    if !audio_path.exists() {
        return Err("No recording found. Record a meeting first.".to_string());
    }

    // Run transcription on a blocking thread to avoid freezing the UI
    let audio_path_clone = audio_path.clone();
    let lang = language.clone();
    let (text, duration_secs) = tauri::async_runtime::spawn_blocking(move || -> Result<(String, f64), String> {
        let transcriber = Transcriber::new(&model_path)?;
        let text = transcriber.transcribe(&audio_path_clone, &lang)?;

        let reader = hound::WavReader::open(&audio_path_clone)
            .map_err(|e| format!("Failed to read WAV: {}", e))?;
        let spec = reader.spec();
        let duration_secs = reader.duration() as f64 / spec.sample_rate as f64;

        Ok((text, duration_secs))
    })
    .await
    .map_err(|e| format!("Transcription task failed: {}", e))??;

    let store = state.store.lock().map_err(|e| e.to_string())?;
    let id = store.save(&title, &text, &language, duration_secs)?;

    // Delete the audio file after transcription
    let _ = std::fs::remove_file(&audio_path);

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
            let model_path = data_dir.join("models").join("ggml-small.bin");

            let store = Store::new(&db_path).expect("Failed to initialize database");

            app.manage(AppState {
                capture: Mutex::new(SendableCapture(None)),
                store: Mutex::new(store),
                model_path,
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
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
