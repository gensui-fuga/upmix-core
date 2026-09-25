//! Internal PCM representation.
//!
//! All DSP runs on planar `f64` samples normalized to `[-1.0, 1.0)`.
//! Decoding divides by `2^(bits-1)`; encoding multiplies back and clamps,
//! so the source bit depth (16/24/32) is preserved exactly.

use anyhow::{bail, Result};

/// Planar, normalized PCM. `data[channel][frame]`.
#[derive(Debug, Clone, PartialEq)]
pub struct AudioBuffer {
    pub sample_rate: u32,
    pub bits_per_sample: u32,
    pub data: Vec<Vec<f64>>,
}

impl AudioBuffer {
    pub fn new(sample_rate: u32, bits_per_sample: u32, channels: usize, frames: usize) -> Self {
        Self {
            sample_rate,
            bits_per_sample,
            data: (0..channels).map(|_| vec![0.0; frames]).collect(),
        }
    }

    pub fn num_channels(&self) -> usize {
        self.data.len()
    }

    pub fn num_frames(&self) -> usize {
        self.data.first().map(|c| c.len()).unwrap_or(0)
    }

    /// `2^(bits-1)` — the integer scale for a signed PCM sample of `bits` bits.
    #[inline]
    pub fn scale(bits: u32) -> f64 {
        (1i64 << (bits - 1)) as f64
    }

    /// Build from planar integer samples (as returned by FLAC decoders).
    pub fn from_i32_planar(
        data: Vec<Vec<i32>>,
        sample_rate: u32,
        bits_per_sample: u32,
    ) -> Self {
        let s = Self::scale(bits_per_sample);
        let data = data
            .into_iter()
            .map(|ch| ch.into_iter().map(|v| v as f64 / s).collect())
            .collect();
        Self {
            sample_rate,
            bits_per_sample,
            data,
        }
    }

    /// Quantize one normalized sample to `bits`-bit signed PCM.
    #[inline]
    pub fn quantize_sample(v: f64, bits: u32) -> i32 {
        let s = Self::scale(bits);
        let lo = -(s as i64);
        let hi = (s - 1.0) as i64;
        ((v * s).round() as i64).clamp(lo, hi) as i32
    }

    /// Interleave back to signed integers for a given bit depth, clamping.
    pub fn to_i32_interleaved(&self, bits: u32) -> Vec<i32> {
        let s = Self::scale(bits);
        let lo = -(s as i64);
        let hi = (s - 1.0) as i64;
        let n = self.num_frames();
        let nc = self.num_channels();
        let mut out = Vec::with_capacity(n * nc);
        for f in 0..n {
            for c in 0..nc {
                let v = (self.data[c][f] * s).round() as i64;
                out.push(v.clamp(lo, hi) as i32);
            }
        }
        out
    }

    /// Peak absolute sample across all channels.
    pub fn peak(&self) -> f64 {
        self.data
            .iter()
            .flat_map(|c| c.iter())
            .fold(0.0f64, |m, &v| m.max(v.abs()))
    }

    /// Scale every sample by `g` in place.
    pub fn apply_gain(&mut self, g: f64) {
        for ch in &mut self.data {
            for v in ch.iter_mut() {
                *v *= g;
            }
        }
    }

    /// Append `n` zero frames of silence to every channel.
    pub fn pad_end(&mut self, n: usize) {
        for ch in &mut self.data {
            ch.extend(std::iter::repeat(0.0).take(n));
        }
    }

    /// Keep only the first `n` frames.
    pub fn truncate(&mut self, n: usize) {
        for ch in &mut self.data {
            ch.truncate(n);
        }
    }

    pub fn validate(&self) -> Result<()> {
        if self.data.is_empty() {
            bail!("buffer has no channels");
        }
        let n = self.data[0].len();
        if self.data.iter().any(|c| c.len() != n) {
            bail!("channels have differing lengths");
        }
        if !(8..=32).contains(&self.bits_per_sample) {
            bail!("unsupported bit depth: {}", self.bits_per_sample);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scale_values() {
        assert_eq!(AudioBuffer::scale(16), 32768.0);
        assert_eq!(AudioBuffer::scale(24), 8388608.0);
        assert_eq!(AudioBuffer::scale(32), 2147483648.0);
    }

    #[test]
    fn roundtrip_16bit_is_exact() {
        let planar = vec![vec![0i32, 32767, -32768, 1234], vec![-1i32, 0, 1, -9999]];
        let buf = AudioBuffer::from_i32_planar(planar.clone(), 48000, 16);
        let back = buf.to_i32_interleaved(16);
        // interleaved: f0:[0,-1] f1:[32767,0] f2:[-32768,1] f3:[1234,-9999]
        assert_eq!(back, vec![0, -1, 32767, 0, -32768, 1, 1234, -9999]);
    }

    #[test]
    fn roundtrip_24bit_is_exact() {
        let planar = vec![vec![0i32, 8388607, -8388608], vec![42i32, -42, 100]];
        let buf = AudioBuffer::from_i32_planar(planar, 96000, 24);
        let back = buf.to_i32_interleaved(24);
        assert_eq!(back, vec![0, 42, 8388607, -42, -8388608, 100]);
    }

    #[test]
    fn peak_and_gain() {
        let mut buf = AudioBuffer::from_i32_planar(vec![vec![16384, -32768]], 44100, 16);
        assert!((buf.peak() - 1.0).abs() < 1e-12);
        buf.apply_gain(0.5);
        assert!((buf.peak() - 0.5).abs() < 1e-12);
    }
}
