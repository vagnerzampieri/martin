use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, Stream};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use tauri::Emitter;

use crate::audio::wav_writer::AudioWavWriter;
use crate::transcribe::whisper::Transcriber;
use std::path::PathBuf;

/// Externally visible dictation state. Encoded as `u8` so it can live in an
/// `AtomicU8` shared between the capture, transcription, and level-poller threads.
/// Values are stable — the frontend depends on them.
pub const STATE_LISTENING: u8 = 0;
pub const STATE_PROCESSING: u8 = 1;
pub const STATE_PAUSED: u8 = 2;

pub fn state_label(state: u8) -> &'static str {
    match state {
        STATE_PROCESSING => "processing",
        STATE_PAUSED => "paused",
        _ => "listening",
    }
}

const WHISPER_SAMPLE_RATE: u32 = 16000;
const POLL_INTERVAL_MS: u64 = 500;
const MIN_SECONDS_TO_TRANSCRIBE: usize = 2;
const MAX_BUFFER_SECONDS: usize = 120;
/// Polls of silence before we treat the gap as a paragraph boundary.
/// 10 × 500ms = ~5s — long enough that normal think-pauses don't trigger.
const PARAGRAPH_PAUSE_POLLS: u32 = 10;
/// Polls of silence before we flip the UI state to PAUSED. Higher than
/// the natural pause between sentences so people aren't constantly told
/// "paused" mid-thought.
const PAUSED_STATE_POLLS: u32 = 4;

pub struct DictationSession {
    stream: Option<Stream>,
    audio_buffer: Arc<Mutex<Vec<f32>>>,
    running: Arc<AtomicBool>,
    source_rate: u32,
    channels: u16,
    committed: Arc<Mutex<Vec<String>>>,
    last_full_text: Arc<Mutex<String>>,
    final_audio: Arc<Mutex<Vec<f32>>>,
    /// Last-known peak amplitude of the audio callback, as f32 bits in u32.
    /// Updated by the cpal callback, read by the level-poller thread.
    last_peak_bits: Arc<AtomicU32>,
    /// Current session state (see `STATE_*` constants).
    state: Arc<AtomicU8>,
    /// Length of `accumulated_raw` (raw interleaved samples) at the time of
    /// the last successful whisper pass. Reset to 0 after a rollover clears
    /// the buffer. Read by `stop_dictation` to compute the un-transcribed tail.
    last_transcribed_raw_len: Arc<AtomicUsize>,
    /// True when a paragraph rollover happened but the next emission has not
    /// yet consumed the break. Read by `stop_dictation` so the finalize tail
    /// uses a paragraph separator instead of a single space.
    pending_paragraph_break_flag: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
    level_worker: Option<JoinHandle<()>>,
    wav_writer: Option<AudioWavWriter>,
    audio_path: Option<PathBuf>,
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
            final_audio: Arc::new(Mutex::new(Vec::new())),
            last_peak_bits: Arc::new(AtomicU32::new(0)),
            state: Arc::new(AtomicU8::new(STATE_LISTENING)),
            last_transcribed_raw_len: Arc::new(AtomicUsize::new(0)),
            pending_paragraph_break_flag: Arc::new(AtomicBool::new(false)),
            worker: None,
            level_worker: None,
            wav_writer: None,
            audio_path: None,
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

