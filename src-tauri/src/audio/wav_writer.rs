use hound::{WavSpec, WavWriter};
use std::path::Path;
use std::sync::{Arc, Mutex};

pub type WavWriterHandle = Arc<Mutex<Option<WavWriter<std::io::BufWriter<std::fs::File>>>>>;

pub struct AudioWavWriter {
    writer: WavWriterHandle,
}

impl AudioWavWriter {
    pub fn new(path: &Path, sample_rate: u32, channels: u16) -> Result<Self, String> {
        let spec = WavSpec {
            channels,
            sample_rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };

        let writer = WavWriter::create(path, spec)
            .map_err(|e| format!("Failed to create WAV file: {}", e))?;

        Ok(Self {
            writer: Arc::new(Mutex::new(Some(writer))),
        })
    }

    pub fn writer_handle(&self) -> WavWriterHandle {
        self.writer.clone()
    }

    pub fn finalize(&self) -> Result<(), String> {
        let mut guard = self.writer.lock().map_err(|e| e.to_string())?;
        if let Some(writer) = guard.take() {
            writer
                .finalize()
                .map_err(|e| format!("Failed to finalize WAV: {}", e))?;
        }
        Ok(())
    }
}
