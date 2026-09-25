//! Stereo -> 5.1 upmixer.
//!
//! Algorithm: short-time Fourier analysis of the two input channels, then a
//! per-time-frequency-bin **spatial decomposition** (the core idea borrowed
//! from FFmpeg's `af_surround`). For every bin we estimate a 2-D position
//! `(x, y)` from the inter-channel magnitude difference and phase difference:
//!
//! * `x`  = left/right panning  (`-1` left .. `+1` right)
//! * `y`  = front/back          (`+1` correlated/front .. `-1` decorrelated/back)
//!
//! A power-law gain factor per output channel then distributes each bin. On top
//! of that base we apply the rules derived from real 5.1 production practice:
//!
//! * **Center** carries the correlated (mid / vocal) energy, with a mild vocal
//!   band emphasis so the singer sits dead centre.
//! * **LFE** is a dedicated 20-120 Hz low-pass of the mid signal, at a level
//!   trimmed for the +10 dB LFE playback convention.
//! * **Surrounds** get only the decorrelated, mid/high band (>= 200 Hz, no bass)
//!   and are attenuated ~3 dB, then delayed (Haas) and all-pass decorrelated so
//!   they read as ambience instead of stealing the front image.
//!
//! Output channel order is FL, FR, FC, LFE, BL, BR which FFmpeg reports as
//! `channel_layout=5.1`.

use crate::dsp::biquad::AllpassChain;
use crate::dsp::delay::DelayLine;
use crate::dsp::stft::Stft;
use crate::io::pcm::AudioBuffer;
use anyhow::{bail, Result};
use rayon::prelude::*;
use rustfft::num_complex::Complex;
use std::f64::consts::PI;

pub const CH_FL: usize = 0;
pub const CH_FR: usize = 1;
pub const CH_FC: usize = 2;
pub const CH_LFE: usize = 3;
pub const CH_BL: usize = 4;
pub const CH_BR: usize = 5;
pub const N_OUT: usize = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NormalizeMode {
    /// Leave levels untouched.
    None,
    /// Only attenuate, by a single global factor, if the mix would clip.
    Peak,
    /// Soft (tanh) limiting on samples above the threshold.
    Limiter,
}

#[derive(Debug, Clone)]
pub struct UpmixConfig {
    /// STFT window size (power of two). 4096 is a good tonal/transient balance.
    pub win_size: usize,
    /// Haas delay for the surround channels, in milliseconds.
    pub surround_delay_ms: f64,
    /// Surround channel level relative to the fronts (dB).
    pub surround_gain_db: f64,
    /// LFE level relative to the mid signal (dB).
    pub lfe_gain_db: f64,
    /// LFE pass-band lower edge (Hz).
    pub lfe_low_hz: f64,
    /// LFE pass-band upper edge (Hz).
    pub lfe_high_hz: f64,
    /// Surround band lower edge (Hz) - keeps bass out of the surrounds.
    pub surround_low_hz: f64,
    /// Surround band upper edge (Hz).
    pub surround_high_hz: f64,
    /// Front/back separation exponent (spread).
    pub fx: f64,
    pub fy: f64,
    /// Center vocal-band emphasis (dB).
    pub vocal_boost_db: f64,
    pub vocal_low_hz: f64,
    pub vocal_high_hz: f64,
    pub normalize: NormalizeMode,
    /// Apply an all-pass decorrelation chain on the surrounds.
    pub decorrelate: bool,
    /// Scale the mix so an ITU-R BS.775 down-mix matches the original stereo level.
    pub downmix_compat: bool,
}

impl Default for UpmixConfig {
    fn default() -> Self {
        Self {
            win_size: 4096,
            surround_delay_ms: 12.0,
            surround_gain_db: -3.0,
            lfe_gain_db: -6.0,
            lfe_low_hz: 20.0,
            lfe_high_hz: 120.0,
            surround_low_hz: 200.0,
            surround_high_hz: 16000.0,
            fx: 0.5,
            fy: 0.5,
            vocal_boost_db: 2.0,
            vocal_low_hz: 200.0,
            vocal_high_hz: 4000.0,
            normalize: NormalizeMode::Peak,
            decorrelate: true,
            downmix_compat: true,
        }
    }
}

#[inline]
fn pow_half(base: f64, exp: f64) -> f64 {
    let b = base.max(0.0);
    if (exp - 0.5).abs() < 1e-9 {
        b.sqrt()
    } else {
        b.powf(exp)
    }
}

