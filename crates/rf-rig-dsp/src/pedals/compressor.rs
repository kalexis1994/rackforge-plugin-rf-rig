//! Compressor — a transconductance cell whose bias current is set by a
//! rectifier watching the cell's own output.
//!
//! The gain element is an OTA (`circuit::ota`), and everything this family is
//! known for follows from where the rectifier sits. Because the detector sees
//! what already left the pedal, the loop keeps reducing the bias current until
//! the output stops growing: the effective ratio is enormous, the knee is soft,
//! and the recovery depends on what was played rather than on a ratio control —
//! there isn't one.
//!
//! The signal is attenuated hard before the cell, because an OTA is linear over
//! about ±25 mV and a guitar is not. What survives of a pick attack after that
//! attenuation still reaches the knee, which is why these compressors thicken
//! transients instead of only ducking them.
//!
//! The detector is the circuit's, not a coefficient pair: a rectifier charging
//! a timing capacitor through a diode (`circuit::rectifier`). That gives the
//! pedal a real threshold, an attack that depends on how far above the
//! capacitor the signal sits, and a release that is an RC discharge.
//!
//! Controls follow the two-knob original plus the attack trimmer later units
//! exposed: `sustain` sets how hard the rectifier pulls the bias down, `attack`
//! moves the timing network, `level` is make-up gain.

use crate::circuit::filters::{CouplingCap, OnePole};
use crate::circuit::ota::TransconductanceCell;
use crate::circuit::rectifier::RectifierDetector;
use crate::math::{clamp, exponential, lerp};

/// A buffered input, as this family has: what drives it barely matters.
pub const INPUT_IMPEDANCE: f32 = 470_000.0;
/// The level control at the output.
pub const OUTPUT_IMPEDANCE: f32 = 10_000.0;

/// What the cell's output current is developed across.
const LOAD_RESISTANCE: f32 = 10_000.0;
/// The divider ahead of the cell. Without it an ordinary guitar level would sit
/// entirely outside the differential pair's linear region.
const INPUT_ATTENUATION: f32 = 1.0 / 30.0;
/// Bias current with no signal present, in amperes. With the load above this is
/// a small-signal gain of about 97 through the cell, or 3.2 after the input
/// divider.
const QUIESCENT_BIAS: f32 = 500.0e-6;
/// The current source never shuts off completely, so the pedal never goes
/// silent no matter how hard the detector pulls.
const MINIMUM_BIAS: f32 = 4.0e-6;

