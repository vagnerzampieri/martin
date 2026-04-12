use std::path::Path;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

pub struct Transcriber {
    ctx: WhisperContext,
}

impl Transcriber {
    pub fn new(model_path: &Path) -> Result<Self, String> {
        let ctx = WhisperContext::new_with_params(
            model_path.to_str().ok_or("Invalid model path")?,
            WhisperContextParameters::default(),
        )
        .map_err(|e| format!("Failed to load Whisper model: {}", e))?;

        Ok(Self { ctx })
    }

    pub fn transcribe(&self, audio_path: &Path, language: &str) -> Result<String, String> {
        let samples = self.load_wav_as_mono_f32(audio_path)?;

        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_language(Some(language));
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(true);

        let mut state = self
            .ctx
            .create_state()
            .map_err(|e| format!("Failed to create state: {}", e))?;

        state
            .full(params, &samples)
            .map_err(|e| format!("Transcription failed: {}", e))?;

        let num_segments = state
            .full_n_segments()
            .map_err(|e| format!("Failed to get segments: {}", e))?;

        let mut text = String::new();
        for i in 0..num_segments {
            if let Ok(segment) = state.full_get_segment_text(i) {
                text.push_str(&segment);
                text.push('\n');
            }
        }

        Ok(text.trim().to_string())
    }

    fn load_wav_as_mono_f32(&self, path: &Path) -> Result<Vec<f32>, String> {
        let mut reader =
            hound::WavReader::open(path).map_err(|e| format!("Failed to open WAV: {}", e))?;

        let spec = reader.spec();
        let samples: Vec<f32> = match spec.sample_format {
            hound::SampleFormat::Int => reader
                .samples::<i16>()
                .filter_map(|s| s.ok())
                .map(|s| s as f32 / i16::MAX as f32)
                .collect(),
            hound::SampleFormat::Float => {
                reader.samples::<f32>().filter_map(|s| s.ok()).collect()
            }
        };

        // Convert to mono if stereo
        if spec.channels == 2 {
            Ok(samples
                .chunks(2)
                .map(|chunk| (chunk[0] + chunk.get(1).copied().unwrap_or(0.0)) / 2.0)
                .collect())
        } else {
            Ok(samples)
        }
    }
}
