//! The operational transconductance amplifier.
//!
//! An OTA is a current source steered by a voltage, and its transfer is not a
//! design choice — it is the differential pair inside it:
//!
//! ```text
//! Iout = Iabc · tanh(Vin / (2·Vt))
//! ```
//!
//! `Vt` is the thermal voltage, 25.85 mV at room temperature, so the cell is
//! linear over about ±25 mV and bends hard outside that. `Iabc` — the amplifier
//! bias current — is the control input: doubling it doubles the gain, which is
//! why this part became the gain cell of every analog compressor, and why those
//! compressors distort when a transient outruns their detector.
//!
//! Two consequences are worth naming, because both are audible and neither is
//! programmed:
//!
//! * the pedal has to attenuate hard before the cell, or an ordinary guitar
//!   signal would sit entirely in the bent region;
//! * what is left of a pick attack after that attenuation still reaches the
//!   knee, so the compressor thickens transients rather than only ducking them.

use crate::circuit::nonlinear::SoftLimiter;

/// Thermal voltage at room temperature, in volts.
pub const THERMAL_VOLTAGE: f32 = 0.025_85;
/// The differential input that puts the cell at the edge of its linear region.
pub const LINEAR_RANGE: f32 = 2.0 * THERMAL_VOLTAGE;

#[derive(Clone, Copy, Default)]
pub struct TransconductanceCell {
    /// Whatever the output current is developed across, in ohms.
    load_resistance: f32,
    shaper: SoftLimiter,
}

impl TransconductanceCell {
    pub fn new(load_resistance: f32) -> Self {
        Self {
            load_resistance,
            shaper: SoftLimiter::default(),
        }
    }

    pub fn reset(&mut self) {
        self.shaper.reset();
    }

    /// Small-signal voltage gain for a given bias current: `Iabc·RL/(2·Vt)`.
    /// Useful for calibration and for the lab tool; the audio path uses
    /// [`Self::process`].
    pub fn gain(&self, bias_current: f32) -> f32 {
        bias_current * self.load_resistance / LINEAR_RANGE
    }

    /// One sample through the cell. The `tanh` is antiderivative-antialiased,
    /// because a pick attack does reach the knee and a memoryless shaper would
    /// fold that back into the band.
    #[inline]
    pub fn process(&mut self, differential_voltage: f32, bias_current: f32) -> f32 {
        let normalised = differential_voltage / LINEAR_RANGE;
        bias_current * self.load_resistance * self.shaper.process(normalised)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::{TAU, sin};
    use crate::testing::{magnitude_at, render_sine};

    const SAMPLE_RATE: f32 = 48_000.0;

    /// Voltage gain measured the way a bench would measure it: a steady tone
    /// in, the fundamental out.
    ///
    /// Single-sample probing would not do. The cell's `tanh` is
    /// antiderivative-antialiased, so one isolated call returns the average
    /// slope since the previous input rather than the instantaneous one — an
    /// artefact of the measurement, not of the circuit.
    fn measured_gain(cell: &mut TransconductanceCell, amplitude: f32, bias: f32) -> f32 {
        let rendered = render_sine(100.0, amplitude, SAMPLE_RATE, 4_096, |sample| {
            cell.process(sample, bias)
        });
        magnitude_at(&rendered, 100.0, SAMPLE_RATE) / amplitude
    }

    #[test]
    fn doubling_the_bias_current_doubles_the_gain() {
        let mut cell = TransconductanceCell::new(10_000.0);
        let quiet = measured_gain(&mut cell, 0.001, 250.0e-6);
        cell.reset();
        let loud = measured_gain(&mut cell, 0.001, 500.0e-6);
        assert!(
            (loud / quiet - 2.0).abs() < 0.02,
            "gain did not track the bias current: {}",
            loud / quiet
        );
    }

    #[test]
    fn the_small_signal_gain_is_the_one_the_closed_form_predicts() {
        let bias = 400.0e-6;
        let mut cell = TransconductanceCell::new(10_000.0);
        let measured = measured_gain(&mut cell, 0.001, bias);
        let predicted = cell.gain(bias);
        assert!(
            (measured / predicted - 1.0).abs() < 0.02,
            "measured {measured}, the cell predicts {predicted}"
        );
    }

    #[test]
    fn the_pair_saturates_once_the_input_passes_the_thermal_voltage() {
        let bias = 400.0e-6;
        let mut cell = TransconductanceCell::new(10_000.0);
        let small = measured_gain(&mut cell, 0.001, bias);
        cell.reset();
        let large = measured_gain(&mut cell, 0.2, bias);
        // tanh(x)/x at x = 0.2/2Vt is about a third of unity slope, and the
        // fundamental of the compressed wave carries a little more than that.
        assert!(
            large < small * 0.45,
            "the differential pair did not saturate: {large} vs {small}"
        );
        assert!(
            large > small * 0.1,
            "the cell collapsed instead of saturating: {large} vs {small}"
        );
    }

    #[test]
    fn a_hard_drive_stays_bounded_at_the_bias_current() {
        let mut cell = TransconductanceCell::new(10_000.0);
        let bias = 500.0e-6;
        let ceiling = bias * 10_000.0;
        for index in 0..4_800 {
            let input = 2.0 * sin(TAU * 220.0 * index as f32 / SAMPLE_RATE);
            let output = cell.process(input, bias);
            assert!(
                output.abs() <= ceiling * 1.02,
                "{output} exceeded {ceiling}"
            );
        }
    }
}
