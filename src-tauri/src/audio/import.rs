#[allow(dead_code)]
fn downmix_to_mono(interleaved: &[f32], channels: usize) -> Vec<f32> {
    if channels <= 1 {
        return interleaved.to_vec();
    }
    interleaved
        .chunks(channels)
        .map(|frame| frame.iter().sum::<f32>() / frame.len() as f32)
        .collect()
}

#[allow(dead_code)]
fn f32_to_i16(sample: f32) -> i16 {
    (sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPSILON: f32 = 0.0001;

    fn assert_close(a: f32, b: f32) {
        assert!((a - b).abs() < EPSILON, "expected {b}, got {a}");
    }

    #[test]
    fn downmix_stereo_averages_channels() {
        // frames: (1.0,3.0) -> 2.0 ; (-2.0,4.0) -> 1.0
        let out = downmix_to_mono(&[1.0, 3.0, -2.0, 4.0], 2);
        assert_eq!(out.len(), 2);
        assert_close(out[0], 2.0);
        assert_close(out[1], 1.0);
    }

    #[test]
    fn downmix_quad_averages_four_channels() {
        // (1+3+5+7)/4 = 4.0
        let out = downmix_to_mono(&[1.0, 3.0, 5.0, 7.0], 4);
        assert_eq!(out.len(), 1);
        assert_close(out[0], 4.0);
    }

    #[test]
    fn downmix_mono_is_passthrough() {
        let out = downmix_to_mono(&[0.5, -0.5, 0.25], 1);
        assert_eq!(out, vec![0.5, -0.5, 0.25]);
    }

    #[test]
    fn f32_to_i16_maps_full_scale() {
        assert_eq!(f32_to_i16(1.0), i16::MAX);
        assert_eq!(f32_to_i16(-1.0), -i16::MAX);
        assert_eq!(f32_to_i16(0.0), 0);
    }

    #[test]
    fn f32_to_i16_clamps_out_of_range() {
        assert_eq!(f32_to_i16(2.0), i16::MAX);
        assert_eq!(f32_to_i16(-2.0), -i16::MAX);
    }
}
