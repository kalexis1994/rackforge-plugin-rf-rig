//! Distortion — a booster into a high-gain stage that hard-clips to ground.
//!
//! Where the overdrive keeps its diodes inside the feedback loop, this family
//! puts them across the signal after the gain stage. Once the node reaches the
//! forward drop it simply cannot go further: the waveform squares off, the
//! harmonics are dense and odd-heavy, and rolling the guitar volume back
//! changes the level far more than it changes the dirt.
//!
//! The other half of the character is the tone network, which is not a tilt at
//! all but a scoop — the midrange is pulled out on purpose, and the treble and
//! bass ends are what the control balances.

use crate::circuit::filters::{CouplingCap, DcBlocker, OnePole};
use crate::circuit::nonlinear::{ClipperSolver, Diode};
use crate::circuit::opamp::{NonInvertingStage, OpAmpDesign};
use crate::circuit::oversample::Oversampler4;
use crate::circuit::tonestack::{ToneNetwork, ToneStack};
use crate::circuit::transistor::{CommonEmitterStage, FeedbackDiodes, StageDesign};
use crate::math::{clamp, exponential};

/// Series resistance into the clipping node.
const CLIPPING_SERIES_RESISTANCE: f32 = 2_200.0;
/// The gain stage's input network: 4.7 kΩ with 0.47 µF, so the corner sits at
/// 72 Hz — far lower than an overdrive's, which is why the bass survives into
/// the clipper.
const INPUT_RESISTANCE: f32 = 4_700.0;
const INPUT_CAPACITANCE: f32 = 470.0e-9;
/// Across the feedback resistor.
const FEEDBACK_CAPACITANCE: f32 = 100.0e-12;
const FIXED_FEEDBACK: f32 = 22_000.0;
const DISTORTION_POT: f32 = 100_000.0;
/// This family buffers its input, so it presents a high impedance whatever the
/// booster behind that buffer looks like. The buffer itself is not modelled:
/// an emitter follower's only audible job is this number.
pub const INPUT_IMPEDANCE: f32 = 470_000.0;
/// The level control at the output.
pub const OUTPUT_IMPEDANCE: f32 = 10_000.0;

/// Recovery after the clipper and the tone network. Calibrated so the pedal
/// reaches unity with its level control near a third of its travel, which is
/// where this family sits.
const OUTPUT_MAKEUP: f32 = 3.0;

#[derive(Clone, Copy, Default)]
pub struct Distortion {
    input_cap: CouplingCap,
    booster: CommonEmitterStage,
    booster_bias: f32,
    oversampler: Oversampler4,
    stage: NonInvertingStage,
    clip_solver: ClipperSolver,
    tone: ToneStack,
    output_lowpass: OnePole,
    dc: DcBlocker,
    level: f32,
}

impl Distortion {
    pub fn prepare(&mut self, sample_rate: f32) {
        let inner_rate = sample_rate * Oversampler4::FACTOR as f32;
        self.input_cap = CouplingCap::new(30.0, sample_rate);
        // The booster is a real common-emitter stage, solved from Ebers-Moll.
        // Its asymmetry — and therefore the even harmonics a booster is valued
        // for — comes from where the bias network parks the collector, not from
        // a shaping function.
        self.booster = CommonEmitterStage::new(StageDesign {
            diodes: FeedbackDiodes::NONE,
            input_resistance: 10_000.0,
            collector_resistance: 10_000.0,
            feedback_resistance: 470_000.0,
            emitter_resistance: 470.0,
            coupling_capacitance: 100.0e-9,
            ..StageDesign::default()
        });
        self.booster.settle(inner_rate);
        self.booster_bias = self.booster.operating_point();
        // The gain stage: a real amplifier with its own bandwidth and its own
        // supply, and no clipping diodes of its own — this family puts them
        // after the stage, shunted to ground.
        self.stage = NonInvertingStage::new(OpAmpDesign {
            feedback_resistance: FIXED_FEEDBACK + 0.5 * DISTORTION_POT,
            input_resistance: INPUT_RESISTANCE,
            input_capacitance: INPUT_CAPACITANCE,
            feedback_capacitance: FEEDBACK_CAPACITANCE,
            diodes: Diode {
                saturation_current: 0.0,
                emission_voltage: 0.045,
            },
            ..OpAmpDesign::default()
        });
        self.stage.prepare(inner_rate);
        // The scoop is not a filter bolted on afterwards: it is what the tone
        // network does between its two branches. See `circuit::tonestack`.
        self.tone.prepare(ToneNetwork::DISTORTION, inner_rate);
        self.output_lowpass = OnePole::new(6_500.0, sample_rate);
        self.dc = DcBlocker::new(sample_rate);
        self.level = 0.5;
        self.reset();
    }

    pub fn reset(&mut self) {
        self.input_cap.reset();
        self.booster.reset();
        self.oversampler.reset();
        self.stage.reset();
        self.clip_solver.reset();
        self.tone.reset();
        self.output_lowpass.reset();
        self.dc.reset();
    }

