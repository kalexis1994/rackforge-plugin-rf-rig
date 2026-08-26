//! Delay lines, including the bucket-brigade model the chorus and the analog
//! echo are built on.
//!
//! The buffers live outside these structs. RF-Rig never allocates: the plugin
//! hands the engine one static block at activation and every line borrows a
//! slice of it for the duration of a call. That is also why a line carries its
//! own length instead of trusting the slice it is given.

use crate::circuit::filters::{OnePole, RmsFollower};
use crate::math::{abs, clamp, floor, sanitise};

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
/// The delay control is the clock, and the clock is the sampling rate:
///
/// ```text
/// delay = stages / (2 · f_clock)
/// ```
///
/// Almost everything people say about these chips comes out of that single
/// relation.
///
/// * **Longer delay means darker.** A 4096-stage register set to 400 ms is
///   clocked at 5.1 kHz, so its Nyquist is 2.6 kHz — the repeats *have* to be
///   dark. The same chip at 40 ms clocks ten times faster and sounds it. A
///   fixed lowpass cannot do that, and a fixed lowpass is what this model had.
/// * **A swept clock breathes.** In a chorus the clock moves with the LFO, so
///   the register's bandwidth moves too: the top dulls slightly at one end of
///   the sweep and opens at the other.
/// * **Charge transfer is imperfect.** Each stage keeps a little of the packet
///   behind, which across N stages is a lowpass whose corner is a fixed
///   fraction of the clock — the same fraction whatever the delay, which is
///   why the two effects above are one effect.
///
/// What is *not* modelled: the aliasing a real register folds back when the
/// clock is low and the anti-alias filter is gentle. The band-limiting that
/// dominates the sound is here; the fold-back is not.
#[derive(Clone, Copy, Default)]
pub struct BucketBrigade {
    line: DelayLine,
    anti_alias: OnePole,
    reconstruction: OnePole,
    reconstruction_second: OnePole,
    compander: Compander,
    compander_second: Compander,
    sample_rate: f32,
    /// How many stages the register has. An MN3007 has 1024, an MN3005 4096.
    stages: f32,
    /// The widest the filters are allowed to open, whatever the clock says:
    /// the fixed filters around the chip.
    ceiling_hz: f32,
    clock_hz: f32,
}

/// The fraction of the clock at which charge-transfer loss has taken the
/// register's response down. Published measurements of these parts put the
/// usable bandwidth somewhere near a quarter of the clock, well below the
/// Nyquist the sampling alone would allow.
const BANDWIDTH_PER_CLOCK: f32 = 0.25;

impl BucketBrigade {
    pub fn prepare(
        &mut self,
        buffer: &mut [f32],
        sample_rate: f32,
        maximum_delay_seconds: f32,
        stages: f32,
    ) {
        let length = (sample_rate * maximum_delay_seconds) as usize + 8;
        self.line.prepare(buffer, length);
        self.sample_rate = sample_rate;
        self.stages = stages.max(16.0);
        // The fixed filters soldered around the chip. The clock-dependent part
        // is applied on top of these, and whichever is lower wins.
        self.ceiling_hz = 6_000.0;
        self.anti_alias = OnePole::new(self.ceiling_hz, sample_rate);
        self.reconstruction = OnePole::new(self.ceiling_hz, sample_rate);
        self.reconstruction_second = OnePole::new(self.ceiling_hz, sample_rate);
        self.compander.prepare(sample_rate);
        self.compander_second.prepare(sample_rate);
        self.clock_hz = 0.0;
    }

    /// The fixed filters around the register.
    pub fn set_ceiling(&mut self, cutoff_hz: f32) {
        self.ceiling_hz = clamp(cutoff_hz, 200.0, 20_000.0);
        self.clock_hz = 0.0;
    }

    /// The clock this delay implies, in hertz.
    pub fn clock_for(&self, delay_seconds: f32) -> f32 {
        let delay = delay_seconds.max(1.0e-6);
        self.stages / (2.0 * delay)
    }

    /// The register's bandwidth at its current clock, in hertz.
    pub fn bandwidth(&self) -> f32 {
        (self.clock_hz * BANDWIDTH_PER_CLOCK).min(self.ceiling_hz)
    }

    /// Points the filters at the clock this delay implies. Cheap enough to call
    /// per sample — the coefficients only move when the delay does.
    #[inline]
    fn follow_clock(&mut self, delay_seconds: f32) {
        let clock = self.clock_for(delay_seconds);
        // A tenth of a percent of clock movement is inaudible; skipping those
        // keeps a swept delay from recomputing coefficients on every sample.
        if abs(clock - self.clock_hz) < self.clock_hz * 0.001 {
            return;
        }
        self.clock_hz = clock;
        let cutoff = self.bandwidth();
        self.anti_alias.set_cutoff(cutoff, self.sample_rate);
        self.reconstruction.set_cutoff(cutoff, self.sample_rate);
        self.reconstruction_second
            .set_cutoff(cutoff, self.sample_rate);
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
        self.follow_clock(delay_seconds);
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
        self.follow_clock(first_delay_seconds);
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
    fn the_clock_is_the_delay_control() {
        // 4096 stages at 400 ms is a 5.1 kHz clock, and a quarter of that is
        // all the bandwidth the register has left.
        let sample_rate = 48_000.0;
        let mut buffer = [0.0_f32; 8_192];
        let mut bbd = BucketBrigade::default();
        bbd.prepare(&mut buffer, sample_rate, 0.15, 4_096.0);
        let slow = bbd.clock_for(0.4);
        let quick = bbd.clock_for(0.04);
        assert!(
            (4_500.0..5_800.0).contains(&slow),
            "400 ms should clock near 5 kHz, got {slow}"
        );
        assert!(
            (quick / slow - 10.0).abs() < 0.01,
            "ten times the delay should be a tenth of the clock"
        );
    }

    #[test]
    fn a_longer_delay_is_a_darker_one() {
        // The characteristic every analog echo has and no fixed filter can
        // give: the repeats darken as the delay lengthens, because the clock
        // slows down and takes the register's bandwidth with it.
        let sample_rate = 48_000.0;
        let treble_through = |delay: f32| {
            let mut buffer = [0.0_f32; 32_768];
            let mut bbd = BucketBrigade::default();
            bbd.prepare(&mut buffer, sample_rate, 0.5, 4_096.0);
            let mut output = std::vec::Vec::new();
            for index in 0..48_000 {
                let value = 0.3 * sin(TAU * 3_000.0 * index as f32 / sample_rate);
                let sample = bbd.process(&mut buffer, value, delay);
                if index > 24_000 {
                    output.push(sample);
                }
            }
            crate::testing::magnitude_at(&output, 3_000.0, sample_rate)
        };

        let short = treble_through(0.04);
        let long = treble_through(0.4);
        assert!(
            long < short * 0.5,
            "the long delay should be much darker: {long} against {short}"
        );
    }

    #[test]
    fn a_bucket_brigade_line_passes_a_recognisable_signal() {
        let sample_rate = 48_000.0;
        let mut buffer = [0.0_f32; 8_192];
        let mut bbd = BucketBrigade::default();
        bbd.prepare(&mut buffer, sample_rate, 0.15, 1_024.0);
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
