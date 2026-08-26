//! The RackForge adapter.
//!
//! Everything musical lives in `rf-rig-dsp`. This file only translates between
//! the host's block-based ABI and the engine's one-sample-in, stereo-pair-out
//! interface, and it holds the two rules the host cares about: no allocation
//! after activation, and no work in the audio callback that could block.

#![cfg_attr(target_arch = "wasm32", no_std)]

use rackforge_plugin_sdk::{MidiEvent, ParameterEvent, Processor, export_processor};
use rf_rig_dsp::Engine;

const MAX_INPUT_CHANNELS: u32 = 2;
const MAX_OUTPUT_CHANNELS: u32 = 2;

/// Claims this instance's delay memory.
///
/// On wasm — the only target RackForge ships — each plugin instance is a
/// separate module instance with its own linear memory, so the static block in
/// `rf-rig-dsp` belongs to exactly one engine and nothing is shared.
#[cfg(target_arch = "wasm32")]
fn claim_workspace() -> Option<&'static mut [f32]> {
    rf_rig_dsp::take_static_workspace()
}

/// Native builds are the test harness and the lab tool, where several engines
/// live in one process. They get their own block, intentionally leaked: an
/// engine borrows its memory for as long as it exists, and these instances live
/// for the life of the tool.
#[cfg(not(target_arch = "wasm32"))]
fn claim_workspace() -> Option<&'static mut [f32]> {
    Some(Box::leak(
        std::vec![0.0_f32; rf_rig_dsp::WORKSPACE_SAMPLES].into_boxed_slice(),
    ))
}

#[derive(Default)]
pub struct RfRigProcessor {
    engine: Engine<'static>,
    attached: bool,
}

impl Processor for RfRigProcessor {
    fn prepare(
        &mut self,
        sample_rate: f64,
        _maximum_frames: u32,
        input_channels: u32,
        output_channels: u32,
    ) -> bool {
        // A pedalboard with no input is not a pedalboard.
        if input_channels == 0 || input_channels > MAX_INPUT_CHANNELS {
            return false;
        }
        if output_channels == 0 || output_channels > MAX_OUTPUT_CHANNELS {
            return false;
        }
        if !self.attached {
            let Some(buffer) = claim_workspace() else {
                return false;
            };
            if !self.engine.attach(buffer) {
                return false;
            }
            self.attached = true;
        }
        self.engine.prepare(sample_rate)
    }

    fn set_parameter(&mut self, index: u32, value: f64) -> bool {
        self.engine.set_parameter(index, value)
    }

    fn get_parameter(&self, index: u32) -> Option<f64> {
        self.engine.parameter(index)
    }

    fn reset(&mut self) {
        self.engine.reset();
    }

    fn load_preset(&mut self, id: &str) -> bool {
        self.engine.load_preset(id)
    }

    fn save_state(&self, destination: &mut [u8]) -> Option<usize> {
        self.engine.save_state(destination)
    }

    fn load_state(&mut self, state: &[u8]) -> bool {
        self.engine.load_state(state)
    }

    fn process(
        &mut self,
        input: &[f32],
        output: &mut [f32],
        _midi: &[MidiEvent],
        parameters: &[ParameterEvent],
        frames: u32,
        input_channels: u32,
        output_channels: u32,
    ) {
        let input_channels = input_channels as usize;
        let output_channels = output_channels as usize;
        let mut parameter_index = 0;

        for frame in 0..frames as usize {
            // Sample-accurate automation: apply everything scheduled for this
            // frame before the frame is processed.
            while let Some(event) = parameters.get(parameter_index) {
                if event.frame as usize != frame {
                    break;
                }
                let _ = self.engine.set_parameter(event.index, event.value);
                parameter_index += 1;
            }

            // The board has one input jack. A host that hands over a stereo
            // capture gets summed, which is what a mono pedal does with one.
            let sample = match input_channels {
                0 => 0.0,
                1 => input.get(frame).copied().unwrap_or(0.0),
                channels => {
                    let base = frame * channels;
                    let left = input.get(base).copied().unwrap_or(0.0);
                    let right = input.get(base + 1).copied().unwrap_or(0.0);
                    0.5 * (left + right)
                }
            };

            let (left, right) = self.engine.process(sample);
            let base = frame * output_channels;
            for channel in 0..output_channels {
                let value = if channel == 0 { left } else { right };
                if let Some(slot) = output.get_mut(base + channel) {
                    *slot = value;
                }
            }
        }
    }
}

export_processor!(
    RfRigProcessor,
    max_frames = 4096,
    max_input_channels = 2,
    max_output_channels = 2,
    max_midi_events = 64,
    max_parameter_events = 256,
    max_transfer_bytes = 4096
);

