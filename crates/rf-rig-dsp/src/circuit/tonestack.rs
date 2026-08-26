//! Tone controls, solved as the networks they are.
//!
//! A stompbox tone control is not a tilt filter with a knob on it. It is two RC
//! branches driven from the same stage and bridged by a potentiometer, and the
//! wiper sees both of them at once:
//!
//! ```text
//!         R1         A
//!   Vin ──/\/\───┬────────┐
//!                │        │
//!               ═══ C1    ▒          wiper
//!                │        ▒ P  ──────┬──── out
//!               gnd       ▒          │
//!         C2         B    │          ▒ RL
//!   Vin ───┤├─────┬───────┘          │
//!                 │                 gnd
//!                 ▒ R2
//!                 │
//!                gnd
//! ```
//!
//! Everything people say about these controls falls out of that picture. The
//! midrange scoop is not a notch filter someone added: at the middle of the pot
//! the wiper sits between a signal that has already lost its treble and one
//! that has already lost its bass, and what is left in the middle is what
//! neither branch passes. The scoop moves and changes depth as the wiper
//! travels, which no fixed peaking filter reproduces.
//!
//! ## The derivation
//!
//! Three nodes, with `Y = sC` and `G = 1/R`:
//!
//! ```text
//! A: VA(G1 + sC1 + Ga) − VW·Ga            = Vin·G1
//! B: VB(sC2 + G2 + Gb) − VW·Gb            = Vin·sC2
//! W: VW(Ga + Gb + GL)  − VA·Ga − VB·Gb    = 0
//! ```
//!
//! where `Ga = 1/(k·P)` and `Gb = 1/((1−k)·P)` are the two halves of the pot.
//! Eliminating `VA` and `VB` gives a second-order rational function — one pole
//! pair for the two capacitors — whose coefficients are written out in
//! [`ToneNetwork::analog`]. A test in this module checks those coefficients
//! against a direct complex solve of the same three equations, so the algebra
//! cannot rot.

use crate::circuit::filters::Biquad;

/// One tone network, in component values.
#[derive(Clone, Copy)]
pub struct ToneNetwork {
    /// `R1`, from the driving stage into the lowpass node.
    pub lowpass_resistance: f64,
    /// `C1`, from the lowpass node to ground.
    pub lowpass_capacitance: f64,
    /// `C2`, from the driving stage into the highpass node.
    pub highpass_capacitance: f64,
    /// `R2`, from the highpass node to ground.
    pub highpass_resistance: f64,
    /// `P`, the tone pot bridging the two nodes.
    pub potentiometer: f64,
    /// `RL`, whatever the wiper drives. A tone stack loaded by a volume pot
    /// does not measure the same as one driving an op-amp, so the load is part
    /// of the network rather than an afterthought.
    pub load: f64,
}

impl ToneNetwork {
    /// The classic scooped stack: 39 kΩ with 10 nF against 22 kΩ with 3.9 nF,
    /// bridged by a 100 kΩ linear pot and loaded by the volume control.
    ///
    /// These are the published values for the fuzz family this models. The
    /// branches cross near 1 kHz, which is where the scoop sits.
    pub const FUZZ: Self = Self {
        lowpass_resistance: 39_000.0,
        lowpass_capacitance: 10.0e-9,
        highpass_capacitance: 3.9e-9,
        highpass_resistance: 22_000.0,
        potentiometer: 100_000.0,
        load: 100_000.0,
    };

    /// A shallower scoop with its crossover higher up, for the distortion.
    ///
    /// Representative rather than canonical: the topology is the family's, and
    /// the values are chosen to put the corners where the published response of
    /// that family sits (a milder dip than the fuzz, crossing near 1.5 kHz).
    /// Tracing a specific unit would replace these six numbers and nothing
    /// else.
    pub const DISTORTION: Self = Self {
        lowpass_resistance: 10_000.0,
        lowpass_capacitance: 22.0e-9,
        highpass_capacitance: 10.0e-9,
        highpass_resistance: 10_000.0,
        potentiometer: 20_000.0,
        load: 100_000.0,
    };

    /// A treble control rather than a scoop, for the overdrive.
    ///
    /// The highpass branch is deliberately degenerate — 1 µF into 100 kΩ turns
    /// over at 1.6 Hz, so that side of the pot is a wire. The wiper therefore
    /// blends between the full signal and one rolled off from 723 Hz
    /// (1 kΩ with 220 nF), which is why this family's tone control moves the
    /// treble without moving the body.
    pub const OVERDRIVE: Self = Self {
        lowpass_resistance: 1_000.0,
        lowpass_capacitance: 220.0e-9,
        highpass_capacitance: 1.0e-6,
        highpass_resistance: 100_000.0,
        potentiometer: 20_000.0,
        load: 100_000.0,
    };

