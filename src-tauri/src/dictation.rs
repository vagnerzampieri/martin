use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, Stream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::transcribe::whisper::Transcriber;

const WHISPER_SAMPLE_RATE: u32 = 16000;
const CHUNK_SECONDS: usize = 5;
const OVERLAP_SECONDS: usize = 1;
const CHUNK_SAMPLES: usize = WHISPER_SAMPLE_RATE as usize * CHUNK_SECONDS;
const OVERLAP_SAMPLES: usize = WHISPER_SAMPLE_RATE as usize * OVERLAP_SECONDS;

pub struct DictationSession {
    stream: Option<Stream>,
    audio_buffer: Arc<Mutex<Vec<f32>>>,
    running: Arc<AtomicBool>,
}

// SAFETY: DictationSession contains cpal::Stream which is !Send.
// Access is serialized through a Mutex in AppState, same pattern as SendableCapture in lib.rs.
unsafe impl Send for DictationSession {}

impl DictationSession {
    pub fn new() -> Self {
        Self {
            stream: None,
            audio_buffer: Arc::new(Mutex::new(Vec::new())),
            running: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Acquire)
    }

    pub fn buffer(&self) -> Arc<Mutex<Vec<f32>>> {
        self.audio_buffer.clone()
    }

    pub fn running_flag(&self) -> Arc<AtomicBool> {
        self.running.clone()
    }

    pub fn start(&mut self) -> Result<(), String> {
        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .ok_or("No input device available")?;

        let config = device
            .default_input_config()
            .map_err(|e| format!("Failed to get input config: {}", e))?;

        let source_rate = config.sample_rate().0;
        let channels = config.channels();
        let sample_format = config.sample_format();
        let buffer = self.audio_buffer.clone();

        let stream = match sample_format {
            SampleFormat::I16 => device
                .build_input_stream(
                    &config.into(),
                    move |data: &[i16], _| {
                        let mono_16k = convert_to_mono_16k_i16(data, channels, source_rate);
                        if let Ok(mut buf) = buffer.lock() {
                            buf.extend_from_slice(&mono_16k);
                        }
                    },
                    |err| eprintln!("Dictation stream error: {}", err),
                    None,
                )
                .map_err(|e| format!("Failed to build input stream: {}", e))?,
            SampleFormat::F32 => device
                .build_input_stream(
                    &config.into(),
                    move |data: &[f32], _| {
                        let mono_16k = convert_to_mono_16k_f32(data, channels, source_rate);
                        if let Ok(mut buf) = buffer.lock() {
                            buf.extend_from_slice(&mono_16k);
                        }
                    },
                    |err| eprintln!("Dictation stream error: {}", err),
                    None,
                )
                .map_err(|e| format!("Failed to build input stream: {}", e))?,
            format => return Err(format!("Unsupported sample format: {:?}", format)),
        };

        stream
            .play()
            .map_err(|e| format!("Failed to play stream: {}", e))?;
        self.stream = Some(stream);
        self.running.store(true, Ordering::Release);

        Ok(())
    }

    pub fn stop(&mut self) {
        self.running.store(false, Ordering::Release);
        self.stream = None;
    }
}

fn convert_to_mono_16k_i16(data: &[i16], channels: u16, source_rate: u32) -> Vec<f32> {
    let float_data: Vec<f32> = data.iter().map(|&s| s as f32 / i16::MAX as f32).collect();
    convert_to_mono_16k(&float_data, channels, source_rate)
}

fn convert_to_mono_16k_f32(data: &[f32], channels: u16, source_rate: u32) -> Vec<f32> {
    convert_to_mono_16k(data, channels, source_rate)
}

fn convert_to_mono_16k(samples: &[f32], channels: u16, source_rate: u32) -> Vec<f32> {
    let mono: Vec<f32> = if channels >= 2 {
        samples
            .chunks(channels as usize)
            .map(|chunk| chunk.iter().sum::<f32>() / channels as f32)
            .collect()
    } else {
        samples.to_vec()
    };

    if source_rate == WHISPER_SAMPLE_RATE {
        return mono;
    }

    let ratio = source_rate as f64 / WHISPER_SAMPLE_RATE as f64;
    let output_len = (mono.len() as f64 / ratio) as usize;
    let mut output = Vec::with_capacity(output_len);

    for i in 0..output_len {
        let src_idx = i as f64 * ratio;
        let idx = src_idx as usize;
        let frac = src_idx - idx as f64;

        let sample = if idx + 1 < mono.len() {
            mono[idx] as f64 * (1.0 - frac) + mono[idx + 1] as f64 * frac
        } else if idx < mono.len() {
            mono[idx] as f64
        } else {
            0.0
        };
        output.push(sample as f32);
    }

    output
}

/// Runs the transcription loop on a blocking thread.
/// Drains the audio buffer every ~5 seconds, transcribes, and emits events.
pub fn run_transcription_loop(
    buffer: Arc<Mutex<Vec<f32>>>,
    running: Arc<AtomicBool>,
    transcriber: &Transcriber,
    language: &str,
    app_handle: tauri::AppHandle,
) -> Vec<String> {
    use tauri::Emitter;

    let mut all_segments: Vec<String> = Vec::new();
    let mut overlap: Vec<f32> = Vec::new();

    while running.load(Ordering::Acquire) {
        std::thread::sleep(std::time::Duration::from_millis(500));

        let buffered = buffer.lock().map(|b| b.len()).unwrap_or(0);
        if buffered < CHUNK_SAMPLES {
            continue;
        }

        let audio_data = {
            let mut buf = match buffer.lock() {
                Ok(b) => b,
                Err(_) => continue,
            };
            let data: Vec<f32> = buf.drain(..).collect();
            data
        };

        let mut chunk = Vec::with_capacity(overlap.len() + audio_data.len());
        chunk.extend_from_slice(&overlap);
        chunk.extend_from_slice(&audio_data);

        if chunk.len() > OVERLAP_SAMPLES {
            overlap = chunk[chunk.len() - OVERLAP_SAMPLES..].to_vec();
        }

        match transcriber.transcribe_samples(&chunk, language) {
            Ok(text) => {
                let text = text.trim().to_string();
                if !text.is_empty() {
                    all_segments.push(text.clone());
                    let full_text = all_segments.join(" ");
                    let _ = app_handle.emit(
                        "dictation://segment",
                        serde_json::json!({
                            "text": text,
                            "fullText": full_text,
                        }),
                    );
                }
            }
            Err(e) => {
                eprintln!("Dictation transcription error: {}", e);
            }
        }
    }

    // Process remaining audio in buffer
    let remaining = {
        let mut buf = match buffer.lock() {
            Ok(b) => b,
            Err(_) => return all_segments,
        };
        let data: Vec<f32> = buf.drain(..).collect();
        data
    };

    if remaining.len() > WHISPER_SAMPLE_RATE as usize {
        let mut chunk = Vec::with_capacity(overlap.len() + remaining.len());
        chunk.extend_from_slice(&overlap);
        chunk.extend_from_slice(&remaining);

        if let Ok(text) = transcriber.transcribe_samples(&chunk, language) {
            let text = text.trim().to_string();
            if !text.is_empty() {
                all_segments.push(text.clone());
                let full_text = all_segments.join(" ");
                let _ = app_handle.emit(
                    "dictation://segment",
                    serde_json::json!({
                        "text": text,
                        "fullText": full_text,
                    }),
                );
            }
        }
    }

    all_segments
}