/// Map a stereo bin to the `(x, y)` soundfield position. Ported from FFmpeg's
/// `stereo_position`.
///
/// * `a` panning difference in `[-1, 1]`
/// * `p` inter-channel phase difference in `[0, pi]`
pub fn stereo_position(a: f64, p: f64) -> (f64, f64) {
    let x = (a + a * (p * p - PI / 2.0).max(0.0)).clamp(-1.0, 1.0);
    let y = ((a * PI / 2.0 + PI).cos() * (PI / 2.0 - p / PI).cos() * 10f64.ln() + 1.0)
        .clamp(-1.0, 1.0);
    (x, y)
}

/// A cosine-tapered band window: 1 inside `[low, high]`, rolling off over
/// `width_oct` octaves on each side, 0 outside.
pub fn cosine_band(f: f64, low: f64, high: f64, width_oct: f64) -> f64 {
    if f <= 0.0 {
        return if low <= 0.0 { 1.0 } else { 0.0 };
    }
    let lo_start = low / 2f64.powf(width_oct);
    let hi_end = high * 2f64.powf(width_oct);
    if f <= lo_start || f >= hi_end {
        return 0.0;
    }
    if f >= low && f <= high {
        return 1.0;
    }
    if f < low {
        let t = (f - lo_start) / (low - lo_start);
        0.5 * (1.0 - (PI * t).cos())
    } else {
        let t = (f - high) / (hi_end - high);
        0.5 * (1.0 + (PI * t).cos())
    }
}

/// Per-bin, position-independent constants.
#[derive(Debug, Clone, Copy)]
pub struct BinConsts {
    pub lfe_win: f64,
    pub surr_win: f64,
    pub vocal_w: f64,
    pub fx: f64,
    pub fy: f64,
}

/// Decompose one stereo FFT bin into the six output-channel bins.
///
/// Pure function, independently testable.
pub fn decompose_bin(
    l: Complex<f64>,
    r: Complex<f64>,
    c: &BinConsts,
) -> [Complex<f64>; N_OUT] {
    const EPS: f64 = 1e-12;
    let l_mag = l.norm();
    let r_mag = r.norm();
    let l_phase = l.arg();
    let r_phase = r.arg();

    let mag_sum = l_mag + r_mag;
    let mag_dif = if mag_sum < EPS {
        0.0
    } else {
        (l_mag - r_mag) / mag_sum
    };
    let mut phase_dif = (l_phase - r_phase).abs();
    if phase_dif > PI {
        phase_dif = 2.0 * PI - phase_dif;
    }
    let mag_total = (l_mag * l_mag + r_mag * r_mag).sqrt();
    let c_mag = mag_sum * 0.5;
    let c_phase = (l.im + r.im).atan2(l.re + r.re);

    let (x, y) = stereo_position(mag_dif, phase_dif);

    let half_xp = 0.5 * (x + 1.0);
    let half_xn = 0.5 * (-x + 1.0);
    let half_yp = 0.5 * (y + 1.0);
    let half_yn = 0.5 * (1.0 - y);

    let fl_f = pow_half(half_xp, c.fx) * pow_half(half_yp, c.fy);
    let fr_f = pow_half(half_xn, c.fx) * pow_half(half_yp, c.fy);
    let fc_f = pow_half(1.0 - x.abs(), c.fx) * pow_half(half_yp, c.fy);
    let bl_f = pow_half(half_xp, c.fx) * pow_half(half_yn, c.fy);
    let br_f = pow_half(half_xn, c.fx) * pow_half(half_yn, c.fy);

    let fl_mag = mag_total * fl_f;
    let fr_mag = mag_total * fr_f;
    let fc_mag = c_mag * fc_f * c.vocal_w;
    let lfe_mag = c_mag * c.lfe_win;
    let bl_mag = mag_total * bl_f * c.surr_win;
    let br_mag = mag_total * br_f * c.surr_win;

    // Energy-preserving normalization: keep the summed bin energy close to the
    // stereo input so the mix does not need heavy peak attenuation.
    let out_e = fl_mag * fl_mag
        + fr_mag * fr_mag
        + fc_mag * fc_mag
        + lfe_mag * lfe_mag
        + bl_mag * bl_mag
        + br_mag * br_mag;
    let in_e = l_mag * l_mag + r_mag * r_mag;
    let g = if out_e > 1e-20 {
        (in_e / out_e).sqrt()
    } else {
        1.0
    };

    [
        Complex::from_polar(fl_mag * g, l_phase),
        Complex::from_polar(fr_mag * g, r_phase),
        Complex::from_polar(fc_mag * g, c_phase),
        Complex::from_polar(lfe_mag * g, c_phase),
        Complex::from_polar(bl_mag * g, l_phase),
        Complex::from_polar(br_mag * g, r_phase),
    ]
}

