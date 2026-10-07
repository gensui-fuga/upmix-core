//! FLAC read (claxon) / write (flacenc). Bit depth and sample rate are kept.
//!
//! FLAC has no per-channel mask metadata, so a 6-channel stream uses the
//! standard assignment FL FR FC LFE BL BR, which FFmpeg reports as `5.1`.

use crate::io::pcm::AudioBuffer;
use anyhow::{bail, Context, Result};
use claxon::frame::Block;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

pub fn read(path: &Path) -> Result<AudioBuffer> {
    let mut reader = claxon::FlacReader::open(path)
        .with_context(|| format!("opening FLAC {}", path.display()))?;
    let info = reader.streaminfo();
    let bits = info.bits_per_sample;
    let sample_rate = info.sample_rate;
    let channels = info.channels as usize;
    let mut data: Vec<Vec<i32>> = vec![Vec::new(); channels];
    let mut frame_reader = reader.blocks();
    let mut block = Block::empty();
    loop {
        match frame_reader.read_next_or_eof(block.into_buffer()) {
            Ok(Some(next)) => block = next,
            Ok(None) => break,
            // `.context()` 而不是 `anyhow!("{e}")`：后者是 Display 插值，
            // claxon::Error 只留下字符串，source 链断掉——上层（fileio 的
            // read_any）就没法用 downcast 判断「这是格式错误，值得降级到
            // ffmpeg 容错解码」还是「IO 错误，降级也白搭」。
            Err(e) => {
                return Err(anyhow::Error::new(e).context("decoding FLAC block"));
            }
        }
        for (ch, chan) in data.iter_mut().enumerate() {
            chan.extend_from_slice(block.channel(ch as u32));
        }
    }
    Ok(AudioBuffer::from_i32_planar(data, sample_rate, bits))
}

