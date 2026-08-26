//! Where the delay memory comes from.
//!
//! RF-Rig allocates nothing. Every delay line, tank and register borrows a
//! slice of one block that the plugin hands over at activation, sized for the
//! highest sample rate the plugin accepts. Two consequences are worth stating
//! plainly:
//!
//! * the audio callback cannot allocate, block or fail on memory, which is the
//!   host's first rule for a real-time plugin;
//! * the block is a `static` in the plugin crate rather than a field of the
//!   processor, because building a megabyte of buffers on the WebAssembly
//!   shadow stack is how a component silently stops instantiating.

use core::sync::atomic::{AtomicBool, Ordering};

/// The highest sample rate the buffers are sized for. Activation above this is
/// refused rather than silently shortening every delay.
pub const MAXIMUM_SAMPLE_RATE: f32 = 96_000.0;

/// 20 ms at 96 kHz, rounded up: the chorus register.
pub const CHORUS_SAMPLES: usize = 2_048;
/// 1.25 s at 96 kHz: one channel of the echo.
pub const ECHO_SAMPLES: usize = 120_064;
/// 60 ms at 96 kHz: one reverb line, of which there are eight.
pub const REVERB_LINE_SAMPLES: usize = 5_824;
pub const REVERB_SAMPLES: usize = REVERB_LINE_SAMPLES * 8;

pub const WORKSPACE_SAMPLES: usize = CHORUS_SAMPLES + ECHO_SAMPLES * 2 + REVERB_SAMPLES;

/// The block, divided among the pedals that need memory.
pub struct Workspace<'a> {
    pub chorus: &'a mut [f32],
    pub echo_left: &'a mut [f32],
    pub echo_right: &'a mut [f32],
    pub reverb: &'a mut [f32],
}

impl<'a> Workspace<'a> {
    /// Divides `buffer`, or returns `None` when it is too small to hold the
    /// declared maximum delays.
    pub fn new(buffer: &'a mut [f32]) -> Option<Self> {
        if buffer.len() < WORKSPACE_SAMPLES {
            return None;
        }
        let (chorus, rest) = buffer.split_at_mut(CHORUS_SAMPLES);
        let (echo_left, rest) = rest.split_at_mut(ECHO_SAMPLES);
        let (echo_right, rest) = rest.split_at_mut(ECHO_SAMPLES);
        let (reverb, _) = rest.split_at_mut(REVERB_SAMPLES);
        Some(Self {
            chorus,
            echo_left,
            echo_right,
            reverb,
        })
    }

    pub fn clear(&mut self) {
        self.chorus.fill(0.0);
        self.echo_left.fill(0.0);
        self.echo_right.fill(0.0);
        self.reverb.fill(0.0);
    }
}

static mut STATIC_WORKSPACE: [f32; WORKSPACE_SAMPLES] = [0.0; WORKSPACE_SAMPLES];
static STATIC_WORKSPACE_TAKEN: AtomicBool = AtomicBool::new(false);

/// Hands out the process-wide block exactly once.
///
/// Each plugin instance is a separate WebAssembly instance with its own linear
/// memory, so "once per process" is once per instance there. A native test or
/// the lab tool that wants several engines passes its own buffer to
/// [`Workspace::new`] instead.
pub fn take_static_workspace() -> Option<&'static mut [f32]> {
    if STATIC_WORKSPACE_TAKEN.swap(true, Ordering::AcqRel) {
        return None;
    }
    // SAFETY: the swap above guarantees this is the only caller that reaches
    // this line, so the `&'static mut` it produces is unique.
    unsafe { Some(&mut *core::ptr::addr_of_mut!(STATIC_WORKSPACE)) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::vec;

    #[test]
    fn a_buffer_of_the_declared_size_divides_completely() {
        let mut buffer = vec![0.0_f32; WORKSPACE_SAMPLES];
        let workspace = Workspace::new(&mut buffer).expect("the exact size is enough");
        assert_eq!(workspace.chorus.len(), CHORUS_SAMPLES);
        assert_eq!(workspace.echo_left.len(), ECHO_SAMPLES);
        assert_eq!(workspace.echo_right.len(), ECHO_SAMPLES);
        assert_eq!(workspace.reverb.len(), REVERB_SAMPLES);
    }

    #[test]
    fn a_short_buffer_is_refused() {
        let mut buffer = vec![0.0_f32; WORKSPACE_SAMPLES - 1];
        assert!(Workspace::new(&mut buffer).is_none());
    }

    #[test]
    fn the_lines_are_long_enough_for_the_declared_maximum_delays() {
        let longest_echo =
            (MAXIMUM_SAMPLE_RATE * crate::pedals::echo::MAXIMUM_DELAY_SECONDS) as usize;
        assert!(ECHO_SAMPLES > longest_echo);
        let longest_chorus =
            (MAXIMUM_SAMPLE_RATE * crate::pedals::chorus::MAXIMUM_DELAY_SECONDS) as usize;
        assert!(CHORUS_SAMPLES > longest_chorus);
        let longest_reverb_line =
            (MAXIMUM_SAMPLE_RATE * crate::pedals::reverb::MAXIMUM_LINE_SECONDS) as usize;
        assert!(REVERB_LINE_SAMPLES > longest_reverb_line);
    }

    #[test]
    fn the_static_block_is_handed_out_once() {
        assert!(take_static_workspace().is_some());
        assert!(take_static_workspace().is_none());
    }
}
