//! The control path of an analog compressor: a rectifier charging a timing
//! capacitor.
//!
//! An envelope follower with an attack coefficient and a release coefficient is
//! a convenient fiction. What the circuit has is a diode — or a transistor
//! acting as one — pushing current into a capacitor, and a resistor bleeding it
//! away:
//!
//! ```text
//!            Rs        D
//!   |audio| ─/\/\──────▶|──┬────── control voltage
//!                          │
//!                         ═══ C
//!                          │
//!                          ▒ Rd
//!                          │
//!                         gnd
//! ```
//!
//! Three consequences, and all three are things players describe:
//!
//! * **The attack depends on level.** The charging current is
//!   `Is·(exp(Vd/nVt) − 1)`, so a transient far above the capacitor's present
//!   voltage charges it hard while a quiet one barely charges it at all. A
//!   fixed attack time cannot do that.
//! * **There is a real threshold.** Below the diode's knee nothing reaches the
//!   capacitor, so the compressor is genuinely transparent rather than always
//!   squeezing a little.
//! * **The release is an RC discharge**, not an exponential applied to an
//!   envelope — and since the control voltage then maps onto a bias current,
//!   the way the gain comes back is not the way the capacitor empties.
//!
//! The diode is solved rather than approximated by a knee: two Newton
//! iterations on
//!
//! ```text
//! (|audio| − Vc − Vd)/Rs = Is·(exp(Vd/(n·Vt)) − 1)
//! ```
//!
//! warm-started from the previous sample, which is the same treatment every
//! other junction in this crate gets.
//!
//! One more thing the circuit has and a bare rectifier does not: **gain before
//! the diode**. Without it the forward drop would swallow most of a guitar
//! signal and the compressor would need volts to do anything. With it, the
//! threshold lands where these pedals actually start working — a few tens of
//! millivolts — and it is still a real threshold rather than a fiction.

use crate::math::{abs, clamp, exp};

/// Beyond this the exponential is meaningless for a nine-volt circuit, and
/// inside `f32` it is the difference between a number and an infinity.
const ARGUMENT_LIMIT: f32 = 30.0;
/// No control path here carries a tenth of an ampere.
const CURRENT_LIMIT: f32 = 0.1;

#[derive(Clone, Copy)]
pub struct RectifierDetector {
    /// Timing capacitor, in farads.
    capacitance: f32,
    /// Resistance in the charging path, which is what the attack trimmer moves.
    series_resistance: f32,
    /// Bleed resistor across the capacitor: the release.
    discharge_resistance: f32,
    /// Gain in the control path, ahead of the rectifier.
    gain: f32,
    /// Rectifier saturation current, in amperes.
    saturation_current: f32,
    /// `n·Vt` for the rectifier.
    emission_voltage: f32,

    voltage: f32,
    drop: f32,
    time_step: f32,
}

impl Default for RectifierDetector {
    fn default() -> Self {
        Self {
            capacitance: 10.0e-6,
            series_resistance: 1_000.0,
            discharge_resistance: 100_000.0,
            gain: 15.0,
            saturation_current: 2.52e-9,
            emission_voltage: 1.752 * 0.025_85,
            voltage: 0.0,
            drop: 0.0,
            time_step: 1.0 / 48_000.0,
        }
    }
}

impl RectifierDetector {
    const ITERATIONS: usize = 2;

    pub fn prepare(&mut self, sample_rate: f32) {
        self.time_step = 1.0 / sample_rate.max(1.0);
        self.reset();
    }

    pub fn reset(&mut self) {
        self.voltage = 0.0;
        self.drop = 0.0;
    }

    /// Sets the timing network. `attack` moves the charging resistance and
    /// `release` the bleed resistance, which is what the trimmer in these
    /// pedals actually adjusts.
    pub fn set_timing(&mut self, series_resistance: f32, discharge_resistance: f32) {
        self.series_resistance = clamp(series_resistance, 10.0, 1.0e6);
        self.discharge_resistance = clamp(discharge_resistance, 1_000.0, 1.0e8);
    }

    /// The gain the control path applies before the rectifier. It sets where
    /// the threshold lands: the diode's forward drop divided by this is the
    /// smallest signal that moves the capacitor at all.
    pub fn set_gain(&mut self, gain: f32) {
        self.gain = clamp(gain, 1.0, 200.0);
    }

    /// The smallest input peak that reaches the capacitor, in volts. Roughly a
    /// forward drop referred back through the control path's gain.
    pub fn threshold(&self) -> f32 {
        0.4 / self.gain
    }

    /// The control voltage on the capacitor.
    pub fn voltage(&self) -> f32 {
        self.voltage
    }

    /// One sample of audio in, the control voltage out.
    #[inline]
    pub fn process(&mut self, input: f32) -> f32 {
        let rectified = abs(input) * self.gain;
        let available = rectified - self.voltage;

        // Solve the diode's drop, and with it the current reaching the
        // capacitor. Below the knee this settles at essentially zero current,
        // which is the threshold the circuit has and a follower does not.
        let charge = if available <= 0.0 {
            self.drop = 0.0;
            0.0
        } else {
            self.solve_charge(available)
        };

        let discharge = self.voltage / self.discharge_resistance;
        let next = self.voltage + (charge - discharge) * self.time_step / self.capacitance;
        self.voltage = if next.is_finite() {
            clamp(next, 0.0, 100.0)
        } else {
            0.0
        };
        self.voltage
    }