/// True when the reference `flac` command-line encoder is on PATH.
fn flac_cli_available() -> bool {
    Command::new("flac")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Encode through the reference `flac` CLI (raw PCM on stdin).
fn write_cli(path: &Path, buf: &AudioBuffer) -> Result<()> {
    let bits = buf.bits_per_sample;
    // 原始 PCM 的样本宽度是**整字节向上取整**（flac 的原始读取器就是这么算的），
    // 所以写 div_ceil 而不是 bits/8。
    //
    // 注意：flac CLI 本身只接受 `--bps` 8/16/24/32，其他位深（12/20bit）它会直接
    // 报 “invalid bits per sample” 退出，由 write() 回退到内置编码器——所以对上面
    // 那四个值来说，div_ceil 和 bits/8 是等价的。写成 div_ceil 只是为了不在这个
    // 位置留一个“只有碰巧才对”的表达式。
    let bps = (bits as usize).div_ceil(8);
    let channels = buf.num_channels();
    let mut child = Command::new("flac")
        .args([
            "-f",
            "--silent",
            "--force-raw-format",
            "--endian=little",
            "--sign=signed",
            "--channels",
            &channels.to_string(),
            "--bps",
            &bits.to_string(),
            "--sample-rate",
            &buf.sample_rate.to_string(),
            "-o",
        ])
        .arg(path)
        .arg("-")
        .stdin(Stdio::piped())
        .spawn()
        .context("spawning flac CLI")?;
    {
        let stdin = child.stdin.as_mut().expect("piped stdin");
        let n = buf.num_frames();
        let batch: usize = 8192;
        let mut chunk: Vec<u8> = Vec::with_capacity(batch * channels * bps.max(1));
        for f in 0..n {
            for ch in 0..channels {
                let v = AudioBuffer::quantize_sample(buf.data[ch][f], bits);
                chunk.extend_from_slice(&v.to_le_bytes()[..bps]);
            }
            if f % batch == batch - 1 {
                stdin.write_all(&chunk).context("writing PCM to flac")?;
                chunk.clear();
            }
        }
        if !chunk.is_empty() {
            stdin.write_all(&chunk).context("writing PCM to flac")?;
        }
    }
    drop(child.stdin.take());
    let status = child.wait().context("waiting for flac CLI")?;
    if !status.success() {
        bail!("flac CLI exited with {status}");
    }
    Ok(())
}

/// Serialize. `source` (the input file) carries FLAC metadata — tags, lyrics,
/// cover art — into the output; the channel-layout tag is added on top.
pub fn write(path: &Path, buf: &AudioBuffer, source: Option<&Path>) -> Result<()> {
    buf.validate()?;
    // FLAC 的常见实现只稳到 24bit：32bit（尤其是浮点 WAV 源）先降到 24 再编。
    let lowered;
    let buf = if buf.bits_per_sample > 24 {
        let mut b = buf.clone();
        b.bits_per_sample = 24;
        lowered = b;
        &lowered
    } else {
        buf
    };
    let mut encoded = false;
    if flac_cli_available() {
        match write_cli(path, buf) {
            Ok(()) => encoded = true,
            Err(e) => eprintln!(
                "warning: external flac encoder failed ({e}); falling back to built-in encoder"
            ),
        }
    }
    if !encoded {
        write_enc(path, buf)?;
    }
    let extra = vec!["WAVEFORMATEXTENSIBLE_CHANNEL_MASK=0x003F".to_string()];
    crate::metadata::inject_into_flac(path, source, &extra)?;
    Ok(())
}

fn write_enc(path: &Path, buf: &AudioBuffer) -> Result<()> {
    use flacenc::component::BitRepr;
    use flacenc::error::Verify;

    buf.validate()?;
    let channels = buf.num_channels();
    let bits = buf.bits_per_sample as usize;
    let sample_rate = buf.sample_rate as usize;
    let interleaved = buf.to_i32_interleaved(buf.bits_per_sample);

    let config = flacenc::config::Encoder::default()
        .into_verified()
        .map_err(|(_enc, e)| anyhow::anyhow!("flac config error: {:?}", e))?;
    let source =
        flacenc::source::MemSource::from_samples(&interleaved, channels, bits, sample_rate);
    let stream = flacenc::encode_with_fixed_block_size(&config, source, config.block_size)
        .map_err(|e| anyhow::anyhow!("flac encode error: {}", e))?;

    let mut sink = flacenc::bitsink::ByteSink::new();
    stream
        .write(&mut sink)
        .map_err(|e| anyhow::anyhow!("flac serialize error: {e:?}"))?;
    std::fs::write(path, sink.as_slice())
        .with_context(|| format!("writing FLAC {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 造一个指定位深的双声道测试信号（样本值落在该位深的有效范围内）。
    fn ramp(bits: u32, n: usize, mul: f64) -> Vec<i32> {
        let s = AudioBuffer::scale(bits);
        (0..n)
            .map(|i| ((i as f64 * mul).sin() * s * 0.7) as i32)
            .collect()
    }

    fn temp(tag: &str) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        std::env::temp_dir().join(format!("upmix_flac_{tag}_{}_{nanos}.flac", std::process::id()))
    }

    /// 回归：坏帧 FLAC（帧边界上同步码没了）不该整文件拒收。
    ///
    /// 真实来源：下载截断、流里夹垃圾字节。claxon 严格，报
    /// "Ill-formed FLAC stream: frame sync code missing"；ffmpeg 容错，
    /// 跳过坏帧继续解。read_any 对 claxon::Error 降级到 ffmpeg，两个都死
    /// 才报错。这个用例拿一个好文件把中段砸烂来模拟——前提是 PATH 上有
    /// ffmpeg（CI 有；发行包也随机附带），没有就跳过而不是假装通过。
    #[test]
    fn read_any_falls_back_to_ffmpeg_for_corrupt_frame() {
        let ff = crate::fileio::ffmpeg_path();
        let probe = std::process::Command::new(&ff)
            .arg("-version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let Ok(status) = probe else { return };
        if !status.success() {
            return;
        }

        let n = 8000;
        let buf = AudioBuffer::from_i32_planar(vec![ramp(16, n, 0.05), ramp(16, n, 0.07)], 48000, 16);
        let path = temp("corrupt");
        write(&path, &buf, None).unwrap();

        // 把中段砸烂：同步码 0b11111111111110xx 任一帧头都会被毁掉。
        let mut bytes = std::fs::read(&path).unwrap();
        let mid = bytes.len() / 2;
        for b in &mut bytes[mid..mid + 512] {
            *b = 0xAA;
        }
        std::fs::write(&path, &bytes).unwrap();

        // claxon 必然报错（不然这个用例测的就不是降级路径了）……
        assert!(
            read(&path).is_err(),
            "claxon 竟然解开了被砸烂的文件——用例失效，得造更深的伤"
        );
        // ……而 read_any 走 ffmpeg 降级，应该给出和源相同的形状。
        let back = crate::fileio::read_any(&path).expect("ffmpeg 降级也没接住坏帧文件");
        assert_eq!(back.sample_rate, 48000);
        assert_eq!(back.bits_per_sample, 24, "ffmpeg 侧走 pcm_s24le，位深是 24");
        assert!(back.num_frames() > n / 2, "解出来的长度短得离谱");
        let _ = std::fs::remove_file(path);
    }

    /// 12bit / 20bit 源：flac CLI 只接受 `--bps` 8/16/24/32，`--bps 12` 会直接以
    /// “invalid bits per sample” 退出（CI 实测）。所以 `write()` 必须干净地回退到
    /// 内置编码器，把文件完整写出来，而不是留个半成品。
    ///
    /// 这个用例同时覆盖两条路：装了 flac 时走「CLI 报错 → 回退」，没装时直接走
    /// 内置编码器。以前这条回退路径一次都没被跑过。
    #[test]
    fn write_falls_back_to_builtin_for_12bit_and_20bit() {
        for bits in [12u32, 20u32] {
            let n = 4000;
            let buf = AudioBuffer::from_i32_planar(
                vec![ramp(bits, n, 0.05), ramp(bits, n, 0.07)],
                48000,
                bits,
            );
            let path = temp(&format!("fallback{bits}"));
            write(&path, &buf, None).unwrap();
            let back = read(&path).unwrap();
            assert_eq!(back.bits_per_sample, bits, "{bits}bit 的位深没保住");
            assert_eq!(back.num_frames(), n, "{bits}bit 的帧数不对");
            assert_eq!(
                back.to_i32_interleaved(bits),
                buf.to_i32_interleaved(bits),
                "{bits}bit 的样本对不上——回退路径写出了坏文件"
            );
            let _ = std::fs::remove_file(path);
        }
    }

    #[test]
    fn flac_roundtrip_16bit_stereo() {
        let dir = std::env::temp_dir();
        let path = dir.join("upmix_flac_test_16.flac");
        let n = 5000;
        let left: Vec<i32> = (0..n).map(|i| ((i as f64 * 0.05).sin() * 20000.0) as i32).collect();
        let right: Vec<i32> = (0..n).map(|i| ((i as f64 * 0.07).cos() * 15000.0) as i32).collect();
        let buf = AudioBuffer::from_i32_planar(vec![left, right], 48000, 16);
        write(&path, &buf, None).unwrap();
        let back = read(&path).unwrap();
        assert_eq!(back.sample_rate, 48000);
        assert_eq!(back.bits_per_sample, 16);
        assert_eq!(back.num_channels(), 2);
        assert_eq!(back.num_frames(), n);
        assert_eq!(back.to_i32_interleaved(16), buf.to_i32_interleaved(16));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn flac_roundtrip_24bit_six_channel() {
        let dir = std::env::temp_dir();
        let path = dir.join("upmix_flac_test_24_6ch.flac");
        let n = 3000;
        let data: Vec<Vec<i32>> = (0..6)
            .map(|c| {
                (0..n)
                    .map(|i| (((i + c * 37) as f64 * 0.03).sin() * 8000000.0) as i32)
                    .collect()
            })
            .collect();
        let buf = AudioBuffer::from_i32_planar(data, 96000, 24);
        write(&path, &buf, None).unwrap();
        let back = read(&path).unwrap();
        assert_eq!(back.num_channels(), 6);
        assert_eq!(back.bits_per_sample, 24);
        assert_eq!(back.sample_rate, 96000);
        assert_eq!(back.to_i32_interleaved(24), buf.to_i32_interleaved(24));
        let _ = std::fs::remove_file(path);
    }
}
