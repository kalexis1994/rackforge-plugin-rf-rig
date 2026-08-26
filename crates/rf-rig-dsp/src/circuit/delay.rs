//! Delay lines, including the bucket-brigade model the chorus and the analog
//! echo are built on.
//!
//! The buffers live outside these structs. RF-Rig never allocates: the plugin
//! hands the engine one static block at activation and every line borrows a
//! slice of it for the duration of a call. That is also why a line carries its
//! own length instead of trusting the slice it is given.

use crate::circuit::filters::{OnePole, RmsFollower};
use crate::math::{clamp, floor, sanitise};

/// A circular delay line with fractional read-out.
#[derive(Clone, Copy, Default)]
pub struct DelayLine {
    write_index: usize,
    length: usize,
}

impl DelayLine {
    /// Claims `length` samples of `buffer` and clears them.
    pub fn prepare(&mut self, buffer: &mut [f32], length: usize) {
        let available = buffer.len();
        self.length = length.min(available).max(8);
        self.write_index = 0;
        buffer[..self.length].fill(0.0);
    }

    pub fn clear(&mut self, buffer: &mut [f32]) {
        buffer[..self.length].fill(0.0);
        self.write_index = 0;
    }

    pub fn length(&self) -> usize {
        self.length
    }

    #[inline]
    pub fn write(&mut self, buffer: &mut [f32], value: f32) {
        if self.length == 0 {
            return;
        }
        buffer[self.write_index] = sanitise(value);
        self.write_index += 1;
        if self.write_index >= self.length {
            self.write_index = 0;
        }
    }

    #[inline]
    fn at(&self, buffer: &[f32], delay: usize) -> f32 {
        let index = (self.write_index + self.length - delay) % self.length;
        buffer[index]
    }

    /// Four-point Hermite read. Linear interpolation would put a lowpass on
    /// the signal that moves with the delay time, which is audible as a
    /// swept dullness on a modulated line.
    #[inline]
    pub fn read(&self, buffer: &[f32], delay_samples: f32) -> f32 {
        if self.length < 8 {
            return 0.0;
        }
        let delay = clamp(delay_samples, 2.0, (self.length - 4) as f32);
        let integer = floor(delay) as usize;
        let fraction = delay - integer as f32;

        let oldest = self.at(buffer, integer + 2);
        let older = self.at(buffer, integer + 1);
        let newer = self.at(buffer, integer);
        let newest = self.at(buffer, integer.saturating_sub(1).max(1));

        // Position between `older` and `newer`, in time order.
        let t = 1.0 - fraction;
        let c0 = older;
        let c1 = 0.5 * (newer - oldest);
        let c2 = oldest - 2.5 * older + 2.0 * newer - 0.5 * newest;
        let c3 = 0.5 * (newest - oldest) + 1.5 * (older - newer);
        ((c3 * t + c2) * t + c1) * t + c0
    }
}

/// The compander that surrounds every bucket-brigade line.
///
/// A BBD has perhaps 70 dB of dynamic range and a noise floor you would
/// otherwise hear on every repeat, so the designers compressed the signal
/// going in and expanded it coming out. The pair is not perfectly
/// complementary in a real NE570, and that mismatch is part of why analog
/// delays breathe.
#[derive(Clone, Copy, Default)]
pub struct Compander {
    detector: RmsFollower,
    smoothing: OnePole,
}

impl Compander {
    pub fn prepare(&mut self, sample_rate: f32) {
        self.detector = RmsFollower::new(20.0, sample_rate);
        self.smoothing = OnePole::new(30.0, sample_rate);
    }

    pub fn reset(&mut self) {
        self.detector.reset();
        self.smoothing.reset();
    }

    /// 2:1 compression on the way into the delay line.
    #[inline]
    pub fn compress(&mut self, input: f32) -> f32 {
        let level = self.smoothing.low(self.detector.process(input));
        input * self.gain(level, -0.5)
    }

    /// 1:2 expansion on the way out.
    #[inline]
    pub fn expand(&mut self, input: f32) -> f32 {
        let level = self.smoothing.low(self.detector.process(input));
        input * self.gain(level, 1.0)
    }

    #[inline]
    fn gain(&self, level: f32, exponent: f32) -> f32 {
        // Below the reference level the law flattens out, exactly as the
        // rectifier in a 570 stops tracking near its noise floor.
        let reference = 0.05_f32;
        let normalised = clamp(level / reference, 0.05, 20.0);
        crate::math::powf(normalised, exponent)
    }
}

