//! The nonlinear elements, solved rather than shaped.
//!
//! A stompbox clipper is one equation. Whatever current the stage pushes into
//! the clipping network has to leave through two paths: the resistor, and the
//! diodes. Kirchhoff writes it in one line,
//!
//! ```text
//! I_in = V/R + 2*Is*sinh(V / (n*Vt))
//! ```
//!
//! and the *shape* everybody tries to approximate with a `tanh` is just the
//! solution of that line. RF-Rig solves it with Newton's method, warm-started
//! from the previous sample, so a diode change is a change of `Is` and `n` —
//! the numbers on a datasheet — instead of a hand-tuned curve.
//!
//! Reference points for the parts used here are recorded in
//! `docs/CIRCUIT_MODELING.md`.

use crate::math::{abs, clamp, exp, tanh};

/// A diode described the way its datasheet describes it.
#[derive(Clone, Copy)]
pub struct Diode {
    /// Reverse saturation current `Is`, in amperes.
    pub saturation_current: f32,
    /// `n * Vt`: the emission coefficient times the thermal voltage, in volts.
    /// At room temperature `Vt` is 25.85 mV.
    pub emission_voltage: f32,
}

impl Diode {
    /// 1N4148 small-signal silicon: the pair inside most overdrive feedback
    /// loops. `Is = 2.52 nA`, `n = 1.752`.
    pub const SILICON: Self = Self {
        saturation_current: 2.52e-9,
        emission_voltage: 1.752 * 0.02585,
    };

    /// Germanium (OA/1N34A family): conducts far earlier, so it rounds the
    /// waveform at a lower level and sounds softer at the same drive.
    pub const GERMANIUM: Self = Self {
        saturation_current: 2.0e-6,
        emission_voltage: 1.2 * 0.02585,
    };

    /// A red LED clipping pair: roughly 1.7 V forward, which is why an LED mod
    /// is louder and cleaner than the stock silicon pair.
    pub const LED: Self = Self {
        saturation_current: 1.0e-16,
        emission_voltage: 2.0 * 0.02585,
    };

    /// Two of the same diode in series on each leg. Doubling the forward drop
    /// is the other classic clipping mod.
    pub const fn stacked(self, count: f32) -> Self {
        Self {
            saturation_current: self.saturation_current,
            emission_voltage: self.emission_voltage * count,
        }
    }
}

/// Solves `I = V/R + 2*Is*sinh(V/(n*Vt))` for `V`.
///
/// The same equation covers both classic clipping topologies:
/// * diodes across the feedback resistor of an op-amp stage (soft clipping,
///   the overdrive), where `V` is the voltage the stage adds to its input;
/// * diodes from the signal node to ground behind a series resistor (hard
///   clipping, the distortion), where `V` is the node voltage itself.
#[derive(Clone, Copy, Default)]
pub struct ClipperSolver {
    voltage: f32,
}

impl ClipperSolver {
    /// Newton iterations per sample. Warm-started from the previous sample the
    /// solver is normally converged in two; the cap bounds the worst case so
    /// the audio callback stays predictable.
    const MAX_ITERATIONS: usize = 6;
    const TOLERANCE: f32 = 1.0e-7;
    /// Keeps the exponential inside `f32` range even if a caller asks for
    /// something absurd. At the 45 mV emission voltage of a silicon pair this
    /// is 2.7 V — past any supply rail a nine-volt pedal can reach, and well
    /// past the knee of the highest-voltage clipping option.
    const ARGUMENT_LIMIT: f32 = 60.0;

    pub fn reset(&mut self) {
        self.voltage = 0.0;
    }

    #[inline]
    pub fn solve(&mut self, drive_current: f32, resistance: f32, diode: Diode) -> f32 {
        let conductance = 1.0 / resistance;
        let emission = diode.emission_voltage;
        let scale = 2.0 * diode.saturation_current;
        let mut voltage = self.voltage;
        let step_limit = 8.0 * emission + 0.05;

        for _ in 0..Self::MAX_ITERATIONS {
            let argument = clamp(
                voltage / emission,
                -Self::ARGUMENT_LIMIT,
                Self::ARGUMENT_LIMIT,
            );
            // One exponential serves both hyperbolic functions. The solver runs
            // four times per sample per clipping stage, so this halves the most
            // expensive part of the audio callback.
            let positive = exp(argument);
            let negative = 1.0 / positive;
            let sinh_argument = 0.5 * (positive - negative);
            let cosh_argument = 0.5 * (positive + negative);
            let current = voltage * conductance + scale * sinh_argument - drive_current;
            let slope = conductance + (scale / emission) * cosh_argument;
            let mut step = current / slope;
            step = clamp(step, -step_limit, step_limit);
            voltage -= step;
            if abs(step) < Self::TOLERANCE {
                break;
            }
        }

        if !voltage.is_finite() {
            voltage = 0.0;
        }
        self.voltage = voltage;
        voltage
    }
}

/// A saturating gain stage standing in for a discrete transistor amplifier.
///
/// The rails are asymmetric on purpose: a 9 V pedal biases its transistor near
/// one third of the supply, so the waveform runs out of room in one direction
/// first. That asymmetry is what puts even harmonics in a booster.
#[derive(Clone, Copy)]
pub struct SaturatingStage {
    pub gain: f32,
    pub positive_headroom: f32,
    pub negative_headroom: f32,
}

