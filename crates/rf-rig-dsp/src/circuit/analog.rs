//! Turning a continuous-time network into a filter that runs.
//!
//! Several things in this crate are solved on paper as a rational function of
//! `s` — a tone network, a pickup loaded by a pedal — and all of them reach the
//! audio path the same way: bilinear transform, `f64` while the coefficients
//! are still spread across twenty orders of magnitude, `f32` once they are
//! normalised.

/// A second-order rational function, as ascending powers of `s`.
///
/// `numerator[0] + numerator[1]·s + numerator[2]·s²`, over the same in the
/// denominator.
pub type Rational = ([f64; 3], [f64; 3]);

/// Bilinear transform at `sample_rate`, returning `[b0, b1, b2, a1, a2]` with
/// the denominator already normalised — the form [`crate::circuit::filters::Biquad`]
/// expects.
///
/// Without prewarping. Adequate for networks whose features sit well below the
/// sample rate, and for anything running inside an oversampled block: measured
/// on the fuzz tone stack, 1.15 dB of error at 10 kHz at 48 kHz, and 0.07 dB at
/// four times that.
///
/// Use [`bilinear_at`] instead when the network has a resonance. The transform
/// compresses the frequency axis, and on the side of a peak a small shift is a
/// large number of decibels.
pub fn bilinear(rational: Rational, sample_rate: f64) -> Option<[f32; 5]> {
    bilinear_with_scale(rational, 2.0 * sample_rate)
}

/// Bilinear transform that maps `frequency` exactly, leaving the rest of the
/// axis to compress around it.
pub fn bilinear_at(rational: Rational, sample_rate: f64, frequency: f64) -> Option<[f32; 5]> {
    if !(sample_rate.is_finite() && sample_rate > 0.0) {
        return None;
    }
    if !(frequency.is_finite() && frequency > 0.0 && frequency < sample_rate * 0.49) {
        return bilinear(rational, sample_rate);
    }
    let omega = core::f64::consts::TAU * frequency;
    let scale = omega / libm::tan(omega / (2.0 * sample_rate));
    bilinear_with_scale(rational, scale)
}

fn bilinear_with_scale(rational: Rational, scale: f64) -> Option<[f32; 5]> {
    let (numerator, denominator) = rational;
    if !(scale.is_finite() && scale > 0.0) {
        return None;
    }
    let square = scale * scale;
    let b0 = numerator[2] * square + numerator[1] * scale + numerator[0];
    let b1 = 2.0 * (numerator[0] - numerator[2] * square);
    let b2 = numerator[2] * square - numerator[1] * scale + numerator[0];
    let a0 = denominator[2] * square + denominator[1] * scale + denominator[0];
    let a1 = 2.0 * (denominator[0] - denominator[2] * square);
    let a2 = denominator[2] * square - denominator[1] * scale + denominator[0];

    if a0 == 0.0 || !a0.is_finite() {
        return None;
    }
    let coefficients = [
        (b0 / a0) as f32,
        (b1 / a0) as f32,
        (b2 / a0) as f32,
        (a1 / a0) as f32,
        (a2 / a0) as f32,
    ];
    coefficients
        .iter()
        .all(|value| value.is_finite())
        .then_some(coefficients)
}

