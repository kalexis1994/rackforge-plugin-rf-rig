//! Fuzz — a booster into two cascaded clipping stages, into a scooped tone
//! network.
//!
//! This family has far more gain than an overdrive and puts diodes around
//! *each* of two stages. By the second one the waveform is essentially a
//! square, which is why the sustain seems endless and why the pedal is so
//! sensitive to what is in front of it.
//!
//! Both stages are solved from Ebers-Moll together with their diodes
//! (`circuit::transistor`), so the transistor and the pair argue about the
//! collector voltage the way they do in the circuit. The stages invert, and two
//! inversions cancel, so the pedal is in phase with the signal that entered it.
//! The asymmetry, the way the bias drifts under a hot signal, and the gating
//! that follows are consequences of that solve rather than features added on
//! top.
//!
//! The tone stack is the family's own, solved from its component values in
//! `circuit::tonestack`: two RC branches bridged by the pot, so the midrange
//! scoop and the way it travels across the control are consequences of the
//! network rather than a notch placed by hand.

use crate::circuit::filters::{CouplingCap, DcBlocker, OnePole};
use crate::circuit::oversample::Oversampler4;
use crate::circuit::tonestack::{ToneNetwork, ToneStack};
use crate::circuit::transistor::{CommonEmitterStage, FeedbackDiodes, StageDesign};
use crate::math::{clamp, exponential};

/// Series resistance into the booster that opens the circuit. The sustain
/// control sits after it, which is why this family cleans up so little when the
/// control comes down: the signal has already been amplified by then.
const BOOSTER_INPUT_RESISTANCE: f32 = 39_000.0;
const BOOSTER_COLLECTOR_RESISTANCE: f32 = 10_000.0;
const BOOSTER_FEEDBACK_RESISTANCE: f32 = 470_000.0;
const BOOSTER_EMITTER_RESISTANCE: f32 = 390.0;
/// Series resistance into each clipping stage.
const STAGE_INPUT_RESISTANCE: f32 = 10_000.0;
/// Collector load.
const STAGE_COLLECTOR_RESISTANCE: f32 = 10_000.0;
/// Collector-to-base feedback, which both biases the stage and carries the
/// clipping diodes.
const STAGE_FEEDBACK_RESISTANCE: f32 = 100_000.0;
/// Emitter degeneration, unbypassed.
const STAGE_EMITTER_RESISTANCE: f32 = 100.0;
/// 0.1 µF coupling into each stage: with 10 kΩ that is a 160 Hz corner.
const STAGE_COUPLING_FARADS: f32 = 100.0e-9;
/// 470 pF across the 100 kΩ feedback resistor, which is what stops each stage
/// from amplifying the fizz the one before it made.
const STAGE_ROLLOFF_HZ: f32 = 3_400.0;
/// What this pedal presents to whatever is in front of it.
///
/// Measured from the model rather than declared — see the test at the bottom of
/// this file — because the number is a consequence of the booster's bias
/// network, and because it is the whole reason this family is famous for
/// caring what comes before it. There is no input buffer here: that is the
/// point of the circuit, not an omission from the model.
pub const INPUT_IMPEDANCE: f32 = 62_000.0;
/// The volume control at the output, seen from the next pedal.
pub const OUTPUT_IMPEDANCE: f32 = 25_000.0;

/// The output buffer after the tone stack. The clipping stages hand over about
/// six tenths of a volt and the tone network throws ten to sixteen decibels of
/// that away; this is the recovery that makes the pedal louder than the guitar,
/// which is the whole reason it has a volume control rather than a level trim.
const OUTPUT_MAKEUP: f32 = 14.0;

#[derive(Clone, Copy, Default)]
struct ClippingStage {
    stage: CommonEmitterStage,
    rolloff: OnePole,
    bias: f32,
}

impl ClippingStage {
    fn prepare(&mut self, inner_rate: f32) {
        self.stage = CommonEmitterStage::new(StageDesign {
            diodes: FeedbackDiodes::SILICON,
            input_resistance: STAGE_INPUT_RESISTANCE,
            collector_resistance: STAGE_COLLECTOR_RESISTANCE,
            feedback_resistance: STAGE_FEEDBACK_RESISTANCE,
            emitter_resistance: STAGE_EMITTER_RESISTANCE,
            coupling_capacitance: STAGE_COUPLING_FARADS,
            ..StageDesign::default()
        });
        self.stage.settle(inner_rate);
        self.bias = self.stage.operating_point();
        self.rolloff = OnePole::new(STAGE_ROLLOFF_HZ, inner_rate);
        self.rolloff.reset();
    }

    fn reset(&mut self) {
        self.stage.reset();
        self.rolloff.reset();
    }

    /// The stage's operating point, for tests that want to see where the bias
    /// network parked it.
    fn operating_point(&self) -> f32 {
        self.stage.operating_point()
    }

    /// Takes a signal referred to ground and returns one referred to ground:
    /// the stage's own coupling capacitor blocks whatever bias arrives, and the
    /// quiescent collector voltage is removed on the way out.
    #[inline]
    fn process(&mut self, input: f32) -> f32 {
        let collector = self.stage.process(input);
        self.rolloff.low(collector - self.bias)
    }
}

#[derive(Clone, Copy, Default)]
pub struct Fuzz {
    input_cap: CouplingCap,
    oversampler: Oversampler4,
    booster: CommonEmitterStage,
    booster_bias: f32,
    first: ClippingStage,
    second: ClippingStage,
    tone: ToneStack,
    dc: DcBlocker,
    sustain: f32,
    volume: f32,
}

