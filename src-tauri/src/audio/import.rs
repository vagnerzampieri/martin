use std::fs::File;
use std::io::BufWriter;
use std::path::{Path, PathBuf};

use hound::{SampleFormat, WavSpec, WavWriter};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

use crate::transcribe::whisper::wav_duration_secs;

// TODO(task-5): drop this allow once the Tauri import command consumes this API.
#[allow(dead_code)]
pub struct Imported {
    pub wav_path: PathBuf,
    pub duration_secs: f64,
}

// TODO(task-5): drop this allow once the Tauri import command consumes this API.
#[allow(dead_code)]
pub fn import_audio(source: &Path, dest_dir: &Path) -> Result<Imported, String> {
    let file = File::open(source).map_err(|e| format!("Failed to open file: {}", e))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    if let Some(ext) = source.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }

    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|e| format!("Unsupported or corrupt audio: {}", e))?;
    let mut format = probed.format;

    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or("No decodable audio track")?;
    let track_id = track.id;
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| format!("No decoder available: {}", e))?;

    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| format!("System clock error: {}", e))?
        .as_millis();
    let wav_path = dest_dir.join(format!("imported_{}.wav", timestamp));

    let mut writer: Option<WavWriter<BufWriter<File>>> = None;
    let mut samples_written: u64 = 0;

    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(SymphoniaError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                break;
            }
            Err(SymphoniaError::ResetRequired) => break,
            Err(e) => return Err(format!("Error reading audio: {}", e)),
        };

        if packet.track_id() != track_id {
            continue;
        }

        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            Err(SymphoniaError::DecodeError(_)) => continue, // skip a bad packet
            Err(e) => return Err(format!("Decode error: {}", e)),
        };

        let spec = *decoded.spec();
        let channels = spec.channels.count();

        if writer.is_none() {
            let wav_spec = WavSpec {
                channels: 1,
                sample_rate: spec.rate,
                bits_per_sample: 16,
                sample_format: SampleFormat::Int,
            };
            writer = Some(
                WavWriter::create(&wav_path, wav_spec)
                    .map_err(|e| format!("Failed to create WAV: {}", e))?,
            );
        }

        // One SampleBuffer per packet keeps memory bounded to a single packet.
        let mut sbuf = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
        sbuf.copy_interleaved_ref(decoded);
        let mono = downmix_to_mono(sbuf.samples(), channels);

        let w = writer.as_mut().expect("writer initialized above");
        for s in mono {
            w.write_sample(f32_to_i16(s))
                .map_err(|e| format!("Failed to write sample: {}", e))?;
            samples_written += 1;
        }
    }

    let writer = writer.ok_or("Audio file contained no decodable frames")?;
    writer
        .finalize()
        .map_err(|e| format!("Failed to finalize WAV: {}", e))?;

    if samples_written == 0 {
        let _ = std::fs::remove_file(&wav_path);
        return Err("Audio file contained no samples".to_string());
    }

    let duration_secs = wav_duration_secs(&wav_path)?;
    Ok(Imported {
        wav_path,
        duration_secs,
    })
}

fn downmix_to_mono(interleaved: &[f32], channels: usize) -> Vec<f32> {
    if channels <= 1 {
        return interleaved.to_vec();
    }
    interleaved
        .chunks(channels)
        .map(|frame| frame.iter().sum::<f32>() / frame.len() as f32)
        .collect()
}

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

    fn write_stereo_wav(path: &std::path::Path, sample_rate: u32, frames: usize) {
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(path, spec).unwrap();
        for _ in 0..frames {
            w.write_sample(1000_i16).unwrap(); // L
            w.write_sample(3000_i16).unwrap(); // R
        }
        w.finalize().unwrap();
    }

    #[test]
    fn import_stereo_wav_produces_mono_wav_with_matching_duration() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("source.wav");
        write_stereo_wav(&src, 16000, 16000); // 1.0s of stereo audio

        let imported = import_audio(&src, dir.path()).expect("import failed");

        assert!(imported.wav_path.exists());
        assert!((imported.duration_secs - 1.0).abs() < 0.05);

        let reader = hound::WavReader::open(&imported.wav_path).unwrap();
        assert_eq!(reader.spec().channels, 1, "output must be mono");
        assert_eq!(reader.spec().sample_rate, 16000);
        assert_eq!(reader.duration(), 16000, "one mono frame per source frame");
    }

    #[test]
    fn import_missing_file_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("does_not_exist.wav");
        assert!(import_audio(&src, dir.path()).is_err());
    }
}
