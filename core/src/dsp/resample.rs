//! 纯 Rust 重采样（Blackman 窗 sinc 插值），不依赖 ffmpeg。

use crate::io::pcm::AudioBuffer;

/// 把 `buf` 重采样到 `target_sr`。采样率相同或空输入直接原样返回。
pub fn resample(buf: &AudioBuffer, target_sr: u32) -> AudioBuffer {
    let src_sr = buf.sample_rate;
    if src_sr == target_sr || src_sr == 0 || buf.data.is_empty() {
        return buf.clone();
    }
    let ratio = target_sr as f64 / src_sr as f64;
    let frames_in = buf.data[0].len();
    if frames_in == 0 {
        return buf.clone();
    }
    let frames_out = ((frames_in as f64) * ratio).round() as usize;
    // 降采样时把 sinc 截止频率压到目标奈奎斯特，避免混叠。
    let cutoff = ratio.min(1.0);
    let taps: i64 = 16;

    let mut data = Vec::with_capacity(buf.num_channels());
    for ch in &buf.data {
        let mut out = Vec::with_capacity(frames_out);
        for i in 0..frames_out {
            let center = i as f64 / ratio;
            let first = center.floor() as i64 - taps / 2;
            let mut acc = 0.0f64;
            let mut norm = 0.0f64;
            for k in 0..taps {
                let idx = first + k;
                if idx < 0 || idx as usize >= frames_in {
                    continue;
                }
                let dist = center - idx as f64;
                let x = std::f64::consts::PI * dist * cutoff;
                let sinc = if x.abs() < 1e-9 { 1.0 } else { x.sin() / x };
                // Blackman 窗
                let wp = (k as f64 / (taps - 1) as f64) * 2.0 - 1.0;
                let w = 0.42
                    + 0.5 * (std::f64::consts::PI * wp).cos()
                    + 0.08 * (2.0 * std::f64::consts::PI * wp).cos();
                let h = sinc * w * cutoff;
                acc += ch[idx as usize] * h;
                norm += h;
            }
            out.push(if norm.abs() > 1e-12 { acc / norm } else { acc });
        }
        data.push(out);
    }
    AudioBuffer {
        sample_rate: target_sr,
        bits_per_sample: buf.bits_per_sample,
        data,
    }
}