    pub fn set_controls(&mut self, distortion: f32, tone: f32, level: f32) {
        self.stage
            .set_feedback_resistance(FIXED_FEEDBACK + clamp(distortion, 0.0, 1.0) * DISTORTION_POT);
        self.tone.set_position(clamp(tone, 0.0, 1.0));
        self.level = exponential(clamp(level, 0.0, 1.0), 0.02, 2.0);
    }

    /// The impedance the pedal in front of this one drives it through. It
    /// reaches the booster, which is what the buffer feeds.
    pub fn set_source_impedance(&mut self, ohms: f32) {
        self.booster.set_source_impedance(ohms);
    }

    #[inline]
    pub fn process(&mut self, input: f32) -> f32 {
        let signal = self.input_cap.process(input);

        let clipped = {
            let bias = self.booster_bias;
            let Self {
                oversampler,
                booster,
                stage,
                clip_solver,
                tone,
                ..
            } = self;
            oversampler.process(signal, |sample| {
                // The booster runs oversampled with everything else: it clips,
                // so it generates harmonics that must not fold back.
                let boosted = booster.process(sample) - bias;
                let amplified = stage.process(boosted);
                // Hard clip: the node is pulled to the diodes' forward drop
                // through the series resistor.
                let clipped = clip_solver.solve(
                    amplified / CLIPPING_SERIES_RESISTANCE,
                    CLIPPING_SERIES_RESISTANCE,
                    Diode::SILICON,
                );
                tone.process(clipped)
            })
        };

        // Clipping to a 0.6 V knee and then losing several decibels more in the
        // tone network leaves a quiet signal; the output stage makes it back up.
        self.output_lowpass.low(self.dc.process(clipped)) * self.level * OUTPUT_MAKEUP
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{magnitude_at, peak, render_sine, total_harmonic_distortion};

    fn pedal(distortion: f32, tone: f32, sample_rate: f32) -> Distortion {
        let mut pedal = Distortion::default();
        pedal.prepare(sample_rate);
        pedal.set_controls(distortion, tone, 0.6);
        pedal
    }

    #[test]
    fn it_squares_off_far_harder_than_an_overdrive_does() {
        let sample_rate = 48_000.0;
        let mut distortion = pedal(0.8, 0.5, sample_rate);
        let mut overdrive = super::super::overdrive::Overdrive::default();
        overdrive.prepare(sample_rate);
        overdrive.set_controls(0.8, 0.5, 0.6);

        let hard = render_sine(220.0, 0.1, sample_rate, 8_192, |sample| {
            distortion.process(sample)
        });
        let soft = render_sine(220.0, 0.1, sample_rate, 8_192, |sample| {
            overdrive.process(sample)
        });
        let hard_thd = total_harmonic_distortion(&hard, 220.0, sample_rate);
        let soft_thd = total_harmonic_distortion(&soft, 220.0, sample_rate);
        assert!(
            hard_thd > soft_thd,
            "the shunt clipper was gentler than the feedback clipper: {hard_thd} vs {soft_thd}"
        );
    }

    #[test]
    fn the_tone_network_scoops_the_midrange() {
        // Where the dip sits is decided by the network, not by taste: with the
        // control at noon the two branches cross near 1.6 kHz, and that is
        // where the wiper is furthest from both of them.
        let sample_rate = 48_000.0;
        let mut pedal = pedal(0.5, 0.5, sample_rate);
        let level_at = |frequency: f32, pedal: &mut Distortion| {
            pedal.reset();
            let rendered = render_sine(frequency, 0.02, sample_rate, 16_384, |sample| {
                pedal.process(sample)
            });
            magnitude_at(&rendered, frequency, sample_rate)
        };
        let low = level_at(120.0, &mut pedal);
        let middle = level_at(1_600.0, &mut pedal);
        let high = level_at(5_400.0, &mut pedal);
        assert!(
            middle < low && middle < high,
            "expected a scoop, measured {low} / {middle} / {high}"
        );
    }

    #[test]
    fn the_tone_control_trades_bass_for_treble() {
        let sample_rate = 48_000.0;
        let level_at = |pedal: &mut Distortion, frequency: f32| {
            pedal.reset();
            let rendered = render_sine(frequency, 0.02, sample_rate, 16_384, |sample| {
                pedal.process(sample)
            });
            magnitude_at(&rendered, frequency, sample_rate)
        };
        let mut dark = pedal(0.5, 0.0, sample_rate);
        let mut bright = pedal(0.5, 1.0, sample_rate);
        let dark_ratio = level_at(&mut dark, 5_000.0) / level_at(&mut dark, 120.0);
        let bright_ratio = level_at(&mut bright, 5_000.0) / level_at(&mut bright, 120.0);
        assert!(
            bright_ratio > dark_ratio * 3.0,
            "the control barely moved the balance: {bright_ratio} vs {dark_ratio}"
        );
    }

    #[test]
    fn it_stays_bounded_on_a_hot_input() {
        let sample_rate = 48_000.0;
        let mut pedal = pedal(1.0, 1.0, sample_rate);
        let rendered = render_sine(180.0, 3.0, sample_rate, 8_192, |sample| {
            pedal.process(sample)
        });
        let level = peak(&rendered);
        assert!(level.is_finite() && level < 12.0, "peak reached {level}");
    }
}