impl Fuzz {
    pub fn prepare(&mut self, sample_rate: f32) {
        let inner_rate = sample_rate * Oversampler4::FACTOR as f32;
        self.input_cap = CouplingCap::new(16.0, sample_rate);
        self.booster = CommonEmitterStage::new(StageDesign {
            diodes: FeedbackDiodes::NONE,
            input_resistance: BOOSTER_INPUT_RESISTANCE,
            collector_resistance: BOOSTER_COLLECTOR_RESISTANCE,
            feedback_resistance: BOOSTER_FEEDBACK_RESISTANCE,
            emitter_resistance: BOOSTER_EMITTER_RESISTANCE,
            coupling_capacitance: STAGE_COUPLING_FARADS,
            ..StageDesign::default()
        });
        self.booster.settle(inner_rate);
        self.booster_bias = self.booster.operating_point();
        self.first.prepare(inner_rate);
        self.second.prepare(inner_rate);
        self.tone.prepare(ToneNetwork::FUZZ, inner_rate);
        self.dc = DcBlocker::new(sample_rate);
        self.sustain = 0.3;
        self.volume = 0.4;
        self.reset();
    }

    pub fn reset(&mut self) {
        self.input_cap.reset();
        self.oversampler.reset();
        self.booster.reset();
        self.first.reset();
        self.second.reset();
        self.tone.reset();
        self.dc.reset();
    }

    pub fn set_controls(&mut self, sustain: f32, tone: f32, volume: f32) {
        // The sustain control is the attenuator between the booster and the
        // first clipping stage. All of the gain is fixed by the circuit; this
        // decides how much of it meets the clippers.
        self.sustain = exponential(clamp(sustain, 0.0, 1.0), 0.004, 1.0);
        self.tone.set_position(clamp(tone, 0.0, 1.0));
        self.volume = exponential(clamp(volume, 0.0, 1.0), 0.02, 2.5);
    }

    /// The impedance the pedal in front of this one drives it through.
    pub fn set_source_impedance(&mut self, ohms: f32) {
        self.booster.set_source_impedance(ohms);
    }

    /// Where the bias network parked each stage, in volts: the booster first,
    /// then the two clipping stages. Published service voltages for this family
    /// put the clipping stages near a volt, so this is a number anyone can
    /// check against a real unit with a meter.
    pub fn operating_points(&self) -> [f32; 3] {
        [
            self.booster.operating_point(),
            self.first.operating_point(),
            self.second.operating_point(),
        ]
    }

    #[inline]
    pub fn process(&mut self, input: f32) -> f32 {
        let signal = self.input_cap.process(input);

        let fuzzed = {
            let sustain = self.sustain;
            let bias = self.booster_bias;
            let Self {
                oversampler,
                booster,
                first,
                second,
                tone,
                ..
            } = self;
            oversampler.process(signal, |sample| {
                let boosted = (booster.process(sample) - bias) * sustain;
                tone.process(second.process(first.process(boosted)))
            })
        };

        self.dc.process(fuzzed) * self.volume * OUTPUT_MAKEUP
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
    fn every_stage_biases_where_the_network_says() {
        // Published service voltages for this family put the clipping stages
        // near a volt at the collector, with the base a forward drop up. The
        // solver is not told that; it is where the bias network lands.
        let mut pedal = Fuzz::default();
        pedal.prepare(48_000.0);
        let [booster, first, second] = pedal.operating_points();
        for collector in [first, second] {
            assert!(
                (0.5..2.0).contains(&collector),
                "a clipping stage biased to {collector} V"
            );
        }
        assert!(
            (2.0..8.0).contains(&booster),
            "the booster biased to {booster} V"
        );
    }

    #[test]
    fn the_declared_input_impedance_is_the_one_the_circuit_has() {
        // Drive the booster with a tone and divide by the current its input
        // branch draws. The constant this file publishes has to match what the
        // circuit actually presents, or the loading model upstream is a
        // fiction.
        let sample_rate = 48_000.0 * 4.0;
        let mut pedal = Fuzz::default();
        pedal.prepare(48_000.0);
        let mut current = std::vec::Vec::new();
        for index in 0..8_192 {
            let drive =
                0.01 * crate::math::sin(crate::math::TAU * 1_000.0 * index as f32 / sample_rate);
            pedal.booster.process(drive);
            current.push(pedal.booster.last_input_current());
        }
        let measured = 0.01 / magnitude_at(&current, 1_000.0, sample_rate);
        let ratio = measured / INPUT_IMPEDANCE;
        assert!(
            (0.75..1.35).contains(&ratio),
            "declared {INPUT_IMPEDANCE} ohms, measured {measured}"
        );
    }

    #[test]
    fn a_lossy_source_takes_the_edge_off_it() {
        // The audible half of the loading model: less signal reaches the
        // clipping stages, so the pedal saturates less.
        let sample_rate = 48_000.0;
        let distortion_with = |source: f32| {
            let mut pedal = Fuzz::default();
            pedal.prepare(sample_rate);
            pedal.set_controls(0.7, 0.5, 0.5);
            pedal.set_source_impedance(source);
            let rendered = render_sine(220.0, 0.1, sample_rate, 8_192, |sample| {
                pedal.process(sample)
            });
            total_harmonic_distortion(&rendered, 220.0, sample_rate)
        };
        let direct = distortion_with(100.0);
        let lossy = distortion_with(150_000.0);
        assert!(
            lossy < direct * 0.9,
            "a lossy source changed nothing: {lossy} against {direct}"
        );
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
