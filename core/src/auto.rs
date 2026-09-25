//! "Auto" mode: place *separated stems* into a native-style 5.1 field.
//!
//! This is the part that is independent of whichever separator produced the
//! stems (Demucs / MDX / RoFormer). We take 4 stereo stems — vocals, drums,
//! bass, other — and route them the way real 5.1 music is mixed:
//!
//! * **vocals** → centre (the front anchor), with a little L/R to keep width.
//! * **drums**  → L/R (+ a touch of centre); the kick band (< 120 Hz) feeds LFE.
//! * **bass**   → L/R; its low band feeds LFE.
//! * **other**  → L/R, and its decorrelated (side) energy becomes the surrounds.
//! * **LFE**    = low-passed sum of drums + bass (and a little vocals/other).
//!
//! Because the sources are genuinely separated, the result reads far closer to
//! a native mix than a pure coherence/STFT upmix: the vocal is truly centred and
//! the kick truly thumps, instead of being *estimated* from phase.

use crate::dsp::biquad::Lr4;
use crate::io::pcm::AudioBuffer;
use anyhow::{bail, Result};

/// Tunable routing gains.
#[derive(Debug, Clone, Copy)]
pub struct StemRouting {
    /// vocals sent to the centre channel.
    pub vocal_center: f64,
    /// vocals also sent to L/R (keeps a natural stereo image).
    pub vocal_width: f64,
    /// drums sent to L/R.
    pub drum_lr: f64,
    /// drums sent to the centre.
    pub drum_center: f64,
    /// bass sent to L/R.
    pub bass_lr: f64,
    /// "other" sent to L/R.
    pub other_lr: f64,
    /// portion of "other" that becomes surround (from its side signal).
    pub other_surround: f64,
    /// LFE level relative to the low-passed mid (dB).
    pub lfe_gain_db: f64,
    /// LFE crossover (Hz).
    pub lfe_hz: f64,
}

impl Default for StemRouting {
    fn default() -> Self {
        Self {
            vocal_center: 1.0,
            vocal_width: 0.30,
            drum_lr: 1.0,
            drum_center: 0.15,
            bass_lr: 1.0,
            other_lr: 0.85,
            other_surround: 0.60,
            lfe_gain_db: -4.0,
            lfe_hz: 120.0,
        }
    }
}

/// The four stems, all stereo, same length / rate.
pub struct Stems {
    pub sample_rate: u32,
    pub bits_per_sample: u32,
    pub vocals: AudioBuffer,
    pub drums: AudioBuffer,
    pub bass: AudioBuffer,
    pub other: AudioBuffer,
}

fn out_channels() -> usize {
    6
}

impl Stems {
    pub fn validate(&self) -> Result<()> {
        let n = self.vocals.num_frames();
        let sr = self.sample_rate;
        for (name, s) in [
            ("vocals", &self.vocals),
            ("drums", &self.drums),
            ("bass", &self.bass),
            ("other", &self.other),
        ] {
            if s.num_channels() != 2 {
                bail!("stem '{name}' must be stereo");
            }
            if s.num_frames() != n {
                bail!("stem '{name}' length {} != {}", s.num_frames(), n);
            }
            if s.sample_rate != sr {
                bail!("stem '{name}' sample rate mismatch");
            }
        }
        Ok(())
    }
}