    pub fn start(&mut self, audio_path: PathBuf) -> Result<(), String> {
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

        let writer = AudioWavWriter::new(&audio_path, self.source_rate, self.channels)?;
        let writer_handle = writer.writer_handle();
        self.wav_writer = Some(writer);
        self.audio_path = Some(audio_path);

        let peak_for_i16 = self.last_peak_bits.clone();
        let peak_for_f32 = self.last_peak_bits.clone();
        let writer_i16 = writer_handle.clone();
        let writer_f32 = writer_handle.clone();
        let stream = match sample_format {
            SampleFormat::I16 => device
                .build_input_stream(
                    &config.into(),
                    move |data: &[i16], _| {
                        let mut peak: f32 = 0.0;
                        if let Ok(mut buf) = buffer.lock() {
                            for &s in data {
                                let f = s as f32 / i16::MAX as f32;
                                let a = f.abs();
                                if a > peak {
                                    peak = a;
                                }
                                buf.push(f);
                            }
                        }
                        peak_for_i16.store(peak.to_bits(), Ordering::Relaxed);
                        if let Ok(mut guard) = writer_i16.lock() {
                            if let Some(ref mut w) = *guard {
                                for &s in data {
                                    let _ = w.write_sample(s);
                                }
                            }
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
                        let mut peak: f32 = 0.0;
                        if let Ok(mut buf) = buffer.lock() {
                            for &s in data {
                                let a = s.abs();
                                if a > peak {
                                    peak = a;
                                }
                                buf.push(s);
                            }
                        }
                        peak_for_f32.store(peak.to_bits(), Ordering::Relaxed);
                        if let Ok(mut guard) = writer_f32.lock() {
                            if let Some(ref mut w) = *guard {
                                for &s in data {
                                    #[allow(clippy::cast_possible_truncation)]
                                    let _ = w.write_sample((s * i16::MAX as f32) as i16);
                                }
                            }
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

    pub fn final_audio(&self) -> Arc<Mutex<Vec<f32>>> {
        self.final_audio.clone()
    }

    pub fn last_peak_bits(&self) -> Arc<AtomicU32> {
        self.last_peak_bits.clone()
    }

    pub fn state(&self) -> Arc<AtomicU8> {
        self.state.clone()
    }

    pub fn last_transcribed_raw_len(&self) -> Arc<AtomicUsize> {
        self.last_transcribed_raw_len.clone()
    }

    pub fn pending_paragraph_break_flag(&self) -> Arc<AtomicBool> {
        self.pending_paragraph_break_flag.clone()
    }

    pub fn set_worker(&mut self, handle: JoinHandle<()>) {
        self.worker = Some(handle);
    }

    pub fn set_level_worker(&mut self, handle: JoinHandle<()>) {
        self.level_worker = Some(handle);
    }

    /// Stops the audio stream, signals workers, and joins them.
    pub fn stop_and_join(&mut self) {
        self.running.store(false, Ordering::Release);
        self.stream.take();
        if let Some(handle) = self.worker.take() {
            let _ = handle.join();
        }
        if let Some(handle) = self.level_worker.take() {
            let _ = handle.join();
        }
        if let Some(writer) = self.wav_writer.take() {
            if let Err(e) = writer.finalize() {
                eprintln!("[dictation] failed to finalize WAV: {}", e);
            }
        }
    }

    #[allow(dead_code)]
    pub fn audio_path(&self) -> Option<PathBuf> {
        self.audio_path.clone()
    }

    /// Take ownership of the audio captured since the last rollover.
    /// Must be called only AFTER `stop_and_join` — otherwise the live
    /// loop may not have handed off its accumulated audio yet.
    pub fn take_final_audio(&self) -> Vec<f32> {
        self.final_audio
            .lock()
            .map(|mut g| std::mem::take(&mut *g))
            .unwrap_or_default()
    }
}

pub fn convert_to_mono_16k(samples: &[f32], channels: u16, source_rate: u32) -> Vec<f32> {
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
/// maximum Whisper accuracy. Skips whisper passes during silence and
/// updates the shared session state (listening/processing/paused).
/// At rollover, commits the current text as a segment and starts a fresh buffer.
///
/// On exit, hands off the post-rollover raw audio into `final_audio_out`
/// so the finalize worker can re-transcribe it with progress callbacks.
#[allow(clippy::too_many_arguments)]
pub fn run_transcription_loop(
    buffer: Arc<Mutex<Vec<f32>>>,
    running: Arc<AtomicBool>,
    committed_out: Arc<Mutex<Vec<String>>>,
    last_full_text_out: Arc<Mutex<String>>,
    final_audio_out: Arc<Mutex<Vec<f32>>>,
    state: Arc<AtomicU8>,
    last_transcribed_raw_len_atom: Arc<AtomicUsize>,
    pending_paragraph_break_atom: Arc<AtomicBool>,
    transcriber: &Transcriber,
    language: &str,
    initial_prompt: Option<String>,
    source_rate: u32,
    channels: u16,
    partial_id: i64,
    store: std::sync::Arc<Mutex<crate::db::store::Store>>,
    app_handle: tauri::AppHandle,
) {
    let mut committed_segments: Vec<String> = Vec::new();
    let mut accumulated_raw: Vec<f32> = Vec::new();
    let mut last_transcribed_len: usize = 0;
    let mut consecutive_silent_polls: u32 = 0;
    let mut pending_paragraph_break: bool = false;
    let mut last_persist = std::time::Instant::now();
    const PERSIST_INTERVAL_MS: u128 = 5000;
    let started_at = std::time::Instant::now();

    let raw_samples_per_second = source_rate as usize * channels as usize;
    let min_raw_samples = raw_samples_per_second * MIN_SECONDS_TO_TRANSCRIBE;
    let max_raw_samples = raw_samples_per_second * MAX_BUFFER_SECONDS;

    while running.load(Ordering::Acquire) {
        std::thread::sleep(std::time::Duration::from_millis(POLL_INTERVAL_MS));

        // Drain new samples from the shared buffer
        let new_chunk_start = accumulated_raw.len();
        if let Ok(mut buf) = buffer.lock() {
            accumulated_raw.extend(buf.drain(..));
        }
        let new_chunk_end = accumulated_raw.len();

        // Compute RMS over the NEW chunk only. We need the new-chunk view to
        // decide whether speech is happening right now, independent of how much
        // total audio we have accumulated.
        let new_chunk_rms = if new_chunk_end > new_chunk_start {
            crate::vad::rms(&accumulated_raw[new_chunk_start..new_chunk_end])
        } else {
            0.0
        };

        let chunk_is_silent = crate::vad::is_silent(new_chunk_rms);
        if chunk_is_silent {
            consecutive_silent_polls = consecutive_silent_polls.saturating_add(1);
        } else {
            consecutive_silent_polls = 0;
        }

        // PAUSED after PAUSED_STATE_POLLS consecutive silent polls. Switch back
        // to LISTENING the moment speech resumes. PROCESSING is set during
        // the whisper pass below.
        if chunk_is_silent && consecutive_silent_polls >= PAUSED_STATE_POLLS {
            state.store(STATE_PAUSED, Ordering::Release);

            // A pause this long is a paragraph boundary. Commit the current
            // pass text (if any) as a segment, clear the buffer, and queue a
            // paragraph break for the NEXT non-empty emission so the break
            // appears between paragraphs, not before an empty next chunk.
            if consecutive_silent_polls == PARAGRAPH_PAUSE_POLLS && !accumulated_raw.is_empty() {
                let mono_16k = convert_to_mono_16k(&accumulated_raw, channels, source_rate);
                if let Ok(text) =
                    transcriber.transcribe_samples(&mono_16k, language, initial_prompt.as_deref())
                {
                    let text = text.trim().to_string();
                    if !text.is_empty() {
                        committed_segments.push(text.clone());
                        if let Ok(mut sink) = committed_out.lock() {
                            sink.push(text);
                        }
                        pending_paragraph_break = true;
                        pending_paragraph_break_atom.store(true, Ordering::Release);

                        // Reflect the just-committed state in `last_full_text_out`
                        // so a stop_dictation that happens before the next emission
                        // (i.e. before new audio is transcribed) still has the
                        // full text up to this paragraph. Trailing "\n\n" marks
                        // that any tail audio is a new paragraph.
                        let joined = committed_segments.join("\n\n");
                        let normalized = crate::postprocess::normalize(&joined);
                        let marker = format!("{}\n\n", normalized);
                        if let Ok(mut last) = last_full_text_out.lock() {
                            *last = marker;
                        }
                    }
                }
                accumulated_raw.clear();
                last_transcribed_len = 0;
                last_transcribed_raw_len_atom.store(0, Ordering::Release);
            }
        } else if !chunk_is_silent && state.load(Ordering::Acquire) == STATE_PAUSED {
            state.store(STATE_LISTENING, Ordering::Release);
        }

        // Only transcribe if we have enough audio AND there is new audio since last run
        if accumulated_raw.len() < min_raw_samples || accumulated_raw.len() == last_transcribed_len
        {
            continue;
        }

        // Silence gate: if the freshly added chunk is silent AND we already
        // produced text recently, don't waste CPU re-transcribing.
        if chunk_is_silent && last_transcribed_len > 0 {
            continue;
        }

        let mono_16k = convert_to_mono_16k(&accumulated_raw, channels, source_rate);
        last_transcribed_len = accumulated_raw.len();
        last_transcribed_raw_len_atom.store(last_transcribed_len, Ordering::Release);

        state.store(STATE_PROCESSING, Ordering::Release);
        let pass_text =
            match transcriber.transcribe_samples(&mono_16k, language, initial_prompt.as_deref()) {
                Ok(text) => text.trim().to_string(),
                Err(e) => {
                    eprintln!("Dictation transcription error: {}", e);
                    String::new()
                }
            };
        // Back to LISTENING unless the silence detector has already flipped us
        // to PAUSED in a subsequent (unlikely, this is the same thread) tick.
        let _ = state.compare_exchange(
            STATE_PROCESSING,
            STATE_LISTENING,
            Ordering::AcqRel,
            Ordering::Acquire,
        );

        if !pass_text.is_empty() {
            let separator = if pending_paragraph_break { "\n\n" } else { " " };
            let stable_text = committed_segments.join(separator);
            let raw_full = if stable_text.is_empty() {
                pass_text.clone()
            } else {
                format!("{}{}{}", stable_text, separator, pass_text)
            };
            // Reset the flag — it has been consumed by this emission.
            pending_paragraph_break = false;
            pending_paragraph_break_atom.store(false, Ordering::Release);
            let full_text = crate::postprocess::normalize(&raw_full);
            let stable_normalized = crate::postprocess::normalize(&stable_text);
            let provisional_normalized = crate::postprocess::normalize(&pass_text);

            let _ = app_handle.emit(
                "dictation://segment",
                serde_json::json!({
                    "stableText": stable_normalized,
                    "provisionalText": provisional_normalized,
                    "fullText": full_text,
                }),
            );
            if let Ok(mut last) = last_full_text_out.lock() {
                *last = full_text.clone();
            }
            if last_persist.elapsed().as_millis() >= PERSIST_INTERVAL_MS {
                last_persist = std::time::Instant::now();
                if let Ok(s) = store.lock() {
                    let elapsed_secs = started_at.elapsed().as_secs_f64();
                    let _ = s.update_text(partial_id, &full_text, elapsed_secs);
                }
            }
        }

        // If buffer exceeds max, commit the text we just produced and start fresh.
        // Size-based rollover is mid-thought (not a paragraph boundary), so we
        // keep `pending_paragraph_break` unchanged and update `last_full_text_out`
        // with a single space separator at the boundary.
        if accumulated_raw.len() > max_raw_samples {
            if !pass_text.is_empty() {
                committed_segments.push(pass_text.clone());
                if let Ok(mut sink) = committed_out.lock() {
                    sink.push(pass_text);
                }
                let joined = committed_segments.join("\n\n");
                let normalized = crate::postprocess::normalize(&joined);
                let marker = format!("{} ", normalized);
                if let Ok(mut last) = last_full_text_out.lock() {
                    *last = marker;
                }
            }
            accumulated_raw.clear();
            last_transcribed_len = 0;
            last_transcribed_raw_len_atom.store(0, Ordering::Release);
        }
    }

    // Final drain of any audio captured between the last poll and the stop signal.
    if let Ok(mut buf) = buffer.lock() {
        accumulated_raw.extend(buf.drain(..));
    }

    if let Ok(mut sink) = final_audio_out.lock() {
        *sink = accumulated_raw;
    }
}

const LEVEL_EMIT_INTERVAL_MS: u64 = 100;

/// Emits `dictation://level` (audio peak amplitude, 0.0–1.0) every
/// LEVEL_EMIT_INTERVAL_MS and `dictation://state` whenever the shared
/// state atomic changes. Runs on its own thread so UI updates stay
/// responsive even while whisper is busy on the transcription thread.
pub fn run_level_emitter(
    running: Arc<AtomicBool>,
    last_peak_bits: Arc<AtomicU32>,
    state: Arc<AtomicU8>,
    app_handle: tauri::AppHandle,
) {
    let mut last_emitted_state: u8 = u8::MAX;

    while running.load(Ordering::Acquire) {
        std::thread::sleep(std::time::Duration::from_millis(LEVEL_EMIT_INTERVAL_MS));

        let peak = f32::from_bits(last_peak_bits.load(Ordering::Relaxed));
        // Reset peak so the next interval reflects only fresh audio.
        last_peak_bits.store(0u32, Ordering::Relaxed);

        let _ = app_handle.emit("dictation://level", serde_json::json!({ "peak": peak }));

        let current = state.load(Ordering::Acquire);
        if current != last_emitted_state {
            last_emitted_state = current;
            let _ = app_handle.emit(
                "dictation://state",
                serde_json::json!({ "state": state_label(current) }),
            );
        }
    }
}
