//! RF-Rig: a pedalboard modelled at circuit level.
//!
//! The crate is organised the way the signal runs:
//!
//! * [`circuit`] holds the parts — RC sections, the diode equation, delay
//!   lines, companders, detectors, oversampling;
//! * [`pedals`] wires those parts into one circuit family per module;
//! * [`Engine`] is the board, cabling the pedals in the order the parameters
//!   describe and handing the host a stereo pair.
//!
//! Two rules hold everywhere. Nothing allocates after activation — the delay
//! memory arrives as one borrowed block, see [`workspace`]. And every constant
//! that shapes the sound should be traceable to a component value or a
//! datasheet number, so that improving the model means correcting a
//! measurement rather than nudging a taste knob.

#![no_std]

#[cfg(test)]
extern crate std;

pub mod circuit;
pub mod engine;
pub mod math;
pub mod pedals;
pub mod workspace;

#[cfg(test)]
mod testing;

pub use engine::{Engine, STATE_BYTES};
pub use workspace::{WORKSPACE_SAMPLES, Workspace, take_static_workspace};

/// One instant of the board's signal.
///
/// `stereo` records whether the two sides are genuinely different yet. Until
/// some pedal widens the signal the chain carries one channel, which halves the
/// work of every clipper in front of it — and when a mono pedal comes *after* a
/// stereo one, summing back down is what a real one-input pedal does.
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct Frame {
    pub left: f32,
    pub right: f32,
    pub stereo: bool,
}

impl Frame {
    pub const fn mono(value: f32) -> Self {
        Self {
            left: value,
            right: value,
            stereo: false,
        }
    }

    pub const fn silence() -> Self {
        Self::mono(0.0)
    }

    /// Collapses the frame to one channel and returns it.
    #[inline]
    pub fn to_mono(&mut self) -> f32 {
        let value = if self.stereo {
            0.5 * (self.left + self.right)
        } else {
            self.left
        };
        self.left = value;
        self.right = value;
        self.stereo = false;
        value
    }

    #[inline]
    pub fn set_mono(&mut self, value: f32) {
        self.left = value;
        self.right = value;
        self.stereo = false;
    }

    #[inline]
    pub fn set_stereo(&mut self, left: f32, right: f32) {
        self.left = left;
        self.right = right;
        self.stereo = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mono_frame_carries_the_same_value_on_both_sides() {
        let frame = Frame::mono(0.25);
        assert_eq!(frame.left, 0.25);
        assert_eq!(frame.right, 0.25);
        assert!(!frame.stereo);
    }

    #[test]
    fn collapsing_a_stereo_frame_averages_it() {
        let mut frame = Frame::default();
        frame.set_stereo(1.0, 0.0);
        assert_eq!(frame.to_mono(), 0.5);
        assert!(!frame.stereo);
        assert_eq!(frame.left, frame.right);
    }
}
