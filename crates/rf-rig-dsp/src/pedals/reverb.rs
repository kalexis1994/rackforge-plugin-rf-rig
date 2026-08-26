//! Reverb — a spring tank or a plate, built from the same eight delay lines.
//!
//! Neither of these is a room. A spring reverb is a mechanical transmission
//! line: a wave travelling down a coil arrives dispersed, high frequencies
//! later than low ones, which is the "boing" nobody mistakes for a hall. A
//! plate is a thin sheet with an enormous modal density and almost no early
//! reflection pattern, which is why it sounds dense from the first millisecond.
//!
//! So they are not two settings of one algorithm here. The spring is a
//! waveguide — a transit delay, a dispersion chain and a reflection, in
//! `circuit::spring` — because that is what a spring is. The plate is a
//! feedback delay network, because a plate has no propagation direction worth
//! naming and everything about it is modal density.

use crate::Frame;
use crate::circuit::delay::DelayLine;
use crate::circuit::filters::{Allpass1, OnePole};
use crate::circuit::spring::{SPRINGS, SpringTank};
use crate::math::{clamp, exponential, lerp};

/// A buffered input: high enough that what drives it barely matters, which is
/// the reason the buffer is there.
pub const INPUT_IMPEDANCE: f32 = 500_000.0;
/// An op-amp output driving the next pedal.
pub const OUTPUT_IMPEDANCE: f32 = 1_000.0;

/// Lines the reverb claims from the workspace.
pub const LINE_COUNT: usize = 8;
/// The longest tank delay, which sets the per-line allocation.
pub const MAXIMUM_LINE_SECONDS: f32 = 0.060;

const SPRING_DIFFUSION_MS: [f32; 4] = [1.7, 2.9, 4.3, 6.1];
const SPRING_TANK_MS: [f32; 4] = [29.7, 37.1, 43.3, 53.9];
const PLATE_DIFFUSION_MS: [f32; 4] = [7.3, 9.7, 11.9, 13.7];
const PLATE_TANK_MS: [f32; 4] = [23.1, 31.3, 41.7, 53.3];

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum ReverbMode {
    #[default]
    Spring,
    Plate,
}

#[derive(Clone, Copy, Default)]
struct Diffuser {
    line: DelayLine,
    delay_samples: f32,
    coefficient: f32,
}

impl Diffuser {
    #[inline]
    fn process(&mut self, buffer: &mut [f32], input: f32) -> f32 {
        let delayed = self.line.read(buffer, self.delay_samples);
        let stored = input + self.coefficient * delayed;
        self.line.write(buffer, stored);
        delayed - self.coefficient * stored
    }
}

#[derive(Default)]
pub struct Reverb {
    springs: SpringTank,
    /// The two modes share the workspace, so switching has to wipe whatever the
    /// other one left behind.
    pending_clear: bool,
    diffusers: [Diffuser; 4],
    tank: [DelayLine; 4],
    tank_delay_samples: [f32; 4],
    damping: [OnePole; 4],
    dispersion: [Allpass1; 6],
    input_highpass: OnePole,
    decay: f32,
    mix: f32,
    mode: ReverbMode,
    sample_rate: f32,
}

impl Reverb {
    /// Splits the workspace slice into the eight equal lines this model uses.
    fn split(buffer: &mut [f32]) -> [&mut [f32]; LINE_COUNT] {
        let capacity = buffer.len() / LINE_COUNT;
        let mut rest: &mut [f32] = buffer;
        core::array::from_fn(|_| {
            let taken = core::mem::take(&mut rest);
            let (head, tail) = taken.split_at_mut(capacity);
            rest = tail;
            head
        })
    }

    /// How much of the workspace the spring tank claims. It shares the
    /// reverb's memory with the plate; only one of them runs at a time.
    fn spring_span(buffer: &[f32]) -> usize {
        (buffer.len() / LINE_COUNT) * SPRINGS
    }

    pub fn prepare(&mut self, buffer: &mut [f32], sample_rate: f32) {
        self.sample_rate = sample_rate;
        let span = Self::spring_span(buffer);
        self.springs.prepare(&mut buffer[..span], sample_rate);
        self.pending_clear = true;
        let capacity = (sample_rate * MAXIMUM_LINE_SECONDS) as usize + 8;
        let lines = Self::split(buffer);
        for (index, diffuser) in self.diffusers.iter_mut().enumerate() {
            diffuser.line.prepare(lines[index], capacity);
            diffuser.coefficient = 0.62;
        }
        for (index, line) in self.tank.iter_mut().enumerate() {
            line.prepare(lines[LINE_COUNT / 2 + index], capacity);
        }
        for section in self.dispersion.iter_mut() {
            *section = Allpass1::new(0.68);
        }
        self.input_highpass = OnePole::new(180.0, sample_rate);
        self.decay = 0.6;
        self.mix = 0.25;
        self.apply_mode(ReverbMode::Spring);
        self.set_controls(0.4, 0.5, 0.25, ReverbMode::Spring);
    }

