use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, SampleFormat, Stream, StreamConfig};
use std::path::PathBuf;
use std::process::{Child, Command};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use super::mix::mix_wav_files;
use super::wav_writer::{AudioWavWriter, WavWriterHandle};

pub struct StopResult {
    pub mic_path: PathBuf,
    pub system_path: PathBuf,
    pub output_path: PathBuf,
    pub pw_child: Option<Child>,
}

/// Heavy phase of stopping: waits for pw-record and mixes WAV files.
/// Safe to call from any thread (all fields are Send).
pub fn finalize_recording(mut result: StopResult) -> Result<PathBuf, String> {
    if let Some(ref mut child) = result.pw_child {
        match child.wait() {
            Ok(status) if !status.success() => {
                eprintln!(
                    "Warning: pw-record exited with status: {}",
                    status
                );
            }
            Err(e) => {
                eprintln!("Warning: failed to wait for pw-record: {}", e);
            }
            _ => {}
        }
    }

    if result.system_path.exists() {
        mix_wav_files(&result.mic_path, &result.system_path, &result.output_path)?;
        if let Err(e) = std::fs::remove_file(&result.mic_path) {
            eprintln!("Warning: failed to remove temp mic file {:?}: {}", result.mic_path, e);
        }
        if let Err(e) = std::fs::remove_file(&result.system_path) {
            eprintln!("Warning: failed to remove temp system file {:?}: {}", result.system_path, e);
        }
    } else {
        std::fs::rename(&result.mic_path, &result.output_path)
            .map_err(|e| format!("Failed to rename mic recording: {}", e))?;
    }

    Ok(result.output_path)
}

pub struct AudioCapture {
    output_path: PathBuf,
    mic_path: PathBuf,
    system_path: PathBuf,
    streams: Vec<Stream>,
    wav_writer: Option<AudioWavWriter>,
    pw_record: Option<Child>,
    write_error: Arc<AtomicBool>,
}

impl AudioCapture {
    pub fn new(output_path: PathBuf) -> Self {
        let mic_path = output_path.with_extension("mic.wav");
        let system_path = output_path.with_extension("sys.wav");
        Self {
            output_path,
            mic_path,
            system_path,
            streams: Vec::new(),
            wav_writer: None,
            pw_record: None,
            write_error: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn start(&mut self) -> Result<(), String> {
        let host = cpal::default_host();

        let mic_device = host
            .default_input_device()
            .ok_or("No input device available")?;

        let mic_config = mic_device
            .default_input_config()
            .map_err(|e| format!("Failed to get input config: {}", e))?;

        let sample_rate = mic_config.sample_rate().0;
        let channels = mic_config.channels();

        let wav_writer = AudioWavWriter::new(&self.mic_path, sample_rate, channels)?;

        let mic_stream = Self::build_input_stream(
            &mic_device,
            &mic_config.clone().into(),
            mic_config.sample_format(),
            wav_writer.writer_handle(),
            self.write_error.clone(),
        )?;
        mic_stream
            .play()
            .map_err(|e| format!("Failed to play mic stream: {}", e))?;
        self.streams.push(mic_stream);
        self.wav_writer = Some(wav_writer);

        // Start pw-record for system audio (best-effort)
        match Self::start_pw_record(&self.system_path, sample_rate, channels) {
            Some(child) => self.pw_record = Some(child),
            None => {
                eprintln!("Warning: system audio capture unavailable, recording mic only");
            }
        }

        Ok(())
    }

    fn start_pw_record(path: &PathBuf, sample_rate: u32, channels: u16) -> Option<Child> {
        let sink_serial = Self::get_default_sink_serial()?;

        Command::new("pw-record")
            .arg("--target")
            .arg(sink_serial)
            .arg("--rate")
            .arg(sample_rate.to_string())
            .arg("--channels")
            .arg(channels.to_string())
            .arg("--format")
            .arg("s16")
            .arg(path.as_os_str())
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .ok()
    }

    fn get_default_sink_serial() -> Option<String> {
        let output = Command::new("wpctl")
            .args(["inspect", "@DEFAULT_AUDIO_SINK@"])
            .output()
            .ok()?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            if line.contains("object.serial") {
                return line.split('"').nth(1).map(|s| s.to_string());
            }
        }
        None
    }

    /// Fast phase: stops audio streams and signals pw-record to exit.
    /// Returns a StopResult that can be finalized on a background thread.
    pub fn stop_streams(&mut self) -> Result<StopResult, String> {
        self.streams.clear();
        if let Some(writer) = self.wav_writer.take() {
            writer.finalize()?;
        }

        // Signal pw-record to stop (non-blocking, just sends SIGTERM)
        if let Some(ref child) = self.pw_record {
            unsafe {
                libc::kill(child.id() as i32, libc::SIGTERM);
            }
        }

        if self.write_error.load(Ordering::Relaxed) {
            return Err("Audio recording encountered write errors. The recording may be incomplete or corrupted.".to_string());
        }

        Ok(StopResult {
            mic_path: self.mic_path.clone(),
            system_path: self.system_path.clone(),
            output_path: self.output_path.clone(),
            pw_child: self.pw_record.take(),
        })
    }

    fn build_input_stream(
        device: &Device,
        config: &StreamConfig,
        sample_format: SampleFormat,
        writer: WavWriterHandle,
        error_flag: Arc<AtomicBool>,
    ) -> Result<Stream, String> {
        let stream_error_flag = error_flag.clone();
        let err_callback = move |_err: cpal::StreamError| {
            stream_error_flag.store(true, Ordering::Relaxed);
        };

        match sample_format {
            SampleFormat::I16 => device
                .build_input_stream(
                    config,
                    move |data: &[i16], _| {
                        if error_flag.load(Ordering::Relaxed) {
                            return;
                        }
                        let Ok(mut guard) = writer.lock() else {
                            error_flag.store(true, Ordering::Relaxed);
                            return;
                        };
                        if let Some(ref mut w) = *guard {
                            for &sample in data {
                                if w.write_sample(sample).is_err() {
                                    error_flag.store(true, Ordering::Relaxed);
                                    return;
                                }
                            }
                        }
                    },
                    err_callback,
                    None,
                )
                .map_err(|e| format!("Failed to build input stream: {}", e)),
            SampleFormat::F32 => device
                .build_input_stream(
                    config,
                    move |data: &[f32], _| {
                        if error_flag.load(Ordering::Relaxed) {
                            return;
                        }
                        let Ok(mut guard) = writer.lock() else {
                            error_flag.store(true, Ordering::Relaxed);
                            return;
                        };
                        if let Some(ref mut w) = *guard {
                            for &sample in data {
                                let sample_i16 = (sample * i16::MAX as f32) as i16;
                                if w.write_sample(sample_i16).is_err() {
                                    error_flag.store(true, Ordering::Relaxed);
                                    return;
                                }
                            }
                        }
                    },
                    err_callback,
                    None,
                )
                .map_err(|e| format!("Failed to build input stream: {}", e)),
            format => Err(format!("Unsupported sample format: {:?}", format)),
        }
    }
}