    /// Newton on the series resistor against the diode.
    #[inline]
    fn solve_charge(&mut self, available: f32) -> f32 {
        let conductance = 1.0 / self.series_resistance;
        let mut drop = clamp(self.drop, 0.0, available);
        let step_limit = 8.0 * self.emission_voltage + 0.05;

        for _ in 0..Self::ITERATIONS {
            let argument = clamp(
                drop / self.emission_voltage,
                -ARGUMENT_LIMIT,
                ARGUMENT_LIMIT,
            );
            let exponential = exp(argument);
            let current = clamp(
                self.saturation_current * (exponential - 1.0),
                -CURRENT_LIMIT,
                CURRENT_LIMIT,
            );
            let slope = clamp(
                self.saturation_current * exponential / self.emission_voltage,
                0.0,
                1.0e3,
            );
            // (available − drop)·G − Id(drop) = 0
            let residual = (available - drop) * conductance - current;
            let derivative = -conductance - slope;
            if derivative == 0.0 {
                break;
            }
            let change = clamp(residual / derivative, -step_limit, step_limit);
            drop -= change;
            if abs(change) < 1.0e-7 {
                break;
            }
        }

        drop = clamp(drop, 0.0, available);
        self.drop = drop;
        let current = (available - drop) * conductance;
        clamp(current, 0.0, CURRENT_LIMIT)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::{TAU, sin};

    const SAMPLE_RATE: f32 = 48_000.0;

    fn fresh() -> RectifierDetector {
        let mut detector = RectifierDetector::default();
        detector.prepare(SAMPLE_RATE);
        detector
    }

    /// Drives a steady tone and returns the settled control voltage.
    fn settled(detector: &mut RectifierDetector, amplitude: f32, seconds: f32) -> f32 {
        let samples = (SAMPLE_RATE * seconds) as usize;
        for index in 0..samples {
            let input = amplitude * sin(TAU * 220.0 * index as f32 / SAMPLE_RATE);
            detector.process(input);
        }
        detector.voltage()
    }

    #[test]
    fn a_signal_below_the_knee_never_reaches_the_capacitor() {
        // The threshold these compressors have, and the reason they can be
        // transparent instead of always working a little. It sits where the
        // control path's gain puts it — a forward drop referred back through
        // that gain — which for these values is a few tens of millivolts.
        let detector = fresh();
        let threshold = detector.threshold();
        assert!(
            (0.01..0.06).contains(&threshold),
            "the threshold landed at {threshold} V"
        );

        let mut quiet_unit = fresh();
        let quiet = settled(&mut quiet_unit, threshold * 0.4, 1.0);
        assert!(
            quiet < 0.05,
            "a signal well under the threshold charged the capacitor to {quiet} V"
        );

        let mut loud_unit = fresh();
        let loud = settled(&mut loud_unit, 0.3, 1.0);
        assert!(loud > 1.0, "a normal guitar peak only reached {loud} V");
    }

    #[test]
    fn a_louder_signal_reaches_a_given_control_voltage_sooner() {
        // The property a fixed attack coefficient cannot have, stated the way
        // it is audible: the loop starts pulling the gain down sooner on a hard
        // transient than on a soft one.
        //
        // Measured against a *fixed* voltage on purpose. Timing each signal to
        // half of its own settled value normalises away the very thing being
        // measured — both the drive and the target scale together — and reads
        // as no difference at all.
        let time_to = |amplitude: f32, target: f32| {
            let mut detector = fresh();
            for index in 0..(SAMPLE_RATE as usize) {
                let input = amplitude * sin(TAU * 220.0 * index as f32 / SAMPLE_RATE);
                if detector.process(input) >= target {
                    return index as f32 / SAMPLE_RATE;
                }
            }
            f32::INFINITY
        };

        let gentle = time_to(0.3, 0.5);
        let hard = time_to(2.0, 0.5);
        assert!(
            hard < gentle * 0.5,
            "a louder transient should get there sooner: {hard} s against {gentle} s"
        );
    }

    #[test]
    fn the_release_is_the_time_constant_the_components_give_it() {
        let mut detector = fresh();
        let charged = settled(&mut detector, 1.0, 1.5);
        assert!(charged > 0.3);

        // One time constant of silence: RC = 100 kΩ × 10 µF = 1 s, so the
        // capacitor should be at 1/e of where it was.
        let seconds = 1.0_f32;
        for _ in 0..(SAMPLE_RATE * seconds) as usize {
            detector.process(0.0);
        }
        let ratio = detector.voltage() / charged;
        let expected = 1.0 / core::f32::consts::E;
        assert!(
            (ratio - expected).abs() < 0.05,
            "after one time constant the capacitor held {ratio} of its charge, not {expected}"
        );
    }

    #[test]
    fn the_timing_network_can_be_moved() {
        let mut quick = fresh();
        quick.set_timing(1_000.0, 20_000.0);
        let mut slow = fresh();
        slow.set_timing(1_000.0, 400_000.0);

        settled(&mut quick, 1.0, 1.0);
        settled(&mut slow, 1.0, 1.0);
        for _ in 0..(SAMPLE_RATE as usize / 4) {
            quick.process(0.0);
            slow.process(0.0);
        }
        assert!(
            quick.voltage() < slow.voltage() * 0.5,
            "the shorter bleed resistor did not release faster: {} against {}",
            quick.voltage(),
            slow.voltage()
        );
    }

    #[test]
    fn it_stays_finite_and_positive_under_abuse() {
        let mut detector = fresh();
        for index in 0..(SAMPLE_RATE as usize * 2) {
            let input = if index % 800 < 400 { 40.0 } else { -40.0 };
            let voltage = detector.process(input);
            assert!(voltage.is_finite() && voltage >= 0.0);
        }
    }
}
