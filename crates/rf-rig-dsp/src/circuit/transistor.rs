//! Bipolar stages, solved from Ebers-Moll.
//!
//! A transistor gain stage is not a gain and a limit. It is a device obeying
//!
//! ```text
//! Ic = Is·(exp(Vbe/Vt) − 1)
//! ```
//!
//! sitting in a bias network that decides where on that exponential it rests,
//! and the whole character of a booster or a fuzz stage comes from that resting
//! point: how much room the collector has in each direction, how the emitter
//! resistor pushes back, how the operating point moves when the signal is large
//! enough to drag it.
//!
//! This module solves the stage rather than shaping it. Three unknowns — the
//! base, collector and emitter voltages — and three node equations, one Newton
//! iteration warm-started from the previous sample:
//!
//! ```text
//! base:       (Vin−Vcap−Vb)/Rin + (Vc−Vb)/Rf + Id(Vc−Vb) − Ic/β = 0
//! collector:  (Vcc−Vc)/Rc − Ic − (Vc−Vb)/Rf − Id(Vc−Vb)         = 0
//! emitter:    Ic(1 + 1/β) − Ve/Re                               = 0
//! ```
//!
//! `Vcap` is the charge on the input coupling capacitor, integrated alongside
//! the solve. It is not a detail: without it the input resistor would be a DC
//! path to ground, the bias network could not hold the base at a forward drop,
//! and the stage would starve — which is what the solver reported the first
//! time this was written without one.
//!
//! `Id` is the optional clipping pair across the feedback resistor, solved in
//! the same system rather than bolted on afterwards — which matters, because in
//! the real circuit the diodes and the transistor argue with each other about
//! the collector voltage.
//!
//! What this buys, all of it emergent:
//!
//! * asymmetry, because the collector sits nearer one rail than the other;
//! * a real input impedance, since the base draws current through `Rin`;
//! * bias movement under a hot signal — the reason a fuzz sputters and gates.

use crate::math::{abs, clamp, exp};

/// The largest exponent any junction in this module is evaluated at. At a
/// silicon pair's 45 mV emission voltage this is 1.35 V — far past anything the
/// converged circuit reaches, and comfortably inside `f32`.
const LIMIT_ARGUMENT: f32 = 30.0;
/// No branch of a nine-volt pedal carries a tenth of an ampere. Capping the
/// current an unconverged guess can claim keeps the Jacobian finite without
/// touching the answer: at the operating point these currents are microamps.
const LIMIT_CURRENT: f32 = 0.1;
/// The matching cap on conductance, in siemens.
const LIMIT_SLOPE: f32 = 20.0;

/// A small-signal silicon NPN, as its datasheet describes it.
#[derive(Clone, Copy)]
pub struct BipolarDevice {
    /// Transport saturation current `Is`, in amperes.
    pub saturation_current: f32,
    /// Thermal voltage `Vt`, 25.85 mV at room temperature.
    pub thermal_voltage: f32,
    /// Forward current gain.
    pub forward_gain: f32,
}

impl BipolarDevice {
    /// The high-gain small-signal part these circuits are built from
    /// (BC239/2N5088 class).
    pub const SMALL_SIGNAL_NPN: Self = Self {
        saturation_current: 6.7e-15,
        thermal_voltage: 0.025_85,
        forward_gain: 300.0,
    };

    /// A lower-gain, leakier device, closer to what a germanium fuzz uses.
    pub const LOW_GAIN_NPN: Self = Self {
        saturation_current: 2.0e-14,
        thermal_voltage: 0.025_85,
        forward_gain: 90.0,
    };
}

impl Default for BipolarDevice {
    fn default() -> Self {
        Self::SMALL_SIGNAL_NPN
    }
}

/// Optional clipping pair across the feedback resistor.
#[derive(Clone, Copy, Default)]
pub struct FeedbackDiodes {
    /// Reverse saturation current. Zero means no diodes are fitted.
    pub saturation_current: f32,
    /// `n·Vt` for the pair.
    pub emission_voltage: f32,
}

impl FeedbackDiodes {
    pub const NONE: Self = Self {
        saturation_current: 0.0,
        emission_voltage: 0.045,
    };

    pub const SILICON: Self = Self {
        saturation_current: 2.52e-9,
        emission_voltage: 1.752 * 0.025_85,
    };