    /// Continuous-time coefficients for a wiper at `position`, where 0 is the
    /// lowpass end of the pot and 1 the highpass end.
    ///
    /// Returns `(numerator, denominator)` as ascending powers of `s`.
    pub fn analog(&self, position: f64) -> ([f64; 3], [f64; 3]) {
        // A pot never quite reaches either end: the wiper has contact
        // resistance, and a zero-ohm half would be an infinite conductance.
        let travel = position.clamp(0.01, 0.99);

        let conductance_lowpass = 1.0 / self.lowpass_resistance;
        let conductance_highpass = 1.0 / self.highpass_resistance;
        let conductance_upper = 1.0 / (travel * self.potentiometer);
        let conductance_lower = 1.0 / ((1.0 - travel) * self.potentiometer);
        let conductance_load = 1.0 / self.load;
        let capacitance_lowpass = self.lowpass_capacitance;
        let capacitance_highpass = self.highpass_capacitance;

        let alpha = conductance_lowpass + conductance_upper;
        let beta = conductance_highpass + conductance_lower;
        let wiper = conductance_upper + conductance_lower + conductance_load;

        let numerator = [
            conductance_upper * conductance_lowpass * beta,
            capacitance_highpass
                * (conductance_upper * conductance_lowpass + conductance_lower * alpha),
            conductance_lower * capacitance_lowpass * capacitance_highpass,
        ];
        let denominator = [
            wiper * alpha * beta
                - conductance_upper * conductance_upper * beta
                - conductance_lower * conductance_lower * alpha,
            wiper * (alpha * capacitance_highpass + beta * capacitance_lowpass)
                - conductance_upper * conductance_upper * capacitance_highpass
                - conductance_lower * conductance_lower * capacitance_lowpass,
            wiper * capacitance_lowpass * capacitance_highpass,
        ];
        (numerator, denominator)
    }
}

impl Default for ToneNetwork {
    fn default() -> Self {
        Self::FUZZ
    }
}

/// A tone network running as a discrete filter.
///
/// The coefficients are recomputed only when the control moves, in `f64`,
/// because the products of a conductance and a capacitance span twenty orders
/// of magnitude before they are normalised.
#[derive(Clone, Copy, Default)]
pub struct ToneStack {
    network: ToneNetwork,
    biquad: Biquad,
    sample_rate: f64,
    position: f32,
}

impl ToneStack {
    /// `sample_rate` is the rate this filter actually runs at — inside an
    /// oversampled block, that is the oversampled rate. It matters: the
    /// bilinear transform compresses the top of the band, and running the
    /// network at four times the host rate takes the error at 10 kHz from
    /// about 1.2 dB down to 0.07 dB.
    pub fn prepare(&mut self, network: ToneNetwork, sample_rate: f32) {
        self.network = network;
        self.sample_rate = sample_rate as f64;
        self.biquad.reset();
        self.apply(self.position);
    }

    pub fn reset(&mut self) {
        self.biquad.reset();
    }

    pub fn set_position(&mut self, position: f32) {
        if (position - self.position).abs() < 1.0e-6 {
            return;
        }
        self.apply(position);
    }

    fn apply(&mut self, position: f32) {
        self.position = position;
        if self.sample_rate <= 0.0 {
            return;
        }
        let (numerator, denominator) = self.network.analog(position as f64);
        // Bilinear transform, s = c(1 − z⁻¹)/(1 + z⁻¹).
        let c = 2.0 * self.sample_rate;
        let square = c * c;
        let b0 = numerator[2] * square + numerator[1] * c + numerator[0];
        let b1 = 2.0 * (numerator[0] - numerator[2] * square);
        let b2 = numerator[2] * square - numerator[1] * c + numerator[0];
        let a0 = denominator[2] * square + denominator[1] * c + denominator[0];
        let a1 = 2.0 * (denominator[0] - denominator[2] * square);
        let a2 = denominator[2] * square - denominator[1] * c + denominator[0];
        if a0 == 0.0 || !a0.is_finite() {
            return;
        }
        self.biquad.set_coefficients(
            (b0 / a0) as f32,
            (b1 / a0) as f32,
            (b2 / a0) as f32,
            (a1 / a0) as f32,
            (a2 / a0) as f32,
        );
    }

    #[inline]
    pub fn process(&mut self, input: f32) -> f32 {
        self.biquad.process(input)
    }

