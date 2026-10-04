//! WAV read/write via `hound`. Integer PCM (16/24/32-bit) is preserved
//! bit-exactly; float WAV is decoded as f64.

use crate::io::pcm::AudioBuffer;
use anyhow::{bail, Context, Result};
use std::path::Path;

pub fn read(path: &Path) -> Result<AudioBuffer> {
    let mut reader = hound::WavReader::open(path)
        .with_context(|| format!("opening WAV {}", path.display()))?;
    let spec = reader.spec();
    let channels = spec.channels as usize;
    if channels == 0 {
        bail!("WAV has zero channels");
    }
    let bits = spec.bits_per_sample as u32;

    match spec.sample_format {
        hound::SampleFormat::Int => {
            if !matches!(bits, 16 | 24 | 32) {
                bail!("unsupported WAV integer depth {bits} (use 16/24/32)");
            }
            let scale = AudioBuffer::scale(bits);
            let samples: Vec<i32> = reader
                .samples::<i32>()
                .collect::<std::result::Result<_, _>>()
                .context("reading WAV samples")?;
            let frames = samples.len() / channels;
            let mut data = vec![vec![0.0f64; frames]; channels];
            for (i, s) in samples.into_iter().enumerate() {
                let ch = i % channels;
                let frame = i / channels;
                if frame >= frames {
                    break;
                }
                data[ch][frame] = s as f64 / scale;
            }
            Ok(AudioBuffer {
                sample_rate: spec.sample_rate,
                bits_per_sample: bits,
                data,
            })
        }
        hound::SampleFormat::Float => {
            let samples: Vec<f32> = reader
                .samples::<f32>()
                .collect::<std::result::Result<_, _>>()
                .context("reading WAV float samples")?;
            let frames = samples.len() / channels;
            let mut data = vec![vec![0.0f64; frames]; channels];
            for (i, s) in samples.into_iter().enumerate() {
                let ch = i % channels;
                let frame = i / channels;
                if frame >= frames {
                    break;
                }
                data[ch][frame] = s as f64;
            }
            Ok(AudioBuffer {
                sample_rate: spec.sample_rate,
                bits_per_sample: 32,
                data,
            })
        }
    }
}

/// 写 WAV。`source` 是输入文件：它的标签会被追加到输出里（RIFF `LIST/INFO`，
/// 歌词另走 `id3 ` 块）。封面进不了 WAV——wav muxer 不支持视频流，硬塞会让
/// 文件变成 0 字节，所以这里只搬文本标签。
pub fn write(path: &Path, buf: &AudioBuffer, source: Option<&Path>) -> Result<()> {
    buf.validate()?;
    let bits = buf.bits_per_sample;
    if !matches!(bits, 16 | 24 | 32) {
        bail!("cannot write {bits}-bit WAV (use 16/24/32)");
    }
    let spec = hound::WavSpec {
        channels: buf.num_channels() as u16,
        sample_rate: buf.sample_rate,
        bits_per_sample: bits as u16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(path, spec)
        .with_context(|| format!("creating WAV {}", path.display()))?;
    for v in buf.to_i32_interleaved(bits) {
        writer.write_sample(v).context("writing WAV sample")?;
    }
    writer.finalize().context("finalizing WAV")?;
    // hound 只写 fmt + data，标签得自己补。
    crate::metadata::inject_into_wav(path, source)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_roundtrip_16bit() {
        let dir = std::env::temp_dir();
        let path = dir.join("upmix_wav_test_16.wav");
        let buf = AudioBuffer::from_i32_planar(
            vec![vec![0i32, 1000, -1000, 32767], vec![5i32, -5, 20000, -32768]],
            44100,
            16,
        );
        write(&path, &buf, None).unwrap();
        let back = read(&path).unwrap();
        assert_eq!(back.sample_rate, 44100);
        assert_eq!(back.bits_per_sample, 16);
        assert_eq!(back.num_channels(), 2);
        assert_eq!(back.to_i32_interleaved(16), buf.to_i32_interleaved(16));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn wav_roundtrip_24bit_six_channel() {
        let dir = std::env::temp_dir();
        let path = dir.join("upmix_wav_test_24_6ch.wav");
        let data: Vec<Vec<i32>> = (0..6)
            .map(|c| vec![c as i32 * 1000, -c as i32 * 1000, 8388607, -8388608])
            .collect();
        let buf = AudioBuffer::from_i32_planar(data, 96000, 24);
        write(&path, &buf, None).unwrap();
        let back = read(&path).unwrap();
        assert_eq!(back.num_channels(), 6);
        assert_eq!(back.bits_per_sample, 24);
        assert_eq!(back.to_i32_interleaved(24), buf.to_i32_interleaved(24));
        let _ = std::fs::remove_file(path);
    }
}
