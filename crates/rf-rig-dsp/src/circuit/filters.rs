//! The passive building blocks every pedal here is made of.
//!
//! A guitar pedal is mostly resistors and capacitors: coupling caps that set
//! where the bass stops, an RC pair that decides what reaches the clipper, a
//! tone stack that is nothing but two RC paths blended. Each of those maps to a
//! one-pole section, so the models below name their cutoffs in hertz and the
//! pedal modules derive those hertz from real component values.

use crate::math::{TAU, clamp, cos, exp, sanitise, sin, sqrt, tan};

/// A single RC section. `low` is the capacitor-to-ground response, `high` is
/// the series-capacitor response, and they always sum to the input.
#[derive(Clone, Copy, Default)]
pub struct OnePole {
    coefficient: f32,
    state: f32,
}

impl OnePole {
    pub fn new(cutoff_hz: f32, sample_rate: f32) -> Self {
        let mut filter = Self::default();
        filter.set_cutoff(cutoff_hz, sample_rate);
        filter
    }

    /// `1 - e^(-2*pi*fc/fs)` is the exact step response of an RC section
    /// sampled at `fs`, so the cutoff stays honest at every sample rate.
    pub fn set_cutoff(&mut self, cutoff_hz: f32, sample_rate: f32) {
        let cutoff = clamp(cutoff_hz, 0.1, sample_rate * 0.45);
        self.coefficient = 1.0 - exp(-TAU * cutoff / sample_rate);
    }

    pub fn reset(&mut self) {
        self.state = 0.0;
    }

    /// Forces the stored state. A smoothing section that starts at zero would
    /// sweep up to its target the first time it runs, which is right when a
    /// player turns a knob and wrong when a pedal is switched on with the knob
    /// already somewhere.
    pub fn set_value(&mut self, value: f32) {
        self.state = value;
    }

    #[inline]
    pub fn low(&mut self, input: f32) -> f32 {
        self.state = sanitise(self.state + self.coefficient * (input - self.state));
        self.state
    }

    #[inline]
    pub fn high(&mut self, input: f32) -> f32 {
        input - self.low(input)
    }

    #[inline]
    pub fn value(&self) -> f32 {
        self.state
    }
}

/// Series capacitor at the input of a stage. Every pedal has one; it is the
/// reason a fuzz cleans up differently on a bass than on a strat.
#[derive(Clone, Copy, Default)]
pub struct CouplingCap {
    section: OnePole,
}

impl CouplingCap {
    pub fn new(cutoff_hz: f32, sample_rate: f32) -> Self {
        Self {
            section: OnePole::new(cutoff_hz, sample_rate),
        }
    }

    pub fn set_cutoff(&mut self, cutoff_hz: f32, sample_rate: f32) {
        self.section.set_cutoff(cutoff_hz, sample_rate);
    }

    pub fn reset(&mut self) {
        self.section.reset();
    }

    #[inline]
    pub fn process(&mut self, input: f32) -> f32 {
        self.section.high(input)
    }
}

/// Removes the operating-point offset an asymmetric stage leaves behind.
#[derive(Clone, Copy)]
pub struct DcBlocker {
    pole: f32,
    last_input: f32,
    last_output: f32,
}

impl Default for DcBlocker {
    fn default() -> Self {
        Self {
            pole: 0.9995,
            last_input: 0.0,
            last_output: 0.0,
        }
    }
}

impl DcBlocker {
    pub fn new(sample_rate: f32) -> Self {
        let mut blocker = Self::default();
        blocker.prepare(sample_rate);
        blocker
    }

    pub fn prepare(&mut self, sample_rate: f32) {
        // A 5 Hz corner: below anything a guitar produces, above the drift an
        // asymmetric clipper introduces.
        self.pole = exp(-TAU * 5.0 / sample_rate);
        self.reset();
    }

    pub fn reset(&mut self) {
        self.last_input = 0.0;
        self.last_output = 0.0;
    }

    #[inline]
    pub fn process(&mut self, input: f32) -> f32 {
        let output = input - self.last_input + self.pole * self.last_output;
        self.last_input = input;
        self.last_output = sanitise(output);
        self.last_output
    }
}

/// Direct-form biquad with the usual RBJ designs. Used where a pedal has a
/// genuine second-order section: resonant tone stacks and the reconstruction
/// filters around a bucket-brigade line.
#[derive(Clone, Copy, Default)]
pub struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    x1: f32,
    x2: f32,
    y1: f32,
    y2: f32,
}

impl Biquad {
    pub fn reset(&mut self) {
        self.x1 = 0.0;
        self.x2 = 0.0;
        self.y1 = 0.0;
        self.y2 = 0.0;
    }

