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
    transcriber: Mutex<Option<Transcriber>>,
    model_path: PathBuf,
    data_dir: PathBuf,
}

impl AppState {
    fn audio_path(&self) -> PathBuf {
        self.data_dir.join("recording.wav")
    }
}

#[tauri::command]
fn start_recording(state: State<'_, AppState>) -> Result<(), String> {
    let mut guard = state.capture.lock().map_err(|e| e.to_string())?;
    if guard.0.is_some() {
        return Err("Recording already in progress".to_string());
    }

    let audio_path = state.audio_path();
    let mut capture = AudioCapture::new(audio_path);
    capture.start()?;
    guard.0 = Some(capture);

    Ok(())
}

#[tauri::command]
fn stop_recording(state: State<'_, AppState>) -> Result<(), String> {
    let mut guard = state.capture.lock().map_err(|e| e.to_string())?;
    let mut capture = guard.0.take().ok_or("No active recording to stop")?;
    capture.stop().map(|_| ())
}

fn wav_duration_secs(path: &std::path::Path) -> Result<f64, String> {
    let reader = hound::WavReader::open(path)
        .map_err(|e| format!("Failed to read WAV: {}", e))?;
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
async fn transcribe_recording(state: State<'_, AppState>, title: String, language: String) -> Result<Transcription, String> {
    let audio_path = state.audio_path();
    if !audio_path.exists() {
        return Err("No recording found. Record a meeting first.".to_string());
    }

    let model_path = state.model_path.clone();
    let cached = state.transcriber.lock().map_err(|e| e.to_string())?.take();
    let audio = audio_path.clone();
    let lang = language.clone();

    let (text, duration_secs, transcriber) = tauri::async_runtime::spawn_blocking(move || -> Result<(String, f64, Transcriber), String> {
        let transcriber = get_or_create_transcriber(cached, &model_path)?;
        let text = transcriber.transcribe(&audio, &lang)?;
        let duration_secs = wav_duration_secs(&audio)?;
        Ok((text, duration_secs, transcriber))
    })
    .await
    .map_err(|e| format!("Transcription task failed: {}", e))??;

    *state.transcriber.lock().map_err(|e| e.to_string())? = Some(transcriber);

    let store = state.store.lock().map_err(|e| e.to_string())?;
    let id = store.save(&title, &text, &language, duration_secs)?;

    if let Err(e) = std::fs::remove_file(&audio_path) {
        eprintln!("Warning: failed to delete recording file: {}", e);
    }

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
                transcriber: Mutex::new(None),
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
