use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, Stream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use tauri::Emitter;

use crate::transcribe::whisper::Transcriber;

const WHISPER_SAMPLE_RATE: u32 = 16000;
const POLL_INTERVAL_MS: u64 = 500;
const MIN_SECONDS_TO_TRANSCRIBE: usize = 3;
const MAX_BUFFER_SECONDS: usize = 120;

pub struct DictationSession {
    stream: Option<Stream>,
    audio_buffer: Arc<Mutex<Vec<f32>>>,
    running: Arc<AtomicBool>,
    source_rate: u32,
    channels: u16,
    committed: Arc<Mutex<Vec<String>>>,
    last_full_text: Arc<Mutex<String>>,
    worker: Option<JoinHandle<()>>,
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
            source_rate: WHISPER_SAMPLE_RATE,
            channels: 1,
            committed: Arc::new(Mutex::new(Vec::new())),
            last_full_text: Arc::new(Mutex::new(String::new())),
            worker: None,
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

        self.source_rate = config.sample_rate().0;
        self.channels = config.channels();
        let sample_format = config.sample_format();
        let buffer = self.audio_buffer.clone();

        // Callback stores raw f32 samples. Mono conversion + resampling
        // happens in the transcription loop to keep the callback fast.
        let stream = match sample_format {
            SampleFormat::I16 => device
                .build_input_stream(
                    &config.into(),
                    move |data: &[i16], _| {
                        if let Ok(mut buf) = buffer.lock() {
                            buf.extend(data.iter().map(|&s| s as f32 / i16::MAX as f32));
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
                        if let Ok(mut buf) = buffer.lock() {
                            buf.extend_from_slice(data);
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

    pub fn source_rate(&self) -> u32 {
        self.source_rate
    }

    pub fn channels(&self) -> u16 {
        self.channels
    }

    #[allow(dead_code)]
    pub fn stop(&mut self) {
        self.running.store(false, Ordering::Release);
        self.stream = None;
    }

    pub fn committed(&self) -> Arc<Mutex<Vec<String>>> {
        self.committed.clone()
    }

    pub fn last_full_text(&self) -> Arc<Mutex<String>> {
        self.last_full_text.clone()
    }

    pub fn set_worker(&mut self, handle: JoinHandle<()>) {
        self.worker = Some(handle);
    }

    /// Stops the audio stream, signals the worker, and joins it.
    /// Returns only after the worker has fully exited.
    pub fn stop_and_join(&mut self) {
        self.running.store(false, Ordering::Release);
        self.stream.take();
        if let Some(handle) = self.worker.take() {
            let _ = handle.join();
        }
    }

    pub fn drain_buffer(&self) -> Vec<f32> {
        self.audio_buffer
            .lock()
            .map(|mut buf| buf.drain(..).collect())
            .unwrap_or_default()
    }
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
/// Re-transcribes the entire accumulated audio buffer each cycle for
/// maximum Whisper accuracy. When the buffer exceeds MAX_BUFFER_SECONDS,
/// commits the current text as a segment and starts a fresh buffer.
#[allow(clippy::too_many_arguments)]
pub fn run_transcription_loop(
    buffer: Arc<Mutex<Vec<f32>>>,
    running: Arc<AtomicBool>,
    committed_out: Arc<Mutex<Vec<String>>>,
    last_full_text_out: Arc<Mutex<String>>,
    transcriber: &Transcriber,
    language: &str,
    source_rate: u32,
    channels: u16,
    app_handle: tauri::AppHandle,
) {
    let mut committed_segments: Vec<String> = Vec::new();
    let mut accumulated_raw: Vec<f32> = Vec::new();
    let mut last_transcribed_len: usize = 0;

    let raw_samples_per_second = source_rate as usize * channels as usize;
    let min_raw_samples = raw_samples_per_second * MIN_SECONDS_TO_TRANSCRIBE;
    let max_raw_samples = raw_samples_per_second * MAX_BUFFER_SECONDS;

    while running.load(Ordering::Acquire) {
        std::thread::sleep(std::time::Duration::from_millis(POLL_INTERVAL_MS));

        // Drain new samples from the shared buffer
        if let Ok(mut buf) = buffer.lock() {
            accumulated_raw.extend(buf.drain(..));
        }

        // Only transcribe if we have enough new audio since last transcription
        if accumulated_raw.len() < min_raw_samples || accumulated_raw.len() == last_transcribed_len
        {
            continue;
        }

        // Convert accumulated audio to mono 16kHz and transcribe the whole thing
        let mono_16k = convert_to_mono_16k(&accumulated_raw, channels, source_rate);
        last_transcribed_len = accumulated_raw.len();

        match transcriber.transcribe_samples(&mono_16k, language) {
            Ok(text) => {
                let text = text.trim().to_string();
                if !text.is_empty() {
                    let full_text = if committed_segments.is_empty() {
                        text.clone()
                    } else {
                        format!("{} {}", committed_segments.join(" "), text)
                    };
                    let _ = app_handle.emit(
                        "dictation://segment",
                        serde_json::json!({
                            "text": text,
                            "fullText": full_text,
                        }),
                    );
                    if let Ok(mut last) = last_full_text_out.lock() {
                        *last = full_text.clone();
                    }
                }
            }
            Err(e) => {
                eprintln!("Dictation transcription error: {}", e);
            }
        }

        // If buffer exceeds max, commit current transcription and start fresh
        if accumulated_raw.len() > max_raw_samples {
            let mono_16k = convert_to_mono_16k(&accumulated_raw, channels, source_rate);
            if let Ok(text) = transcriber.transcribe_samples(&mono_16k, language) {
                let text = text.trim().to_string();
                if !text.is_empty() {
                    committed_segments.push(text.clone());
                    if let Ok(mut sink) = committed_out.lock() {
                        sink.push(text);
                    }
                }
            }
            accumulated_raw.clear();
            last_transcribed_len = 0;
        }
    }

    // Final transcription of remaining audio
    if let Ok(mut buf) = buffer.lock() {
        accumulated_raw.extend(buf.drain(..));
    }

    if accumulated_raw.len() > raw_samples_per_second {
        let mono_16k = convert_to_mono_16k(&accumulated_raw, channels, source_rate);
        if let Ok(text) = transcriber.transcribe_samples(&mono_16k, language) {
            let text = text.trim().to_string();
            if !text.is_empty() {
                let full_text = if committed_segments.is_empty() {
                    text.clone()
                } else {
                    format!("{} {}", committed_segments.join(" "), text)
                };
                let _ = app_handle.emit(
                    "dictation://segment",
                    serde_json::json!({
                        "text": text,
                        "fullText": full_text,
                    }),
                );
                if let Ok(mut last) = last_full_text_out.lock() {
                    *last = full_text;
                }
                committed_segments.push(text.clone());
                if let Ok(mut sink) = committed_out.lock() {
                    sink.push(text);
                }
            }
        }
    }
}
