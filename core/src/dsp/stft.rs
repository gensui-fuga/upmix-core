//! Short-time Fourier transform with perfect reconstruction.
//!
//! Analysis and synthesis both use a periodic **square-root Hann** window with
//! 50% overlap (hop = win/2). For a periodic Hann window the squared-window
//! overlap sum is exactly 1, so `sqrt-hann` analysis + `sqrt-hann` synthesis +
//! 1/N FFT normalization reconstructs the input to machine precision.

use rustfft::num_complex::Complex;
use rustfft::{Fft, FftPlanner};
use std::f64::consts::PI;
use std::sync::Arc;

pub struct Stft {
    pub win_size: usize,
    pub hop: usize,
    /// Number of non-negative frequency bins: `win_size/2 + 1`.
    pub bins: usize,
    window: Vec<f64>,
    fft: Arc<dyn Fft<f64>>,
    ifft: Arc<dyn Fft<f64>>,
    buf: Vec<Complex<f64>>,
}

impl Stft {
    pub fn new(win_size: usize) -> Self {
        assert!(win_size.is_power_of_two(), "win_size must be a power of two");
        assert!(win_size >= 16, "win_size too small");
        let hop = win_size / 2;
        let n = win_size as f64;
        let window: Vec<f64> = (0..win_size)
            .map(|i| (0.5 * (1.0 - (2.0 * PI * i as f64 / n).cos())).sqrt())
            .collect();
        let mut planner = FftPlanner::new();
        let fft = planner.plan_fft_forward(win_size);
        let ifft = planner.plan_fft_inverse(win_size);
        Self {
            win_size,
            hop,
            bins: win_size / 2 + 1,
            window,
            fft,
            ifft,
            buf: vec![Complex::new(0.0, 0.0); win_size],
        }
    }

    #[inline]
    pub fn window(&self) -> &[f64] {
        &self.window
    }

    /// Center frequency (Hz) of bin `k` for a given sample rate.
    #[inline]
    pub fn bin_hz(&self, k: usize, sample_rate: u32) -> f64 {
        k as f64 * sample_rate as f64 / self.win_size as f64
    }

    /// Forward transform. `input.len() == win_size`, `out.len() == bins`.
    /// The window is applied internally.
    pub fn forward(&mut self, input: &[f64], out: &mut [Complex<f64>]) {
        debug_assert_eq!(input.len(), self.win_size);
        debug_assert_eq!(out.len(), self.bins);
        for i in 0..self.win_size {
            self.buf[i] = Complex::new(input[i] * self.window[i], 0.0);
        }
        self.fft.process(&mut self.buf);
        out.copy_from_slice(&self.buf[..self.bins]);
    }

    /// Inverse transform. `input.len() == bins`, `out.len() == win_size`.
    /// Applies 1/N normalization but no window / overlap-add.
    pub fn inverse(&mut self, input: &[Complex<f64>], out: &mut [f64]) {
        debug_assert_eq!(input.len(), self.bins);
        debug_assert_eq!(out.len(), self.win_size);
        let n = self.win_size;
        self.buf[..self.bins].copy_from_slice(input);
        for k in 1..self.bins - 1 {
            self.buf[n - k] = input[k].conj();
        }
        // DC and Nyquist bins must be purely real.
        self.buf[0].im = 0.0;
        self.buf[n / 2].im = 0.0;
        self.ifft.process(&mut self.buf);
        let inv = 1.0 / n as f64;
        for i in 0..n {
            out[i] = self.buf[i].re * inv;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reconstruct(stft: &mut Stft, sig: &[f64]) -> Vec<f64> {
        let win = stft.win_size;
        let hop = stft.hop;
        let n = sig.len();
        let lut = stft.window().to_vec();
        let mut acc = vec![0.0f64; n + win];
        let mut spec = vec![Complex::new(0.0, 0.0); stft.bins];
        let mut frame = vec![0.0; win];
        let mut time = vec![0.0; win];
        let mut pos = 0;
        while pos + win <= n {
            frame.copy_from_slice(&sig[pos..pos + win]);
            stft.forward(&frame, &mut spec);
            stft.inverse(&spec, &mut time);
            for i in 0..win {
                acc[pos + i] += time[i] * lut[i];
            }
            pos += hop;
        }
        acc
    }

    #[test]
    fn perfect_reconstruction_of_sine() {
        let mut stft = Stft::new(1024);
        let fs = 44100.0;
        let n = 20000;
        let sig: Vec<f64> = (0..n)
            .map(|i| (2.0 * PI * 440.0 * i as f64 / fs).sin())
            .collect();
        let acc = reconstruct(&mut stft, &sig);
        let win = stft.win_size;
        let err = (win..n - win)
            .map(|i| (acc[i] - sig[i]).abs())
            .fold(0.0f64, f64::max);
        assert!(err < 1e-9, "reconstruction error {err}");
    }

    #[test]
    fn perfect_reconstruction_of_noise() {
        let mut stft = Stft::new(512);
        // deterministic pseudo-random signal
        let mut x = 0x12345678u32;
        let sig: Vec<f64> = (0..8000)
            .map(|_| {
                x = x.wrapping_mul(1664525).wrapping_add(1013904223);
                (x as f64 / u32::MAX as f64) * 2.0 - 1.0
            })
            .collect();
        let acc = reconstruct(&mut stft, &sig);
        let win = stft.win_size;
        let err = (win..sig.len() - win)
            .map(|i| (acc[i] - sig[i]).abs())
            .fold(0.0f64, f64::max);
        assert!(err < 1e-9, "reconstruction error {err}");
    }

    #[test]
    fn bin_hz_mapping() {
        let stft = Stft::new(4096);
        assert!((stft.bin_hz(0, 48000) - 0.0).abs() < 1e-9);
        assert!((stft.bin_hz(2048, 48000) - 24000.0).abs() < 1e-6);
        assert!((stft.bin_hz(1024, 48000) - 12000.0).abs() < 1e-6);
    }
}
