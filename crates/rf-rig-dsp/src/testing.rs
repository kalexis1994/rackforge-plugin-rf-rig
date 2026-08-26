//! Measurement helpers shared by the unit tests.
//!
//! These are the same two questions the lab tool asks of a rendered file — how
//! much of frequency `f` is in this signal, and how much of it is harmonic —
//! kept small enough to run inside `cargo test`.

use crate::math::{TAU, cos, sin, sqrt};
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

/// Amplitude of one frequency component, by correlation. Exact enough for the
/// ratios these tests compare, and it needs no transform.
pub fn magnitude_at(samples: &[f32], frequency: f32, sample_rate: f32) -> f32 {
    let mut real = 0.0_f32;
    let mut imaginary = 0.0_f32;
    for (index, sample) in samples.iter().enumerate() {
        let phase = TAU * frequency * index as f32 / sample_rate;
        real += sample * cos(phase);
        imaginary += sample * sin(phase);
    }
    let count = samples.len() as f32;
    2.0 * sqrt(real * real + imaginary * imaginary) / count
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

/// Root mean square.
pub fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum: f32 = samples.iter().map(|sample| sample * sample).sum();
    sqrt(sum / samples.len() as f32)
}
