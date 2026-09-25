//! Integer delay line used for the Haas-effect on the surround channels.

/// A simple ring-buffer delay line with a fixed maximum capacity.
pub struct DelayLine {
    buf: Vec<f64>,
    idx: usize,
}

impl DelayLine {
    pub fn new(max_delay: usize) -> Self {
        let len = max_delay.max(1) + 1;
        Self {
            buf: vec![0.0; len],
            idx: 0,
        }
    }

    /// Push `x` and return the sample delayed by `delay` (clamped to capacity).
    #[inline]
    pub fn process(&mut self, x: f64, delay: usize) -> f64 {
        let len = self.buf.len();
        let d = delay.min(len - 1);
        // write current sample
        self.buf[self.idx] = x;
        // read delayed sample
        let read = (self.idx + len - d) % len;
        let y = self.buf[read];
        self.idx = (self.idx + 1) % len;
        y
    }

    /// Delay an entire channel in place (prepend `delay` zeros, drop the tail).
    pub fn delay_channel(buf: &mut [f64], delay: usize) {
        if delay == 0 || delay >= buf.len() {
            if delay >= buf.len() {
                buf.fill(0.0);
            }
            return;
        }
        buf.copy_within(0..buf.len() - delay, delay);
        buf[..delay].fill(0.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integer_delay_works() {
        let mut d = DelayLine::new(8);
        let mut out = Vec::new();
        for i in 0..10 {
            out.push(d.process(i as f64, 3));
        }
        // first 3 outputs are zeros (from initial buffer), then 0,1,2,...
        assert_eq!(out[0], 0.0);
        assert_eq!(out[1], 0.0);
        assert_eq!(out[2], 0.0);
        assert_eq!(out[3], 0.0); // delayed sample of input 0
        assert_eq!(out[4], 1.0);
        assert_eq!(out[9], 6.0);
    }

    #[test]
    fn delay_channel_shifts() {
        let mut buf = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        DelayLine::delay_channel(&mut buf, 2);
        assert_eq!(buf, vec![0.0, 0.0, 1.0, 2.0, 3.0]);
    }

    #[test]
    fn zero_delay_is_identity() {
        let mut buf = vec![1.0, 2.0, 3.0];
        DelayLine::delay_channel(&mut buf, 0);
        assert_eq!(buf, vec![1.0, 2.0, 3.0]);
    }
}
