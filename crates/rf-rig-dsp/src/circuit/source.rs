//! What the guitar sees, and why the first pedal changes it.
//!
//! A magnetic pickup is not a signal generator. It is a coil — several henries
//! of inductance with kilohms of wire resistance — feeding the capacitance of
//! its own winding and the cable. That network resonates, typically between two
//! and five kilohertz, and the height of that resonance depends on **what is
//! plugged into it**:
//!
//! ```text
//!      R      L
//!  ○──/\/\──∿∿∿∿──┬──────┬──── to the pedal
//!                 │      │
//!   e(t)         ═══ C   ▒ R_load
//!                 │      │
//!  ○──────────────┴──────┴──── ground
//! ```
//!
//! Solving it,
//!
//! ```text
//! H(s) = 1 / (1 + R·G + s(R·C + L·G) + s²·L·C)      G = 1/R_load
//! ```
//!
//! A megohm load barely damps the resonance and the guitar sounds bright and
//! peaky. A fuzz's tens of kilohms flattens it and takes the top with it, which
//! is the whole reason players argue about what goes first on a board.
//!
//! ## Why this is a *correction* and not a filter
//!
//! RF-Rig never sees a pickup. It receives a signal that already went through
//! one, loaded by whatever the interface presents — a megohm or so — and that
//! loading is already baked into the recording. Applying the loaded response on
//! top would count the pickup twice.
//!
//! So what this models is the *difference* between two loading conditions:
//!
//! ```text
//! correction(s) = H(s, R_pedal) / H(s, R_reference)
//! ```
//!
//! which is exactly the part that is missing, and which collapses to unity when
//! the pedal's input impedance equals the reference. That identity is a test.

use crate::circuit::analog::{Rational, bilinear_at};
use crate::circuit::filters::Biquad;

/// Two resistances in parallel.
fn parallel(first: f64, second: f64) -> f64 {
    if first <= 0.0 || second <= 0.0 {
        return 0.0;
    }
    (first * second) / (first + second)
}

/// A pickup and the cable hanging off it.
#[derive(Clone, Copy)]
pub struct PickupSource {
    /// Coil resistance, in ohms.
    pub resistance: f64,
    /// Coil inductance, in henries.
    pub inductance: f64,
    /// Winding plus cable capacitance, in farads.
    pub capacitance: f64,
    /// The guitar's own volume control, which sits across the pickup whatever
    /// is plugged in after it. It belongs in every load, including the one the
    /// signal was captured through.
    pub volume_control: f64,
    /// The instrument input the signal was captured through.
    pub capture_load: f64,
}

impl PickupSource {
    /// A single coil on a few metres of cable: about 4 kHz of resonance.
    pub const SINGLE_COIL: Self = Self {
        resistance: 6_000.0,
        inductance: 2.2,
        capacitance: 700.0e-12,
        volume_control: 250_000.0,
        capture_load: 1_000_000.0,
    };

    /// A humbucker: more turns, so more inductance and more resistance, and the
    /// resonance drops towards 3 kHz.
    pub const HUMBUCKER: Self = Self {
        resistance: 8_500.0,
        inductance: 4.5,
        capacitance: 700.0e-12,
        volume_control: 500_000.0,
        capture_load: 1_000_000.0,
    };

    /// Where the network resonates into a given load, in hertz. Reported for
    /// the lab tool and for tests; the audio path uses the whole transfer.
    pub fn resonance(&self) -> f64 {
        1.0 / (core::f64::consts::TAU * crate::math::sqrt64(self.inductance * self.capacitance))
    }

    /// The transfer from coil to output for one load.
    fn response(&self, load: f64) -> Rational {
        let conductance = 1.0 / load;
        (
            [1.0, 0.0, 0.0],
            [
                1.0 + self.resistance * conductance,
                self.resistance * self.capacitance + self.inductance * conductance,
                self.inductance * self.capacitance,
            ],
        )
    }

    /// The load the pickup actually sees when a pedal of `input_impedance` is
    /// plugged in: the volume control is in parallel with it.
    pub fn total_load(&self, input_impedance: f64) -> f64 {
        parallel(self.volume_control, input_impedance)
    }

    /// The load the signal was captured through.
    pub fn reference_load(&self) -> f64 {
        parallel(self.volume_control, self.capture_load)
    }

    /// What changes when the pickup is loaded by a pedal of `input_impedance`
    /// instead of by the input it was recorded through: the ratio of the two
    /// responses. Unity when they are the same.
    pub fn load_correction(&self, input_impedance: f64) -> Rational {
        let (_, reference) = self.response(self.reference_load());
        let (_, loaded) = self.response(self.total_load(input_impedance));
        (reference, loaded)
    }
}

/// The correction, running.
#[derive(Clone, Copy, Default)]
pub struct SourceLoading {
    biquad: Biquad,
    engaged: bool,
}

impl SourceLoading {
    /// Configures the filter for a pickup driving `load` ohms. Passing `None`
    /// for the pickup means the signal arrived buffered, and nothing is
    /// applied — which is the honest answer for a line-level source, not a
    /// missing feature.
    pub fn prepare(&mut self, pickup: Option<PickupSource>, load: f32, sample_rate: f32) {
        self.biquad.reset();
        self.engaged = false;
        let Some(pickup) = pickup else {
            return;
        };
        if !(load.is_finite() && load > 0.0) {
            return;
        }
        // Prewarped at the resonance, which is the only part of this filter
        // steep enough for the transform's frequency compression to matter.
        let Some(coefficients) = bilinear_at(
            pickup.load_correction(load as f64),
            sample_rate as f64,
            pickup.resonance(),
        ) else {
            return;
        };
        let [b0, b1, b2, a1, a2] = coefficients;
        self.biquad.set_coefficients(b0, b1, b2, a1, a2);
        self.engaged = true;
    }

