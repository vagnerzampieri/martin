use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, SampleFormat, Stream, StreamConfig};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use super::wav_writer::{AudioWavWriter, WavWriterHandle};

pub struct AudioCapture {
    output_path: PathBuf,
    streams: Vec<Stream>,
    wav_writer: Option<AudioWavWriter>,
    write_error: Arc<AtomicBool>,
}

impl AudioCapture {
    pub fn new(output_path: PathBuf) -> Self {
        Self {
            output_path,
            streams: Vec::new(),
            wav_writer: None,
            write_error: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn start(&mut self) -> Result<(), String> {
        let host = cpal::default_host();

        // Get the default input device (microphone)
        let input_device = host
            .default_input_device()
            .ok_or("No input device available")?;

        let config = input_device
            .default_input_config()
            .map_err(|e| format!("Failed to get input config: {}", e))?;

        let sample_rate = config.sample_rate().0;
        let channels = config.channels();

        let wav_writer = AudioWavWriter::new(&self.output_path, sample_rate, channels)?;
        let writer_handle = wav_writer.writer_handle();

        let sample_format = config.sample_format();
        let stream = Self::build_input_stream(
            &input_device,
            &config.into(),
            sample_format,
            writer_handle,
            self.write_error.clone(),
        )?;

        stream.play().map_err(|e| format!("Failed to play stream: {}", e))?;
        self.streams.push(stream);
        self.wav_writer = Some(wav_writer);

        Ok(())
    }

    pub fn stop(&mut self) -> Result<PathBuf, String> {
        // Drop streams to stop recording
        self.streams.clear();

        // Finalize the WAV file
        if let Some(writer) = self.wav_writer.take() {
            writer.finalize()?;
        }

        if self.write_error.load(Ordering::Relaxed) {
            return Err("Audio recording encountered write errors. The recording may be incomplete or corrupted.".to_string());
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