    pub fn set_lowpass(&mut self, cutoff_hz: f32, q: f32, sample_rate: f32) {
        let (cos_w, alpha) = Self::intermediates(cutoff_hz, q, sample_rate);
        let a0 = 1.0 + alpha;
        self.b0 = ((1.0 - cos_w) * 0.5) / a0;
        self.b1 = (1.0 - cos_w) / a0;
        self.b2 = self.b0;
        self.a1 = (-2.0 * cos_w) / a0;
        self.a2 = (1.0 - alpha) / a0;
    }

    pub fn set_highpass(&mut self, cutoff_hz: f32, q: f32, sample_rate: f32) {
        let (cos_w, alpha) = Self::intermediates(cutoff_hz, q, sample_rate);
        let a0 = 1.0 + alpha;
        self.b0 = ((1.0 + cos_w) * 0.5) / a0;
        self.b1 = -(1.0 + cos_w) / a0;
        self.b2 = self.b0;
        self.a1 = (-2.0 * cos_w) / a0;
        self.a2 = (1.0 - alpha) / a0;
    }

    pub fn set_peaking(&mut self, cutoff_hz: f32, q: f32, gain_db: f32, sample_rate: f32) {
        let amplitude = crate::math::exp(gain_db * (core::f32::consts::LN_10 / 40.0));
        let (cos_w, alpha) = Self::intermediates(cutoff_hz, q, sample_rate);
        let a0 = 1.0 + alpha / amplitude;
        self.b0 = (1.0 + alpha * amplitude) / a0;
        self.b1 = (-2.0 * cos_w) / a0;
        self.b2 = (1.0 - alpha * amplitude) / a0;
        self.a1 = self.b1;
        self.a2 = (1.0 - alpha / amplitude) / a0;
    }

    /// Installs coefficients computed elsewhere — by a solved network, for
    /// instance. The convention is the usual one: the denominator's leading
    /// coefficient is already normalised to 1.
    pub fn set_coefficients(&mut self, b0: f32, b1: f32, b2: f32, a1: f32, a2: f32) {
        if !(b0.is_finite() && b1.is_finite() && b2.is_finite() && a1.is_finite() && a2.is_finite())
        {
            return;
        }
        self.b0 = b0;
        self.b1 = b1;
        self.b2 = b2;
        self.a1 = a1;
        self.a2 = a2;
    }

    /// `[b0, b1, b2, a1, a2]`, for tests that need to evaluate the response
    /// rather than measure it.
    pub fn coefficients(&self) -> [f32; 5] {
        [self.b0, self.b1, self.b2, self.a1, self.a2]
    }

    fn intermediates(cutoff_hz: f32, q: f32, sample_rate: f32) -> (f32, f32) {
        let cutoff = clamp(cutoff_hz, 10.0, sample_rate * 0.45);
        let omega = TAU * cutoff / sample_rate;
        let sin_w = sin(omega);
        let cos_w = cos(omega);
        let alpha = sin_w / (2.0 * clamp(q, 0.05, 20.0));
        (cos_w, alpha)
    }

    #[inline]
    pub fn process(&mut self, input: f32) -> f32 {
        let output = self.b0 * input + self.b1 * self.x1 + self.b2 * self.x2
            - self.a1 * self.y1
            - self.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = input;
        self.y2 = self.y1;
        self.y1 = sanitise(output);
        self.y1
    }
}

/// Topology-preserving state variable filter. Its lowpass and highpass outputs
/// stay complementary while the cutoff moves, which matters for the reverb
/// damping and for anything swept by an LFO.
#[derive(Clone, Copy, Default)]
pub struct StateVariable {
    g: f32,
    r2: f32,
    denominator: f32,
    integrator_1: f32,
    integrator_2: f32,
}

impl StateVariable {
    pub fn new(cutoff_hz: f32, q: f32, sample_rate: f32) -> Self {
        let mut filter = Self::default();
        filter.set(cutoff_hz, q, sample_rate);
        filter
    }

    pub fn set(&mut self, cutoff_hz: f32, q: f32, sample_rate: f32) {
        let cutoff = clamp(cutoff_hz, 10.0, sample_rate * 0.45);
        self.g = tan(core::f32::consts::PI * cutoff / sample_rate);
        self.r2 = 1.0 / clamp(q, 0.05, 20.0);
        self.denominator = 1.0 / (1.0 + self.r2 * self.g + self.g * self.g);
    }

    pub fn reset(&mut self) {
        self.integrator_1 = 0.0;
        self.integrator_2 = 0.0;
    }

