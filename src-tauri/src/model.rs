use std::io::Read;
use std::path::{Path, PathBuf};

use serde::Serialize;
use tauri::Emitter;

const MODEL_URL: &str =
    "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin";
const MODEL_FILENAME: &str = "ggml-small.bin";

pub fn model_path(data_dir: &Path) -> PathBuf {
    data_dir.join("models").join(MODEL_FILENAME)
}

pub fn model_exists(data_dir: &Path) -> bool {
    model_path(data_dir).exists()
}

#[derive(Clone, Serialize)]
pub struct DownloadProgress {
    pub percent: u8,
    pub downloaded_mb: f64,
    pub total_mb: f64,
}

pub fn download_model(data_dir: &Path, app_handle: &tauri::AppHandle) -> Result<PathBuf, String> {
    let models_dir = data_dir.join("models");
    std::fs::create_dir_all(&models_dir)
        .map_err(|e| format!("Failed to create models directory: {}", e))?;

    let dest = model_path(data_dir);
    let part = dest.with_extension("bin.part");

    let response = reqwest::blocking::get(MODEL_URL)
        .map_err(|e| format!("Failed to start download: {}", e))?;

    let total = response.content_length().unwrap_or(0);
    let total_mb = total as f64 / 1_048_576.0;

    let mut file = std::fs::File::create(&part)
        .map_err(|e| format!("Failed to create temp file: {}", e))?;

    let mut downloaded: u64 = 0;
    let mut last_percent: u8 = 255;
    let mut buf = [0u8; 65536];
    let mut reader = std::io::BufReader::new(response);

    loop {
        let n = reader
            .read(&mut buf)
            .map_err(|e| format!("Read error during download: {}", e))?;
        if n == 0 {
            break;
        }

        std::io::Write::write_all(&mut file, &buf[..n])
            .map_err(|e| format!("Write error during download: {}", e))?;

        downloaded += n as u64;

        if total > 0 {
            let percent = ((downloaded * 100) / total) as u8;
            if percent != last_percent {
                last_percent = percent;
                let _ = app_handle.emit(
                    "model://download-progress",
                    DownloadProgress {
                        percent,
                        downloaded_mb: downloaded as f64 / 1_048_576.0,
                        total_mb,
                    },
                );
            }
        }
    }

    std::fs::rename(&part, &dest)
        .map_err(|e| format!("Failed to finalize download: {}", e))?;

    let _ = app_handle.emit("model://download-complete", ());

    Ok(dest)
}

pub fn ensure_model(data_dir: &Path, app_handle: &tauri::AppHandle) -> Result<PathBuf, String> {
    if model_exists(data_dir) {
        return Ok(model_path(data_dir));
    }
    download_model(data_dir, app_handle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn model_path_returns_expected_path() {
        let base = Path::new("/some/data/dir");
        let path = model_path(base);
        assert_eq!(path, PathBuf::from("/some/data/dir/models/ggml-small.bin"));
    }

    #[test]
    fn model_exists_returns_false_when_missing() {
        let dir = tempdir().expect("tempdir");
        assert!(!model_exists(dir.path()));
    }

    #[test]
    fn model_exists_returns_true_when_present() {
        let dir = tempdir().expect("tempdir");
        let models_dir = dir.path().join("models");
        std::fs::create_dir_all(&models_dir).expect("create models dir");
        std::fs::write(models_dir.join("ggml-small.bin"), b"fake model")
            .expect("write fake model");
        assert!(model_exists(dir.path()));
    }
}