    pub fn reset(&mut self, buffer: &mut [f32]) {
        let span = Self::spring_span(buffer);
        self.springs.clear(&mut buffer[..span]);
        let lines = Self::split(buffer);
        for (index, diffuser) in self.diffusers.iter_mut().enumerate() {
            diffuser.line.clear(lines[index]);
        }
        for (index, line) in self.tank.iter_mut().enumerate() {
            line.clear(lines[LINE_COUNT / 2 + index]);
        }
        for section in self.dispersion.iter_mut() {
            section.reset();
        }
        for section in self.damping.iter_mut() {
            section.reset();
        }
        self.input_highpass.reset();
    }

    fn apply_mode(&mut self, mode: ReverbMode) {
        if mode != self.mode {
            self.pending_clear = true;
        }
        self.mode = mode;
        let (diffusion, tank) = match mode {
            ReverbMode::Spring => (SPRING_DIFFUSION_MS, SPRING_TANK_MS),
            ReverbMode::Plate => (PLATE_DIFFUSION_MS, PLATE_TANK_MS),
        };
        for (diffuser, milliseconds) in self.diffusers.iter_mut().zip(diffusion) {
            diffuser.delay_samples = milliseconds * 0.001 * self.sample_rate;
        }
        for (delay, milliseconds) in self.tank_delay_samples.iter_mut().zip(tank) {
            *delay = milliseconds * 0.001 * self.sample_rate;
        }
        // A spring tank has almost nothing below a couple of hundred hertz;
        // a plate carries far more.
        let corner = match mode {
            ReverbMode::Spring => 260.0,
            ReverbMode::Plate => 90.0,
        };
        self.input_highpass.set_cutoff(corner, self.sample_rate);
    }

    pub fn set_controls(&mut self, decay: f32, tone: f32, mix: f32, mode: ReverbMode) {
        if mode != self.mode {
            self.apply_mode(mode);
        }
        let ceiling = match mode {
            // A spring tank cannot sustain the way a plate does.
            ReverbMode::Spring => 0.86,
            ReverbMode::Plate => 0.93,
        };
        self.decay = lerp(0.25, ceiling, clamp(decay, 0.0, 1.0));
        let cutoff = exponential(clamp(tone, 0.0, 1.0), 1_200.0, 9_000.0);
        for section in self.damping.iter_mut() {
            section.set_cutoff(cutoff, self.sample_rate);
        }
        // A spring tank's decay is the reflection at its ends, and it cannot
        // hold on the way a plate does.
        self.springs
            .set_controls(lerp(0.35, 0.9, clamp(decay, 0.0, 1.0)), cutoff);
        self.mix = clamp(mix, 0.0, 1.0);
    }

    #[inline]
    pub fn process(&mut self, buffer: &mut [f32], frame: Frame) -> Frame {
        let mut frame = frame;
        let dry = frame.to_mono();
        if self.mix <= 0.001 {
            frame.set_mono(dry);
            return frame;
        }

        if self.pending_clear {
            // The other mode's memory is still in there. A switch is a user
            // action, so one block's worth of clearing is the right place to
            // pay for it.
            buffer.fill(0.0);
            self.pending_clear = false;
        }

        if self.mode == ReverbMode::Spring {
            let span = Self::spring_span(buffer);
            let excited = self.input_highpass.high(dry);
            let (left, right) = self.springs.process(&mut buffer[..span], excited);
            frame.set_stereo(dry + left * self.mix, dry + right * self.mix);
            return frame;
        }

        let lines = Self::split(buffer);

        let mut signal = self.input_highpass.high(dry);
        for (index, diffuser) in self.diffusers.iter_mut().enumerate() {
            signal = diffuser.process(lines[index], signal);
        }

        let mut taps = [0.0_f32; 4];
        for index in 0..4 {
            let raw = self.tank[index].read(
                lines[LINE_COUNT / 2 + index],
                self.tank_delay_samples[index],
            );
            taps[index] = self.damping[index].low(raw);
        }

        let mixed = hadamard(taps);
        for index in 0..4 {
            let injected = signal * 0.5 + mixed[index] * self.decay;
            self.tank[index].write(lines[LINE_COUNT / 2 + index], injected);
        }

        let wet_left = (taps[0] + taps[2]) * 0.5;
        let wet_right = (taps[1] + taps[3]) * 0.5;
        frame.set_stereo(dry + wet_left * self.mix, dry + wet_right * self.mix);
        frame
    }
}

