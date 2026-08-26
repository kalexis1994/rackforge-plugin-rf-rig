//! Operational amplifiers, with the two limits that matter and none of the
//! ones that do not.
//!
//! An ideal op-amp forces its inverting input to follow its non-inverting one
//! exactly, at every frequency, instantly. Real ones have a finite open-loop
//! gain that falls at twenty decibels per decade from a few hertz, so what is
//! left to control the loop is
//!
//! ```text
//! A(f) = GBW / f
//! ```
//!
//! and a closed-loop stage runs out of loop gain at `GBW / noise_gain`. That is
//! not an academic figure here. The overdrive's clipping stage has a noise gain
//! near 75 with its control up, and the chip these circuits are built around
//! has about a megahertz to spend: the loop is out of authority by 13 kHz,
//! inside the band, exactly where the diodes are generating harmonics. Part of
//! what people call a "smooth" or "dark" overdrive is an amplifier that cannot
//! keep up with its own clipping.
//!
//! ## What is solved
//!
//! The stage is a non-inverting amplifier whose feedback network carries an
//! optional clipping pair, with a series capacitor and resistor from the
//! inverting node to ground:
//!
//! ```text
//!            ┌────/\/\ Rf ────┐
//!            │      ─┤├─ Cf    │
//!            │      ─▶|◀─     │        (diodes optional)
//!            │                │
//!   Vin ──▶──┤ +            − ├──┬──── Vout
//!            └────────────────┘  │
//!                    V−  ──/\/\──┴──┤├── gnd
//!                          Rin      Cin
//! ```
//!
//! Two unknowns per sample. The amplifier's own pole is integrated with
//! backward Euler,
//!
//! ```text
//! Vout = (Vout_prev + k·A0·(Vin − V−)) / (1 + k)      k = Δt·ω0
//! ```
//!
//! which is unconditionally stable and needs no oversampling of its own, and
//! the inverting node satisfies
//!
//! ```text
//! (Vout − V−)/Rf + Id(Vout − V−) = (V− − Vc)/Rin
//! ```
//!
//! Substituting the first into the second leaves one equation in `V−`, solved
//! by Newton in two iterations from the previous sample's answer. The input
//! capacitor's charge is then integrated, which is where the frequency-
//! dependent gain comes from — it is a real capacitor here, not a one-pole
//! approximation of one.

use crate::circuit::nonlinear::Diode;
use crate::math::{abs, clamp, exp};

/// Keeps the diode exponential inside `f32` while an iteration is still
/// wandering.
const ARGUMENT_LIMIT: f32 = 30.0;
const CURRENT_LIMIT: f32 = 0.1;
const SLOPE_LIMIT: f32 = 20.0;

/// An op-amp as its datasheet describes it.
#[derive(Clone, Copy)]
pub struct OperationalAmplifier {
    /// Open-loop gain at DC.
    pub open_loop_gain: f32,
    /// Gain-bandwidth product, in hertz.
    pub gain_bandwidth: f32,
    /// How fast the output can move, in volts per second.
    pub slew_rate: f32,
    /// How close to each supply the output can get.
    pub positive_swing: f32,
    pub negative_swing: f32,
}

impl OperationalAmplifier {
    /// The dual bipolar op-amp these circuits were designed around: about a
    /// megahertz of gain-bandwidth and a volt per microsecond.
    pub const CLASSIC_DUAL: Self = Self {
        open_loop_gain: 100_000.0,
        gain_bandwidth: 1.0e6,
        slew_rate: 1.0e6,
        positive_swing: 3.9,
        negative_swing: -3.4,
    };

    /// A faster JFET-input part, the usual modern substitution: three times the
    /// bandwidth and an order more slew rate.
    pub const FAST_JFET: Self = Self {
        open_loop_gain: 200_000.0,
        gain_bandwidth: 3.0e6,
        slew_rate: 13.0e6,
        positive_swing: 4.0,
        negative_swing: -3.6,
    };

    /// The dominant pole, in radians per second: `2π·GBW/A0`.
    pub fn pole(&self) -> f32 {
        core::f32::consts::TAU * self.gain_bandwidth / self.open_loop_gain
    }

    /// Where a closed-loop stage of this noise gain runs out of loop gain.
    pub fn closed_loop_bandwidth(&self, noise_gain: f32) -> f32 {
        self.gain_bandwidth / noise_gain.max(1.0)
    }
}

impl Default for OperationalAmplifier {
    fn default() -> Self {
        Self::CLASSIC_DUAL
    }
}