#[cfg(all(target_arch = "wasm32", not(test)))]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo<'_>) -> ! {
    core::arch::wasm32::unreachable()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rf_rig_contract::index::{DRIVE_DRIVE, DRIVE_ENGAGED, RIG_OUTPUT, RIG_SOURCE};
    use rf_rig_dsp::STATE_BYTES;

    fn prepared() -> RfRigProcessor {
        let mut processor = RfRigProcessor::default();
        assert!(processor.prepare(48_000.0, 256, 1, 2));
        processor
    }

    fn render(processor: &mut RfRigProcessor, frames: usize) -> (f32, f32) {
        let mut input = [0.0_f32; 256];
        let mut output = [0.0_f32; 512];
        let mut left_peak = 0.0_f32;
        let mut right_peak = 0.0_f32;
        for block in 0..frames / 256 {
            for (index, slot) in input.iter_mut().enumerate() {
                let position = (block * 256 + index) as f32;
                *slot = 0.2 * libm::sinf(core::f32::consts::TAU * 220.0 * position / 48_000.0);
            }
            processor.process(&input, &mut output, &[], &[], 256, 1, 2);
            for frame in 0..256 {
                left_peak = left_peak.max(libm::fabsf(output[frame * 2]));
                right_peak = right_peak.max(libm::fabsf(output[frame * 2 + 1]));
            }
        }
        (left_peak, right_peak)
    }

    #[test]
    fn it_refuses_a_configuration_it_cannot_serve() {
        let mut processor = RfRigProcessor::default();
        assert!(!processor.prepare(48_000.0, 256, 0, 2));
        assert!(!processor.prepare(48_000.0, 256, 4, 2));
        assert!(!processor.prepare(48_000.0, 256, 1, 6));
        assert!(!processor.prepare(192_000.0, 256, 1, 2));
    }

    #[test]
    fn a_bypassed_board_passes_audio_through() {
        let mut processor = prepared();
        let (left, right) = render(&mut processor, 4_096);
        assert!((left - 0.2).abs() < 0.01, "peak {left}");
        assert!((right - 0.2).abs() < 0.01, "peak {right}");
    }

    #[test]
    fn automation_events_are_applied_at_their_frame() {
        let mut processor = prepared();
        let mut input = [0.0_f32; 256];
        let mut output = [0.0_f32; 512];
        for (index, slot) in input.iter_mut().enumerate() {
            *slot = 0.2 * libm::sinf(core::f32::consts::TAU * 220.0 * index as f32 / 48_000.0);
        }
        // Silence the output halfway through the block.
        let events = [ParameterEvent {
            frame: 128,
            index: RIG_OUTPUT,
            value: -24.0,
        }];
        processor.process(&input, &mut output, &[], &events, 256, 1, 2);
        let before = (0..128).fold(0.0_f32, |worst, frame| {
            worst.max(libm::fabsf(output[frame * 2]))
        });
        let after = (128..256).fold(0.0_f32, |worst, frame| {
            worst.max(libm::fabsf(output[frame * 2]))
        });
        assert!(after < before * 0.2, "{after} did not drop below {before}");
    }

    #[test]
    fn state_survives_a_round_trip_through_the_host() {
        let mut processor = prepared();
        assert!(processor.set_parameter(DRIVE_ENGAGED, 1.0));
        assert!(processor.set_parameter(DRIVE_DRIVE, 0.8));
        let mut bytes = [0_u8; STATE_BYTES];
        assert_eq!(processor.save_state(&mut bytes), Some(STATE_BYTES));

        let mut restored = prepared();
        assert!(restored.load_state(&bytes));
        assert_eq!(restored.get_parameter(DRIVE_ENGAGED), Some(1.0));
        assert_eq!(restored.get_parameter(DRIVE_DRIVE), Some(0.8_f32 as f64));
        // A block from the layout before the source selector was appended is a
        // migration, not a corruption: it loads, and the parameter it predates
        // keeps its default.
        assert!(restored.load_state(&bytes[..STATE_BYTES - 4]));
        assert_eq!(restored.get_parameter(RIG_SOURCE), Some(0.0));
        // A length belonging to no layout is refused rather than half-applied.
        assert!(!restored.load_state(&bytes[..STATE_BYTES - 8]));
        assert!(!restored.load_state(&bytes[..STATE_BYTES - 1]));
    }

    #[test]
    fn every_factory_preset_is_reachable_by_identifier() {
        let mut processor = prepared();
        for preset in rf_rig_contract::PRESETS.iter() {
            assert!(processor.load_preset(preset.id), "{}", preset.id);
        }
        assert!(!processor.load_preset("not-a-board"));
    }
}