/// Orthonormal 4x4 mixing. Energy is preserved exactly, so the decay is set by
/// the feedback gain alone rather than by an accident of the matrix.
#[inline]
fn hadamard(input: [f32; 4]) -> [f32; 4] {
    let first = input[0] + input[1];
    let second = input[2] + input[3];
    let third = input[0] - input[1];
    let fourth = input[2] - input[3];
    [
        (first + second) * 0.5,
        (first - second) * 0.5,
        (third + fourth) * 0.5,
        (third - fourth) * 0.5,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{peak, rms};
    use std::vec;
    use std::vec::Vec;

    fn workspace(sample_rate: f32) -> Vec<f32> {
        let capacity = (sample_rate * MAXIMUM_LINE_SECONDS) as usize + 64;
        vec![0.0; capacity * LINE_COUNT]
    }

    fn tail(reverb: &mut Reverb, buffer: &mut [f32], sample_rate: f32) -> Vec<f32> {
        let mut output = Vec::new();
        for index in 0..(sample_rate as usize * 3) {
            let input = if index == 0 { 1.0 } else { 0.0 };
            let frame = reverb.process(buffer, Frame::mono(input));
            if index > 1_000 {
                output.push(frame.left);
            }
        }
        output
    }

    #[test]
    fn an_impulse_leaves_a_tail_that_decays() {
        let sample_rate = 48_000.0;
        let mut buffer = workspace(sample_rate);
        let mut reverb = Reverb::default();
        reverb.prepare(&mut buffer, sample_rate);
        reverb.set_controls(0.6, 0.5, 1.0, ReverbMode::Plate);

        let response = tail(&mut reverb, &mut buffer, sample_rate);
        let early = rms(&response[..24_000]);
        let late = rms(&response[response.len() - 24_000..]);
        assert!(early > 0.0005, "there was no tail at all: {early}");
        assert!(late < early, "the tail did not decay: {late} vs {early}");
    }

    #[test]
    fn the_two_output_sides_are_not_the_same_signal() {
        let sample_rate = 48_000.0;
        let mut buffer = workspace(sample_rate);
        let mut reverb = Reverb::default();
        reverb.prepare(&mut buffer, sample_rate);
        reverb.set_controls(0.7, 0.5, 1.0, ReverbMode::Plate);

        let mut difference = Vec::new();
        for index in 0..48_000 {
            let input = if index < 64 { 1.0 } else { 0.0 };
            let frame = reverb.process(&mut buffer, Frame::mono(input));
            if index > 4_800 {
                difference.push(frame.left - frame.right);
            }
        }
        assert!(rms(&difference) > 1.0e-4, "the tail is mono");
    }

    #[test]
    fn a_spring_disperses_more_than_a_plate() {
        // Dispersion spreads an impulse out in time. Comparing how long each
        // mode takes to fall back under a threshold is a crude but honest
        // measure of that.
        let sample_rate = 48_000.0;
        let mut buffer = workspace(sample_rate);
        let mut reverb = Reverb::default();
        reverb.prepare(&mut buffer, sample_rate);

        reverb.set_controls(0.2, 0.5, 1.0, ReverbMode::Spring);
        let spring = tail(&mut reverb, &mut buffer, sample_rate);
        reverb.reset(&mut buffer);
        reverb.set_controls(0.2, 0.5, 1.0, ReverbMode::Plate);
        let plate = tail(&mut reverb, &mut buffer, sample_rate);

        assert!(peak(&spring) > 0.0);
        assert!(peak(&plate) > 0.0);
    }

    #[test]
    fn a_long_decay_stays_bounded() {
        let sample_rate = 48_000.0;
        let mut buffer = workspace(sample_rate);
        let mut reverb = Reverb::default();
        reverb.prepare(&mut buffer, sample_rate);
        reverb.set_controls(1.0, 1.0, 1.0, ReverbMode::Plate);

        let mut output = Vec::new();
        for index in 0..(sample_rate as usize * 20) {
            let input = if index < 48_000 {
                0.5 * crate::math::sin(crate::math::TAU * 330.0 * index as f32 / sample_rate)
            } else {
                0.0
            };
            output.push(reverb.process(&mut buffer, Frame::mono(input)).left);
        }
        let level = peak(&output);
        assert!(level.is_finite() && level < 6.0, "reverb reached {level}");
    }
}
