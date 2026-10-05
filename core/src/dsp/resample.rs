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
    // 抽头数必须随 1/cutoff 增长：截止频率越低，sinc 主瓣越宽，固定 16 抽头会
    // 把主瓣截断，结果是通带下垂 + 过渡带混叠（实测 192k→44.1k 在 0.9fc 只有
    // -4.1 dB，1.5fc 处才 -14 dB）。取偶数，并设上限避免极端采样率下开销失控。
    let taps: i64 = (((16.0 / cutoff).ceil() as i64) + 1) & !1;
    let taps = taps.clamp(16, 512);
    let half = taps as f64 / 2.0;

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
                let dist = center - idx as f64;
                let x = std::f64::consts::PI * dist * cutoff;
                let sinc = if x.abs() < 1e-9 { 1.0 } else { x.sin() / x };
                // Blackman 窗必须按「到插值点的距离」算，不能按抽头序号 k 算：
                // center 一般不是整数，按序号算出来的窗中心会偏离 sinc 中心
                // 0.5~1.5 个样本，核就不对称了，近 Nyquist 处会出现随小数部分
                // 变化的相位调制（实测 0.9π 处相位误差 +20.2°）。
                let wp = dist / half;
                let w = 0.42
                    + 0.5 * (std::f64::consts::PI * wp).cos()
                    + 0.08 * (2.0 * std::f64::consts::PI * wp).cos();
                let h = sinc * w * cutoff;
                // 边缘用「钳位复制」而不是跳过越界抽头：跳过会让首尾的核变成
                // 单边，再按 norm 归一化就产生瞬态（实测首个插值样本偏差 33%，
                // 听感是文件首尾各一下轻微咔哒）。
                let idx = idx.clamp(0, frames_in as i64 - 1) as usize;
                acc += ch[idx] * h;
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    fn one_channel(sr: u32, n: usize, f: impl Fn(usize) -> f64) -> AudioBuffer {
        AudioBuffer {
            sample_rate: sr,
            bits_per_sample: 32,
            data: vec![(0..n).map(f).collect()],
        }
    }

    /// 直流重采样后必须还是直流——**包括首尾**。
    /// 旧实现在边缘直接跳过越界抽头，核变成单边，再按 norm 归一化就产生瞬态
    /// （实测 2x 上采样时首个插值样本偏差 33%，听感是首尾各一下轻微咔哒）。
    #[test]
    fn dc_is_preserved_including_edges() {
        let b = one_channel(48000, 4096, |_| 0.5);
        for target in [44100u32, 96000, 22050, 16000, 8000] {
            let r = resample(&b, target);
            for (i, v) in r.data[0].iter().enumerate() {
                assert!(
                    (v - 0.5).abs() < 1e-9,
                    "target={target} 第 {i} 个样本 = {v}，直流被破坏"
                );
            }
        }
    }

    /// 降采样时通带必须平。旧实现抽头固定 16、不随 1/cutoff 缩放，
    /// 192k→44.1k 在 0.9fc 处只有 -4.1 dB，过渡带 1.5fc 才 -14 dB。
    #[test]
    fn downsample_passband_is_flat() {
        let src = 192000u32;
        let dst = 44100u32;
        let n = 192000;
        let f = 1000.0; // 远低于 44.1k 的新奈奎斯特
        let b = one_channel(src, n, |i| (2.0 * PI * f * i as f64 / src as f64).sin());
        let r = resample(&b, dst);
        let d = &r.data[0];
        let warm = d.len() / 4;
        let peak = d[warm..].iter().fold(0.0f64, |m, v| m.max(v.abs()));
        assert!(
            (peak - 1.0).abs() < 0.02,
            "1 kHz 通带增益 {peak}，应接近 1.0（下垂说明抽头数不够）"
        );
    }

    /// 输出长度按比例算，且同采样率直接原样返回。
    #[test]
    fn length_and_noop() {
        let b = one_channel(48000, 48000, |i| (i as f64 * 0.001).sin());
        let r = resample(&b, 24000);
        assert_eq!(r.data[0].len(), 24000);
        assert_eq!(r.sample_rate, 24000);
        let same = resample(&b, 48000);
        assert_eq!(same.data[0], b.data[0], "同采样率不该改动数据");
    }
}
