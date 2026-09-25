//! Biquad IIR filters (RBJ Audio EQ Cookbook) plus a 4th-order Linkwitz-Riley
//! crossover, used for the LFE low-pass and surround band-pass shaping.

use std::f64::consts::PI;

/// Direct-form-I biquad section.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Biquad {
    pub b0: f64,
    pub b1: f64,
    pub b2: f64,
    pub a1: f64,
    pub a2: f64,
    x1: f64,
    x2: f64,
    y1: f64,
    y2: f64,
}

impl Biquad {
    pub fn new(b0: f64, b1: f64, b2: f64, a1: f64, a2: f64) -> Self {
        Self {
            b0,
            b1,
            b2,
            a1,
            a2,
            x1: 0.0,
            x2: 0.0,
            y1: 0.0,
            y2: 0.0,
        }
    }

    /// Low-pass with cutoff `f` (Hz) and resonance `q`.
    pub fn lowpass(sample_rate: f64, f: f64, q: f64) -> Self {
        let w0 = 2.0 * PI * f / sample_rate;
        let (sn, cs) = w0.sin_cos();
        let alpha = sn / (2.0 * q);
        let b0 = (1.0 - cs) / 2.0;
        let b1 = 1.0 - cs;
        let b2 = (1.0 - cs) / 2.0;
        let a0 = 1.0 + alpha;
        let a1 = -2.0 * cs;
        let a2 = 1.0 - alpha;
        Self::new(b0 / a0, b1 / a0, b2 / a0, a1 / a0, a2 / a0)
    }

    /// High-pass with cutoff `f` (Hz) and resonance `q`.
    pub fn highpass(sample_rate: f64, f: f64, q: f64) -> Self {
        let w0 = 2.0 * PI * f / sample_rate;
        let (sn, cs) = w0.sin_cos();
        let alpha = sn / (2.0 * q);
        let b0 = (1.0 + cs) / 2.0;
        let b1 = -(1.0 + cs);
        let b2 = (1.0 + cs) / 2.0;
        let a0 = 1.0 + alpha;
        let a1 = -2.0 * cs;
        let a2 = 1.0 - alpha;
        Self::new(b0 / a0, b1 / a0, b2 / a0, a1 / a0, a2 / a0)
    }

    /// First-order all-pass at `f` (Hz). Useful for phase decorrelation.
    pub fn allpass1(sample_rate: f64, f: f64) -> Self {
        let t = (PI * f / sample_rate).tan();
        let a = (t - 1.0) / (t + 1.0);
        // H(z) = (a + z^-1) / (1 + a z^-1)
        Self::new(a, 1.0, 0.0, a, 0.0)
    }

    #[inline]
    pub fn process(&mut self, x: f64) -> f64 {
        let y = self.b0 * x + self.b1 * self.x1 + self.b2 * self.x2
            - self.a1 * self.y1
            - self.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = x;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }

    pub fn process_block(&mut self, buf: &mut [f64]) {
        for v in buf.iter_mut() {
            *v = self.process(*v);
        }
    }

    pub fn reset(&mut self) {
        self.x1 = 0.0;
        self.x2 = 0.0;
        self.y1 = 0.0;
        self.y2 = 0.0;
    }
}

/// 4th-order Linkwitz-Riley topology (two cascaded Butterworth sections).
/// Provides a flat magnitude sum for low-pass + high-pass pairs.
#[derive(Debug, Clone, Copy)]
pub struct Lr4 {
    s1: Biquad,
    s2: Biquad,
}

impl Lr4 {
    pub fn lowpass(sample_rate: f64, f: f64) -> Self {
        // Butterworth 4th-order Q values
        let q1 = 0.541_196_1;
        let q2 = 1.306_563;
        Self {
            s1: Biquad::lowpass(sample_rate, f, q1),
            s2: Biquad::lowpass(sample_rate, f, q2),
        }
    }

    pub fn highpass(sample_rate: f64, f: f64) -> Self {
        let q1 = 0.541_196_1;
        let q2 = 1.306_563;
        Self {
            s1: Biquad::highpass(sample_rate, f, q1),
            s2: Biquad::highpass(sample_rate, f, q2),
        }
    }

