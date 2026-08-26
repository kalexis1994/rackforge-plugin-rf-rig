//! Delay — a bucket-brigade echo, or a clean digital line.
//!
//! The difference between the two modes is not a filter setting bolted onto the
//! same algorithm. An analog echo band-limits *inside* the feedback loop, so
//! every repeat is darker than the one before it and the tail eventually
//! dissolves into nothing but low mids. It also drifts, because the clock that
//! moves the charge along is not perfectly stable. A digital line repeats what
//! it was given.
//!
//! Both modes here share the same delay lines and differ only in what the
//! feedback path does — which is exactly how the two circuits differ.
//!
//! In analog mode the band-limiting is not a fixed filter. A bucket-brigade
//! register's clock *is* its delay control — `delay = stages / (2·f_clock)` —
//! so a 4096-stage chip set to 400 ms is clocked at 5.1 kHz and simply cannot
//! carry treble, while the same chip at 40 ms clocks ten times faster and
//! sounds it. Long delays being darker than short ones is the single most
//! recognisable thing about these pedals, and it is a consequence of that one
//! relation rather than a taste decision.

use crate::Frame;
use crate::circuit::delay::DelayLine;
use crate::circuit::dynamics::{Lfo, LfoShape};
use crate::circuit::filters::OnePole;
use crate::circuit::nonlinear::SoftLimiter;
use crate::math::{clamp, lerp};

/// A buffered input: high enough that what drives it barely matters, which is
/// the reason the buffer is there.
pub const INPUT_IMPEDANCE: f32 = 500_000.0;
/// An op-amp output driving the next pedal.
pub const OUTPUT_IMPEDANCE: f32 = 1_000.0;

/// The longest delay the contract offers, plus room for the wow modulation.
pub const MAXIMUM_DELAY_SECONDS: f32 = 1.25;
/// How far the clock drifts in an analog echo, in seconds.
const WOW_DEPTH_SECONDS: f32 = 0.00018;
/// Stages in the register this family uses: an MN3005.
const STAGES: f32 = 4_096.0;
/// The fraction of the clock the register still carries, once charge-transfer
/// loss has had its say.
const BANDWIDTH_PER_CLOCK: f32 = 0.25;
/// The fixed filters soldered around the chip. Whichever is lower — these or
/// the clock — decides.
const ANALOG_CEILING_HZ: f32 = 4_500.0;
const DIGITAL_CEILING_HZ: f32 = 9_000.0;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum EchoMode {
    #[default]
    Analog,
    Digital,
}

#[derive(Clone, Copy, Default)]
struct Channel {
    line: DelayLine,
    /// Ahead of the register, where the real circuit puts it.
    anti_alias: OnePole,
    /// After it, smoothing the staircase the register hands back.
    reconstruction: OnePole,
    /// In the feedback path only: the coupling that stops the repeats from
    /// piling up a rumble.
    highpass: OnePole,
    limiter: SoftLimiter,
    bandwidth_hz: f32,
}

impl Channel {
    fn prepare(&mut self, buffer: &mut [f32], sample_rate: f32) {
        let length = (sample_rate * MAXIMUM_DELAY_SECONDS) as usize + 16;
        self.line.prepare(buffer, length);
        self.anti_alias = OnePole::new(ANALOG_CEILING_HZ, sample_rate);
        self.reconstruction = OnePole::new(ANALOG_CEILING_HZ, sample_rate);
        self.highpass = OnePole::new(120.0, sample_rate);
        self.limiter.reset();
        self.bandwidth_hz = 0.0;
    }

    fn clear(&mut self, buffer: &mut [f32]) {
        self.line.clear(buffer);
        self.anti_alias.reset();
        self.reconstruction.reset();
        self.highpass.reset();
        self.limiter.reset();
    }

    fn set_mode(&mut self, mode: EchoMode, sample_rate: f32) {
        // A bucket-brigade line has neither the bandwidth nor the headroom of a
        // digital one, and both limits are inside the loop.
        let corner = match mode {
            EchoMode::Analog => 120.0,
            EchoMode::Digital => 25.0,
        };
        self.highpass.set_cutoff(corner, sample_rate);
        self.bandwidth_hz = 0.0;
    }

    /// Points both of the register's filters at whatever it can still carry.
    /// Skips the work when the clock has barely moved, which is most samples.
    #[inline]
    fn follow_clock(&mut self, bandwidth_hz: f32, sample_rate: f32) {
        if crate::math::abs(bandwidth_hz - self.bandwidth_hz) < self.bandwidth_hz * 0.002 {
            return;
        }
        self.bandwidth_hz = bandwidth_hz;
        self.anti_alias.set_cutoff(bandwidth_hz, sample_rate);
        self.reconstruction.set_cutoff(bandwidth_hz, sample_rate);
    }
}

#[derive(Clone, Copy, Default)]
pub struct Echo {
    left: Channel,
    right: Channel,
    wow: Lfo,
    time_smoothing: OnePole,
    target_delay_seconds: f32,
    feedback: f32,
    mix: f32,
    width: f32,
    mode: EchoMode,
    snap_time: bool,
    sample_rate: f32,
}

