//! A spring tank, modelled as the dispersive transmission line it is.
//!
//! A spring reverb is not a small room. It is two or three helical springs with
//! a transducer at one end and a pickup at the other, and what makes it
//! recognisable in one note is that a spring is **dispersive**: the speed of a
//! wave along it depends on frequency. High frequencies arrive first, low ones
//! trail behind, and a single impulse comes back as a descending chirp — the
//! "boing" that nobody mistakes for a hall.
//!
//! ```text
//!   in ──▶ transducer ──▶ [ delay · dispersion · loss ] ──┬──▶ pickup ──▶ out
//!                              ▲                          │
//!                              └────── reflection ────────┘
//! ```
//!
//! The wave does not stop at the far end; it reflects and travels back, so the
//! tank rings and each pass adds another chirp on top of the last. That is the
//! whole structure: a delay for the transit time, a dispersion section, a loss,
//! and a loop.
//!
//! ## Where the dispersion comes from
//!
//! Group delay that *falls* with frequency is what a cascade of first-order
//! all-pass sections gives, provided the coefficient has the right sign. For
//! `H(z) = (a + z⁻¹)/(1 + a z⁻¹)` the group delay is
//!
//! ```text
//! τ(ω) = (1 − a²) / (1 + 2a·cos ω + a²)
//! ```
//!
//! which at direct current is `(1−a)/(1+a)` and at the Nyquist rate
//! `(1+a)/(1−a)`. A *positive* coefficient therefore delays the top, which is
//! backwards for a spring — and worse, it delays exactly the part the tank's
//! own damping then removes, so the chirp collapses into a click. That is what
//! the first version of this did, and what the tests caught. With a negative
//! coefficient the low end trails, and a chain of `K` sections spreads an
//! impulse over
//!
//! ```text
//! spread ≈ K · [ (1−a)/(1+a) − (1+a)/(1−a) ]   samples
//! ```
//!
//! and that spread is the length of the chirp. A real tank chirps over roughly
//! twenty to forty milliseconds, which is what the section count and
//! coefficient here are chosen to give — stated as a target rather than
//! derived from a wire diameter, because that is the honest description of what
//! is known here. Everything else — the direction of the sweep, the way each
//! reflection adds another one, the spacing of the arrivals — is structural.
//!
//! This is the established way to model these tanks in a waveguide: see
//! `docs/REFERENCES.md`.

use crate::circuit::delay::DelayLine;
use crate::circuit::filters::{Allpass1, OnePole};
use crate::math::{clamp, sanitise};

/// All-pass sections per spring.
///
/// The count and the coefficient below are chosen *together*: what they have to
/// produce is a chirp of the right length, and there are many pairs that do.
/// Two hundred gentle sections and ninety steep ones both spread an impulse
/// over about twenty-five milliseconds at 48 kHz, but the first costs twice the
/// arithmetic — measured, 9.6 % of a core against 3.7 %.
///
/// What the pair does *not* agree on is the shape of the sweep between the two
/// ends. A real tank's dispersion curve would settle both numbers instead of
/// one; measuring one is the next step, and it is recorded as such in
/// `docs/IMPLEMENTATION_PLAN.md`.
pub const SECTIONS: usize = 90;
/// Springs in the tank. Real ones use two or three of slightly different
/// lengths, which is why the arrivals do not line up into a single comb.
pub const SPRINGS: usize = 3;
/// The all-pass coefficient. Negative on purpose: it is the sign that decides
/// which end of the spectrum trails, and a spring's low end trails.
const DISPERSION: f32 = -0.86;
/// One-way transit times, in seconds. Different per spring, as in the tank.
const TRANSIT_SECONDS: [f32; SPRINGS] = [0.0293, 0.0347, 0.0411];
/// How much of each spring reaches the pickup.
const PICKUP: [f32; SPRINGS] = [0.42, 0.36, 0.30];

/// One spring: a delay, a dispersion chain, a loss, and the reflection that
/// sends the wave back down it.
struct Spring {
    line: DelayLine,
    sections: [Allpass1; SECTIONS],
    damping: OnePole,
    transit_samples: f32,
}

impl Default for Spring {
    fn default() -> Self {
        Self {
            line: DelayLine::default(),
            sections: [Allpass1::new(DISPERSION); SECTIONS],
            damping: OnePole::default(),
            transit_samples: 0.0,
        }
    }
}

impl Spring {
    fn prepare(&mut self, buffer: &mut [f32], transit_seconds: f32, sample_rate: f32) {
        let length = (transit_seconds * sample_rate) as usize + 32;
        self.line.prepare(buffer, length.min(buffer.len()));
        self.transit_samples = transit_seconds * sample_rate;
        for section in self.sections.iter_mut() {
            *section = Allpass1::new(DISPERSION);
        }
        self.damping = OnePole::new(3_000.0, sample_rate);
    }

