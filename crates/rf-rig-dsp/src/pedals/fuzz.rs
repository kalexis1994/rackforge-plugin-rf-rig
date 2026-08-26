//! Fuzz — two cascaded feedback-clipping stages and a scooped tone network.
//!
//! This family has far more gain than an overdrive and puts a clipping pair
//! around *each* of two stages. By the second one the waveform is essentially a
//! square, which is why the sustain seems endless and why the pedal is so
//! sensitive to what is in front of it.
//!
//! The stages are inverting: the output is the drop across the feedback
//! network, negated. Two inversions cancel, so the pedal is in phase with the
//! signal that entered it.
//!
//! Known approximation: the tone network here is a two-path blend plus a fixed
//! midrange cut. A real tone stack of this family is a resistive junction of
//! two RC branches whose notch depth changes with the control; solving that
//! network properly is on the plan in `docs/IMPLEMENTATION_PLAN.md`.

use crate::circuit::filters::{Biquad, CouplingCap, DcBlocker, OnePole};
use crate::circuit::nonlinear::{ClipperSolver, Diode};
use crate::circuit::oversample::Oversampler4;
use crate::math::{clamp, exponential, lerp};

/// Series resistance into each clipping stage.
const STAGE_INPUT_RESISTANCE: f32 = 10_000.0;
/// Collector feedback resistance.
const STAGE_FEEDBACK_RESISTANCE: f32 = 100_000.0;
/// 0.1 µF into 10 kΩ between the stages.
const STAGE_CORNER_HZ: f32 = 160.0;
/// 470 pF across the 100 kΩ feedback resistor.
const STAGE_ROLLOFF_HZ: f32 = 3_400.0;
/// The tone network's two branches: 39 kΩ with 10 nF, and 100 kΩ with 3.9 nF.
const TONE_LOW_HZ: f32 = 408.0;
const TONE_HIGH_HZ: f32 = 408.0;
/// The output buffer after the tone stack. The clipping stages hand over about
/// six tenths of a volt and the tone network throws most of that away; this is
/// the recovery that makes the pedal louder than the guitar, which is the whole
/// reason it has a volume control rather than a level trim.
const OUTPUT_MAKEUP: f32 = 6.0;

#[derive(Clone, Copy, Default)]
struct ClippingStage {
    coupling: CouplingCap,
    solver: ClipperSolver,
    rolloff: OnePole,
}

impl ClippingStage {
    fn prepare(&mut self, inner_rate: f32) {
        self.coupling = CouplingCap::new(STAGE_CORNER_HZ, inner_rate);
        self.rolloff = OnePole::new(STAGE_ROLLOFF_HZ, inner_rate);
        self.reset();
    }

    fn reset(&mut self) {
        self.coupling.reset();
        self.solver.reset();
        self.rolloff.reset();
    }

    #[inline]
    fn process(&mut self, input: f32) -> f32 {
        let current = self.coupling.process(input) / STAGE_INPUT_RESISTANCE;
        let drop = self
            .solver
            .solve(current, STAGE_FEEDBACK_RESISTANCE, Diode::SILICON);
        -self.rolloff.low(drop)
    }
}

#[derive(Clone, Copy, Default)]
pub struct Fuzz {
    input_cap: CouplingCap,
    oversampler: Oversampler4,
    first: ClippingStage,
    second: ClippingStage,
    tone_low: OnePole,
    tone_high: OnePole,
    scoop: Biquad,
    dc: DcBlocker,
    sustain: f32,
    tone: f32,
    volume: f32,
}

impl Fuzz {
    pub fn prepare(&mut self, sample_rate: f32) {
        let inner_rate = sample_rate * Oversampler4::FACTOR as f32;
        self.input_cap = CouplingCap::new(16.0, sample_rate);
        self.first.prepare(inner_rate);
        self.second.prepare(inner_rate);
        self.tone_low = OnePole::new(TONE_LOW_HZ, sample_rate);
        self.tone_high = OnePole::new(TONE_HIGH_HZ, sample_rate);
        self.scoop = Biquad::default();
        self.scoop.set_peaking(1_000.0, 0.7, -8.0, sample_rate);
        self.dc = DcBlocker::new(sample_rate);
        self.sustain = 0.3;
        self.tone = 0.5;
        self.volume = 0.4;
        self.reset();
    }

    pub fn reset(&mut self) {
        self.input_cap.reset();
        self.oversampler.reset();
        self.first.reset();
        self.second.reset();
        self.tone_low.reset();
        self.tone_high.reset();
        self.scoop.reset();
        self.dc.reset();
    }