/// Upmix a disjoint sample range `[s0, s1)` of `input` into `out_chunk`
/// (interleaved, length `(s1 - s0) * N_OUT`). Blocks run in parallel over
/// disjoint ranges, so no synchronisation is required.
#[allow(clippy::too_many_arguments)]
fn process_range(
    input: &AudioBuffer,
    consts: &[BinConsts],
    win: usize,
    hop: usize,
    bins: usize,
    n: usize,
    s0: usize,
    s1: usize,
    out_chunk: &mut [f64],
) {
    if s1 <= s0 {
        return;
    }
    // Frames whose window covers any sample in [s0, s1).
    let f_start = (s0 / hop).saturating_sub(1);
    let f_end = (s1 - 1) / hop + 1;
    let lb_start = f_start * hop;
    let lb_end = (f_end - 1) * hop + win;
    let lb_len = lb_end - lb_start;

    let mut local = vec![0.0f64; lb_len * N_OUT];
    let mut stft = Stft::new(win);
    let lut = stft.window().to_vec();
    let mut spec_l = vec![Complex::new(0.0, 0.0); bins];
    let mut spec_r = vec![Complex::new(0.0, 0.0); bins];
    let mut spec_out: Vec<Vec<Complex<f64>>> =
        (0..N_OUT).map(|_| vec![Complex::new(0.0, 0.0); bins]).collect();
    let mut frame_l = vec![0.0f64; win];
    let mut frame_r = vec![0.0f64; win];
    let mut time = vec![0.0f64; win];

    for f in f_start..f_end {
        let p = f * hop;
        let end = (p + win).min(n);
        let len = end.saturating_sub(p);
        if len > 0 {
            frame_l[..len].copy_from_slice(&input.data[0][p..end]);
            frame_r[..len].copy_from_slice(&input.data[1][p..end]);
        }
        frame_l[len..].fill(0.0);
        frame_r[len..].fill(0.0);

        stft.forward(&frame_l, &mut spec_l);
        stft.forward(&frame_r, &mut spec_r);

        for k in 0..bins {
            let outs = decompose_bin(spec_l[k], spec_r[k], &consts[k]);
            for ch in 0..N_OUT {
                spec_out[ch][k] = outs[ch];
            }
        }

        let off = (p - lb_start) * N_OUT;
        for ch in 0..N_OUT {
            stft.inverse(&spec_out[ch], &mut time);
            for i in 0..win {
                local[off + i * N_OUT + ch] += time[i] * lut[i];
            }
        }
    }

    let lo = (s0 - lb_start) * N_OUT;
    let cnt = (s1 - s0) * N_OUT;
    out_chunk.copy_from_slice(&local[lo..lo + cnt]);
}

pub struct Upmixer {
    pub cfg: UpmixConfig,
}

impl Upmixer {
    pub fn new(cfg: UpmixConfig) -> Self {
        Self { cfg }
    }

    pub fn process(&self, input: &AudioBuffer) -> Result<AudioBuffer> {
        self.process_with_progress(input, |_, _| {})
    }