    /// Current through the pair, and its slope, for a voltage across it.
    ///
    /// Both are limited. See [`LIMIT_CURRENT`] for why: an intermediate
    /// iteration can ask this what happens at four volts across a silicon pair,
    /// and the honest answer overflows the arithmetic before it reaches the
    /// Jacobian.
    #[inline]
    fn current_and_slope(&self, voltage: f32) -> (f32, f32) {
        if self.saturation_current <= 0.0 {
            return (0.0, 0.0);
        }
        let argument = clamp(
            voltage / self.emission_voltage,
            -LIMIT_ARGUMENT,
            LIMIT_ARGUMENT,
        );
        let positive = exp(argument);
        let negative = 1.0 / positive;
        let scale = 2.0 * self.saturation_current;
        let current = scale * 0.5 * (positive - negative);
        let slope = (scale / self.emission_voltage) * 0.5 * (positive + negative);
        (
            clamp(current, -LIMIT_CURRENT, LIMIT_CURRENT),
            clamp(slope, 0.0, LIMIT_SLOPE),
        )
    }
}

/// The component values of a common-emitter stage: everything a schematic
/// would tell you, and nothing the solver decides.
#[derive(Clone, Copy)]
pub struct StageDesign {
    pub device: BipolarDevice,
    pub diodes: FeedbackDiodes,
    /// Series resistance from the driving stage into the base.
    pub input_resistance: f32,
    /// Collector load to the supply.
    pub collector_resistance: f32,
    /// Bias feedback from collector to base.
    pub feedback_resistance: f32,
    /// Emitter degeneration. Unbypassed, which is what sets the stage's gain
    /// once the exponential is steep.
    pub emitter_resistance: f32,
    /// Input coupling capacitor, in farads. With the input resistance it sets
    /// where the stage stops passing bass.
    pub coupling_capacitance: f32,
    /// Supply rail.
    pub supply: f32,
}

impl Default for StageDesign {
    fn default() -> Self {
        Self {
            device: BipolarDevice::SMALL_SIGNAL_NPN,
            diodes: FeedbackDiodes::NONE,
            input_resistance: 10_000.0,
            collector_resistance: 10_000.0,
            feedback_resistance: 100_000.0,
            emitter_resistance: 100.0,
            coupling_capacitance: 100.0e-9,
            supply: 9.0,
        }
    }
}

/// The design's resistances, as conductances. Computed once: the audio path
/// would otherwise divide by the same four constants on every iteration.
#[derive(Clone, Copy, Default)]
struct Conductances {
    input: f32,
    collector: f32,
    feedback: f32,
    emitter: f32,
    inverse_gain: f32,
    inverse_coupling: f32,
}

impl Conductances {
    fn of(design: &StageDesign) -> Self {
        Self {
            input: 1.0 / design.input_resistance,
            collector: 1.0 / design.collector_resistance,
            feedback: 1.0 / design.feedback_resistance,
            emitter: 1.0 / design.emitter_resistance,
            inverse_gain: 1.0 / design.device.forward_gain,
            inverse_coupling: 1.0 / design.coupling_capacitance,
        }
    }
}

/// A common-emitter stage biased by feedback from its own collector.
#[derive(Clone, Copy)]
pub struct CommonEmitterStage {
    design: StageDesign,
    conductance: Conductances,
    base: f32,
    collector: f32,
    emitter: f32,
    coupling_voltage: f32,
    time_step: f32,
}

impl Default for CommonEmitterStage {
    fn default() -> Self {
        Self::new(StageDesign::default())
    }
}

impl CommonEmitterStage {
    pub fn new(design: StageDesign) -> Self {
        Self {
            design,
            conductance: Conductances::of(&design),
            base: 0.6,
            collector: design.supply * 0.5,
            emitter: 0.02,
            coupling_voltage: -0.6,
            time_step: 1.0 / 192_000.0,
        }
    }

    /// The component values this stage was built from.
    pub fn design(&self) -> StageDesign {
        self.design
    }

    /// Iterations spent finding the operating point when the pedal powers on.
    const SETTLE_ITERATIONS: usize = 200;
    /// Iterations per audio sample, warm-started from the previous one.
    ///
    /// Two is the measured answer, not a guess: see the convergence test in
    /// this module, which compares each count against a twelve-iteration
    /// reference.
    const RUNNING_ITERATIONS: u8 = 2;
    /// The most any node may move in one iteration. Newton on an exponential
    /// overshoots spectacularly without this.
    const STEP_LIMIT: f32 = 0.15;

