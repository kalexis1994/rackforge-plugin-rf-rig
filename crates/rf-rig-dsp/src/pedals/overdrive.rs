//! Overdrive — an op-amp stage with a diode pair across its feedback
//! resistor.
//!
//! Two component values decide almost everything this circuit is known for.
//!
//! * The inverting input sees a series capacitor into a small resistor, so the
//!   *gain* is frequency dependent: the corner sits near 720 Hz and everything
//!   below it is amplified far less. That is the mid-hump, and it is also why
//!   the pedal stays tight on a low E instead of turning to mud.
//! * The diodes sit in the feedback loop rather than shunting to ground, so the
//!   stage never stops following its input. The knee is soft and the pedal
//!   cleans up when the player rolls back the guitar volume.
//!
//! The clipping itself is not shaped. It is solved from
//! `I_in = V/R_f + 2*Is*sinh(V/(n*Vt))` at four times the sample rate; the
//! curve is whatever those numbers imply.

use crate::circuit::filters::{CouplingCap, DcBlocker, OnePole};
use crate::circuit::nonlinear::{ClipperSolver, Diode};
use crate::circuit::oversample::Oversampler4;
use crate::circuit::tonestack::{ToneNetwork, ToneStack};
use crate::math::{clamp, exponential};

/// Series resistance from the inverting node to the clipping network.
const INPUT_RESISTANCE: f32 = 4_700.0;
/// 4.7 kΩ with 47 nF. Below this corner the stage barely amplifies at all.
const INPUT_CORNER_HZ: f32 = 720.0;
/// The fixed part of the feedback resistance.
const FIXED_FEEDBACK: f32 = 51_000.0;
/// The drive pot in series with it.
const DRIVE_POT: f32 = 500_000.0;

#[derive(Clone, Copy, Default)]
pub struct Overdrive {
    input_cap: CouplingCap,
    oversampler: Oversampler4,
    input_shaper: OnePole,
    solver: ClipperSolver,
    stage_lowpass: OnePole,
    tone: ToneStack,
    output_lowpass: OnePole,
    dc: DcBlocker,
    feedback_resistance: f32,
    level: f32,
}

impl Overdrive {
    pub fn prepare(&mut self, sample_rate: f32) {
        let inner_rate = sample_rate * Oversampler4::FACTOR as f32;
        self.input_cap = CouplingCap::new(15.0, sample_rate);
        self.input_shaper = OnePole::new(INPUT_CORNER_HZ, inner_rate);
        // The small capacitor across the feedback resistor. It is what stops
        // the stage from amplifying the fizz its own clipping creates.
        self.stage_lowpass = OnePole::new(6_000.0, inner_rate);
        // The tone network runs inside the oversampled section, where the
        // bilinear transform barely warps it and where the real circuit has it:
        // in the continuous path, right after the clipping stage.
        self.tone.prepare(ToneNetwork::OVERDRIVE, inner_rate);
        self.output_lowpass = OnePole::new(5_500.0, sample_rate);
        self.dc = DcBlocker::new(sample_rate);
        self.feedback_resistance = FIXED_FEEDBACK + 0.5 * DRIVE_POT;
        self.level = 0.5;
        self.reset();
    }

    pub fn reset(&mut self) {
        self.input_cap.reset();
        self.oversampler.reset();
        self.input_shaper.reset();
        self.solver.reset();
        self.stage_lowpass.reset();
        self.tone.reset();
        self.output_lowpass.reset();
        self.dc.reset();
    }

    pub fn set_controls(&mut self, drive: f32, tone: f32, level: f32) {
        self.feedback_resistance = FIXED_FEEDBACK + clamp(drive, 0.0, 1.0) * DRIVE_POT;
        self.tone.set_position(clamp(tone, 0.0, 1.0));
        self.level = exponential(clamp(level, 0.0, 1.0), 0.02, 3.0);
    }