/// Route separated stems into a 5.1 buffer (FL FR FC LFE BL BR).
pub fn route(stems: &Stems, cfg: &StemRouting) -> Result<AudioBuffer> {
    stems.validate()?;
    let n = stems.vocals.num_frames();
    let sr = stems.sample_rate as f64;
    let bits = stems.bits_per_sample;

    let mut out = AudioBuffer::new(stems.sample_rate, bits, out_channels(), n);

    // LFE accumulators (mono mid), low-passed and summed.
    let mut lfe_mid = vec![0.0f64; n];

    // Helper: mid = (L+R)/2, side = (L-R)/2 for a stem.
    let mid = |s: &AudioBuffer, i: usize| (s.data[0][i] + s.data[1][i]) * 0.5;
    let l = |s: &AudioBuffer, i: usize| s.data[0][i];
    let r = |s: &AudioBuffer, i: usize| s.data[1][i];

    for i in 0..n {
        let v_l = l(&stems.vocals, i);
        let v_r = r(&stems.vocals, i);
        let d_l = l(&stems.drums, i);
        let d_r = r(&stems.drums, i);
        let b_l = l(&stems.bass, i);
        let b_r = r(&stems.bass, i);
        let o_l = l(&stems.other, i);
        let o_r = r(&stems.other, i);

        // vocals -> C (+ a little L/R for width)
        out.data[crate::upmix::CH_FC][i] += cfg.vocal_center * mid(&stems.vocals, i);
        out.data[crate::upmix::CH_FL][i] += cfg.vocal_width * v_l;
        out.data[crate::upmix::CH_FR][i] += cfg.vocal_width * v_r;

        // drums -> L/R (+ touch of C)
        out.data[crate::upmix::CH_FL][i] += cfg.drum_lr * d_l;
        out.data[crate::upmix::CH_FR][i] += cfg.drum_lr * d_r;
        out.data[crate::upmix::CH_FC][i] += cfg.drum_center * mid(&stems.drums, i);

        // bass -> L/R
        out.data[crate::upmix::CH_FL][i] += cfg.bass_lr * b_l;
        out.data[crate::upmix::CH_FR][i] += cfg.bass_lr * b_r;

        // other -> L/R
        out.data[crate::upmix::CH_FL][i] += cfg.other_lr * o_l;
        out.data[crate::upmix::CH_FR][i] += cfg.other_lr * o_r;

        // other's side energy -> surrounds (decorrelated ambience)
        let o_side = (o_l - o_r) * 0.5;
        out.data[crate::upmix::CH_BL][i] += cfg.other_surround * o_side;
        out.data[crate::upmix::CH_BR][i] += cfg.other_surround * -o_side;

        // LFE source: bass + drums (+ a little other)
        lfe_mid[i] = 0.6 * mid(&stems.bass, i) + 0.7 * mid(&stems.drums, i) + 0.2 * mid(&stems.other, i);
    }

    // Low-pass the LFE bus (4th-order Linkwitz-Riley) and trim.
    let mut lp = Lr4::lowpass(sr, cfg.lfe_hz);
    lp.process_block(&mut lfe_mid);
    let lg = 10f64.powf(cfg.lfe_gain_db / 20.0);
    for i in 0..n {
        out.data[crate::upmix::CH_LFE][i] += lfe_mid[i] * lg;
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(sr: u32, n: usize, f: f64, phase: f64) -> Vec<f64> {
        (0..n)
            .map(|i| (2.0 * std::f64::consts::PI * f * i as f64 / sr as f64 + phase).sin() * 0.3)
            .collect()
    }

    fn stem(sr: u32, n: usize, f: f64) -> AudioBuffer {
        let a = tone(sr, n, f, 0.0);
        AudioBuffer {
            sample_rate: sr,
            bits_per_sample: 16,
            data: vec![a.clone(), a],
        }
    }

    #[test]
    fn vocals_land_in_center() {
        let sr = 48000;
        let n = 48000;
        let stems = Stems {
            sample_rate: sr,
            bits_per_sample: 16,
            vocals: stem(sr, n, 1000.0),
            drums: AudioBuffer::new(sr, 16, 2, n),
            bass: AudioBuffer::new(sr, 16, 2, n),
            other: AudioBuffer::new(sr, 16, 2, n),
        };
        let out = route(&stems, &StemRouting::default()).unwrap();
        // Center must dominate; L/R only carry the small vocal_width term.
        let c: f64 = out.data[crate::upmix::CH_FC].iter().map(|x| x * x).sum();
        let fl: f64 = out.data[crate::upmix::CH_FL].iter().map(|x| x * x).sum();
        assert!(c > fl * 4.0, "center {c} should dominate front-left {fl}");
    }

    #[test]
    fn kick_band_reaches_lfe() {
        let sr = 48000;
        let n = 48000;
        // drums = 60 Hz (kick) ; others silent
        let stems = Stems {
            sample_rate: sr,
            bits_per_sample: 16,
            vocals: AudioBuffer::new(sr, 16, 2, n),
            drums: stem(sr, n, 60.0),
            bass: AudioBuffer::new(sr, 16, 2, n),
            other: AudioBuffer::new(sr, 16, 2, n),
        };
        let out = route(&stems, &StemRouting::default()).unwrap();
        let lfe: f64 = out.data[crate::upmix::CH_LFE].iter().map(|x| x * x).sum();
        assert!(lfe > 0.0, "LFE should receive the kick band");
    }

    #[test]
    fn routes_to_six_channels() {
        let sr = 48000;
        let n = 2048;
        let stems = Stems {
            sample_rate: sr,
            bits_per_sample: 16,
            vocals: stem(sr, n, 500.0),
            drums: stem(sr, n, 200.0),
            bass: stem(sr, n, 80.0),
            other: stem(sr, n, 800.0),
        };
        let out = route(&stems, &StemRouting::default()).unwrap();
        assert_eq!(out.num_channels(), 6);
        assert_eq!(out.num_frames(), n);
        for ch in &out.data {
            assert!(ch.iter().all(|v| v.is_finite()));
        }
    }
}
