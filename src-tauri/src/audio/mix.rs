use std::path::Path;

/// Mix two WAV files into the destination path.
/// Both files are read as i16 samples, summed with clamping, and written as i16.
/// If files have different lengths, the shorter one is zero-padded.
pub fn mix_wav_files(mic_path: &Path, system_path: &Path, output_path: &Path) -> Result<(), String> {
    let mut mic_reader = hound::WavReader::open(mic_path)
        .map_err(|e| format!("Failed to open mic WAV: {}", e))?;
    let mut sys_reader = hound::WavReader::open(system_path)
        .map_err(|e| format!("Failed to open system WAV: {}", e))?;

    let spec = mic_reader.spec();

    let mic_samples: Vec<i32> = mic_reader
        .samples::<i16>()
        .map(|s| s.map(|v| v as i32))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("Failed to read mic samples: {}", e))?;

    let sys_samples: Vec<i32> = sys_reader
        .samples::<i16>()
        .map(|s| s.map(|v| v as i32))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("Failed to read system samples: {}", e))?;

    let len = mic_samples.len().max(sys_samples.len());

    let out_spec = hound::WavSpec {
        channels: spec.channels,
        sample_rate: spec.sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(output_path, out_spec)
        .map_err(|e| format!("Failed to create output WAV: {}", e))?;

    for i in 0..len {
        let mic = *mic_samples.get(i).unwrap_or(&0);
        let sys = *sys_samples.get(i).unwrap_or(&0);
        let mixed = (mic + sys).clamp(i16::MIN as i32, i16::MAX as i32) as i16;
        writer
            .write_sample(mixed)
            .map_err(|e| format!("Failed to write sample: {}", e))?;
    }

    writer
        .finalize()
        .map_err(|e| format!("Failed to finalize WAV: {}", e))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_wav(path: &Path, sample_rate: u32, channels: u16, samples: &[i16]) {
        let spec = hound::WavSpec {
            channels,
            sample_rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(path, spec).unwrap();
        for &s in samples {
            writer.write_sample(s).unwrap();
        }
        writer.finalize().unwrap();
    }

    fn read_wav_samples(path: &Path) -> Vec<i16> {
        let mut reader = hound::WavReader::open(path).unwrap();
        reader.samples::<i16>().map(|s| s.unwrap()).collect()
    }

    #[test]
    fn mix_two_equal_length_wavs() {
        let dir = tempfile::tempdir().unwrap();
        let mic = dir.path().join("mic.wav");
        let sys = dir.path().join("sys.wav");
        let out = dir.path().join("out.wav");

        write_wav(&mic, 16000, 1, &[1000, 2000, 3000]);
        write_wav(&sys, 16000, 1, &[500, 1000, 1500]);

        mix_wav_files(&mic, &sys, &out).unwrap();

        let result = read_wav_samples(&out);
        assert_eq!(result, vec![1500, 3000, 4500]);
    }

    #[test]
    fn mix_different_length_wavs_pads_shorter() {
        let dir = tempfile::tempdir().unwrap();
        let mic = dir.path().join("mic.wav");
        let sys = dir.path().join("sys.wav");
        let out = dir.path().join("out.wav");

        write_wav(&mic, 16000, 1, &[1000, 2000, 3000, 4000]);
        write_wav(&sys, 16000, 1, &[500, 500]);

        mix_wav_files(&mic, &sys, &out).unwrap();

        let result = read_wav_samples(&out);
        assert_eq!(result, vec![1500, 2500, 3000, 4000]);
    }

    #[test]
    fn mix_clamps_on_overflow() {
        let dir = tempfile::tempdir().unwrap();
        let mic = dir.path().join("mic.wav");
        let sys = dir.path().join("sys.wav");
        let out = dir.path().join("out.wav");

        write_wav(&mic, 16000, 1, &[30000, -30000]);
        write_wav(&sys, 16000, 1, &[30000, -30000]);

        mix_wav_files(&mic, &sys, &out).unwrap();

        let result = read_wav_samples(&out);
        assert_eq!(result[0], i16::MAX);
        assert_eq!(result[1], i16::MIN);
    }

    #[test]
    fn mix_with_missing_system_file_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        let mic = dir.path().join("mic.wav");
        let sys = dir.path().join("nonexistent.wav");
        let out = dir.path().join("out.wav");

        write_wav(&mic, 16000, 1, &[1000]);

        let result = mix_wav_files(&mic, &sys, &out);
        assert!(result.is_err());
    }
}
