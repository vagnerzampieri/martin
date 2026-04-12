use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, SampleFormat, Stream, StreamConfig};
use std::path::PathBuf;
use std::process::{Child, Command};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use super::mix::mix_wav_files;
use super::wav_writer::{AudioWavWriter, WavWriterHandle};

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
        mic_stream.play().map_err(|e| format!("Failed to play mic stream: {}", e))?;
        self.streams.push(mic_stream);
        self.wav_writer = Some(wav_writer);

        // Start pw-record for system audio (best-effort)
        self.pw_record = Self::start_pw_record(&self.system_path, sample_rate, channels);

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

    pub fn stop(&mut self) -> Result<PathBuf, String> {
        // Stop mic stream
        self.streams.clear();
        if let Some(writer) = self.wav_writer.take() {
            writer.finalize()?;
        }

        // Stop pw-record gracefully (SIGTERM lets it finalize the WAV header)
        if let Some(mut child) = self.pw_record.take() {
            unsafe {
                libc::kill(child.id() as i32, libc::SIGTERM);
            }
            let _ = child.wait();
        }

        if self.write_error.load(Ordering::Relaxed) {
            return Err("Audio recording encountered write errors. The recording may be incomplete or corrupted.".to_string());
        }

        // Mix mic + system audio if system WAV exists
        if self.system_path.exists() {
            mix_wav_files(&self.mic_path, &self.system_path, &self.output_path)?;
            let _ = std::fs::remove_file(&self.mic_path);
            let _ = std::fs::remove_file(&self.system_path);
        } else {
            std::fs::rename(&self.mic_path, &self.output_path)
                .map_err(|e| format!("Failed to rename mic recording: {}", e))?;
        }

        Ok(self.output_path.clone())
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