impl Default for SaturatingStage {
    /// Unity gain with symmetric headroom well above any signal the chain
    /// carries: a stage that has not been configured yet must not colour
    /// anything.
    fn default() -> Self {
        Self::new(1.0, 100.0, 100.0)
    }
}

impl SaturatingStage {
    pub const fn new(gain: f32, positive_headroom: f32, negative_headroom: f32) -> Self {
        Self {
            gain,
            positive_headroom,
            negative_headroom,
        }
    }

    #[inline]
    pub fn process(&self, input: f32) -> f32 {
        let amplified = input * self.gain;
        let headroom = if amplified >= 0.0 {
            self.positive_headroom
        } else {
            self.negative_headroom
        };
        headroom * tanh(amplified / headroom)
    }
}

/// Antiderivative-antialiased `tanh`.
///
/// Used where a Newton solve would be wasted: the soft limit inside a delay's
/// feedback loop, the recovery stage of a compressor. First-order ADAA removes
/// most of the aliasing a memoryless shaper would fold back, at the cost of one
/// sample of state and a half-sample delay.
#[derive(Clone, Copy, Default)]
pub struct SoftLimiter {
    previous_input: f32,
    previous_antiderivative: f32,
}

impl SoftLimiter {
    const EPSILON: f32 = 1.0e-5;

    pub fn reset(&mut self) {
        self.previous_input = 0.0;
        self.previous_antiderivative = 0.0;
    }

    /// `F1(x) = ln(cosh(x))`, evaluated so it cannot overflow for large `x`.
    #[inline]
    fn antiderivative(input: f32) -> f32 {
        let magnitude = abs(input);
        magnitude + crate::math::ln(1.0 + crate::math::exp(-2.0 * magnitude))
            - core::f32::consts::LN_2
    }

    #[inline]
    pub fn process(&mut self, input: f32) -> f32 {
        let antiderivative = Self::antiderivative(input);
        let difference = input - self.previous_input;
        let output = if abs(difference) < Self::EPSILON {
            tanh((input + self.previous_input) * 0.5)
        } else {
            (antiderivative - self.previous_antiderivative) / difference
        };
        self.previous_input = input;
        self.previous_antiderivative = antiderivative;
        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::{TAU, sin};

    #[test]
    fn a_silicon_pair_clamps_near_its_forward_drop() {
        let mut solver = ClipperSolver::default();
        // One milliamp into a 51k feedback resistor: far more than the diodes
        // will let past their knee.
        let voltage = solver.solve(1.0e-3, 51_000.0, Diode::SILICON);
        assert!(
            (0.45..0.75).contains(&voltage),
            "silicon clamped at {voltage} V"
        );
    }

    #[test]
    fn germanium_clamps_lower_than_silicon_and_leds_clamp_higher() {
        let mut germanium = ClipperSolver::default();
        let mut silicon = ClipperSolver::default();
        let mut led = ClipperSolver::default();
        let drive = 1.0e-3;
        let resistance = 51_000.0;
        let germanium_voltage = germanium.solve(drive, resistance, Diode::GERMANIUM);
        let silicon_voltage = silicon.solve(drive, resistance, Diode::SILICON);
        let led_voltage = led.solve(drive, resistance, Diode::LED);
        assert!(germanium_voltage < silicon_voltage);
        assert!(silicon_voltage < led_voltage);
    }

    #[test]
    fn small_signals_pass_through_the_resistor_untouched() {
        let mut solver = ClipperSolver::default();
        // A microamp is well below the knee: the diodes are effectively open
        // and the stage should behave like the resistor alone.
        let resistance = 51_000.0;
        let voltage = solver.solve(1.0e-6, resistance, Diode::SILICON);
        let linear = 1.0e-6 * resistance;
        assert!(
            (voltage - linear).abs() / linear < 0.05,
            "expected {linear} V, solved {voltage} V"
        );
    }

    #[test]
    fn the_solver_stays_converged_while_a_sine_sweeps_it() {
        let mut solver = ClipperSolver::default();
        let mut peak = 0.0_f32;
        for index in 0..4_800 {
            let current = 2.0e-3 * sin(TAU * 220.0 * index as f32 / 48_000.0);
            let voltage = solver.solve(current, 51_000.0, Diode::SILICON);
            assert!(voltage.is_finite());
            peak = peak.max(voltage.abs());
        }
        assert!(peak < 1.0, "clipper ran away to {peak} V");
    }

    #[test]
    fn the_soft_limiter_tracks_tanh_on_slow_signals() {
        let mut limiter = SoftLimiter::default();
        let mut worst = 0.0_f32;
        for index in 0..4_800 {
            let input = 2.0 * sin(TAU * 50.0 * index as f32 / 48_000.0);
            let output = limiter.process(input);
            if index > 100 {
                worst = worst.max((output - crate::math::tanh(input)).abs());
            }
        }
        assert!(worst < 0.02, "ADAA drifted from tanh by {worst}");
    }

    #[test]
    fn the_saturating_stage_is_asymmetric() {
        let stage = SaturatingStage::new(20.0, 4.5, 3.0);
        let positive = stage.process(0.5);
        let negative = stage.process(-0.5);
        assert!(positive > negative.abs());
    }
}