    fn clear(&mut self, buffer: &mut [f32]) {
        self.line.clear(buffer);
        for section in self.sections.iter_mut() {
            section.reset();
        }
        self.damping.reset();
    }

    fn set_damping(&mut self, cutoff_hz: f32, sample_rate: f32) {
        self.damping.set_cutoff(cutoff_hz, sample_rate);
    }

    /// One sample down the spring and back.
    #[inline]
    fn process(&mut self, buffer: &mut [f32], input: f32, reflection: f32) -> f32 {
        let travelled = self.line.read(buffer, self.transit_samples);
        let mut dispersed = travelled;
        for section in self.sections.iter_mut() {
            dispersed = section.process(dispersed);
        }
        let returned = self.damping.low(dispersed);
        self.line
            .write(buffer, sanitise(input + returned * reflection));
        returned
    }
}

/// The tank.
pub struct SpringTank {
    springs: [Spring; SPRINGS],
    reflection: f32,
    sample_rate: f32,
}

impl Default for SpringTank {
    fn default() -> Self {
        Self {
            springs: [Spring::default(), Spring::default(), Spring::default()],
            reflection: 0.7,
            sample_rate: 48_000.0,
        }
    }
}

impl SpringTank {
    /// The longest one-way transit the tank needs to hold, in seconds.
    pub const LONGEST_TRANSIT_SECONDS: f32 = 0.045;

    /// Splits `buffer` between the springs and sets them up. The buffer must
    /// hold `SPRINGS` lines of the longest transit.
    pub fn prepare(&mut self, buffer: &mut [f32], sample_rate: f32) {
        self.sample_rate = sample_rate;
        let capacity = buffer.len() / SPRINGS;
        let mut rest: &mut [f32] = buffer;
        for (index, spring) in self.springs.iter_mut().enumerate() {
            let taken = core::mem::take(&mut rest);
            let (head, tail) = taken.split_at_mut(capacity);
            rest = tail;
            spring.prepare(head, TRANSIT_SECONDS[index], sample_rate);
        }
    }

    pub fn clear(&mut self, buffer: &mut [f32]) {
        let capacity = buffer.len() / SPRINGS;
        let mut rest: &mut [f32] = buffer;
        for spring in self.springs.iter_mut() {
            let taken = core::mem::take(&mut rest);
            let (head, tail) = taken.split_at_mut(capacity);
            rest = tail;
            spring.clear(head);
        }
    }

    /// `decay` is the reflection coefficient at the ends; `damping_hz` is where
    /// the spring stops carrying treble on each pass.
    pub fn set_controls(&mut self, decay: f32, damping_hz: f32) {
        self.reflection = clamp(decay, 0.0, 0.96);
        for spring in self.springs.iter_mut() {
            spring.set_damping(damping_hz, self.sample_rate);
        }
    }

    /// How long an impulse is spread by one pass through the dispersion, in
    /// seconds — the length of one chirp.
    pub fn chirp_seconds(&self) -> f32 {
        let at_dc = (1.0 - DISPERSION) / (1.0 + DISPERSION);
        let at_nyquist = (1.0 + DISPERSION) / (1.0 - DISPERSION);
        SECTIONS as f32 * crate::math::abs(at_dc - at_nyquist) / self.sample_rate
    }

    /// The one-way transit of each spring, in seconds.
    pub fn transits(&self) -> [f32; SPRINGS] {
        TRANSIT_SECONDS
    }