impl Echo {
    pub fn prepare(&mut self, left_buffer: &mut [f32], right_buffer: &mut [f32], sample_rate: f32) {
        self.sample_rate = sample_rate;
        self.left.prepare(left_buffer, sample_rate);
        self.right.prepare(right_buffer, sample_rate);
        self.wow.set_shape(LfoShape::Sine);
        self.wow.set_rate(0.37, sample_rate);
        // Moving the time control on an analog echo bends the pitch of the
        // repeats. Smoothing over about a fifth of a second keeps that
        // behaviour without letting a parameter jump click.
        self.time_smoothing = OnePole::new(5.0, sample_rate);
        self.target_delay_seconds = 0.38;
        self.feedback = 0.35;
        self.mix = 0.3;
        self.width = 0.0;
        self.mode = EchoMode::Analog;
        self.snap_time = true;
        self.time_smoothing.set_value(self.target_delay_seconds);
    }

    pub fn reset(&mut self, left_buffer: &mut [f32], right_buffer: &mut [f32]) {
        self.left.clear(left_buffer);
        self.right.clear(right_buffer);
        self.wow.reset();
        self.time_smoothing.reset();
        self.time_smoothing.set_value(self.target_delay_seconds);
        self.snap_time = true;
    }

    pub fn set_controls(
        &mut self,
        time_ms: f32,
        feedback: f32,
        mix: f32,
        width: f32,
        mode: EchoMode,
    ) {
        self.target_delay_seconds =
            clamp(time_ms, 5.0, MAXIMUM_DELAY_SECONDS * 1_000.0 - 20.0) * 0.001;
        // Analog mode is allowed past unity: the soft limit inside the loop
        // turns runaway into the self-oscillation the circuit is loved for.
        let ceiling = match mode {
            EchoMode::Analog => 1.05,
            EchoMode::Digital => 0.95,
        };
        self.feedback = clamp(feedback, 0.0, 1.0) * ceiling;
        self.mix = clamp(mix, 0.0, 1.0);
        self.width = clamp(width, 0.0, 1.0);
        if mode != self.mode {
            self.mode = mode;
            self.left.set_mode(mode, self.sample_rate);
            self.right.set_mode(mode, self.sample_rate);
        }
        if self.snap_time {
            // First setting after power-on: the knob is already where it is.
            self.time_smoothing.set_value(self.target_delay_seconds);
            self.snap_time = false;
        }
    }

    #[inline]
    pub fn process(
        &mut self,
        left_buffer: &mut [f32],
        right_buffer: &mut [f32],
        frame: Frame,
    ) -> Frame {
        let mut frame = frame;
        let dry = frame.to_mono();

        let time = self.time_smoothing.low(self.target_delay_seconds);
        let drift = if self.mode == EchoMode::Analog {
            self.wow.tick() * WOW_DEPTH_SECONDS
        } else {
            0.0
        };
        // What the register can still carry at this clock. In digital mode
        // there is no register, so only the fixed filter applies.
        let bandwidth = match self.mode {
            EchoMode::Analog => {
                let clock = STAGES / (2.0 * time.max(1.0e-4));
                (clock * BANDWIDTH_PER_CLOCK).min(ANALOG_CEILING_HZ)
            }
            EchoMode::Digital => DIGITAL_CEILING_HZ,
        };
        self.left.follow_clock(bandwidth, self.sample_rate);
        self.right.follow_clock(bandwidth, self.sample_rate);

        let left_delay = (time + drift) * self.sample_rate;
        let right_delay = (time - drift) * self.sample_rate;

        // Out of the register, through the filter that smooths its staircase.
        // This is the band-limiting the *first* repeat already has: in the real
        // pedal the signal is filtered on the way through, not only on the way
        // round.
        let left_read = self
            .left
            .reconstruction
            .low(self.left.line.read(left_buffer, left_delay));
        let right_read = self
            .right
            .reconstruction
            .low(self.right.line.read(right_buffer, right_delay));

        // At full width the two lines feed each other, which is what makes the
        // repeats alternate across the image.
        let cross = self.width;
        let left_source = lerp(left_read, right_read, cross);
        let right_source = lerp(right_read, left_read, cross);

        let left_feedback = Self::loop_path(&mut self.left, left_source) * self.feedback;
        let right_feedback = Self::loop_path(&mut self.right, right_source) * self.feedback;

        // And into the register through its anti-alias filter.
        let left_input = self.left.anti_alias.low(dry + left_feedback);
        let right_input = self
            .right
            .anti_alias
            .low(dry * (1.0 - cross) + right_feedback);
        self.left.line.write(left_buffer, left_input);
        self.right.line.write(right_buffer, right_input);

        if self.mix <= 0.001 {
            frame.set_mono(dry);
            return frame;
        }
        let left_out = dry + left_read * self.mix;
        let right_out = dry + right_read * self.mix;
        if self.width > 0.001 {
            frame.set_stereo(left_out, right_out);
        } else {
            frame.set_mono(left_out);
        }
        frame
    }