/// Evaluates a rational function's magnitude at one frequency, for tests and
/// for the lab tool: this is the answer the discrete filter is supposed to
/// approximate.
pub fn analogue_magnitude(rational: Rational, frequency: f64) -> f64 {
    let (numerator, denominator) = rational;
    let omega = core::f64::consts::TAU * frequency;
    // s = jw, so s² = −w².
    let evaluate = |terms: [f64; 3]| {
        let real = terms[0] - terms[2] * omega * omega;
        let imaginary = terms[1] * omega;
        crate::math::sqrt64(real * real + imaginary * imaginary)
    };
    let bottom = evaluate(denominator);
    if bottom == 0.0 {
        return 0.0;
    }
    evaluate(numerator) / bottom
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Magnitude of a discrete biquad at one frequency.
    fn discrete_magnitude(coefficients: [f32; 5], frequency: f64, sample_rate: f64) -> f64 {
        let [b0, b1, b2, a1, a2] = coefficients.map(|value| value as f64);
        let angle = -core::f64::consts::TAU * frequency / sample_rate;
        let (sin1, cos1) = angle.sin_cos();
        let (sin2, cos2) = (2.0 * angle).sin_cos();
        let numerator_real = b0 + b1 * cos1 + b2 * cos2;
        let numerator_imaginary = b1 * sin1 + b2 * sin2;
        let denominator_real = 1.0 + a1 * cos1 + a2 * cos2;
        let denominator_imaginary = a1 * sin1 + a2 * sin2;
        ((numerator_real * numerator_real + numerator_imaginary * numerator_imaginary)
            / (denominator_real * denominator_real + denominator_imaginary * denominator_imaginary))
            .sqrt()
    }

    #[test]
    fn a_first_order_lowpass_lands_on_its_corner() {
        // 1/(1 + sRC), R = 1 kΩ, C = 159 nF: a corner at 1 kHz.
        let resistance = 1_000.0;
        let capacitance = 159.155e-9;
        let rational = ([1.0, 0.0, 0.0], [1.0, resistance * capacitance, 0.0]);
        let sample_rate = 192_000.0;
        let coefficients = bilinear(rational, sample_rate).expect("coefficients");
        let magnitude = discrete_magnitude(coefficients, 1_000.0, sample_rate);
        assert!(
            (magnitude - core::f64::consts::FRAC_1_SQRT_2).abs() < 0.005,
            "expected -3 dB at the corner, measured {magnitude}"
        );
    }

    #[test]
    fn the_discrete_filter_follows_the_analogue_one() {
        // A resonant second order, the shape a loaded pickup has.
        let rational = ([1.0, 0.0, 0.0], [1.0, 1.5e-5, 1.54e-9]);
        let sample_rate = 192_000.0;
        let coefficients = bilinear(rational, sample_rate).expect("coefficients");
        let mut frequency = 20.0_f64;
        let mut worst = 0.0_f64;
        while frequency < 10_000.0 {
            let analogue = analogue_magnitude(rational, frequency);
            let discrete = discrete_magnitude(coefficients, frequency, sample_rate);
            worst = worst.max((20.0 * (analogue / discrete).log10()).abs());
            frequency *= 1.2;
        }
        assert!(worst < 0.25, "discretisation drifted {worst:.3} dB");
    }

    #[test]
    fn prewarping_pins_the_resonance_where_it_belongs() {
        // The same resonant network at the host rate, where the plain
        // transform's frequency compression is worth decibels near the peak.
        let rational = ([1.0, 0.0, 0.0], [1.0, 1.5e-5, 1.54e-9]);
        let sample_rate = 48_000.0;
        let resonance = 1.0 / (core::f64::consts::TAU * 1.54e-9_f64.sqrt());

        let plain = bilinear(rational, sample_rate).expect("coefficients");
        let warped = bilinear_at(rational, sample_rate, resonance).expect("coefficients");
        let reference = analogue_magnitude(rational, resonance);
        let plain_error =
            (20.0 * (reference / discrete_magnitude(plain, resonance, sample_rate)).log10()).abs();
        let warped_error =
            (20.0 * (reference / discrete_magnitude(warped, resonance, sample_rate)).log10()).abs();
        assert!(
            warped_error < plain_error * 0.2,
            "prewarping did not help: {warped_error:.3} dB against {plain_error:.3} dB"
        );
        assert!(
            warped_error < 0.05,
            "still {warped_error:.3} dB out at the peak"
        );
    }

    #[test]
    fn nonsense_is_refused_rather_than_returned() {
        assert!(bilinear(([1.0, 0.0, 0.0], [0.0, 0.0, 0.0]), 48_000.0).is_none());
        assert!(bilinear(([1.0, 0.0, 0.0], [1.0, 0.0, 0.0]), 0.0).is_none());
    }
}