    #[inline]
    pub fn process(&mut self, input: f32) -> SvfOutput {
        let highpass =
            (input - (self.r2 + self.g) * self.integrator_1 - self.integrator_2) * self.denominator;
        let bandpass = highpass * self.g + self.integrator_1;
        let lowpass = bandpass * self.g + self.integrator_2;
        self.integrator_1 = sanitise(bandpass + highpass * self.g);
        self.integrator_2 = sanitise(lowpass + bandpass * self.g);
        SvfOutput {
            lowpass,
            bandpass,
            highpass,
        }
    }
}

#[derive(Clone, Copy, Default)]
pub struct SvfOutput {
    pub lowpass: f32,
    pub bandpass: f32,
    pub highpass: f32,
}

/// First-order allpass in the z-domain. Cascades of these are the dispersion
/// of a spring tank and the diffusion of a plate.
#[derive(Clone, Copy, Default)]
pub struct Allpass1 {
    coefficient: f32,
    last_input: f32,
    last_output: f32,
}

impl Allpass1 {
    pub const fn new(coefficient: f32) -> Self {
        Self {
            coefficient,
            last_input: 0.0,
            last_output: 0.0,
        }
    }

    pub fn reset(&mut self) {
        self.last_input = 0.0;
        self.last_output = 0.0;
    }

    #[inline]
    pub fn process(&mut self, input: f32) -> f32 {
        let output = self.coefficient * (input - self.last_output) + self.last_input;
        self.last_input = input;
        self.last_output = sanitise(output);
        self.last_output
    }
}

/// Root-mean-square of a signal over a short window, used by the compander
/// that surrounds a bucket-brigade line.
#[derive(Clone, Copy, Default)]
pub struct RmsFollower {
    section: OnePole,
}

impl RmsFollower {
    pub fn new(window_hz: f32, sample_rate: f32) -> Self {
        Self {
            section: OnePole::new(window_hz, sample_rate),
        }
    }

    pub fn reset(&mut self) {
        self.section.reset();
    }

    #[inline]
    pub fn process(&mut self, input: f32) -> f32 {
        let mean_square = self.section.low(input * input);
        if mean_square <= 0.0 {
            return 0.0;
        }
        sqrt(mean_square)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::{TAU, abs, sin};

    fn magnitude_at(sample_rate: f32, frequency: f32, mut process: impl FnMut(f32) -> f32) -> f32 {
        let samples = (sample_rate * 0.5) as usize;
        let mut peak = 0.0_f32;
        for index in 0..samples {
            let phase = TAU * frequency * index as f32 / sample_rate;
            let output = process(sin(phase));
            // Ignore the first half so the filter settles first.
            if index > samples / 2 {
                peak = peak.max(abs(output));
            }
        }
        peak
    }

    #[test]
    fn one_pole_is_minus_three_decibels_at_its_cutoff() {
        let sample_rate = 48_000.0;
        let mut filter = OnePole::new(1_000.0, sample_rate);
        let magnitude = magnitude_at(sample_rate, 1_000.0, |input| filter.low(input));
        // A first-order section is down by 1/sqrt(2) at its corner. That is the
        // definition of the corner, not a fitted number.
        assert!(
            (magnitude - core::f32::consts::FRAC_1_SQRT_2).abs() < 0.02,
            "expected -3 dB at the cutoff, measured {magnitude}"
        );
    }

    #[test]
    fn one_pole_outputs_sum_back_to_the_input() {
        let sample_rate = 48_000.0;
        let mut filter = OnePole::new(400.0, sample_rate);
        let mut state = filter;
        for index in 0..512 {
            let input = sin(TAU * 220.0 * index as f32 / sample_rate);
            let low = filter.low(input);
            let high = state.high(input);
            assert!((low + high - input).abs() < 1.0e-5);
        }
    }

    #[test]
    fn dc_blocker_removes_a_constant_offset() {
        let mut blocker = DcBlocker::new(48_000.0);
        let mut last = 0.0;
        for _ in 0..48_000 {
            last = blocker.process(0.5);
        }
        assert!(abs(last) < 1.0e-3, "offset survived: {last}");
    }

    #[test]
    fn allpass_preserves_energy() {
        let mut allpass = Allpass1::new(0.6);
        let mut input_energy = 0.0;
        let mut output_energy = 0.0;
        for index in 0..4_096 {
            let input = sin(TAU * 700.0 * index as f32 / 48_000.0);
            let output = allpass.process(input);
            if index > 512 {
                input_energy += input * input;
                output_energy += output * output;
            }
        }
        let ratio = output_energy / input_energy;
        assert!((ratio - 1.0).abs() < 0.02, "allpass gain was {ratio}");
    }
}