    /// Like [`Self::process`], but reports `progress(done_frames, total_frames)`
    /// after every STFT frame so a UI can drive a progress bar.
    pub fn process_with_progress<F: FnMut(usize, usize)>(
        &self,
        input: &AudioBuffer,
        mut progress: F,
    ) -> Result<AudioBuffer> {
        input.validate()?;
        if input.num_channels() != 2 {
            bail!("upmix expects a stereo input, got {} channels", input.num_channels());
        }
        let sr = input.sample_rate;
        let n = input.num_frames();
        let win = self.cfg.win_size;
        let hop = win / 2;
        let bins = win / 2 + 1;

        // Per-bin constant tables.
        let boost = 10f64.powf(self.cfg.vocal_boost_db / 20.0);
        let consts: Vec<BinConsts> = (0..bins)
            .map(|k| {
                let f = k as f64 * sr as f64 / win as f64;
                BinConsts {
                    lfe_win: cosine_band(f, self.cfg.lfe_low_hz, self.cfg.lfe_high_hz, 0.5),
                    surr_win: cosine_band(
                        f,
                        self.cfg.surround_low_hz,
                        self.cfg.surround_high_hz,
                        1.0,
                    ),
                    vocal_w: 1.0
                        + (boost - 1.0)
                            * cosine_band(f, self.cfg.vocal_low_hz, self.cfg.vocal_high_hz, 1.0),
                    fx: self.cfg.fx,
                    fy: self.cfg.fy,
                }
            })
            .collect();

        // Interleaved output scratch: `out_il[s * N_OUT + ch]`. Each block owns a
        // contiguous sample range, so blocks can fill it in parallel (no locks).
        let mut out_il: Vec<f64> = vec![0.0f64; n * N_OUT];

        let threads = std::thread::available_parallelism()
            .map(|x| x.get())
            .unwrap_or(1)
            .clamp(1, 8);
        let batches = 24usize;
        let batch_samples = ((n + batches - 1) / batches).max(1);
        let total_frames = if n == 0 { 0 } else { (n + hop - 1) / hop };

        for bi in 0..batches {
            let bs = bi * batch_samples;
            let be = ((bi + 1) * batch_samples).min(n);
            if bs >= be {
                break;
            }
            let per = ((be - bs) + threads - 1) / threads;
            out_il[bs * N_OUT..be * N_OUT]
                .par_chunks_mut(per * N_OUT)
                .enumerate()
                .for_each(|(k, chunk)| {
                    let s0 = bs + k * per;
                    let s1 = s0 + chunk.len() / N_OUT;
                    process_range(input, &consts, win, hop, bins, n, s0, s1, chunk);
                });
            progress((be + hop - 1) / hop, total_frames);
        }

        // De-interleave into planar channels for post-processing.
        let mut data: Vec<Vec<f64>> = (0..N_OUT).map(|_| vec![0.0f64; n]).collect();
        for s in 0..n {
            for ch in 0..N_OUT {
                data[ch][s] = out_il[s * N_OUT + ch];
            }
        }

        let mut result = AudioBuffer {
            sample_rate: sr,
            bits_per_sample: input.bits_per_sample,
            data,
        };

        self.post_process(&mut result, input);
        Ok(result)
    }

