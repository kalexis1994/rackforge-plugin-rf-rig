//! Level detection and modulation sources.

use crate::math::{TAU, abs, clamp, exp, sanitise, sin};

/// A rectifier followed by an RC network: the detector every analog compressor
/// uses to decide how hard to squeeze.
#[derive(Clone, Copy, Default)]
pub struct EnvelopeFollower {
    attack: f32,
    release: f32,
    envelope: f32,
}

impl EnvelopeFollower {
    pub fn new(attack_ms: f32, release_ms: f32, sample_rate: f32) -> Self {
        let mut follower = Self::default();
        follower.set_times(attack_ms, release_ms, sample_rate);
        follower
    }

    pub fn set_times(&mut self, attack_ms: f32, release_ms: f32, sample_rate: f32) {
        self.attack = Self::coefficient(attack_ms, sample_rate);
        self.release = Self::coefficient(release_ms, sample_rate);
    }

    fn coefficient(milliseconds: f32, sample_rate: f32) -> f32 {
        let seconds = clamp(milliseconds, 0.05, 10_000.0) * 0.001;
        exp(-1.0 / (seconds * sample_rate))
    }

    pub fn reset(&mut self) {
        self.envelope = 0.0;
    }

    pub fn value(&self) -> f32 {
        self.envelope
    }

    #[inline]
    pub fn process(&mut self, input: f32) -> f32 {
        let rectified = abs(input);
        let coefficient = if rectified > self.envelope {
            self.attack
        } else {
            self.release
        };
        self.envelope = sanitise(rectified + coefficient * (self.envelope - rectified));
        self.envelope
    }
}

/// A downward gate with a simple hysteresis window. Pedal chains with a fuzz in
/// them amplify the noise floor of everything ahead of them; this is the same
/// utility every player puts at the front of the board.
#[derive(Clone, Copy, Default)]
pub struct NoiseGate {
    follower: EnvelopeFollower,
    gain: f32,
    open_threshold: f32,
    close_threshold: f32,
    smoothing: f32,
}

impl NoiseGate {
    pub fn prepare(&mut self, sample_rate: f32) {
        self.follower = EnvelopeFollower::new(1.0, 80.0, sample_rate);
        self.gain = 1.0;
        self.smoothing = exp(-1.0 / (0.005 * sample_rate));
    }

    /// A threshold at or below -89 dBFS means the player left the gate off.
    pub fn set_threshold_db(&mut self, threshold_db: f32) {
        if threshold_db <= -89.0 {
            self.open_threshold = 0.0;
            self.close_threshold = 0.0;
            return;
        }
        self.open_threshold = crate::math::db_to_gain(threshold_db);
        self.close_threshold = crate::math::db_to_gain(threshold_db - 6.0);
    }

    pub fn reset(&mut self) {
        self.follower.reset();
        self.gain = 1.0;
    }

    #[inline]
    pub fn process(&mut self, input: f32) -> f32 {
        if self.open_threshold <= 0.0 {
            return input;
        }
        let envelope = self.follower.process(input);
        let target = if envelope > self.open_threshold {
            1.0
        } else if envelope < self.close_threshold {
            0.0
        } else {
            self.gain
        };
        self.gain = sanitise(target + self.smoothing * (self.gain - target));
        input * self.gain
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LfoShape {
    Sine,
    /// The asymmetric ramp an op-amp triangle generator actually produces, and
    /// what most analog modulation sweeps with.
    Triangle,
}

/// Low-frequency oscillator with a quadrature output, so a mono pedal can put
/// two different sweeps into a stereo pair without a second oscillator.
#[derive(Clone, Copy)]
pub struct Lfo {
    phase: f32,
    increment: f32,
    shape: LfoShape,
}

impl Default for Lfo {
    fn default() -> Self {
        Self {
            phase: 0.0,
            increment: 0.0,
            shape: LfoShape::Triangle,
        }
    }
}

impl Lfo {
    pub fn set_shape(&mut self, shape: LfoShape) {
        self.shape = shape;
    }

    pub fn set_rate(&mut self, rate_hz: f32, sample_rate: f32) {
        self.increment = clamp(rate_hz, 0.01, 40.0) / sample_rate;
    }

    pub fn reset(&mut self) {
        self.phase = 0.0;
    }

    /// Advances one sample and returns the sweep in -1..1.
    #[inline]
    pub fn tick(&mut self) -> f32 {
        self.phase += self.increment;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
        }
        self.value_at(self.phase)
    }

    /// The same sweep a quarter cycle later.
    #[inline]
    pub fn quadrature(&self) -> f32 {
        let mut phase = self.phase + 0.25;
        if phase >= 1.0 {
            phase -= 1.0;
        }
        self.value_at(phase)
    }

    #[inline]
    fn value_at(&self, phase: f32) -> f32 {
        match self.shape {
            LfoShape::Sine => sin(TAU * phase),
            LfoShape::Triangle => {
                if phase < 0.5 {
                    phase * 4.0 - 1.0
                } else {
                    3.0 - phase * 4.0
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_follower_rises_fast_and_falls_slowly() {
        let sample_rate = 48_000.0;
        let mut follower = EnvelopeFollower::new(1.0, 200.0, sample_rate);
        for _ in 0..(sample_rate as usize / 100) {
            follower.process(1.0);
        }
        let attacked = follower.value();
        assert!(attacked > 0.9, "attack only reached {attacked}");
        for _ in 0..(sample_rate as usize / 100) {
            follower.process(0.0);
        }
        let released = follower.value();
        assert!(
            released > 0.5,
            "release was far too quick, fell to {released}"
        );
    }

    #[test]
    fn a_disabled_gate_is_transparent() {
        let mut gate = NoiseGate::default();
        gate.prepare(48_000.0);
        gate.set_threshold_db(-90.0);
        assert_eq!(gate.process(0.25), 0.25);
    }

    #[test]
    fn a_gate_closes_on_a_quiet_signal_and_opens_on_a_loud_one() {
        let sample_rate = 48_000.0;
        let mut gate = NoiseGate::default();
        gate.prepare(sample_rate);
        gate.set_threshold_db(-40.0);
        let mut quiet = 0.0_f32;
        for index in 0..24_000 {
            let input = 0.001 * sin(TAU * 220.0 * index as f32 / sample_rate);
            let output = gate.process(input);
            if index > 12_000 {
                quiet = quiet.max(output.abs());
            }
        }
        assert!(quiet < 0.0005, "the gate stayed open at {quiet}");

        let mut loud = 0.0_f32;
        for index in 0..24_000 {
            let input = 0.5 * sin(TAU * 220.0 * index as f32 / sample_rate);
            let output = gate.process(input);
            if index > 12_000 {
                loud = loud.max(output.abs());
            }
        }
        assert!(loud > 0.4, "the gate never opened, peak {loud}");
    }

    #[test]
    fn the_lfo_stays_inside_its_range_and_its_quadrature_leads_it() {
        let mut lfo = Lfo::default();
        lfo.set_rate(2.0, 48_000.0);
        let mut sum = 0.0_f32;
        for _ in 0..48_000 {
            let value = lfo.tick();
            assert!((-1.0..=1.0).contains(&value));
            assert!((-1.0..=1.0).contains(&lfo.quadrature()));
            sum += value;
        }
        assert!(
            (sum / 48_000.0).abs() < 0.01,
            "the sweep is not centred: {sum}"
        );
    }
}
