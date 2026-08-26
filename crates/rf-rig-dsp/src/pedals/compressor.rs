//! Compressor — an OTA whose gain is set by a detector watching its own
//! output.
//!
//! The family this models (a transconductance amplifier with the rectifier on
//! the *output* side) behaves differently from a studio compressor with a
//! ratio control. Because the detector sees what already left the pedal, the
//! loop drives the gain until the output level stops moving: the effective
//! ratio is enormous, the knee is soft, and the recovery is program dependent.
//! That is the whole character, and it falls out of the topology rather than
//! out of a ratio parameter.
//!
//! Controls follow the two-knob original plus the attack trimmer later units
//! exposed: `sustain` sets how hard the loop squeezes, `attack` sets the
//! detector timing, `level` is make-up gain.

use crate::circuit::dynamics::EnvelopeFollower;
use crate::circuit::filters::{CouplingCap, OnePole};
use crate::circuit::nonlinear::SoftLimiter;
use crate::math::{clamp, exponential, lerp};

#[derive(Clone, Copy, Default)]
pub struct Compressor {
    input_cap: CouplingCap,
    detector_highpass: OnePole,
    detector: EnvelopeFollower,
    output_lowpass: OnePole,
    limiter: SoftLimiter,
    previous_output: f32,
    gain: f32,
    depth: f32,
    makeup: f32,
    sample_rate: f32,
}

impl Compressor {
    pub fn prepare(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate;
        // 47 nF into the 1 M input impedance is a corner far below the
        // instrument; what actually sets the bass response is the detector's
        // own highpass, below.
        self.input_cap = CouplingCap::new(20.0, sample_rate);
        // Feeding the rectifier through a highpass is why a low E does not
        // duck the whole chain: the detector is deliberately deaf to it.
        self.detector_highpass = OnePole::new(120.0, sample_rate);
        self.output_lowpass = OnePole::new(9_000.0, sample_rate);
        self.detector = EnvelopeFollower::new(6.0, 260.0, sample_rate);
        self.gain = 1.0;
        self.depth = 8.0;
        self.makeup = 3.2;
        self.reset();
    }

    pub fn reset(&mut self) {
        self.input_cap.reset();
        self.detector_highpass.reset();
        self.detector.reset();
        self.output_lowpass.reset();
        self.limiter.reset();
        self.previous_output = 0.0;
        self.gain = 1.0;
    }

    /// `sustain`, `attack` and `level` all arrive as 0..1 pot positions.
    pub fn set_controls(&mut self, sustain: f32, attack: f32, level: f32) {
        // How much control current the detector produces. The top of the pot
        // is the "everything sustains forever" setting the pedal is known for.
        self.depth = exponential(clamp(sustain, 0.0, 1.0), 1.5, 260.0);
        // The attack trimmer moves the whole timing network, so release
        // follows attack instead of being independent.
        let attack_ms = lerp(1.5, 28.0, clamp(attack, 0.0, 1.0));
        let release_ms = lerp(120.0, 700.0, clamp(attack, 0.0, 1.0));
        self.detector
            .set_times(attack_ms, release_ms, self.sample_rate);
        self.makeup = exponential(clamp(level, 0.0, 1.0), 0.5, 20.0);
    }

    #[inline]
    pub fn process(&mut self, input: f32) -> f32 {
        let signal = self.input_cap.process(input);

        // Feedback detection: the rectifier watches the previous output, not
        // the input.
        let observed = self.detector_highpass.high(self.previous_output);
        let envelope = self.detector.process(observed);
        self.gain = 1.0 / (1.0 + self.depth * envelope);

        let amplified = signal * self.gain * self.makeup;
        // The OTA runs out of headroom before the supply does; the soft limit
        // is what keeps a hard pick attack from squaring off.
        let limited = self.limiter.process(amplified * 0.8) * 1.25;
        let output = self.output_lowpass.low(limited);
        self.previous_output = output;
        output
    }

    /// Current gain reduction as a linear factor, for the lab tool.
    pub fn gain_reduction(&self) -> f32 {
        self.gain
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::{TAU, sin};

    fn peak_of(compressor: &mut Compressor, amplitude: f32, sample_rate: f32) -> f32 {
        let mut peak = 0.0_f32;
        let samples = sample_rate as usize; // one second, so the loop settles
        for index in 0..samples {
            let input = amplitude * sin(TAU * 220.0 * index as f32 / sample_rate);
            let output = compressor.process(input);
            if index > samples / 2 {
                peak = peak.max(output.abs());
            }
        }
        peak
    }

    #[test]
    fn twelve_decibels_in_becomes_far_less_than_twelve_decibels_out() {
        let sample_rate = 48_000.0;
        let mut compressor = Compressor::default();
        compressor.prepare(sample_rate);
        compressor.set_controls(0.8, 0.5, 0.5);

        let quiet = peak_of(&mut compressor, 0.05, sample_rate);
        compressor.reset();
        let loud = peak_of(&mut compressor, 0.2, sample_rate);

        let input_ratio = 0.2 / 0.05;
        let output_ratio = loud / quiet;
        assert!(
            output_ratio < input_ratio * 0.5,
            "a 12 dB input step produced {output_ratio}x at the output"
        );
    }

    #[test]
    fn the_sustain_control_changes_how_hard_it_squeezes() {
        let sample_rate = 48_000.0;
        let mut gentle = Compressor::default();
        gentle.prepare(sample_rate);
        gentle.set_controls(0.05, 0.5, 0.5);
        let mut heavy = Compressor::default();
        heavy.prepare(sample_rate);
        heavy.set_controls(1.0, 0.5, 0.5);

        let gentle_peak = peak_of(&mut gentle, 0.3, sample_rate);
        let heavy_peak = peak_of(&mut heavy, 0.3, sample_rate);
        assert!(
            heavy_peak < gentle_peak,
            "more sustain did not reduce the level: {heavy_peak} vs {gentle_peak}"
        );
        assert!(heavy.gain_reduction() < gentle.gain_reduction());
    }

    #[test]
    fn silence_in_is_silence_out() {
        let mut compressor = Compressor::default();
        compressor.prepare(48_000.0);
        compressor.set_controls(1.0, 0.5, 1.0);
        for _ in 0..48_000 {
            assert_eq!(compressor.process(0.0), 0.0);
        }
    }
}
