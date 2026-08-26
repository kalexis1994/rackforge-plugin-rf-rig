//! Chorus — one bucket-brigade line whose clock is swept by an LFO.
//!
//! A analog chorus does not crossfade between two copies of the signal. It runs
//! the signal through a clocked shift register and *moves the clock*, so the
//! delayed copy is genuinely pitch-shifted while the sweep is in motion. That
//! is why the effect sounds like two players rather than like a filter, and it
//! is why the model here modulates a delay time instead of an amplitude.
//!
//! The register is noisy enough that every design of this kind wraps it in a
//! compander, and the companding is not perfectly complementary — some of the
//! warmth people attribute to "analog" is that mismatch.

use crate::Frame;
use crate::circuit::delay::BucketBrigade;
use crate::circuit::dynamics::{Lfo, LfoShape};
use crate::math::clamp;

/// Delay at the centre of the sweep.
const CENTRE_DELAY_SECONDS: f32 = 0.0045;
/// Maximum excursion either side of it at full depth.
const MAXIMUM_SWEEP_SECONDS: f32 = 0.0030;
/// What the line has to be able to hold.
pub const MAXIMUM_DELAY_SECONDS: f32 = 0.020;

#[derive(Clone, Copy, Default)]
pub struct Chorus {
    line: BucketBrigade,
    lfo: Lfo,
    sweep: f32,
    mix: f32,
    sample_rate: f32,
}

impl Chorus {
    pub fn prepare(&mut self, buffer: &mut [f32], sample_rate: f32) {
        self.sample_rate = sample_rate;
        self.line
            .prepare(buffer, sample_rate, MAXIMUM_DELAY_SECONDS);
        self.lfo.set_shape(LfoShape::Triangle);
        self.lfo.set_rate(0.8, sample_rate);
        self.sweep = MAXIMUM_SWEEP_SECONDS * 0.5;
        self.mix = 0.5;
    }

    pub fn reset(&mut self, buffer: &mut [f32]) {
        self.line.clear(buffer);
        self.lfo.reset();
    }

    pub fn set_controls(&mut self, rate_hz: f32, depth: f32, mix: f32) {
        self.lfo.set_rate(rate_hz, self.sample_rate);
        self.sweep = MAXIMUM_SWEEP_SECONDS * clamp(depth, 0.0, 1.0);
        self.mix = clamp(mix, 0.0, 1.0);
    }

    #[inline]
    pub fn process(&mut self, buffer: &mut [f32], frame: Frame) -> Frame {
        let mut frame = frame;
        let dry = frame.to_mono();

        let sweep = self.lfo.tick();
        let quadrature = self.lfo.quadrature();
        let first = CENTRE_DELAY_SECONDS + self.sweep * sweep;
        let second = CENTRE_DELAY_SECONDS + self.sweep * quadrature;

        let (wet_left, wet_right) = self.line.process_stereo(buffer, dry, first, second);

        if self.mix <= 0.001 {
            frame.set_mono(dry);
            return frame;
        }
        // Two taps off one register: the two sides are different sweeps of the
        // same signal, which is a real stereo image rather than a delayed copy.
        frame.set_stereo(dry + wet_left * self.mix, dry + wet_right * self.mix);
        frame
    }

    /// The delay currently at the first tap, in seconds. Used by the lab tool
    /// to plot the sweep against the LFO.
    pub fn centre_delay_seconds(&self) -> f32 {
        CENTRE_DELAY_SECONDS
    }

    pub fn sweep_seconds(&self) -> f32 {
        self.sweep
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::{TAU, sin};
    use crate::testing::{peak, rms};
    use std::vec::Vec;

    fn run(
        chorus: &mut Chorus,
        buffer: &mut [f32],
        samples: usize,
        sample_rate: f32,
    ) -> Vec<Frame> {
        let mut frames = Vec::with_capacity(samples);
        for index in 0..samples {
            let input = 0.3 * sin(TAU * 440.0 * index as f32 / sample_rate);
            frames.push(chorus.process(buffer, Frame::mono(input)));
        }
        frames
    }

    #[test]
    fn a_dry_setting_leaves_the_signal_alone() {
        let sample_rate = 48_000.0;
        let mut buffer = [0.0_f32; 2_048];
        let mut chorus = Chorus::default();
        chorus.prepare(&mut buffer, sample_rate);
        chorus.set_controls(1.0, 0.5, 0.0);
        let frames = run(&mut chorus, &mut buffer, 4_096, sample_rate);
        assert!(frames.iter().all(|frame| !frame.stereo));
        let left: Vec<f32> = frames.iter().map(|frame| frame.left).collect();
        assert!(rms(&left) > 0.15, "the dry path lost the signal");
    }

    #[test]
    fn the_two_taps_differ_once_the_effect_is_engaged() {
        let sample_rate = 48_000.0;
        let mut buffer = [0.0_f32; 2_048];
        let mut chorus = Chorus::default();
        chorus.prepare(&mut buffer, sample_rate);
        chorus.set_controls(1.5, 1.0, 1.0);
        let frames = run(&mut chorus, &mut buffer, 48_000, sample_rate);
        let difference: Vec<f32> = frames
            .iter()
            .skip(24_000)
            .map(|frame| frame.left - frame.right)
            .collect();
        assert!(
            rms(&difference) > 0.02,
            "the two sides are identical: {}",
            rms(&difference)
        );
        assert!(frames.last().unwrap().stereo);
    }

    #[test]
    fn the_output_stays_bounded_with_everything_at_maximum() {
        let sample_rate = 48_000.0;
        let mut buffer = [0.0_f32; 2_048];
        let mut chorus = Chorus::default();
        chorus.prepare(&mut buffer, sample_rate);
        chorus.set_controls(8.0, 1.0, 1.0);
        let frames = run(&mut chorus, &mut buffer, 48_000, sample_rate);
        let left: Vec<f32> = frames.iter().map(|frame| frame.left).collect();
        let level = peak(&left);
        assert!(level.is_finite() && level < 4.0, "peak reached {level}");
    }
}