#[derive(Clone, Copy, Default)]
pub struct Compressor {
    input_cap: CouplingCap,
    detector_highpass: OnePole,
    detector: RectifierDetector,
    cell: TransconductanceCell,
    output_lowpass: OnePole,
    previous_output: f32,
    bias_current: f32,
    sensitivity: f32,
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
        self.detector.prepare(sample_rate);
        self.cell = TransconductanceCell::new(LOAD_RESISTANCE);
        self.bias_current = QUIESCENT_BIAS;
        self.sensitivity = 3.0e-4;
        self.makeup = 1.0;
        self.reset();
    }

    pub fn reset(&mut self) {
        self.input_cap.reset();
        self.detector_highpass.reset();
        self.detector.reset();
        self.output_lowpass.reset();
        self.cell.reset();
        self.previous_output = 0.0;
        self.bias_current = QUIESCENT_BIAS;
    }

    /// `sustain`, `attack` and `level` all arrive as 0..1 pot positions.
    pub fn set_controls(&mut self, sustain: f32, attack: f32, level: f32) {
        // How many amperes of bias the rectifier removes per volt of output.
        //
        // This number sets where the loop settles. The control voltage is
        // roughly the output peak times the detector's own gain, so the ceiling
        // is near `Iq / (gain·s)`: volts at the bottom of the travel, where the
        // pedal is effectively clean, and tens of millivolts at the top, where
        // everything sustains forever.
        self.sensitivity = exponential(clamp(sustain, 0.0, 1.0), 2.0e-5, 1.5e-3);
        // The trimmer moves the timing network itself: more resistance in the
        // charging path slows the attack, and a larger bleed resistor holds the
        // capacitor longer. They move together because one trimmer does both in
        // the pedals that expose it.
        let attack = clamp(attack, 0.0, 1.0);
        self.detector.set_timing(
            lerp(300.0, 6_000.0, attack),
            lerp(33_000.0, 220_000.0, attack),
        );
        self.makeup = exponential(clamp(level, 0.0, 1.0), 0.25, 8.0);
    }

    #[inline]
    pub fn process(&mut self, input: f32) -> f32 {
        let signal = self.input_cap.process(input);

        // Feedback detection: the rectifier watches the previous output, not
        // the input.
        let observed = self.detector_highpass.high(self.previous_output);
        let control = self.detector.process(observed);
        // The control voltage steals bias current from the cell.
        let bias = QUIESCENT_BIAS - self.sensitivity * control;
        self.bias_current = if bias < MINIMUM_BIAS {
            MINIMUM_BIAS
        } else {
            bias
        };

        let cell_output = self
            .cell
            .process(signal * INPUT_ATTENUATION, self.bias_current);
        // The rectifier taps the cell, not the output jack: the level control
        // comes after it, so turning the pedal up does not tighten the loop.
        self.previous_output = cell_output;
        self.output_lowpass.low(cell_output * self.makeup)
    }

    /// What the gain cell is doing right now, through the input divider: the
    /// loop's working, visible rather than inferred.
    pub fn cell_gain(&self) -> f32 {
        self.cell.gain(self.bias_current) * INPUT_ATTENUATION
    }

    /// The whole pedal's instantaneous small-signal gain, make-up included.
    pub fn current_gain(&self) -> f32 {
        self.cell_gain() * self.makeup
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

    fn compressor(sustain: f32, level: f32, sample_rate: f32) -> Compressor {
        let mut compressor = Compressor::default();
        compressor.prepare(sample_rate);
        compressor.set_controls(sustain, 0.5, level);
        compressor
    }

    #[test]
    fn twelve_decibels_in_becomes_far_less_than_twelve_decibels_out() {
        let sample_rate = 48_000.0;
        let mut unit = compressor(0.8, 0.5, sample_rate);
        let quiet = peak_of(&mut unit, 0.05, sample_rate);
        unit.reset();
        let loud = peak_of(&mut unit, 0.2, sample_rate);

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
        let mut gentle = compressor(0.05, 0.5, sample_rate);
        let mut heavy = compressor(1.0, 0.5, sample_rate);

        let gentle_peak = peak_of(&mut gentle, 0.3, sample_rate);
        let heavy_peak = peak_of(&mut heavy, 0.3, sample_rate);
        assert!(
            heavy_peak < gentle_peak,
            "more sustain did not reduce the level: {heavy_peak} vs {gentle_peak}"
        );
        assert!(heavy.cell_gain() < gentle.cell_gain());
    }

    #[test]
    fn the_bias_current_never_reaches_zero() {
        // A current source that shut off completely would mute the pedal; the
        // real one has a floor and so does this.
        let sample_rate = 48_000.0;
        let mut unit = compressor(1.0, 1.0, sample_rate);
        let peak = peak_of(&mut unit, 2.0, sample_rate);
        assert!(unit.bias_current >= MINIMUM_BIAS);
        assert!(peak > 1.0e-3, "the loop squeezed the signal to nothing");
    }

    #[test]
    fn the_cell_thickens_a_transient_rather_than_only_ducking_it() {
        // What survives the input divider still reaches the differential
        // pair's knee, so the compressor adds harmonics of its own. This is a
        // property of the gain element, not a flaw in the detector.
        let sample_rate = 48_000.0;
        let mut unit = compressor(0.9, 0.5, sample_rate);
        let rendered = crate::testing::render_sine(220.0, 0.5, sample_rate, 8_192, |sample| {
            unit.process(sample)
        });
        let distortion = crate::testing::total_harmonic_distortion(&rendered, 220.0, sample_rate);
        assert!(
            distortion > 0.01,
            "the gain cell added nothing at all: {distortion}"
        );
    }

    #[test]
    fn a_signal_under_the_detector_threshold_passes_untouched() {
        // The rectifier has a real knee, so there is a level below which this
        // pedal is a gain stage and nothing else.
        let sample_rate = 48_000.0;
        let mut unit = compressor(0.9, 0.5, sample_rate);
        let quiet = peak_of(&mut unit, 0.004, sample_rate);
        let expected = 0.004 * unit.current_gain();
        assert!(
            (quiet / expected - 1.0).abs() < 0.15,
            "the loop moved on a signal under its threshold: {quiet} against {expected}"
        );
    }

    #[test]
    fn silence_in_is_silence_out() {
        let mut unit = compressor(1.0, 1.0, 48_000.0);
        for _ in 0..48_000 {
            assert_eq!(unit.process(0.0), 0.0);
        }
    }
}