/// The component values of a non-inverting stage: what a schematic would tell
/// you, and nothing the solver decides.
#[derive(Clone, Copy)]
pub struct OpAmpDesign {
    pub amplifier: OperationalAmplifier,
    /// Feedback resistance from the output to the inverting node.
    pub feedback_resistance: f32,
    /// Series resistance from the inverting node towards ground.
    pub input_resistance: f32,
    /// The capacitor in series with it, which is what makes the gain
    /// frequency-dependent.
    pub input_capacitance: f32,
    /// The capacitor across the feedback resistor — the one that stops the
    /// stage amplifying the fizz its own clipping makes. It is inside the loop
    /// here, where the circuit has it, rather than a filter applied afterwards.
    pub feedback_capacitance: f32,
    /// The pair across the feedback resistor. `saturation_current` of zero
    /// leaves the stage clean.
    pub diodes: Diode,
}

impl Default for OpAmpDesign {
    fn default() -> Self {
        Self {
            amplifier: OperationalAmplifier::CLASSIC_DUAL,
            feedback_resistance: 51_000.0,
            input_resistance: 4_700.0,
            input_capacitance: 47.0e-9,
            feedback_capacitance: 51.0e-12,
            diodes: Diode::SILICON,
        }
    }
}

/// A non-inverting stage with a clipping pair across its feedback resistor.
#[derive(Clone, Copy)]
pub struct NonInvertingStage {
    design: OpAmpDesign,
    output: f32,
    capacitor: f32,
    node: f32,
    across: f32,
    time_step: f32,
}

impl Default for NonInvertingStage {
    fn default() -> Self {
        Self::new(OpAmpDesign::default())
    }
}

impl NonInvertingStage {
    const ITERATIONS: usize = 2;

    pub fn new(design: OpAmpDesign) -> Self {
        Self {
            design,
            output: 0.0,
            capacitor: 0.0,
            node: 0.0,
            across: 0.0,
            time_step: 1.0 / 192_000.0,
        }
    }

    /// The component values this stage was built from.
    pub fn design(&self) -> OpAmpDesign {
        self.design
    }

    /// Moves the feedback resistance — the drive control, in every pedal that
    /// has one.
    pub fn set_feedback_resistance(&mut self, ohms: f32) {
        self.design.feedback_resistance = clamp(ohms, 100.0, 1.0e7);
    }

    pub fn prepare(&mut self, sample_rate: f32) {
        self.time_step = 1.0 / sample_rate.max(1.0);
        self.reset();
    }

    pub fn reset(&mut self) {
        self.output = 0.0;
        self.capacitor = 0.0;
        self.node = 0.0;
        self.across = 0.0;
    }

    /// Where the feedback capacitor takes the stage's own gain away, in hertz.
    pub fn feedback_corner(&self) -> f32 {
        1.0 / (core::f32::consts::TAU
            * self.design.feedback_resistance
            * self.design.feedback_capacitance)
    }

    /// The gain the loop has to work against at high frequencies, where the
    /// input capacitor is a short: `1 + Rf/Rin`.
    pub fn noise_gain(&self) -> f32 {
        1.0 + self.design.feedback_resistance / self.design.input_resistance
    }

    /// Where this stage runs out of loop gain, in hertz.
    pub fn bandwidth(&self) -> f32 {
        self.design
            .amplifier
            .closed_loop_bandwidth(self.noise_gain())
    }