    /// Finds the quiescent point. Call once the component values and the
    /// sample rate are set.
    pub fn settle(&mut self, sample_rate: f32) {
        self.time_step = 1.0 / sample_rate;
        self.base = 0.6;
        // Where the collector starts matters. With clipping diodes across the
        // feedback resistor the answer is a forward drop above the base, not
        // half the supply: the pair carries the bias current and pins the two
        // nodes together. Starting at half the rail asks the solver to cross a
        // region where the diodes are conducting amperes.
        self.collector = if self.design.diodes.saturation_current > 0.0 {
            self.base + 0.55
        } else {
            self.design.supply * 0.5
        };
        self.emitter = 0.02;
        for _ in 0..Self::SETTLE_ITERATIONS {
            self.iterate(0.0);
            // At rest no current flows through the coupling capacitor, so it
            // holds exactly the base voltage. Charging it in real time would
            // take a second of simulated silence to reach the same place.
            self.coupling_voltage = -self.base;
        }
    }

    /// Clears the signal history without disturbing the operating point.
    pub fn reset(&mut self) {
        self.coupling_voltage = -self.base;
    }

    /// The collector voltage with no signal present, which is what a meter
    /// would read on the real stage.
    pub fn operating_point(&self) -> f32 {
        self.collector
    }

    /// One sample. `input` is the voltage the previous stage presents at the
    /// far side of the input resistor; the returned value is the collector
    /// voltage, still sitting on its bias.
    #[inline]
    pub fn process(&mut self, input: f32) -> f32 {
        self.process_with(input, Self::RUNNING_ITERATIONS)
    }

    /// One sample with an explicit iteration count, so the convergence of the
    /// default can be measured rather than asserted.
    #[inline]
    pub fn process_with(&mut self, input: f32, iterations: u8) -> f32 {
        for _ in 0..iterations {
            self.iterate(input);
        }
        // Integrate the charge the branch just moved onto the coupling
        // capacitor. Explicit integration is stable here by a wide margin: the
        // branch's time constant is thousands of samples long.
        let branch_current = (input - self.coupling_voltage - self.base) * self.conductance.input;
        self.coupling_voltage +=
            branch_current * self.time_step * self.conductance.inverse_coupling;
        if !self.coupling_voltage.is_finite() {
            self.coupling_voltage = -self.base;
        }
        self.collector
    }

    #[inline]
    fn iterate(&mut self, input: f32) {
        let device = self.design.device;
        let conductance_input = self.conductance.input;
        let conductance_collector = self.conductance.collector;
        let conductance_feedback = self.conductance.feedback;
        let conductance_emitter = self.conductance.emitter;
        let inverse_gain = self.conductance.inverse_gain;

        // Collector current and its slope with respect to the base-emitter
        // voltage, under the same limiting as the diodes above.
        let base_emitter = clamp(
            (self.base - self.emitter) / device.thermal_voltage,
            -LIMIT_ARGUMENT,
            LIMIT_ARGUMENT,
        );
        let exponential = exp(base_emitter);
        let collector_current = clamp(
            device.saturation_current * (exponential - 1.0),
            -LIMIT_CURRENT,
            LIMIT_CURRENT,
        );
        let transconductance = clamp(
            device.saturation_current * exponential / device.thermal_voltage,
            0.0,
            LIMIT_SLOPE,
        );

        let across_feedback = self.collector - self.base;
        let (diode_current, diode_slope) = self.design.diodes.current_and_slope(across_feedback);

        // Residuals.
        let base_residual = (input - self.coupling_voltage - self.base) * conductance_input
            + across_feedback * conductance_feedback
            + diode_current
            - collector_current * inverse_gain;
        let collector_residual = (self.design.supply - self.collector) * conductance_collector
            - collector_current
            - across_feedback * conductance_feedback
            - diode_current;
        let emitter_residual =
            collector_current * (1.0 + inverse_gain) - self.emitter * conductance_emitter;

        // Jacobian, in the order (base, collector, emitter).
        let jacobian = [
            [
                -conductance_input
                    - conductance_feedback
                    - diode_slope
                    - transconductance * inverse_gain,
                conductance_feedback + diode_slope,
                transconductance * inverse_gain,
            ],
            [
                conductance_feedback + diode_slope - transconductance,
                -conductance_collector - conductance_feedback - diode_slope,
                transconductance,
            ],
            [
                transconductance * (1.0 + inverse_gain),
                0.0,
                -transconductance * (1.0 + inverse_gain) - conductance_emitter,
            ],
        ];

        let Some(step) = solve3(
            jacobian,
            [base_residual, collector_residual, emitter_residual],
        ) else {
            return;
        };

        self.base -= clamp(step[0], -Self::STEP_LIMIT, Self::STEP_LIMIT);
        self.collector -= clamp(step[1], -Self::STEP_LIMIT, Self::STEP_LIMIT);
        self.emitter -= clamp(step[2], -Self::STEP_LIMIT, Self::STEP_LIMIT);

        // The stage cannot leave its own supply, and the solver should never
        // need to be told — but a clamp here costs nothing and keeps one bad
        // block from becoming a permanent state.
        let supply = self.design.supply;
        self.base = clamp(self.base, -1.0, supply);
        self.collector = clamp(self.collector, 0.0, supply);
        self.emitter = clamp(self.emitter, -1.0, supply);
        if !self.base.is_finite() || !self.collector.is_finite() || !self.emitter.is_finite() {
            self.base = 0.6;
            self.collector = supply * 0.5;
            self.emitter = 0.02;
        }
    }
}