    /// Time-domain finishing: surround delay + gain, decorrelation, LFE trim,
    /// headroom management.
    fn post_process(&self, buf: &mut AudioBuffer, input: &AudioBuffer) {
        let sr = buf.sample_rate as f64;

        // LFE trim.
        let lg = 10f64.powf(self.cfg.lfe_gain_db / 20.0);
        if (lg - 1.0).abs() > f64::EPSILON {
            for v in buf.data[CH_LFE].iter_mut() {
                *v *= lg;
            }
        }

        // Surround Haas delay.
        let d = (self.cfg.surround_delay_ms / 1000.0 * sr).round() as usize;
        if d > 0 {
            DelayLine::delay_channel(&mut buf.data[CH_BL], d);
            DelayLine::delay_channel(&mut buf.data[CH_BR], d);
        }

        // Surround decorrelation (different all-pass sets per side).
        if self.cfg.decorrelate {
            let mut lchain = AllpassChain::new(sr, &[257.0, 1153.0, 3307.0, 7717.0]);
            let mut rchain = AllpassChain::new(sr, &[331.0, 1427.0, 4129.0, 9127.0]);
            lchain.process_block(&mut buf.data[CH_BL]);
            rchain.process_block(&mut buf.data[CH_BR]);
        }

        // Surround level trim.
        let sg = 10f64.powf(self.cfg.surround_gain_db / 20.0);
        if (sg - 1.0).abs() > f64::EPSILON {
            for v in buf.data[CH_BL].iter_mut() {
                *v *= sg;
            }
            for v in buf.data[CH_BR].iter_mut() {
                *v *= sg;
            }
        }

        // Down-mix compatibility: scale so an ITU-R BS.775 fold-down keeps the
        // original stereo level (least-squares unity gain).
        if self.cfg.downmix_compat {
            let k = std::f64::consts::FRAC_1_SQRT_2; // -3 dB
            let n = buf.num_frames();
            let mut num = 0.0f64;
            let mut den = 0.0f64;
            for i in 0..n {
                let dml =
                    buf.data[CH_FL][i] + k * buf.data[CH_FC][i] + k * buf.data[CH_BL][i];
                let dmr =
                    buf.data[CH_FR][i] + k * buf.data[CH_FC][i] + k * buf.data[CH_BR][i];
                num += dml * input.data[0][i] + dmr * input.data[1][i];
                den += dml * dml + dmr * dmr;
            }
            if den > 1e-18 {
                let g = (num / den).clamp(0.1, 4.0);
                buf.apply_gain(g);
            }
        }

        match self.cfg.normalize {
            NormalizeMode::None => {}
            NormalizeMode::Peak => {
                let peak = buf.peak();
                if peak > 1.0 {
                    buf.apply_gain(1.0 / peak);
                }
            }
            NormalizeMode::Limiter => {
                const THRESH: f64 = 0.95;
                for ch in &mut buf.data {
                    for v in ch.iter_mut() {
                        let a = v.abs();
                        if a > THRESH {
                            let over = (a - THRESH) / (1.0 - THRESH);
                            let limited = THRESH + (1.0 - THRESH) * over.tanh();
                            *v = v.signum() * limited;
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bin(l: (f64, f64), r: (f64, f64)) -> (Complex<f64>, Complex<f64>) {
        (Complex::new(l.0, l.1), Complex::new(r.0, r.1))
    }

    fn consts() -> BinConsts {
        BinConsts {
            lfe_win: 0.0,
            surr_win: 1.0,
            vocal_w: 1.0,
            fx: 0.5,
            fy: 0.5,
        }
    }

    #[test]
    fn stereo_position_front_center() {
        let (x, y) = stereo_position(0.0, 0.0);
        assert!(x.abs() < 1e-9);
        assert!((y - 1.0).abs() < 1e-9, "y={y}");
    }

    #[test]
    fn stereo_position_back_for_antiphase() {
        let (_, y) = stereo_position(0.0, PI);
        assert!(y < -0.5, "decorrelated should map to back, y={y}");
    }

    #[test]
    fn stereo_position_right() {
        let (x, _) = stereo_position(1.0, 0.0);
        assert!((x - 1.0).abs() < 1e-9);
    }

    #[test]
    fn correlated_mid_goes_to_center_not_surround() {
        // identical channels -> fully correlated, centred
        let (l, r) = bin((0.5, 0.5), (0.5, 0.5));
        let o = decompose_bin(l, r, &consts());
        // center should carry energy
        assert!(o[CH_FC].norm() > 1e-6);
        // surrounds should be ~zero
        assert!(o[CH_BL].norm() < 1e-6);
        assert!(o[CH_BR].norm() < 1e-6);
        // front L/R equal
        assert!((o[CH_FL].norm() - o[CH_FR].norm()).abs() < 1e-9);
    }

    #[test]
    fn antiphase_side_goes_to_surround() {
        // L = -R -> fully decorrelated
        let (l, r) = bin((0.0, 0.7), (0.0, -0.7));
        let o = decompose_bin(l, r, &consts());
        assert!(o[CH_BL].norm() > 1e-6, "left surround should get antiphase content");
        assert!(o[CH_BR].norm() > 1e-6);
        // surround must dominate over center and fronts for decorrelated content
        assert!(o[CH_FC].norm() < o[CH_BL].norm());
        assert!(o[CH_FL].norm() < o[CH_BL].norm());
    }

    #[test]
    fn lfe_window_keeps_only_bass() {
        let mut c = consts();
        // simulate a bin where LFE window is open
        c.lfe_win = 1.0;
        let (l, r) = bin((0.5, 0.0), (0.5, 0.0));
        let o = decompose_bin(l, r, &c);
        assert!(o[CH_LFE].norm() > 1e-6);
    }

    #[test]
    fn cosine_band_shape() {
        assert_eq!(cosine_band(50.0, 20.0, 120.0, 0.5), 1.0);
        assert_eq!(cosine_band(10.0, 20.0, 120.0, 0.5), 0.0);
        assert_eq!(cosine_band(200.0, 20.0, 120.0, 0.5), 0.0);
        let mid = cosine_band(15.0, 20.0, 120.0, 0.5);
        assert!(mid > 0.0 && mid < 1.0);
    }

    #[test]
    fn full_pipeline_produces_six_channels_and_no_nan() {
        let sr = 48000;
        let n = 24000;
        let sig: Vec<f64> = (0..n)
            .map(|i| (2.0 * PI * 1000.0 * i as f64 / sr as f64).sin() * 0.5)
            .collect();
        let input = AudioBuffer {
            sample_rate: sr,
            bits_per_sample: 16,
            data: vec![sig.clone(), sig],
        };
        let out = Upmixer::new(UpmixConfig::default()).process(&input).unwrap();
        assert_eq!(out.num_channels(), 6);
        assert_eq!(out.num_frames(), n);
        for ch in &out.data {
            assert!(ch.iter().all(|v| v.is_finite()), "non-finite sample");
        }
    }
}
