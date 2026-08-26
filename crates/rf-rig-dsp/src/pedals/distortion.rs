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

use crate::circuit::filters::{Biquad, CouplingCap, DcBlocker, OnePole};
use crate::circuit::nonlinear::{ClipperSolver, Diode, SaturatingStage};
use crate::circuit::oversample::Oversampler4;
use crate::math::{clamp, exponential, lerp};

/// Series resistance into the clipping node.
const CLIPPING_SERIES_RESISTANCE: f32 = 2_200.0;
/// The gain stage's input network: 4.7 kΩ with 0.47 µF, so the corner sits far
/// lower than an overdrive's and the bass survives into the clipper.
const INPUT_RESISTANCE: f32 = 4_700.0;
const INPUT_CORNER_HZ: f32 = 72.0;
const FIXED_FEEDBACK: f32 = 22_000.0;
const DISTORTION_POT: f32 = 100_000.0;

#[derive(Clone, Copy, Default)]
pub struct Distortion {
    input_cap: CouplingCap,
    booster: SaturatingStage,
    rail: SaturatingStage,
    oversampler: Oversampler4,
    input_shaper: OnePole,
    clip_solver: ClipperSolver,
    stage_lowpass: OnePole,
    scoop: Biquad,
    tone_split: OnePole,
    output_lowpass: OnePole,
    dc: DcBlocker,
    feedback_resistance: f32,
    brightness: f32,
    level: f32,
}

impl Distortion {
    pub fn prepare(&mut self, sample_rate: f32) {
        let inner_rate = sample_rate * Oversampler4::FACTOR as f32;
        self.input_cap = CouplingCap::new(30.0, sample_rate);
        // A 9 V rail biased near a third of the supply: the stage runs out of
        // room in one direction first, which is where the even harmonics of a
        // booster come from.
        self.booster = SaturatingStage::new(12.0, 4.2, 2.6);
        // The gain stage cannot swing past its own supply. This is the rail,
        // not a diode: it is what a distortion pedal does before the clipping
        // diodes ever see the signal.
        self.rail = SaturatingStage::new(1.0, 4.0, 3.4);
        self.input_shaper = OnePole::new(INPUT_CORNER_HZ, inner_rate);
        self.stage_lowpass = OnePole::new(7_500.0, inner_rate);
        self.scoop = Biquad::default();
        self.scoop.set_peaking(650.0, 0.8, -7.0, sample_rate);
        self.tone_split = OnePole::new(900.0, sample_rate);
        self.output_lowpass = OnePole::new(6_500.0, sample_rate);
        self.dc = DcBlocker::new(sample_rate);
        self.feedback_resistance = FIXED_FEEDBACK + 0.5 * DISTORTION_POT;
        self.brightness = 1.0;
        self.level = 0.5;
        self.reset();
    }

    pub fn reset(&mut self) {
        self.input_cap.reset();
        self.oversampler.reset();
        self.input_shaper.reset();
        self.clip_solver.reset();
        self.stage_lowpass.reset();
        self.scoop.reset();
        self.tone_split.reset();
        self.output_lowpass.reset();
        self.dc.reset();
    }

    pub fn set_controls(&mut self, distortion: f32, tone: f32, level: f32) {
        self.feedback_resistance = FIXED_FEEDBACK + clamp(distortion, 0.0, 1.0) * DISTORTION_POT;
        self.brightness = lerp(0.2, 2.6, clamp(tone, 0.0, 1.0));
        self.level = exponential(clamp(level, 0.0, 1.0), 0.02, 2.0);
    }

    #[inline]
    pub fn process(&mut self, input: f32) -> f32 {
        let boosted = self.booster.process(self.input_cap.process(input));

        let clipped = {
            let rail = self.rail;
            let Self {
                oversampler,
                input_shaper,
                clip_solver,
                stage_lowpass,
                feedback_resistance,
                ..
            } = self;
            let resistance = *feedback_resistance;
            oversampler.process(boosted, |sample| {
                let current = input_shaper.high(sample) / INPUT_RESISTANCE;
                // Op-amp gain: the input network's current across the feedback
                // resistance, added to the signal already at the node.
                let amplified = rail.process(sample + current * resistance);
                let filtered = stage_lowpass.low(amplified);
                // Hard clip: the node is pulled to the diodes' forward drop
                // through the series resistor.
                clip_solver.solve(
                    filtered / CLIPPING_SERIES_RESISTANCE,
                    CLIPPING_SERIES_RESISTANCE,
                    Diode::SILICON,
                )
            })
        };

        let scooped = self.scoop.process(clipped);
        let low = self.tone_split.low(scooped);
        let high = scooped - low;
        let toned = low + high * self.brightness;
        // Clipping to a 0.6 V knee leaves a quiet signal; the output stage
        // makes that back up.
        self.output_lowpass.low(self.dc.process(toned)) * self.level * 3.0
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
        let middle = level_at(650.0, &mut pedal);
        let high = level_at(3_000.0, &mut pedal);
        assert!(
            middle < low && middle < high,
            "expected a scoop, measured {low} / {middle} / {high}"
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
