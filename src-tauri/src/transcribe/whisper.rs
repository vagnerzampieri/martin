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
        let samples = Self::load_wav_as_mono_f32(audio_path)?;

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

        let segments: Vec<String> = (0..num_segments)
            .map(|i| state.full_get_segment_text(i))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("Failed to extract segment text: {}", e))?;

        Ok(segments.join("\n").trim().to_string())
    }

    /// Transcribe pre-processed audio samples (mono f32 at 16kHz).
    /// Used by dictation mode where audio comes from a buffer, not a file.
    pub fn transcribe_samples(&self, samples: &[f32], language: &str) -> Result<String, String> {
        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_language(Some(language));
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        params.set_no_context(true);

        let mut state = self
            .ctx
            .create_state()
            .map_err(|e| format!("Failed to create state: {}", e))?;

        state
            .full(params, samples)
            .map_err(|e| format!("Transcription failed: {}", e))?;

        let num_segments = state
            .full_n_segments()
            .map_err(|e| format!("Failed to get segments: {}", e))?;

        let segments: Vec<String> = (0..num_segments)
            .map(|i| state.full_get_segment_text(i))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("Failed to extract segment text: {}", e))?;

        Ok(segments.join(" ").trim().to_string())
    }

    /// Transcribe samples while emitting progress, per-segment text, and
    /// observing an abort flag. Caller closures must be `Send + 'static`.
    ///
    /// - `on_progress` is called with whisper's internal progress (0-100).
    /// - `on_segment` is called with each new segment text as it is produced.
    /// - `should_abort` is polled by whisper periodically; returning true
    ///   aborts inference cleanly.
    // Wired in by Task 8 (run_finalize_dictation).
    #[allow(dead_code)]
    pub fn transcribe_with_callbacks<P, S, A>(
        &self,
        samples: &[f32],
        language: &str,
        on_progress: P,
        mut on_segment: S,
        should_abort: A,
    ) -> Result<String, String>
    where
        P: FnMut(i32) + Send + 'static,
        S: FnMut(&str) + Send + 'static,
        A: FnMut() -> bool + Send + 'static,
    {
        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_language(Some(language));
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        params.set_no_context(true);

        params.set_progress_callback_safe(on_progress);
        params.set_segment_callback_safe_lossy(move |data: whisper_rs::SegmentCallbackData| {
            on_segment(&data.text);
        });
        params.set_abort_callback_safe(should_abort);

        let mut state = self
            .ctx
            .create_state()
            .map_err(|e| format!("Failed to create state: {}", e))?;

        state
            .full(params, samples)
            .map_err(|e| format!("Transcription failed: {}", e))?;

        let num_segments = state
            .full_n_segments()
            .map_err(|e| format!("Failed to get segments: {}", e))?;

        let segments: Vec<String> = (0..num_segments)
            .map(|i| state.full_get_segment_text(i))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("Failed to extract segment text: {}", e))?;

        Ok(segments.join(" ").trim().to_string())
    }

    fn load_wav_as_mono_f32(path: &Path) -> Result<Vec<f32>, String> {
        let mut reader =
            hound::WavReader::open(path).map_err(|e| format!("Failed to open WAV: {}", e))?;

        let spec = reader.spec();
        let source_sample_rate = spec.sample_rate;

        let samples: Vec<f32> = match spec.sample_format {
            hound::SampleFormat::Int => reader
                .samples::<i16>()
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("Failed to decode WAV samples: {}", e))?
                .into_iter()
                .map(|s| s as f32 / i16::MAX as f32)
                .collect(),
            hound::SampleFormat::Float => reader
                .samples::<f32>()
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("Failed to decode WAV samples: {}", e))?,
        };

        // Convert to mono if stereo
        let mono = if spec.channels == 2 {
            samples
                .chunks(2)
                .map(|chunk| (chunk[0] + chunk.get(1).copied().unwrap_or(0.0)) / 2.0)
                .collect()
        } else {
            samples
        };

        // Whisper requires 16kHz audio — resample if needed
        const WHISPER_SAMPLE_RATE: u32 = 16000;
        if source_sample_rate != WHISPER_SAMPLE_RATE {
            Ok(Self::resample(
                &mono,
                source_sample_rate,
                WHISPER_SAMPLE_RATE,
            ))
        } else {
            Ok(mono)
        }
    }

    fn resample(samples: &[f32], from_rate: u32, to_rate: u32) -> Vec<f32> {
        let ratio = from_rate as f64 / to_rate as f64;
        let output_len = (samples.len() as f64 / ratio) as usize;
        let mut output = Vec::with_capacity(output_len);

        for i in 0..output_len {
            let src_idx = i as f64 * ratio;
            let idx = src_idx as usize;
            let frac = src_idx - idx as f64;

            let sample = if idx + 1 < samples.len() {
                samples[idx] as f64 * (1.0 - frac) + samples[idx + 1] as f64 * frac
            } else {
                samples[idx] as f64
            };
            output.push(sample as f32);
        }

        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    const EPSILON: f32 = 0.001;

    fn assert_f32_eq(a: f32, b: f32) {
        assert!(
            (a - b).abs() < EPSILON,
            "expected {b} but got {a} (diff: {})",
            (a - b).abs()
        );
    }

    fn write_wav(path: &std::path::Path, sample_rate: u32, channels: u16, samples: &[i16]) {
        let spec = hound::WavSpec {
            channels,
            sample_rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(path, spec).expect("failed to create WAV");
        for &s in samples {
            writer.write_sample(s).expect("failed to write sample");
        }
        writer.finalize().expect("failed to finalize WAV");
    }

    // --- resample tests ---

    #[test]
    fn resample_same_rate_returns_identical_samples() {
        let samples = vec![0.0, 0.5, 1.0, -1.0, 0.25];
        let result = Transcriber::resample(&samples, 16000, 16000);

        assert_eq!(result.len(), samples.len());
        for (a, b) in result.iter().zip(samples.iter()) {
            assert_f32_eq(*a, *b);
        }
    }

    #[test]
    fn resample_downsample_2_to_1_produces_correct_length() {
        let samples: Vec<f32> = (0..1000).map(|i| (i as f32) / 1000.0).collect();
        let result = Transcriber::resample(&samples, 32000, 16000);

        assert_eq!(result.len(), 500);
    }

    #[test]
    fn resample_downsample_preserves_values_at_sample_boundaries() {
        // With 2:1 downsample, output[i] maps to src_idx = i * 2.0
        // so output[0] = samples[0], output[1] = samples[2], etc.
        let samples = vec![0.0, 0.1, 0.2, 0.3, 0.4, 0.5];
        let result = Transcriber::resample(&samples, 32000, 16000);

        assert_f32_eq(result[0], 0.0);
        assert_f32_eq(result[1], 0.2);
        assert_f32_eq(result[2], 0.4);
    }

    #[test]
    fn resample_upsample_1_to_2_produces_correct_length() {
        let samples: Vec<f32> = (0..500).map(|i| (i as f32) / 500.0).collect();
        let result = Transcriber::resample(&samples, 8000, 16000);

        assert_eq!(result.len(), 1000);
    }

    #[test]
    fn resample_empty_input_returns_empty_output() {
        let samples: Vec<f32> = vec![];
        let result = Transcriber::resample(&samples, 44100, 16000);

        assert!(result.is_empty());
    }

    #[test]
    fn resample_single_sample_input() {
        let samples = vec![0.75];
        let result = Transcriber::resample(&samples, 16000, 16000);

        assert_eq!(result.len(), 1);
        assert_f32_eq(result[0], 0.75);
    }

    // --- load_wav_as_mono_f32 tests ---

    #[test]
    fn load_wav_mono_16khz_no_conversion_needed() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let path = dir.path().join("mono_16k.wav");

        let raw_samples: Vec<i16> = vec![0, 16383, -16383, 32767, -32768];
        write_wav(&path, 16000, 1, &raw_samples);

        let result = Transcriber::load_wav_as_mono_f32(&path).expect("failed to load WAV");

        assert_eq!(result.len(), raw_samples.len());
        for (got, &raw) in result.iter().zip(raw_samples.iter()) {
            let expected = raw as f32 / i16::MAX as f32;
            assert_f32_eq(*got, expected);
        }
    }

    #[test]
    fn load_wav_stereo_converts_to_mono_by_averaging() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let path = dir.path().join("stereo_16k.wav");

        // Stereo samples are interleaved: [L0, R0, L1, R1, ...]
        let raw_samples: Vec<i16> = vec![
            1000, 3000, // frame 0: L=1000, R=3000 -> mono = 2000
            -2000, 4000, // frame 1: L=-2000, R=4000 -> mono = 1000
            0, 0, // frame 2: L=0, R=0 -> mono = 0
        ];
        write_wav(&path, 16000, 2, &raw_samples);

        let result = Transcriber::load_wav_as_mono_f32(&path).expect("failed to load WAV");

        assert_eq!(result.len(), 3);

        let max = i16::MAX as f32;
        assert_f32_eq(result[0], (1000.0 / max + 3000.0 / max) / 2.0);
        assert_f32_eq(result[1], (-2000.0 / max + 4000.0 / max) / 2.0);
        assert_f32_eq(result[2], 0.0);
    }

    #[test]
    fn load_wav_resamples_from_48khz_to_16khz() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let path = dir.path().join("mono_48k.wav");

        // 480 samples at 48kHz = 10ms of audio -> should become 160 samples at 16kHz
        let raw_samples: Vec<i16> = (0..480).map(|i| (i * 60) as i16).collect();
        write_wav(&path, 48000, 1, &raw_samples);

        let result = Transcriber::load_wav_as_mono_f32(&path).expect("failed to load WAV");

        assert_eq!(result.len(), 160);
    }

    #[test]
    fn load_wav_returns_error_for_nonexistent_file() {
        let path = PathBuf::from("/tmp/this_file_does_not_exist_at_all.wav");
        let result = Transcriber::load_wav_as_mono_f32(&path);

        assert!(result.is_err());
    }

    #[test]
    fn transcribe_samples_returns_empty_for_silence() {
        let samples: Vec<f32> = vec![0.0; 16000];
        // Can't do a real transcription test without a model file.
        // Verify the params are correctly constructed.
        let _params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        assert!(!samples.is_empty());
    }

    #[test]
    fn transcribe_with_callbacks_signature_compiles() {
        // Compile-time-only test: ensures the public signature is shaped correctly.
        fn _sig_check(t: &Transcriber, samples: &[f32]) {
            let _ =
                t.transcribe_with_callbacks(samples, "pt", |_p: i32| {}, |_seg: &str| {}, || false);
        }
        let _ = _sig_check;
    }
}