/// One bucket-brigade device: a clocked analog shift register.
///
/// The clock rate sets the delay (`stages / (2 * clock)`), the register is
/// band-limited on both sides, and the companding above hides its noise. The
/// delay is swept by moving the clock, which is why a real BBD chorus changes
/// pitch rather than crossfading.
#[derive(Clone, Copy, Default)]
pub struct BucketBrigade {
    line: DelayLine,
    anti_alias: OnePole,
    reconstruction: OnePole,
    reconstruction_second: OnePole,
    compander: Compander,
    compander_second: Compander,
    sample_rate: f32,
}

impl BucketBrigade {
    pub fn prepare(&mut self, buffer: &mut [f32], sample_rate: f32, maximum_delay_seconds: f32) {
        let length = (sample_rate * maximum_delay_seconds) as usize + 8;
        self.line.prepare(buffer, length);
        self.sample_rate = sample_rate;
        // A MN3007 running fast enough for a 5 ms chorus delay clocks near
        // 100 kHz; the filters around it are set well below that.
        self.anti_alias = OnePole::new(6_500.0, sample_rate);
        self.reconstruction = OnePole::new(6_000.0, sample_rate);
        self.reconstruction_second = OnePole::new(6_000.0, sample_rate);
        self.compander.prepare(sample_rate);
        self.compander_second.prepare(sample_rate);
    }

    pub fn set_bandwidth(&mut self, cutoff_hz: f32) {
        self.anti_alias.set_cutoff(cutoff_hz, self.sample_rate);
        self.reconstruction.set_cutoff(cutoff_hz, self.sample_rate);
        self.reconstruction_second
            .set_cutoff(cutoff_hz, self.sample_rate);
    }

    pub fn clear(&mut self, buffer: &mut [f32]) {
        self.line.clear(buffer);
        self.anti_alias.reset();
        self.reconstruction.reset();
        self.reconstruction_second.reset();
        self.compander.reset();
        self.compander_second.reset();
    }

    #[inline]
    pub fn read(&self, buffer: &[f32], delay_seconds: f32) -> f32 {
        self.line.read(buffer, delay_seconds * self.sample_rate)
    }

    /// Reads the delayed sample and clocks one new sample in.
    #[inline]
    pub fn process(&mut self, buffer: &mut [f32], input: f32, delay_seconds: f32) -> f32 {
        let delayed = self.read(buffer, delay_seconds);
        let expanded = self.compander.expand(self.reconstruction.low(delayed));
        let compressed = self.compander.compress(self.anti_alias.low(input));
        self.line.write(buffer, compressed);
        expanded
    }

    /// Two taps off the same register, which is how one delay chip feeds a
    /// stereo output without a second chip.
    #[inline]
    pub fn process_stereo(
        &mut self,
        buffer: &mut [f32],
        input: f32,
        first_delay_seconds: f32,
        second_delay_seconds: f32,
    ) -> (f32, f32) {
        let first = self.read(buffer, first_delay_seconds);
        let second = self.read(buffer, second_delay_seconds);
        let first_out = self.compander.expand(self.reconstruction.low(first));
        let second_out = self
            .compander_second
            .expand(self.reconstruction_second.low(second));
        let compressed = self.compander.compress(self.anti_alias.low(input));
        self.line.write(buffer, compressed);
        (first_out, second_out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::{TAU, sin};

    #[test]
    fn a_delay_line_returns_what_was_written_a_known_time_ago() {
        let mut buffer = [0.0_f32; 512];
        let mut line = DelayLine::default();
        line.prepare(&mut buffer, 512);
        for index in 0..256 {
            line.write(&mut buffer, index as f32);
        }
        let delayed = line.read(&buffer, 10.0);
        assert!((delayed - 246.0).abs() < 0.01, "read back {delayed}");
    }

    #[test]
    fn fractional_reads_land_between_neighbours() {
        let mut buffer = [0.0_f32; 512];
        let mut line = DelayLine::default();
        line.prepare(&mut buffer, 512);
        for index in 0..256 {
            line.write(&mut buffer, index as f32);
        }
        let delayed = line.read(&buffer, 10.5);
        assert!((delayed - 245.5).abs() < 0.01, "read back {delayed}");
    }

    #[test]
    fn a_bucket_brigade_line_passes_a_recognisable_signal() {
        let sample_rate = 48_000.0;
        let mut buffer = [0.0_f32; 8_192];
        let mut bbd = BucketBrigade::default();
        bbd.prepare(&mut buffer, sample_rate, 0.15);
        let mut peak = 0.0_f32;
        for index in 0..24_000 {
            let input = 0.4 * sin(TAU * 440.0 * index as f32 / sample_rate);
            let output = bbd.process(&mut buffer, input, 0.05);
            if index > 12_000 {
                peak = peak.max(output.abs());
            }
        }
        assert!(peak > 0.05, "the delayed signal all but vanished: {peak}");
        assert!(peak < 2.0, "the compander ran away: {peak}");
    }
}