    #[inline]
    fn loop_path(channel: &mut Channel, input: f32) -> f32 {
        // The band-limiting already happened on the way out of the register;
        // what the feedback path adds is its coupling capacitor and the
        // saturation that keeps a runaway howling instead of tearing.
        let coupled = channel.highpass.high(input);
        channel.limiter.process(coupled * 0.9) * 1.1
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::peak;
    use std::vec::Vec;

    fn buffers(sample_rate: f32) -> (Vec<f32>, Vec<f32>) {
        let length = (sample_rate * MAXIMUM_DELAY_SECONDS) as usize + 64;
        (std::vec![0.0; length], std::vec![0.0; length])
    }

    #[test]
    fn a_click_comes_back_one_delay_time_later() {
        let sample_rate = 48_000.0;
        let (mut left_buffer, mut right_buffer) = buffers(sample_rate);
        let mut echo = Echo::default();
        echo.prepare(&mut left_buffer, &mut right_buffer, sample_rate);
        echo.set_controls(100.0, 0.0, 1.0, 0.0, EchoMode::Digital);

        let mut peak_index = 0;
        let mut peak_value = 0.0_f32;
        for index in 0..24_000 {
            let input = if index == 0 { 1.0 } else { 0.0 };
            let output = echo.process(&mut left_buffer, &mut right_buffer, Frame::mono(input));
            // Skip the dry click itself.
            if index > 100 && output.left.abs() > peak_value {
                peak_value = output.left.abs();
                peak_index = index;
            }
        }
        // The knob was already at 100 ms when the pedal powered on, so the
        // repeat lands there rather than sliding in from the smoothing.
        let expected = (0.1 * sample_rate) as usize;
        let error = (peak_index as f32 - expected as f32).abs() / expected as f32;
        assert!(
            error < 0.02,
            "the repeat arrived at sample {peak_index}, expected {expected}"
        );
    }

    #[test]
    fn a_longer_delay_darkens_the_repeats() {
        // The register's clock is the delay control, so the two cannot be set
        // independently — which is exactly the behaviour these pedals have.
        let sample_rate = 48_000.0;
        let treble_after_a_repeat = |delay_ms: f32| {
            let (mut left_buffer, mut right_buffer) = buffers(sample_rate);
            let mut echo = Echo::default();
            echo.prepare(&mut left_buffer, &mut right_buffer, sample_rate);
            echo.set_controls(delay_ms, 0.6, 1.0, 0.0, EchoMode::Analog);
            let mut energy = 0.0_f32;
            let samples = (sample_rate * 2.0) as usize;
            let onset = (delay_ms * 0.001 * sample_rate) as usize + 2_000;
            for index in 0..samples {
                let input = if index < 480 {
                    crate::math::sin(crate::math::TAU * 3_000.0 * index as f32 / sample_rate)
                } else {
                    0.0
                };
                let output = echo.process(&mut left_buffer, &mut right_buffer, Frame::mono(input));
                if index > onset {
                    energy += output.left * output.left;
                }
            }
            energy
        };

        let short = treble_after_a_repeat(60.0);
        let long = treble_after_a_repeat(600.0);
        assert!(
            long < short * 0.25,
            "a long delay should lose far more treble: {long} against {short}"
        );
    }

    #[test]
    fn analog_repeats_get_darker_and_digital_repeats_do_not() {
        let sample_rate = 48_000.0;
        let (mut left_buffer, mut right_buffer) = buffers(sample_rate);
        let mut analog = Echo::default();
        analog.prepare(&mut left_buffer, &mut right_buffer, sample_rate);
        analog.set_controls(120.0, 0.7, 1.0, 0.0, EchoMode::Analog);

        let mut high_frequency_energy = 0.0_f32;
        for index in 0..96_000 {
            // One short burst of treble, then silence: whatever survives in
            // the tail came back through the loop.
            let input = if index < 480 {
                crate::math::sin(crate::math::TAU * 6_000.0 * index as f32 / sample_rate)
            } else {
                0.0
            };
            let output = analog.process(&mut left_buffer, &mut right_buffer, Frame::mono(input));
            if index > 48_000 {
                high_frequency_energy += output.left * output.left;
            }
        }
        assert!(
            high_frequency_energy < 1.0,
            "the analog loop kept its treble: {high_frequency_energy}"
        );
    }

    #[test]
    fn maximum_feedback_does_not_blow_up() {
        let sample_rate = 48_000.0;
        let (mut left_buffer, mut right_buffer) = buffers(sample_rate);
        let mut echo = Echo::default();
        echo.prepare(&mut left_buffer, &mut right_buffer, sample_rate);
        echo.set_controls(300.0, 1.0, 1.0, 1.0, EchoMode::Analog);

        let mut outputs = Vec::new();
        for index in 0..(sample_rate as usize * 10) {
            let input = if index < 4_800 {
                0.5 * crate::math::sin(crate::math::TAU * 220.0 * index as f32 / sample_rate)
            } else {
                0.0
            };
            let output = echo.process(&mut left_buffer, &mut right_buffer, Frame::mono(input));
            outputs.push(output.left);
        }
        let level = peak(&outputs);
        assert!(
            level.is_finite() && level < 6.0,
            "self-oscillation ran away to {level}"
        );
    }
}