    pub fn reset(&mut self) {
        self.biquad.reset();
    }

    #[inline]
    pub fn process(&mut self, input: f32) -> f32 {
        if !self.engaged {
            return input;
        }
        self.biquad.process(input)
    }

    pub fn coefficients(&self) -> [f32; 5] {
        self.biquad.coefficients()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::circuit::analog::analogue_magnitude;

    fn decibels(value: f64) -> f64 {
        20.0 * value.max(1.0e-12).log10()
    }

    #[test]
    fn the_resonance_sits_where_the_coil_puts_it() {
        let single = PickupSource::SINGLE_COIL.resonance();
        let humbucker = PickupSource::HUMBUCKER.resonance();
        assert!(
            (3_500.0..4_600.0).contains(&single),
            "single coil resonance at {single} Hz"
        );
        assert!(
            (2_400.0..3_300.0).contains(&humbucker),
            "humbucker resonance at {humbucker} Hz"
        );
        assert!(humbucker < single, "a humbucker resonates lower");
    }

    #[test]
    fn loading_with_the_reference_impedance_changes_nothing() {
        // The identity that keeps this a correction rather than a second
        // pickup: with the load it was recorded through, the filter is a wire.
        let pickup = PickupSource::SINGLE_COIL;
        let correction = pickup.load_correction(pickup.capture_load);
        let mut frequency = 20.0_f64;
        while frequency < 15_000.0 {
            let magnitude = analogue_magnitude(correction, frequency);
            assert!(
                (magnitude - 1.0).abs() < 1.0e-9,
                "{magnitude} at {frequency} Hz"
            );
            frequency *= 1.3;
        }
    }

    #[test]
    fn a_low_impedance_pedal_flattens_the_resonance_and_takes_the_top() {
        let pickup = PickupSource::SINGLE_COIL;
        let correction = pickup.load_correction(30_000.0);
        let resonance = pickup.resonance();
        let at_resonance = decibels(analogue_magnitude(correction, resonance));
        let at_bass = decibels(analogue_magnitude(correction, 100.0));
        let at_top = decibels(analogue_magnitude(correction, 8_000.0));

        assert!(
            at_resonance < -4.0,
            "the peak should be damped, measured {at_resonance:.2} dB"
        );
        assert!(
            (-3.0..-0.5).contains(&at_bass),
            "the coil resistance divides the low end by {at_bass:.2} dB"
        );
        assert!(
            at_top < at_bass,
            "the treble should suffer more than the bass: {at_top:.2} vs {at_bass:.2} dB"
        );
    }

    #[test]
    fn a_high_impedance_pedal_barely_touches_it() {
        // Which is exactly why buffered pedals exist, and why the answer here
        // is allowed to be "almost nothing".
        let pickup = PickupSource::SINGLE_COIL;
        let correction = pickup.load_correction(470_000.0);
        let mut worst = 0.0_f64;
        let mut frequency = 20.0_f64;
        while frequency < 10_000.0 {
            worst = worst.max(decibels(analogue_magnitude(correction, frequency)).abs());
            frequency *= 1.2;
        }
        assert!(worst < 1.5, "a 470 kΩ input moved things by {worst:.2} dB");
    }

    #[test]
    fn the_volume_control_is_in_the_circuit_whatever_follows_it() {
        // A pedal with a megohm input does not present a megohm to the coil:
        // the guitar's own volume pot is across it either way.
        let pickup = PickupSource::SINGLE_COIL;
        let total = pickup.total_load(1_000_000.0);
        assert!(
            total < pickup.volume_control,
            "the parallel load should be below the pot alone, got {total}"
        );
        assert!((total - pickup.reference_load()).abs() < 1.0);
    }

    #[test]
    fn a_buffered_source_is_left_alone() {
        let mut loading = SourceLoading::default();
        loading.prepare(None, 30_000.0, 192_000.0);
        for value in [0.0_f32, 0.25, -0.5, 1.0] {
            assert_eq!(loading.process(value), value);
        }
    }

    #[test]
    fn the_running_filter_matches_the_network() {
        let pickup = PickupSource::HUMBUCKER;
        let load = 47_000.0_f64;
        let sample_rate = 192_000.0_f32;
        let mut loading = SourceLoading::default();
        loading.prepare(Some(pickup), load as f32, sample_rate);
        let correction = pickup.load_correction(load);

        let [b0, b1, b2, a1, a2] = loading.coefficients().map(|value| value as f64);
        let mut frequency = 20.0_f64;
        let mut worst = 0.0_f64;
        while frequency < 10_000.0 {
            let angle = -core::f64::consts::TAU * frequency / sample_rate as f64;
            let (sin1, cos1) = angle.sin_cos();
            let (sin2, cos2) = (2.0 * angle).sin_cos();
            let top =
                ((b0 + b1 * cos1 + b2 * cos2).powi(2) + (b1 * sin1 + b2 * sin2).powi(2)).sqrt();
            let bottom =
                ((1.0 + a1 * cos1 + a2 * cos2).powi(2) + (a1 * sin1 + a2 * sin2).powi(2)).sqrt();
            let discrete = top / bottom;
            let analogue = analogue_magnitude(correction, frequency);
            worst = worst.max((decibels(analogue) - decibels(discrete)).abs());
            frequency *= 1.25;
        }
        assert!(
            worst < 0.2,
            "the filter drifted {worst:.3} dB from the network"
        );
    }
}