    /// The discrete coefficients, for tests and the lab tool.
    pub fn coefficients(&self) -> [f32; 5] {
        self.biquad.coefficients()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::vec::Vec;

    /// Just enough complex arithmetic to solve the network directly.
    #[derive(Clone, Copy)]
    struct Complex {
        real: f64,
        imaginary: f64,
    }

    impl Complex {
        const fn new(real: f64, imaginary: f64) -> Self {
            Self { real, imaginary }
        }

        fn add(self, other: Self) -> Self {
            Self::new(self.real + other.real, self.imaginary + other.imaginary)
        }

        fn subtract(self, other: Self) -> Self {
            Self::new(self.real - other.real, self.imaginary - other.imaginary)
        }

        fn multiply(self, other: Self) -> Self {
            Self::new(
                self.real * other.real - self.imaginary * other.imaginary,
                self.real * other.imaginary + self.imaginary * other.real,
            )
        }

        fn divide(self, other: Self) -> Self {
            let denominator = other.real * other.real + other.imaginary * other.imaginary;
            Self::new(
                (self.real * other.real + self.imaginary * other.imaginary) / denominator,
                (self.imaginary * other.real - self.real * other.imaginary) / denominator,
            )
        }

        fn magnitude(self) -> f64 {
            (self.real * self.real + self.imaginary * self.imaginary).sqrt()
        }
    }

    /// The ground truth: solve the three node equations at one frequency with
    /// no algebra in between, by Cramer's rule.
    fn network_response(network: &ToneNetwork, position: f64, frequency: f64) -> f64 {
        let travel = position.clamp(0.01, 0.99);
        let s = Complex::new(0.0, core::f64::consts::TAU * frequency);
        let real = Complex::new;

        let conductance_lowpass = real(1.0 / network.lowpass_resistance, 0.0);
        let conductance_highpass = real(1.0 / network.highpass_resistance, 0.0);
        let upper = real(1.0 / (travel * network.potentiometer), 0.0);
        let lower = real(1.0 / ((1.0 - travel) * network.potentiometer), 0.0);
        let load = real(1.0 / network.load, 0.0);
        let admittance_lowpass = s.multiply(real(network.lowpass_capacitance, 0.0));
        let admittance_highpass = s.multiply(real(network.highpass_capacitance, 0.0));
        let zero = real(0.0, 0.0);

        let matrix = [
            [
                conductance_lowpass.add(admittance_lowpass).add(upper),
                zero,
                zero.subtract(upper),
            ],
            [
                zero,
                admittance_highpass.add(conductance_highpass).add(lower),
                zero.subtract(lower),
            ],
            [
                zero.subtract(upper),
                zero.subtract(lower),
                upper.add(lower).add(load),
            ],
        ];
        let right = [conductance_lowpass, admittance_highpass, zero];

        let determinant = |m: [[Complex; 3]; 3]| {
            m[0][0]
                .multiply(
                    m[1][1]
                        .multiply(m[2][2])
                        .subtract(m[1][2].multiply(m[2][1])),
                )
                .subtract(
                    m[0][1].multiply(
                        m[1][0]
                            .multiply(m[2][2])
                            .subtract(m[1][2].multiply(m[2][0])),
                    ),
                )
                .add(
                    m[0][2].multiply(
                        m[1][0]
                            .multiply(m[2][1])
                            .subtract(m[1][1].multiply(m[2][0])),
                    ),
                )
        };

        let mut replaced = matrix;
        for row in 0..3 {
            replaced[row][2] = right[row];
        }
        determinant(replaced)
            .divide(determinant(matrix))
            .magnitude()
    }

    /// The response the discrete filter actually has, from its coefficients.
    fn filter_response(stack: &ToneStack, frequency: f64, sample_rate: f64) -> f64 {
        let [b0, b1, b2, a1, a2] = stack.coefficients();
        let angle = -core::f64::consts::TAU * frequency / sample_rate;
        let z1 = Complex::new(angle.cos(), angle.sin());
        let z2 = z1.multiply(z1);
        let numerator = Complex::new(b0 as f64, 0.0)
            .add(z1.multiply(Complex::new(b1 as f64, 0.0)))
            .add(z2.multiply(Complex::new(b2 as f64, 0.0)));
        let denominator = Complex::new(1.0, 0.0)
            .add(z1.multiply(Complex::new(a1 as f64, 0.0)))
            .add(z2.multiply(Complex::new(a2 as f64, 0.0)));
        numerator.divide(denominator).magnitude()
    }

    fn decibels(value: f64) -> f64 {
        20.0 * value.max(1.0e-12).log10()
    }

    fn stack(network: ToneNetwork, position: f32, sample_rate: f32) -> ToneStack {
        let mut stack = ToneStack::default();
        stack.prepare(network, sample_rate);
        stack.set_position(position);
        stack
    }

    #[test]
    fn the_running_filter_matches_a_direct_solve_of_the_netlist() {
        // The oversampled rate the pedals actually run these at.
        let sample_rate = 192_000.0_f32;
        let mut worst: f64 = 0.0;
        for network in [
            ToneNetwork::FUZZ,
            ToneNetwork::DISTORTION,
            ToneNetwork::OVERDRIVE,
        ] {
            for position in [0.0_f32, 0.15, 0.35, 0.5, 0.7, 0.9, 1.0] {
                let stack = stack(network, position, sample_rate);
                let mut frequency = 20.0_f64;
                while frequency < 10_000.0 {
                    let analogue = network_response(&network, position as f64, frequency);
                    let discrete = filter_response(&stack, frequency, sample_rate as f64);
                    worst = worst.max((decibels(analogue) - decibels(discrete)).abs());
                    frequency *= 1.25;
                }
            }
        }
        assert!(
            worst < 0.15,
            "the discrete filter drifted {worst:.3} dB from the network"
        );
    }

    #[test]
    fn the_fuzz_stack_scoops_the_midrange_at_the_centre_of_its_travel() {
        let network = ToneNetwork::FUZZ;
        let low = decibels(network_response(&network, 0.5, 100.0));
        let middle = decibels(network_response(&network, 0.5, 1_200.0));
        let high = decibels(network_response(&network, 0.5, 6_000.0));
        assert!(
            middle < low - 3.0 && middle < high - 3.0,
            "expected a scoop, measured {low:.1} / {middle:.1} / {high:.1} dB"
        );
    }

    #[test]
    fn the_scoop_travels_with_the_control() {
        // The reason a network beats a fixed notch: where the dip sits depends
        // on where the wiper is — and it travels *downwards* as the control
        // opens, because the further the wiper sits from the lowpass node the
        // lower the frequency at which neither branch is still delivering.
        // Measured on the network: 1.6 kHz at 0.3, 680 Hz at 0.7.
        let network = ToneNetwork::FUZZ;
        let dip = |position: f64| {
            let mut worst = (0.0_f64, f64::INFINITY);
            let mut frequency = 200.0_f64;
            while frequency < 8_000.0 {
                let level = decibels(network_response(&network, position, frequency));
                if level < worst.1 {
                    worst = (frequency, level);
                }
                frequency *= 1.05;
            }
            worst.0
        };
        let dark = dip(0.3);
        let bright = dip(0.7);
        assert!(
            dark > bright * 1.5,
            "the dip did not travel: {dark:.0} Hz at 0.3, {bright:.0} Hz at 0.7"
        );
    }

    #[test]
    fn the_overdrive_stack_moves_treble_and_leaves_the_body_alone() {
        let network = ToneNetwork::OVERDRIVE;
        let bass: Vec<f64> = [0.05, 0.5, 0.95]
            .iter()
            .map(|position| decibels(network_response(&network, *position, 100.0)))
            .collect();
        let treble: Vec<f64> = [0.05, 0.5, 0.95]
            .iter()
            .map(|position| decibels(network_response(&network, *position, 5_000.0)))
            .collect();
        let bass_span = bass.iter().cloned().fold(f64::MIN, f64::max)
            - bass.iter().cloned().fold(f64::MAX, f64::min);
        let treble_span = treble.iter().cloned().fold(f64::MIN, f64::max)
            - treble.iter().cloned().fold(f64::MAX, f64::min);
        assert!(
            bass_span < 1.0,
            "the control moved the bass by {bass_span:.2} dB"
        );
        assert!(
            treble_span > 12.0,
            "the control barely moved the treble: {treble_span:.2} dB"
        );
    }

    #[test]
    fn the_filter_is_stable_at_both_ends_of_the_pot() {
        for network in [
            ToneNetwork::FUZZ,
            ToneNetwork::DISTORTION,
            ToneNetwork::OVERDRIVE,
        ] {
            for position in [0.0_f32, 0.5, 1.0] {
                let mut stack = stack(network, position, 192_000.0);
                let mut worst = 0.0_f32;
                for index in 0..8_192 {
                    let input =
                        crate::math::sin(crate::math::TAU * 440.0 * index as f32 / 192_000.0);
                    let output = stack.process(input);
                    assert!(output.is_finite());
                    worst = worst.max(output.abs());
                }
                assert!(worst < 4.0, "a passive network amplified to {worst}");
            }
        }
    }
}