/// Solves a 3x3 system by Cramer's rule.
///
/// Cramer is usually the wrong choice — it evaluates four determinants where
/// elimination needs a third of the arithmetic — but it is branch-free, and
/// that wins here by a wide margin. Measured on this board: swapping it for
/// Gaussian elimination with partial pivoting, whose pivot search branches on
/// data, took the fuzz from 794 to 1252 microseconds per block. Straight-line
/// arithmetic beats fewer operations when the operations are this small.
///
/// Returns `None` when the matrix is singular, which for these networks means a
/// component value is nonsense or an iteration wandered somewhere the
/// exponentials have flattened.
#[inline]
fn solve3(matrix: [[f32; 3]; 3], right: [f32; 3]) -> Option<[f32; 3]> {
    let determinant = |m: [[f32; 3]; 3]| {
        m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
            - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
            + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
    };
    let base = determinant(matrix);
    if abs(base) < 1.0e-30 || !base.is_finite() {
        return None;
    }
    let inverse = 1.0 / base;
    let mut solution = [0.0_f32; 3];
    for column in 0..3 {
        let mut replaced = matrix;
        for row in 0..3 {
            replaced[row][column] = right[row];
        }
        solution[column] = determinant(replaced) * inverse;
    }
    Some(solution)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{magnitude_at, peak, render_sine, total_harmonic_distortion};
    use std::vec::Vec;

    const SAMPLE_RATE: f32 = 48_000.0;

    fn booster() -> CommonEmitterStage {
        let mut stage = CommonEmitterStage::new(StageDesign {
            input_resistance: 10_000.0,
            collector_resistance: 10_000.0,
            feedback_resistance: 470_000.0,
            emitter_resistance: 470.0,
            ..StageDesign::default()
        });
        stage.settle(SAMPLE_RATE);
        stage
    }

    #[test]
    fn the_stage_finds_an_operating_point_inside_its_supply() {
        let stage = booster();
        let supply = stage.design().supply;
        let collector = stage.operating_point();
        assert!(
            collector > 0.5 && collector < supply - 0.5,
            "the stage biased itself to {collector} V on a {supply} V supply"
        );
    }

    fn clipping_stage() -> CommonEmitterStage {
        let mut stage = CommonEmitterStage::new(StageDesign {
            diodes: FeedbackDiodes::SILICON,
            ..booster().design()
        });
        stage.settle(SAMPLE_RATE);
        stage
    }

    #[test]
    fn the_operating_point_is_the_one_the_bias_network_implies() {
        // Collector feedback: Ib = (Vc − Vbe − Ve)/Rf and Ic = β·Ib, so
        // Vc = Vcc − Ic·Rc has a closed-form answer to check the solver
        // against. Vbe near 0.65 V for a milliamp-ish collector current.
        let stage = booster();
        let design = stage.design();
        let device = design.device;
        let predicted = {
            let base_emitter = 0.65_f32;
            let emitter_factor = design.emitter_resistance * (1.0 + 1.0 / device.forward_gain);
            let feedback_factor = design.feedback_resistance / device.forward_gain;
            // Vcc − Ic·Rc = Vbe + Ic·Re + Ic·Rf/β
            (design.supply - base_emitter)
                / (design.collector_resistance + emitter_factor + feedback_factor)
        };
        let predicted_collector = design.supply - predicted * design.collector_resistance;
        let measured = stage.operating_point();
        assert!(
            (measured - predicted_collector).abs() < 0.6,
            "solver settled at {measured} V, the bias network says {predicted_collector} V"
        );
    }

    #[test]
    fn it_amplifies_and_inverts() {
        let mut stage = booster();
        let quiescent = stage.operating_point();
        let rendered = render_sine(220.0, 0.005, SAMPLE_RATE, 4_096, |sample| {
            stage.process(sample) - quiescent
        });
        let gain = magnitude_at(&rendered, 220.0, SAMPLE_RATE) / 0.005;
        assert!(
            gain > 3.0,
            "a common-emitter stage should have real gain, measured {gain}"
        );
        // Inverting: a positive step at the base pulls the collector down.
        let mut probe = booster();
        let rest = probe.operating_point();
        for _ in 0..64 {
            probe.process(0.02);
        }
        assert!(
            probe.operating_point() < rest,
            "the collector moved the wrong way"
        );
    }

    #[test]
    fn a_hot_input_clips_asymmetrically() {
        // The collector sits nearer one rail than the other, so it runs out of
        // room on one side first. That is where a booster's even harmonics
        // come from, and it is not something the model was told to do.
        let mut stage = booster();
        let quiescent = stage.operating_point();
        let rendered = render_sine(220.0, 0.3, SAMPLE_RATE, 8_192, |sample| {
            stage.process(sample) - quiescent
        });
        let positive = rendered.iter().cloned().fold(f32::MIN, f32::max);
        let negative = -rendered.iter().cloned().fold(f32::MAX, f32::min);
        let asymmetry = (positive - negative).abs() / (positive + negative);
        assert!(
            asymmetry > 0.05,
            "the stage clipped symmetrically: +{positive} / -{negative}"
        );
        assert!(
            total_harmonic_distortion(&rendered, 220.0, SAMPLE_RATE) > 0.05,
            "a hot input produced no distortion at all"
        );
    }

    #[test]
    fn feedback_diodes_soften_the_clipping() {
        let mut bare = booster();
        let mut clamped = clipping_stage();

        let bare_rest = bare.operating_point();
        let clamped_rest = clamped.operating_point();
        let bare_render = render_sine(220.0, 0.2, SAMPLE_RATE, 8_192, |sample| {
            bare.process(sample) - bare_rest
        });
        let clamped_render = render_sine(220.0, 0.2, SAMPLE_RATE, 8_192, |sample| {
            clamped.process(sample) - clamped_rest
        });
        assert!(
            peak(&clamped_render) < peak(&bare_render),
            "the diodes did not limit the swing: {} vs {}",
            peak(&clamped_render),
            peak(&bare_render)
        );
    }

    #[test]
    fn two_iterations_are_enough_when_the_solver_is_warm_started() {
        // The stage moves very little between samples, so the previous answer
        // is an excellent starting guess. This measures how much is still left
        // to converge after each count, against a reference that iterates until
        // there is nothing left.
        let sample_rate = SAMPLE_RATE;
        let reference: Vec<f32> = {
            let mut stage = booster();
            let bias = stage.operating_point();
            render_sine(220.0, 0.2, sample_rate, 4_096, |sample| {
                stage.process_with(sample, 12) - bias
            })
        };
        let scale = crate::testing::rms(&reference);

        let mut worst_accepted = 0.0_f32;
        for iterations in [1_u8, 2, 3] {
            let mut stage = booster();
            let bias = stage.operating_point();
            let measured = render_sine(220.0, 0.2, sample_rate, 4_096, |sample| {
                stage.process_with(sample, iterations) - bias
            });
            let difference: Vec<f32> = reference
                .iter()
                .zip(&measured)
                .map(|(want, have)| want - have)
                .collect();
            let error = crate::testing::rms(&difference) / scale;
            if iterations >= 2 {
                assert!(
                    error < 0.01,
                    "{iterations} iterations left {:.3} % of error",
                    error * 100.0
                );
                worst_accepted = worst_accepted.max(error);
            }
        }
        assert!(worst_accepted < 0.01);
    }

    #[test]
    fn the_solver_stays_finite_under_abuse() {
        let mut stage = booster();
        let quiescent = stage.operating_point();
        let supply = stage.design().supply;
        for index in 0..48_000 {
            let input = if index % 1_000 < 500 { 5.0 } else { -5.0 };
            let output = stage.process(input);
            assert!(output.is_finite());
            assert!(output >= 0.0 && output <= supply);
        }
        // And it recovers its bias afterwards.
        for _ in 0..4_800 {
            stage.process(0.0);
        }
        assert!((stage.operating_point() - quiescent).abs() < 0.5);
    }
}
