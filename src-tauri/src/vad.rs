//! Voice activity detection helpers. Pure functions only — no I/O, no state.
//! Used by the dictation loop to skip whisper passes during silence and to
//! detect paragraph boundaries.

/// RMS (root mean square) amplitude of a slice of mono samples in [-1.0, 1.0].
/// Returns 0.0 for an empty slice.
pub fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum_sq: f32 = samples.iter().map(|s| s * s).sum();
    (sum_sq / samples.len() as f32).sqrt()
}

/// Returns true when `rms_value` is at or below the silence threshold.
/// Threshold tuned for typical laptop mics in a quiet room.
pub const SILENCE_THRESHOLD: f32 = 0.01;

pub fn is_silent(rms_value: f32) -> bool {
    rms_value <= SILENCE_THRESHOLD
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rms_of_empty_slice_is_zero() {
        assert_eq!(rms(&[]), 0.0);
    }

    #[test]
    fn rms_of_silence_is_zero() {
        let samples = vec![0.0_f32; 1000];
        assert_eq!(rms(&samples), 0.0);
    }

    #[test]
    fn rms_of_dc_signal_equals_its_amplitude() {
        let samples = vec![0.5_f32; 100];
        assert!((rms(&samples) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn rms_of_sign_alternating_signal_equals_amplitude() {
        let samples: Vec<f32> = (0..1000).map(|i| if i % 2 == 0 { 0.3 } else { -0.3 }).collect();
        // 0.3 is not exactly representable in f32; allow slightly looser bound
        assert!((rms(&samples) - 0.3).abs() < 1e-5);
    }

    #[test]
    fn is_silent_true_at_and_below_threshold() {
        assert!(is_silent(0.0));
        assert!(is_silent(SILENCE_THRESHOLD));
        assert!(is_silent(SILENCE_THRESHOLD - 0.001));
    }

    #[test]
    fn is_silent_false_above_threshold() {
        assert!(!is_silent(SILENCE_THRESHOLD + 0.001));
        assert!(!is_silent(0.5));
    }
}