    #[inline]
    pub fn process(&mut self, input: f32) -> f32 {
        let amplifier = self.design.amplifier;
        let step = self.time_step * amplifier.pole();
        // Backward Euler on the amplifier's own pole gives the output in terms
        // of the inverting node: `Vout = offset − slope·V−`.
        let scale = 1.0 / (1.0 + step);
        let offset = (self.output + step * amplifier.open_loop_gain * input) * scale;
        let slope = (step * amplifier.open_loop_gain * scale).max(1.0e-6);
        let inverse_slope = 1.0 / slope;

        let feedback = 1.0 / self.design.feedback_resistance;
        // The feedback capacitor as a conductance for this step: backward Euler
        // turns `C·du/dt` into `C/Δt` worked against its previous voltage.
        let feedback_capacitive = self.design.feedback_capacitance / self.time_step;
        let inbound = 1.0 / self.design.input_resistance;
        let emission = self.design.diodes.emission_voltage;
        let saturation = 2.0 * self.design.diodes.saturation_current;

        // Newton on the *output*, not on the inverting node.
        //
        // Both describe the same circuit, but the node is multiplied by the
        // amplifier's per-step gain — thirty-odd — on its way to the output, so
        // a small error there becomes a large one here, and that error is then
        // carried into the next sample by the stage's own state. Measured
        // symptom of getting this wrong: with a hot input the output grew
        // steadily over seconds instead of settling.
        let mut output = self.output;
        for _ in 0..Self::ITERATIONS {
            let node = (offset - output) * inverse_slope;
            let across = output - node;
            let (diode_current, diode_slope) = if saturation <= 0.0 {
                (0.0, 0.0)
            } else {
                let argument = clamp(across / emission, -ARGUMENT_LIMIT, ARGUMENT_LIMIT);
                let positive = exp(argument);
                let negative = 1.0 / positive;
                (
                    clamp(
                        saturation * 0.5 * (positive - negative),
                        -CURRENT_LIMIT,
                        CURRENT_LIMIT,
                    ),
                    clamp(
                        (saturation / emission) * 0.5 * (positive + negative),
                        0.0,
                        SLOPE_LIMIT,
                    ),
                )
            };

            // Current arriving at the inverting node has to leave through the
            // input network.
            let residual =
                across * feedback + (across - self.across) * feedback_capacitive + diode_current
                    - (node - self.capacitor) * inbound;
            let derivative = (feedback + feedback_capacitive + diode_slope) * (1.0 + inverse_slope)
                + inbound * inverse_slope;
            if derivative == 0.0 {
                break;
            }
            let change = residual / derivative;
            output -= clamp(change, -4.0, 4.0);
            if abs(change) < 1.0e-9 {
                break;
            }
        }

        if !output.is_finite() {
            output = 0.0;
        }
        // The output cannot move faster than the part allows, nor further than
        // its supply lets it.
        let ceiling = amplifier.slew_rate * self.time_step;
        output = clamp(output, self.output - ceiling, self.output + ceiling);
        output = clamp(output, amplifier.negative_swing, amplifier.positive_swing);

        let node = (offset - output) * inverse_slope;
        self.node = node;
        self.across = output - node;
        self.output = output;

        // The input capacitor takes whatever the inverting node pushed into it.
        let current = (node - self.capacitor) * inbound;
        self.capacitor += current * self.time_step / self.design.input_capacitance;
        if !self.capacitor.is_finite() {
            self.capacitor = 0.0;
        }

        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::circuit::nonlinear::Diode;
    use crate::testing::{magnitude_at, render_sine};

    const SAMPLE_RATE: f32 = 192_000.0;

    fn clean_design(feedback: f32) -> OpAmpDesign {
        OpAmpDesign {
            diodes: Diode {
                saturation_current: 0.0,
                emission_voltage: 0.045,
            },
            feedback_resistance: feedback,
            ..OpAmpDesign::default()
        }
    }

    fn clean_stage(feedback: f32) -> NonInvertingStage {
        let mut stage = NonInvertingStage::new(clean_design(feedback));
        stage.prepare(SAMPLE_RATE);
        stage
    }

    fn gain_at(stage: &mut NonInvertingStage, frequency: f32, amplitude: f32) -> f32 {
        let rendered = render_sine(frequency, amplitude, SAMPLE_RATE, 16_384, |sample| {
            stage.process(sample)
        });
        magnitude_at(&rendered, frequency, SAMPLE_RATE) / amplitude
    }

    #[test]
    fn the_midband_gain_is_the_one_the_resistors_ask_for() {
        // Well above the input network's corner and well below where the
        // amplifier gives up: the textbook answer should hold.
        let mut stage = clean_stage(51_000.0);
        let expected = stage.noise_gain();
        let measured = gain_at(&mut stage, 3_000.0, 0.001);
        assert!(
            (measured / expected - 1.0).abs() < 0.1,
            "measured {measured}, the resistors say {expected}"
        );
    }

    #[test]
    fn the_capacitor_takes_the_gain_away_below_its_corner() {
        // 4.7 kΩ with 47 nF: 720 Hz. Two octaves under that the stage should be
        // most of the way back to unity.
        let mut stage = clean_stage(51_000.0);
        let low = gain_at(&mut stage, 180.0, 0.001);
        let mid = gain_at(&mut stage, 3_000.0, 0.001);
        assert!(
            low < mid * 0.35,
            "the input network did not roll the gain off: {low} against {mid}"
        );
        assert!(low > 1.0, "it should not fall below unity, measured {low}");
    }

    #[test]
    fn the_stage_runs_out_of_bandwidth_where_the_datasheet_says() {
        // A megahertz of gain-bandwidth against a noise gain of 76 is 13 kHz.
        //
        // Measured with the feedback capacitor taken out, because it turns over
        // at 8.9 kHz and would be answering the question as well. One mechanism
        // at a time, or the number means nothing.
        let mut stage = NonInvertingStage::new(OpAmpDesign {
            feedback_capacitance: 1.0e-15,
            ..clean_design(351_000.0)
        });
        stage.prepare(SAMPLE_RATE);
        let predicted = stage.bandwidth();
        assert!(
            (10_000.0..20_000.0).contains(&predicted),
            "the closed loop should give up near 13 kHz, not {predicted}"
        );

        let reference = gain_at(&mut stage, 2_000.0, 0.0005);
        let at_corner = gain_at(&mut stage, predicted, 0.0005);
        let ratio = at_corner / reference;
        assert!(
            (0.55..0.85).contains(&ratio),
            "expected roughly -3 dB at the closed-loop corner, measured {ratio}"
        );
    }

    #[test]
    fn the_feedback_capacitor_takes_the_top_off_inside_the_loop() {
        // 51 pF across 351 kΩ turns over at 8.9 kHz, and it is a component
        // rather than a filter applied afterwards — which matters, because
        // inside the loop it also removes the gain that would otherwise have
        // amplified the clipping's own harmonics.
        let mut stage = clean_stage(351_000.0);
        let corner = stage.feedback_corner();
        assert!(
            (7_000.0..12_000.0).contains(&corner),
            "the feedback capacitor turns over at {corner} Hz"
        );

        let mut open = NonInvertingStage::new(OpAmpDesign {
            feedback_capacitance: 1.0e-15,
            ..clean_design(351_000.0)
        });
        open.prepare(SAMPLE_RATE);
        let with_capacitor = gain_at(&mut stage, 12_000.0, 0.0005);
        let without = gain_at(&mut open, 12_000.0, 0.0005);
        assert!(
            with_capacitor < without * 0.9,
            "the capacitor did nothing: {with_capacitor} against {without}"
        );
    }

    #[test]
    fn a_faster_part_keeps_more_of_the_top() {
        let mut classic = clean_stage(351_000.0);
        let mut fast = NonInvertingStage::new(OpAmpDesign {
            amplifier: OperationalAmplifier::FAST_JFET,
            ..clean_design(351_000.0)
        });
        fast.prepare(SAMPLE_RATE);

        // At 12 kHz the classic part is just under its 13 kHz corner and the
        // fast one is well inside its 39 kHz corner, so the ratio should be
        // around 0.95/0.73 — a third more, not an order more. Substituting a
        // faster chip is a real change and a modest one, which is roughly what
        // players report.
        let classic_top = gain_at(&mut classic, 12_000.0, 0.0005);
        let fast_top = gain_at(&mut fast, 12_000.0, 0.0005);
        assert!(
            fast_top > classic_top * 1.25,
            "the faster part should hold on longer: {fast_top} against {classic_top}"
        );
    }

    #[test]
    fn the_diodes_still_clip_where_they_did() {
        let mut stage = NonInvertingStage::default();
        stage.prepare(SAMPLE_RATE);
        let rendered = render_sine(220.0, 0.2, SAMPLE_RATE, 8_192, |sample| {
            stage.process(sample)
        });
        // The output is the input plus the pair's drop, and the drop is set by
        // the current the input network delivers: 0.2 V through 16 kΩ at
        // 220 Hz is 12 µA, which a silicon pair holds at about 0.39 V.
        let peak = crate::testing::peak(&rendered);
        assert!(
            (0.35..1.2).contains(&peak),
            "a silicon pair should hold the swing near half a volt, measured {peak}"
        );
    }

    #[test]
    fn it_survives_a_signal_no_pedal_would_ever_see() {
        let mut stage = NonInvertingStage::default();
        stage.prepare(SAMPLE_RATE);
        for index in 0..96_000 {
            let input = if index % 500 < 250 { 20.0 } else { -20.0 };
            let output = stage.process(input);
            assert!(output.is_finite());
            assert!(output <= stage.design().amplifier.positive_swing + 0.01);
            assert!(output >= stage.design().amplifier.negative_swing - 0.01);
        }
    }
}