    pub fn set_controls(&mut self, sustain: f32, tone: f32, volume: f32) {
        // The sustain control is an attenuator ahead of the first stage. All
        // of the gain is fixed by the circuit; this decides how much signal
        // meets it.
        self.sustain = exponential(clamp(sustain, 0.0, 1.0), 0.002, 1.0);
        self.tone = clamp(tone, 0.0, 1.0);
        self.volume = exponential(clamp(volume, 0.0, 1.0), 0.02, 2.5);
    }

    #[inline]
    pub fn process(&mut self, input: f32) -> f32 {
        let signal = self.input_cap.process(input) * self.sustain;

        let fuzzed = {
            let Self {
                oversampler,
                first,
                second,
                ..
            } = self;
            oversampler.process(signal, |sample| second.process(first.process(sample)))
        };

        let low = self.tone_low.low(fuzzed);
        let high = self.tone_high.high(fuzzed);
        let blended = lerp(low, high, self.tone);
        let scooped = self.scoop.process(blended);
        self.dc.process(scooped) * self.volume * OUTPUT_MAKEUP
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{magnitude_at, peak, render_sine, rms, total_harmonic_distortion};

    fn pedal(sustain: f32, tone: f32, sample_rate: f32) -> Fuzz {
        let mut pedal = Fuzz::default();
        pedal.prepare(sample_rate);
        pedal.set_controls(sustain, tone, 0.5);
        pedal
    }

    #[test]
    fn full_sustain_squares_the_waveform_off() {
        // Crest factor says this more honestly than harmonic content does,
        // because the tone network sits between the clipping stages and the
        // measurement. A sine is 1.414; a square is 1.0.
        let sample_rate = 48_000.0;
        let mut clean = pedal(0.0, 0.5, sample_rate);
        let mut dirty = pedal(1.0, 0.5, sample_rate);
        let clean_render = render_sine(220.0, 0.2, sample_rate, 8_192, |sample| {
            clean.process(sample)
        });
        let dirty_render = render_sine(220.0, 0.2, sample_rate, 8_192, |sample| {
            dirty.process(sample)
        });
        let clean_crest = peak(&clean_render) / rms(&clean_render);
        let dirty_crest = peak(&dirty_render) / rms(&dirty_render);
        assert!(
            clean_crest > 1.38,
            "the quiet setting should still be a sine, measured {clean_crest}"
        );
        assert!(
            dirty_crest < 1.25,
            "full sustain did not square the wave off, crest {dirty_crest}"
        );
    }

    #[test]
    fn the_sustain_control_changes_how_much_of_the_signal_meets_the_gain() {
        let sample_rate = 48_000.0;
        let mut low = pedal(0.0, 0.5, sample_rate);
        let mut high = pedal(1.0, 0.5, sample_rate);
        let clean = render_sine(220.0, 0.05, sample_rate, 8_192, |sample| {
            low.process(sample)
        });
        let dirty = render_sine(220.0, 0.05, sample_rate, 8_192, |sample| {
            high.process(sample)
        });
        let clean_thd = total_harmonic_distortion(&clean, 220.0, sample_rate);
        let dirty_thd = total_harmonic_distortion(&dirty, 220.0, sample_rate);
        assert!(
            dirty_thd > clean_thd * 2.0,
            "sustain had little effect: {dirty_thd} vs {clean_thd}"
        );
    }

    #[test]
    fn the_tone_control_trades_bass_for_treble() {
        let sample_rate = 48_000.0;
        let mut dark = pedal(0.6, 0.0, sample_rate);
        let mut bright = pedal(0.6, 1.0, sample_rate);
        let dark_bass = render_sine(110.0, 0.1, sample_rate, 16_384, |sample| {
            dark.process(sample)
        });
        let bright_bass = render_sine(110.0, 0.1, sample_rate, 16_384, |sample| {
            bright.process(sample)
        });
        let dark_level = magnitude_at(&dark_bass, 110.0, sample_rate);
        let bright_level = magnitude_at(&bright_bass, 110.0, sample_rate);
        assert!(
            dark_level > bright_level * 2.0,
            "the tone control did not remove bass: {dark_level} vs {bright_level}"
        );
    }

    #[test]
    fn it_stays_bounded_on_a_hot_input() {
        let sample_rate = 48_000.0;
        let mut pedal = pedal(1.0, 1.0, sample_rate);
        let rendered = render_sine(150.0, 5.0, sample_rate, 8_192, |sample| {
            pedal.process(sample)
        });
        let level = peak(&rendered);
        assert!(level.is_finite() && level < 12.0, "peak reached {level}");
    }
}
