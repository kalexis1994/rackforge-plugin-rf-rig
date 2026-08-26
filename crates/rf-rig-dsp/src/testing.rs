//! Measurement helpers shared by the unit tests.
//!
//! These are the same two questions the lab tool asks of a rendered file — how
//! much of frequency `f` is in this signal, and how much of it is harmonic —
//! kept small enough to run inside `cargo test`.

use crate::math::{TAU, sin, sqrt};
use std::vec::Vec;

/// Renders `samples` of a sine through `process`, discarding a settling
/// window first so filters and detectors are in steady state.
pub fn render_sine<F>(
    frequency: f32,
    amplitude: f32,
    sample_rate: f32,
    samples: usize,
    mut process: F,
) -> Vec<f32>
where
    F: FnMut(f32) -> f32,
{
    let settle = (sample_rate * 0.2) as usize;
    let mut output = Vec::with_capacity(samples);
    for index in 0..settle + samples {
        let phase = TAU * frequency * index as f32 / sample_rate;
        let value = process(amplitude * sin(phase));
        if index >= settle {
            output.push(value);
        }
    }
    output
}

/// Amplitude of one frequency component, by windowed correlation.
///
/// Two details are not fussiness, and both were measured rather than assumed.
///
/// The accumulation is `f64`: summing thousands of millivolt products into an
/// `f32` leaves a noise floor of the same order as the distortion some of these
/// tests look for.
///
/// The window is a Hann. Without it, a tone that does not fit a whole number of
/// cycles into the analysis window leaks into its neighbours, and the leak
/// lands squarely on the harmonic probes: a *perfect* sine measured bare reads
/// 0.62 % distorted. Windowed, the same sine reads 0.0006 %. An instrument with
/// a floor near the thing it measures is how a project ends up chasing its own
/// ruler.
pub fn magnitude_at(samples: &[f32], frequency: f32, sample_rate: f32) -> f32 {
    let mut real = 0.0_f64;
    let mut imaginary = 0.0_f64;
    let mut weight = 0.0_f64;
    let count = samples.len() as f64;
    let step = core::f64::consts::TAU * frequency as f64 / sample_rate as f64;
    for (index, sample) in samples.iter().enumerate() {
        let position = index as f64;
        let window = 0.5 - 0.5 * (core::f64::consts::TAU * position / count).cos();
        let phase = step * position;
        real += *sample as f64 * window * phase.cos();
        imaginary += *sample as f64 * window * phase.sin();
        weight += window;
    }
    if weight <= 0.0 {
        return 0.0;
    }
    (2.0 * (real * real + imaginary * imaginary).sqrt() / weight) as f32
}

/// Total harmonic distortion as a fraction, measured against the first eight
/// harmonics that still fit below Nyquist.
pub fn total_harmonic_distortion(samples: &[f32], fundamental: f32, sample_rate: f32) -> f32 {
    let first = magnitude_at(samples, fundamental, sample_rate);
    if first <= 1.0e-9 {
        return 0.0;
    }
    let mut harmonics = 0.0_f32;
    for order in 2..=8 {
        let frequency = fundamental * order as f32;
        if frequency >= sample_rate * 0.45 {
            break;
        }
        let magnitude = magnitude_at(samples, frequency, sample_rate);
        harmonics += magnitude * magnitude;
    }
    sqrt(harmonics) / first
}

/// Peak absolute value.
pub fn peak(samples: &[f32]) -> f32 {
    samples
        .iter()
        .fold(0.0_f32, |worst, sample| worst.max(sample.abs()))
}

/// Root mean square, accumulated in `f64` for the same reason as
/// [`magnitude_at`].
pub fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum: f64 = samples
        .iter()
        .map(|sample| *sample as f64 * *sample as f64)
        .sum();
    (sum / samples.len() as f64).sqrt() as f32
}