    #[inline]
    pub fn process(&mut self, input: f32) -> f32 {
        let signal = self.input_cap.process(input);

        let amplified = {
            let Self {
                oversampler,
                input_shaper,
                solver,
                stage_lowpass,
                tone,
                feedback_resistance,
                ..
            } = self;
            let resistance = *feedback_resistance;
            oversampler.process(signal, |sample| {
                // Current the input network pushes into the clipping node.
                let current = input_shaper.high(sample) / INPUT_RESISTANCE;
                let drop = solver.solve(current, resistance, Diode::SILICON);
                // Non-inverting stage: the output is the input plus whatever
                // the feedback network drops across itself.
                let stage = stage_lowpass.low(sample + drop);
                tone.process(stage)
            })
        };

        self.output_lowpass.low(self.dc.process(amplified)) * self.level
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{magnitude_at, peak, render_sine, total_harmonic_distortion};

    fn pedal(drive: f32, tone: f32, sample_rate: f32) -> Overdrive {
        let mut pedal = Overdrive::default();
        pedal.prepare(sample_rate);
        pedal.set_controls(drive, tone, 0.7);
        pedal
    }

    #[test]
    fn more_drive_means_more_harmonic_content() {
        let sample_rate = 48_000.0;
        let mut low = pedal(0.05, 0.5, sample_rate);
        let mut high = pedal(1.0, 0.5, sample_rate);
        let quiet = render_sine(440.0, 0.05, sample_rate, 8_192, |sample| {
            low.process(sample)
        });
        let loud = render_sine(440.0, 0.05, sample_rate, 8_192, |sample| {
            high.process(sample)
        });
        let quiet_thd = total_harmonic_distortion(&quiet, 440.0, sample_rate);
        let loud_thd = total_harmonic_distortion(&loud, 440.0, sample_rate);
        assert!(
            loud_thd > quiet_thd * 1.5,
            "drive did not add harmonics: {loud_thd} vs {quiet_thd}"
        );
    }

    #[test]
    fn the_gain_stage_favours_the_midrange_over_the_bass() {
        // The signature of this topology: the series capacitor at the
        // inverting input has barely started conducting at 82 Hz, so the stage
        // amplifies a low note far less than a midrange one.
        //
        // Measured below the clipping knee on purpose. Higher up the midrange
        // is already compressing while the bass is still linear, which hides
        // the very difference this test is about — a comparison is only a
        // comparison when both sides are the same experiment.
        let sample_rate = 48_000.0;
        let mut pedal = pedal(0.6, 1.0, sample_rate);
        let bass = render_sine(82.0, 0.003, sample_rate, 16_384, |sample| {
            pedal.process(sample)
        });
        pedal.reset();
        let middle = render_sine(1_000.0, 0.003, sample_rate, 16_384, |sample| {
            pedal.process(sample)
        });
        let bass_level = magnitude_at(&bass, 82.0, sample_rate);
        let middle_level = magnitude_at(&middle, 1_000.0, sample_rate);
        assert!(
            middle_level > bass_level * 3.0,
            "no midrange emphasis: {middle_level} vs {bass_level}"
        );
    }

    #[test]
    fn the_tone_control_moves_treble_without_moving_the_body() {
        let sample_rate = 48_000.0;
        let mut dark = pedal(0.5, 0.0, sample_rate);
        let mut bright = pedal(0.5, 1.0, sample_rate);
        let dark_render = render_sine(3_000.0, 0.05, sample_rate, 8_192, |sample| {
            dark.process(sample)
        });
        let bright_render = render_sine(3_000.0, 0.05, sample_rate, 8_192, |sample| {
            bright.process(sample)
        });
        let dark_treble = magnitude_at(&dark_render, 3_000.0, sample_rate);
        let bright_treble = magnitude_at(&bright_render, 3_000.0, sample_rate);
        assert!(
            bright_treble > dark_treble * 2.0,
            "the tone control did nothing: {bright_treble} vs {dark_treble}"
        );
    }

    #[test]
    fn a_hot_signal_cannot_run_away() {
        let sample_rate = 48_000.0;
        let mut pedal = pedal(1.0, 1.0, sample_rate);
        let rendered = render_sine(220.0, 4.0, sample_rate, 8_192, |sample| {
            pedal.process(sample)
        });
        let level = peak(&rendered);
        assert!(level.is_finite() && level < 12.0, "peak reached {level}");
    }

    #[test]
    fn it_cleans_up_when_the_guitar_volume_comes_down() {
        // The reason players choose a feedback clipper: at a low input level
        // the diodes never reach their knee and the stage stays linear.
        let sample_rate = 48_000.0;
        let mut pedal = pedal(0.7, 0.5, sample_rate);
        let rolled_back = render_sine(440.0, 0.002, sample_rate, 8_192, |sample| {
            pedal.process(sample)
        });
        let distortion = total_harmonic_distortion(&rolled_back, 440.0, sample_rate);
        assert!(distortion < 0.05, "still dirty at low level: {distortion}");
    }
}