    #[inline]
    pub fn process(&mut self, buffer: &mut [f32], input: f32) -> (f32, f32) {
        let capacity = buffer.len() / SPRINGS;
        let mut rest: &mut [f32] = buffer;
        let mut left = 0.0;
        let mut right = 0.0;
        for (index, spring) in self.springs.iter_mut().enumerate() {
            let taken = core::mem::take(&mut rest);
            let (head, tail) = taken.split_at_mut(capacity);
            rest = tail;
            let value = spring.process(head, input, self.reflection) * PICKUP[index];
            // The springs sit side by side over the pickup; alternating which
            // side each favours is what gives the tank its width.
            if index % 2 == 0 {
                left += value;
                right += value * 0.6;
            } else {
                right += value;
                left += value * 0.6;
            }
        }
        (left, right)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::circuit::filters::OnePole;
    use std::vec;
    use std::vec::Vec;

    const SAMPLE_RATE: f32 = 48_000.0;

    fn workspace() -> Vec<f32> {
        let capacity = (SAMPLE_RATE * SpringTank::LONGEST_TRANSIT_SECONDS) as usize + 64;
        vec![0.0; capacity * SPRINGS]
    }

    fn impulse_response(tank: &mut SpringTank, buffer: &mut [f32], samples: usize) -> Vec<f32> {
        (0..samples)
            .map(|index| {
                let input = if index == 0 { 1.0 } else { 0.0 };
                tank.process(buffer, input).0
            })
            .collect()
    }

    /// Where a band's energy peaks, in samples.
    fn arrival(response: &[f32], centre_hz: f32, width: f32) -> usize {
        let mut low = OnePole::new(centre_hz * width, SAMPLE_RATE);
        let mut high = OnePole::new(centre_hz / width, SAMPLE_RATE);
        let mut follower = OnePole::new(80.0, SAMPLE_RATE);
        let mut best = (0_usize, 0.0_f32);
        for (index, sample) in response.iter().enumerate() {
            let banded = low.low(*sample) - high.low(*sample);
            let envelope = follower.low(banded.abs());
            if envelope > best.1 {
                best = (index, envelope);
            }
        }
        best.0
    }

    #[test]
    fn the_chirp_is_as_long_as_the_dispersion_makes_it() {
        let tank = SpringTank::default();
        let chirp = tank.chirp_seconds();
        assert!(
            (0.020..0.040).contains(&chirp),
            "a tank should chirp over twenty to forty milliseconds, this one over {chirp} s"
        );
    }

    #[test]
    fn treble_arrives_before_bass() {
        // The property that makes a spring a spring: group delay falls with
        // frequency, so one impulse comes back as a descending sweep.
        let mut buffer = workspace();
        let mut tank = SpringTank::default();
        tank.prepare(&mut buffer, SAMPLE_RATE);
        tank.set_controls(0.5, 3_000.0);
        let response = impulse_response(&mut tank, &mut buffer, 12_000);

        let treble = arrival(&response, 2_500.0, 1.6);
        let bass = arrival(&response, 400.0, 1.6);
        assert!(
            treble + 200 < bass,
            "the top should arrive first: treble at {treble}, bass at {bass}"
        );
    }

    #[test]
    fn an_impulse_comes_back_spread_out_rather_than_as_a_click() {
        let mut buffer = workspace();
        let mut tank = SpringTank::default();
        tank.prepare(&mut buffer, SAMPLE_RATE);
        tank.set_controls(0.4, 3_000.0);
        let response = impulse_response(&mut tank, &mut buffer, 8_000);

        // Nothing should arrive before the shortest spring's transit.
        let earliest = (TRANSIT_SECONDS[0] * SAMPLE_RATE) as usize;
        let before: f32 = response[..earliest - 100]
            .iter()
            .map(|sample| sample.abs())
            .sum();
        assert!(before < 1.0e-4, "something arrived early: {before}");

        // And the energy after it should be spread over milliseconds, not
        // concentrated in a handful of samples.
        let peak = crate::testing::peak(&response);
        let loud = response
            .iter()
            .filter(|sample| sample.abs() > peak * 0.1)
            .count();
        assert!(
            loud > 200,
            "the response is a click, not a chirp: {loud} samples above a tenth of the peak"
        );
    }

    #[test]
    fn the_tank_rings_and_then_stops() {
        let mut buffer = workspace();
        let mut tank = SpringTank::default();
        tank.prepare(&mut buffer, SAMPLE_RATE);
        tank.set_controls(0.85, 3_000.0);
        let response = impulse_response(&mut tank, &mut buffer, 48_000 * 3);

        let early = crate::testing::rms(&response[4_800..24_000]);
        let late = crate::testing::rms(&response[response.len() - 24_000..]);
        assert!(early > 1.0e-4, "the tank did not ring: {early}");
        assert!(
            late < early * 0.5,
            "it never decayed: {late} against {early}"
        );
    }

    #[test]
    fn a_long_decay_stays_finite() {
        let mut buffer = workspace();
        let mut tank = SpringTank::default();
        tank.prepare(&mut buffer, SAMPLE_RATE);
        tank.set_controls(1.0, 6_000.0);
        let mut worst = 0.0_f32;
        for index in 0..(48_000 * 10) {
            let input = if index < 4_800 {
                crate::math::sin(crate::math::TAU * 330.0 * index as f32 / SAMPLE_RATE)
            } else {
                0.0
            };
            let (left, right) = tank.process(&mut buffer, input);
            assert!(left.is_finite() && right.is_finite());
            worst = worst.max(left.abs());
        }
        assert!(worst < 8.0, "the tank ran away to {worst}");
    }

    #[test]
    fn the_two_sides_are_not_the_same_signal() {
        let mut buffer = workspace();
        let mut tank = SpringTank::default();
        tank.prepare(&mut buffer, SAMPLE_RATE);
        tank.set_controls(0.7, 3_000.0);
        let mut difference = Vec::new();
        for index in 0..24_000 {
            let input = if index == 0 { 1.0 } else { 0.0 };
            let (left, right) = tank.process(&mut buffer, input);
            difference.push(left - right);
        }
        assert!(
            crate::testing::rms(&difference) > 1.0e-4,
            "the tank is mono"
        );
    }
}
