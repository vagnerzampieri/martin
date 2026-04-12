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

        let mic_device = host
            .default_input_device()
            .ok_or("No input device available")?;

        let mic_config = mic_device
            .default_input_config()
            .map_err(|e| format!("Failed to get input config: {}", e))?;

        let sample_rate = mic_config.sample_rate().0;
        let channels = mic_config.channels();

        let wav_writer = AudioWavWriter::new(&self.output_path, sample_rate, channels)?;

        // Start microphone stream
        let mic_stream = Self::build_input_stream(
            &mic_device,
            &mic_config.clone().into(),
            mic_config.sample_format(),
            wav_writer.writer_handle(),
            self.write_error.clone(),
        )?;
        mic_stream.play().map_err(|e| format!("Failed to play mic stream: {}", e))?;
        self.streams.push(mic_stream);

        // Try to find and start monitor stream (system audio)
        if let Some(monitor_device) = Self::find_monitor_device(&host) {
            match Self::start_monitor_stream(
                &monitor_device,
                sample_rate,
                wav_writer.writer_handle(),
                self.write_error.clone(),
            ) {
                Ok(stream) => self.streams.push(stream),
                Err(e) => eprintln!("Warning: could not start monitor stream: {}", e),
            }
        }

        self.wav_writer = Some(wav_writer);
        Ok(())
    }

    fn find_monitor_device(host: &cpal::Host) -> Option<Device> {
        let devices: Vec<(Device, String)> = host
            .input_devices()
            .ok()?
            .filter_map(|d| {
                let name = d.name().ok()?;
                Some((d, name))
            })
            .collect();

        let names: Vec<&str> = devices.iter().map(|(_, n)| n.as_str()).collect();
        let monitor_name = find_monitor_device_name(&names)?.to_string();

        devices
            .into_iter()
            .find(|(_, name)| *name == monitor_name)
            .map(|(device, _)| device)
    }

    fn start_monitor_stream(
        device: &Device,
        target_sample_rate: u32,
        writer: WavWriterHandle,
        error_flag: Arc<AtomicBool>,
    ) -> Result<Stream, String> {
        let config = device
            .default_input_config()
            .map_err(|e| format!("Failed to get monitor config: {}", e))?;

        let monitor_config = StreamConfig {
            channels: config.channels(),
            sample_rate: cpal::SampleRate(target_sample_rate),
            buffer_size: cpal::BufferSize::Default,
        };

        let stream = Self::build_input_stream(
            device,
            &monitor_config,
            config.sample_format(),
            writer,
            error_flag,
        )?;
        stream.play().map_err(|e| format!("Failed to play monitor stream: {}", e))?;
        Ok(stream)
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

fn find_monitor_device_name<'a>(names: &[&'a str]) -> Option<&'a str> {
    names.iter().find(|n| n.contains(".monitor")).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_monitor_name_returns_first_monitor_source() {
        let names = vec![
            "alsa_input.pci-0000_00_1f.3.analog-stereo",
            "alsa_output.pci-0000_00_1f.3.analog-stereo.monitor",
            "alsa_output.hdmi-stereo.monitor",
        ];
        let result = find_monitor_device_name(&names);
        assert_eq!(result, Some("alsa_output.pci-0000_00_1f.3.analog-stereo.monitor"));
    }

    #[test]
    fn find_monitor_name_returns_none_when_no_monitor() {
        let names = vec![
            "alsa_input.pci-0000_00_1f.3.analog-stereo",
            "bluez_input.some-bluetooth-mic",
        ];
        let result = find_monitor_device_name(&names);
        assert!(result.is_none());
    }

    #[test]
    fn find_monitor_name_returns_none_for_empty_list() {
        let names: Vec<&str> = vec![];
        let result = find_monitor_device_name(&names);
        assert!(result.is_none());
    }
}
