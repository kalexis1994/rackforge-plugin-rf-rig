//! Polyphase half-band oversampling.
//!
//! Every clipper here is solved in continuous voltage but evaluated at discrete
//! times, and a hard knee generates harmonics far above the audio band. Without
//! oversampling those fold back as inharmonic tones that no real pedal makes —
//! the single most recognisable "digital distortion" artefact.
//!
//! The structure is the standard polyphase IIR half-band: two all-pass branches
//! whose outputs differ by a half sample, so the image at `fs/2` cancels. It
//! costs four multiplies per branch and has no passband ripple worth naming.

use crate::math::sanitise;

/// First-order all-pass running at the *decimated* rate, which makes it a
/// `z^-2` section at the interpolated rate.
#[derive(Clone, Copy, Default)]
struct AllpassSection {
    coefficient: f32,
    last_input: f32,
    last_output: f32,
}

impl AllpassSection {
    const fn new(coefficient: f32) -> Self {
        Self {
            coefficient,
            last_input: 0.0,
            last_output: 0.0,
        }
    }

    fn reset(&mut self) {
        self.last_input = 0.0;
        self.last_output = 0.0;
    }

    #[inline]
    fn process(&mut self, input: f32) -> f32 {
        let output = self.coefficient * (input - self.last_output) + self.last_input;
        self.last_input = input;
        self.last_output = sanitise(output);
        self.last_output
    }
}

/// A two-branch half-band filter usable as either an interpolator or a
/// decimator. The four coefficients give roughly 60 dB of image rejection with
/// a transition band that a guitar signal never reaches.
#[derive(Clone, Copy)]
pub struct Halfband {
    path_0: [AllpassSection; 2],
    path_1: [AllpassSection; 2],
}

impl Default for Halfband {
    fn default() -> Self {
        Self {
            path_0: [
                AllpassSection::new(0.079_866_43),
                AllpassSection::new(0.545_353_65),
            ],
            path_1: [
                AllpassSection::new(0.283_829_34),
                AllpassSection::new(0.834_411_9),
            ],
        }
    }
}

impl Halfband {
    pub fn reset(&mut self) {
        for section in self.path_0.iter_mut().chain(self.path_1.iter_mut()) {
            section.reset();
        }
    }

    #[inline]
    fn run_path_0(&mut self, input: f32) -> f32 {
        let mut value = input;
        for section in self.path_0.iter_mut() {
            value = section.process(value);
        }
        value
    }

    #[inline]
    fn run_path_1(&mut self, input: f32) -> f32 {
        let mut value = input;
        for section in self.path_1.iter_mut() {
            value = section.process(value);
        }
        value
    }

    /// One input sample becomes two, in time order.
    #[inline]
    pub fn up(&mut self, input: f32) -> (f32, f32) {
        let first = self.run_path_1(input);
        let second = self.run_path_0(input);
        (first, second)
    }

    /// Two input samples become one.
    #[inline]
    pub fn down(&mut self, first: f32, second: f32) -> f32 {
        0.5 * (self.run_path_0(second) + self.run_path_1(first))
    }
}

/// Runs a nonlinearity at four times the host sample rate.
#[derive(Clone, Copy, Default)]
pub struct Oversampler4 {
    up_first: Halfband,
    up_second: Halfband,
    down_second: Halfband,
    down_first: Halfband,
}

impl Oversampler4 {
    pub const FACTOR: usize = 4;

    pub fn reset(&mut self) {
        self.up_first.reset();
        self.up_second.reset();
        self.down_second.reset();
        self.down_first.reset();
    }

    /// Applies `nonlinear` to four intermediate samples and returns one.
    #[inline]
    pub fn process<F>(&mut self, input: f32, mut nonlinear: F) -> f32
    where
        F: FnMut(f32) -> f32,
    {
        let (half_first, half_second) = self.up_first.up(input);
        let (quarter_0, quarter_1) = self.up_second.up(half_first);
        let (quarter_2, quarter_3) = self.up_second.up(half_second);

        let shaped_0 = nonlinear(quarter_0);
        let shaped_1 = nonlinear(quarter_1);
        let shaped_2 = nonlinear(quarter_2);
        let shaped_3 = nonlinear(quarter_3);

        let folded_first = self.down_second.down(shaped_0, shaped_1);
        let folded_second = self.down_second.down(shaped_2, shaped_3);
        self.down_first.down(folded_first, folded_second)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::{TAU, sin};

    /// Correlates two signals over a range of lags and returns the best match,
    /// which lets the round-trip test ignore the filter's group delay.
    fn best_match(reference: &[f32], measured: &[f32], max_lag: usize) -> (usize, f32) {
        let mut best = (0, f32::INFINITY);
        for lag in 0..max_lag {
            let mut error = 0.0_f32;
            let mut count = 0.0_f32;
            for index in 200..reference.len() - max_lag {
                let difference = measured[index + lag] - reference[index];
                error += difference * difference;
                count += 1.0;
            }
            let rms = (error / count).sqrt();
            if rms < best.1 {
                best = (lag, rms);
            }
        }
        best
    }

    #[test]
    fn a_transparent_round_trip_returns_the_signal() {
        let sample_rate = 48_000.0;
        let mut oversampler = Oversampler4::default();
        let mut reference = std::vec::Vec::new();
        let mut measured = std::vec::Vec::new();
        for index in 0..4_096 {
            let input = 0.5 * sin(TAU * 500.0 * index as f32 / sample_rate);
            reference.push(input);
            measured.push(oversampler.process(input, |sample| sample));
        }
        let (lag, rms) = best_match(&reference, &measured, 16);
        assert!(
            rms < 0.01,
            "round trip error {rms} at lag {lag} is too large"
        );
    }

    #[test]
    fn oversampled_clipping_folds_back_less_than_naive_clipping() {
        // A 7 kHz tone hard-clipped at 48 kHz puts its third harmonic at
        // 21 kHz and its fifth at 35 kHz, which aliases to 13 kHz. Measuring
        // the energy that lands at 13 kHz separates the two approaches.
        let sample_rate = 48_000.0;
        let frequency = 7_000.0;
        let alias = 13_000.0;
        let clip = |sample: f32| crate::math::clamp(sample * 6.0, -1.0, 1.0);

        let mut oversampler = Oversampler4::default();
        let mut naive_alias = 0.0_f32;
        let mut oversampled_alias = 0.0_f32;
        let samples = 8_192;
        for index in 0..samples {
            let phase = TAU * frequency * index as f32 / sample_rate;
            let input = sin(phase);
            let naive = clip(input);
            let oversampled = oversampler.process(input, clip);
            let probe = TAU * alias * index as f32 / sample_rate;
            naive_alias += naive * sin(probe);
            oversampled_alias += oversampled * sin(probe);
        }
        let naive_alias = naive_alias.abs() / samples as f32;
        let oversampled_alias = oversampled_alias.abs() / samples as f32;
        assert!(
            oversampled_alias < naive_alias * 0.5,
            "oversampling did not reduce the alias: {oversampled_alias} vs {naive_alias}"
        );
    }
}