    #[inline]
    pub fn process(&mut self, x: f64) -> f64 {
        self.s2.process(self.s1.process(x))
    }

    pub fn process_block(&mut self, buf: &mut [f64]) {
        for v in buf.iter_mut() {
            *v = self.process(*v);
        }
    }
}

/// A short chain of first-order all-pass filters used to decorrelate a signal
/// (breaking its phase relationship without changing its magnitude spectrum).
pub struct AllpassChain {
    stages: Vec<Biquad>,
}

impl AllpassChain {
    pub fn new(sample_rate: f64, freqs: &[f64]) -> Self {
        Self {
            stages: freqs.iter().map(|&f| Biquad::allpass1(sample_rate, f)).collect(),
        }
    }

    pub fn process_block(&mut self, buf: &mut [f64]) {
        for v in buf.iter_mut() {
            let mut y = *v;
            for s in self.stages.iter_mut() {
                y = s.process(y);
            }
            *v = y;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rms(buf: &[f64]) -> f64 {
        (buf.iter().map(|v| v * v).sum::<f64>() / buf.len() as f64).sqrt()
    }

    /// Generate a sine of `f` Hz at `sr`, run through a filter, return output RMS.
    fn filtered_rms(filt: &mut Biquad, sr: f64, f: f64, secs: f64) -> f64 {
        let n = (sr * secs) as usize;
        let mut buf: Vec<f64> = (0..n)
            .map(|i| (2.0 * PI * f * i as f64 / sr).sin())
            .collect();
        // skip transient
        let warm = n / 4;
        for i in 0..warm {
            filt.process(buf[i]);
        }
        filt.process_block(&mut buf[warm..]);
        rms(&buf[warm..])
    }

    #[test]
    fn lowpass_passes_dc_rejects_high() {
        let sr = 48000.0;
        let mut f = Biquad::lowpass(sr, 100.0, 0.707);
        assert!(filtered_rms(&mut f, sr, 20.0, 1.0) > 0.6);
        f.reset();
        assert!(filtered_rms(&mut f, sr, 5000.0, 1.0) < 0.05);
    }

    #[test]
    fn highpass_rejects_dc_passes_high() {
        let sr = 48000.0;
        let mut f = Biquad::highpass(sr, 200.0, 0.707);
        assert!(filtered_rms(&mut f, sr, 5000.0, 1.0) > 0.6);
        f.reset();
        assert!(filtered_rms(&mut f, sr, 20.0, 1.0) < 0.05);
    }

    #[test]
    fn lr4_lowpass_is_steeper_than_biquad() {
        let sr = 48000.0;
        let mut b = Biquad::lowpass(sr, 100.0, 0.707);
        let mut lr = Lr4::lowpass(sr, 100.0);
        let n = 48000;
        let mut bbuf: Vec<f64> = (0..n)
            .map(|i| (2.0 * PI * 400.0 * i as f64 / sr).sin())
            .collect();
        let mut lbuf = bbuf.clone();
        for v in bbuf.iter_mut() {
            *v = b.process(*v);
        }
        lr.process_block(&mut lbuf);
        // LR4 (4th order) must attenuate 400 Hz more than a single biquad.
        assert!(rms(&lbuf) < rms(&bbuf));
    }

    #[test]
    fn allpass_preserves_magnitude() {
        let sr = 48000.0;
        let n = 48000;
        let sig: Vec<f64> = (0..n)
            .map(|i| (2.0 * PI * 1000.0 * i as f64 / sr).sin())
            .collect();
        let mut chain = AllpassChain::new(sr, &[300.0, 1200.0, 4800.0]);
        let mut out = sig.clone();
        chain.process_block(&mut out);
        let warm = n / 4;
        let a = rms(&sig[warm..]);
        let b = rms(&out[warm..]);
        assert!((a - b).abs() / a < 0.01, "allpass changed magnitude: {a} vs {b}");
    }
}
