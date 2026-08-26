//! The board: seven pedals, cabled in the order the parameters describe.

use rf_rig_contract::index::*;
use rf_rig_contract::{PARAMETER_COUNT, PEDAL_COUNT, PEDALS, Settings, chain_order, preset};

use crate::Frame;
use crate::circuit::dynamics::NoiseGate;
use crate::math::db_to_gain;
use crate::pedals::chorus::Chorus;
use crate::pedals::compressor::Compressor;
use crate::pedals::distortion::Distortion;
use crate::pedals::echo::{Echo, EchoMode};
use crate::pedals::fuzz::Fuzz;
use crate::pedals::overdrive::Overdrive;
use crate::pedals::reverb::{Reverb, ReverbMode};
use crate::workspace::{MAXIMUM_SAMPLE_RATE, Workspace};

/// Length of the serialised state, in bytes.
pub const STATE_BYTES: usize = PARAMETER_COUNT * 4;

/// Slot numbers, matching the order pedals are declared in the contract.
const SLOT_COMPRESSOR: usize = 0;
const SLOT_OVERDRIVE: usize = 1;
const SLOT_DISTORTION: usize = 2;
const SLOT_FUZZ: usize = 3;
const SLOT_CHORUS: usize = 4;
const SLOT_ECHO: usize = 5;
const SLOT_REVERB: usize = 6;

#[derive(Default)]
pub struct Engine<'a> {
    settings: Settings,
    workspace: Option<Workspace<'a>>,
    sample_rate: f32,
    prepared: bool,

    order: [usize; PEDAL_COUNT],
    engaged: [bool; PEDAL_COUNT],
    input_gain: f32,
    output_gain: f32,

    gate: NoiseGate,
    compressor: Compressor,
    overdrive: Overdrive,
    distortion: Distortion,
    fuzz: Fuzz,
    chorus: Chorus,
    echo: Echo,
    reverb: Reverb,
}

impl<'a> Engine<'a> {
    /// Hands the engine its delay memory. Called once per instance, before
    /// the first [`Engine::prepare`].
    ///
    /// Memory and sample rate are separate steps because a host may re-prepare
    /// an instance at a new rate, and the block it was given at activation is
    /// still the same block.
    pub fn attach(&mut self, buffer: &'a mut [f32]) -> bool {
        let Some(mut workspace) = Workspace::new(buffer) else {
            return false;
        };
        workspace.clear();
        self.workspace = Some(workspace);
        self.prepared = false;
        true
    }

    /// Configures every pedal for a sample rate.
    ///
    /// Refuses rather than degrades: a rate the buffers were not sized for
    /// means the host asked for something this build cannot deliver honestly.
    pub fn prepare(&mut self, sample_rate: f64) -> bool {
        let rate = sample_rate as f32;
        if !rate.is_finite() || !(8_000.0..=MAXIMUM_SAMPLE_RATE).contains(&rate) {
            return false;
        }
        {
            let Self {
                workspace,
                gate,
                compressor,
                overdrive,
                distortion,
                fuzz,
                chorus,
                echo,
                reverb,
                ..
            } = self;
            let Some(workspace) = workspace.as_mut() else {
                return false;
            };
            workspace.clear();
            gate.prepare(rate);
            compressor.prepare(rate);
            overdrive.prepare(rate);
            distortion.prepare(rate);
            fuzz.prepare(rate);
            chorus.prepare(workspace.chorus, rate);
            echo.prepare(workspace.echo_left, workspace.echo_right, rate);
            reverb.prepare(workspace.reverb, rate);
        }
        self.sample_rate = rate;
        self.prepared = true;
        self.apply_settings();
        true
    }

    /// Convenience for tests and the lab tool: attach and prepare in one call.
    pub fn prepare_with(&mut self, sample_rate: f64, buffer: &'a mut [f32]) -> bool {
        self.attach(buffer) && self.prepare(sample_rate)
    }

    /// Clears every delay line and detector without touching the settings.
    pub fn reset(&mut self) {
        self.gate.reset();
        self.compressor.reset();
        self.overdrive.reset();
        self.distortion.reset();
        self.fuzz.reset();
        if let Some(workspace) = self.workspace.as_mut() {
            self.chorus.reset(workspace.chorus);
            self.echo.reset(workspace.echo_left, workspace.echo_right);
            self.reverb.reset(workspace.reverb);
        }
    }

    pub fn set_parameter(&mut self, index: u32, value: f64) -> bool {
        if !self.settings.set(index, value) {
            return false;
        }
        self.apply_settings();
        true
    }

    pub fn parameter(&self, index: u32) -> Option<f64> {
        self.settings.get(index)
    }

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    pub fn chain(&self) -> [usize; PEDAL_COUNT] {
        self.order
    }

    pub fn load_preset(&mut self, id: &str) -> bool {
        let Some(settings) = preset::settings_for(id) else {
            return false;
        };
        self.settings = settings;
        self.apply_settings();
        true
    }

    pub fn save_state(&self, destination: &mut [u8]) -> Option<usize> {
        let target = destination.get_mut(..STATE_BYTES)?;
        for (slot, value) in target.chunks_exact_mut(4).zip(self.settings.as_array()) {
            slot.copy_from_slice(&value.to_le_bytes());
        }
        Some(STATE_BYTES)
    }

    pub fn load_state(&mut self, state: &[u8]) -> bool {
        if state.len() != STATE_BYTES {
            return false;
        }
        let mut values = [0.0_f32; PARAMETER_COUNT];
        for (value, bytes) in values.iter_mut().zip(state.chunks_exact(4)) {
            let Ok(word) = <[u8; 4]>::try_from(bytes) else {
                return false;
            };
            *value = f32::from_le_bytes(word);
        }
        let Some(settings) = Settings::from_array(values) else {
            return false;
        };
        self.settings = settings;
        self.apply_settings();
        true
    }

    /// Pushes the current settings into every pedal. Cheap enough to run on
    /// each parameter change, which keeps the engine free of "did I forget to
    /// apply that one" bugs.
    fn apply_settings(&mut self) {
        let settings = self.settings;
        let value = |index: u32| settings.value(index);
        self.order = chain_order(&settings);
        for (slot, pedal) in PEDALS.iter().enumerate() {
            self.engaged[slot] = settings.engaged(pedal.engaged);
        }

        self.input_gain = db_to_gain(value(RIG_INPUT));
        self.output_gain = db_to_gain(value(RIG_OUTPUT));
        self.gate.set_threshold_db(value(RIG_GATE));

        self.compressor
            .set_controls(value(COMP_SUSTAIN), value(COMP_ATTACK), value(COMP_LEVEL));
        self.overdrive
            .set_controls(value(DRIVE_DRIVE), value(DRIVE_TONE), value(DRIVE_LEVEL));
        self.distortion
            .set_controls(value(DIST_DISTORTION), value(DIST_TONE), value(DIST_LEVEL));
        self.fuzz
            .set_controls(value(FUZZ_SUSTAIN), value(FUZZ_TONE), value(FUZZ_VOLUME));
        self.chorus
            .set_controls(value(CHORUS_RATE), value(CHORUS_DEPTH), value(CHORUS_MIX));
        let echo_mode = if settings.selection(DELAY_MODE) == 0 {
            EchoMode::Analog
        } else {
            EchoMode::Digital
        };
        self.echo.set_controls(
            value(DELAY_TIME),
            value(DELAY_FEEDBACK),
            value(DELAY_MIX),
            value(DELAY_WIDTH),
            echo_mode,
        );
        let reverb_mode = if settings.selection(REVERB_MODE) == 0 {
            ReverbMode::Spring
        } else {
            ReverbMode::Plate
        };
        self.reverb.set_controls(
            value(REVERB_DECAY),
            value(REVERB_TONE),
            value(REVERB_MIX),
            reverb_mode,
        );
    }

    /// One sample in, a stereo pair out.
    ///
    /// A pedal that has no stereo behaviour sums the frame back to mono before
    /// it runs, exactly as a real one-input pedal would. Placing the fuzz after
    /// the delay therefore collapses the image, which is not a limitation but
    /// the same thing that happens on a physical board.
    #[inline]
    pub fn process(&mut self, input: f32) -> (f32, f32) {
        if !self.prepared {
            return (0.0, 0.0);
        }
        let Self {
            workspace,
            order,
            engaged,
            input_gain,
            output_gain,
            gate,
            compressor,
            overdrive,
            distortion,
            fuzz,
            chorus,
            echo,
            reverb,
            ..
        } = self;
        let Some(workspace) = workspace.as_mut() else {
            return (0.0, 0.0);
        };

        let mut frame = Frame::mono(gate.process(input * *input_gain));

        for slot in order.iter() {
            if !engaged[*slot] {
                continue;
            }
            match *slot {
                SLOT_COMPRESSOR => {
                    let sample = compressor.process(frame.to_mono());
                    frame.set_mono(sample);
                }
                SLOT_OVERDRIVE => {
                    let sample = overdrive.process(frame.to_mono());
                    frame.set_mono(sample);
                }
                SLOT_DISTORTION => {
                    let sample = distortion.process(frame.to_mono());
                    frame.set_mono(sample);
                }
                SLOT_FUZZ => {
                    let sample = fuzz.process(frame.to_mono());
                    frame.set_mono(sample);
                }
                SLOT_CHORUS => frame = chorus.process(workspace.chorus, frame),
                SLOT_ECHO => frame = echo.process(workspace.echo_left, workspace.echo_right, frame),
                SLOT_REVERB => frame = reverb.process(workspace.reverb, frame),
                _ => {}
            }
        }

        (frame.left * *output_gain, frame.right * *output_gain)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{peak, rms};
    use crate::workspace::WORKSPACE_SAMPLES;
    use rf_rig_contract::PRESETS;
    use std::vec;
    use std::vec::Vec;

    fn buffer() -> Vec<f32> {
        vec![0.0_f32; WORKSPACE_SAMPLES]
    }

    fn render(engine: &mut Engine<'_>, samples: usize, sample_rate: f32) -> (Vec<f32>, Vec<f32>) {
        let mut left = Vec::with_capacity(samples);
        let mut right = Vec::with_capacity(samples);
        for index in 0..samples {
            let input =
                0.2 * crate::math::sin(crate::math::TAU * 196.0 * index as f32 / sample_rate);
            let (l, r) = engine.process(input);
            left.push(l);
            right.push(r);
        }
        (left, right)
    }

    #[test]
    fn an_unprepared_engine_is_silent_rather_than_wrong() {
        let mut engine = Engine::default();
        assert_eq!(engine.process(0.5), (0.0, 0.0));
    }

    #[test]
    fn preparation_refuses_a_rate_the_buffers_cannot_serve() {
        let mut memory = buffer();
        let mut engine = Engine::default();
        assert!(!engine.prepare_with(192_000.0, &mut memory));
    }

    #[test]
    fn preparation_refuses_a_block_that_is_too_small() {
        let mut memory = vec![0.0_f32; 128];
        let mut engine = Engine::default();
        assert!(!engine.prepare_with(48_000.0, &mut memory));
    }

    #[test]
    fn an_empty_board_passes_the_signal_through() {
        let sample_rate = 48_000.0;
        let mut memory = buffer();
        let mut engine = Engine::default();
        assert!(engine.prepare_with(sample_rate as f64, &mut memory));
        let (left, right) = render(&mut engine, 4_800, sample_rate);
        // Nothing is engaged, so the chain is a wire.
        assert!((rms(&left) - 0.1414).abs() < 0.002, "level {}", rms(&left));
        assert_eq!(left, right);
    }

    #[test]
    fn each_pedal_changes_the_signal_when_it_is_switched_on() {
        let sample_rate = 48_000.0;
        for pedal in PEDALS.iter() {
            let mut memory = buffer();
            let mut engine = Engine::default();
            assert!(engine.prepare_with(sample_rate as f64, &mut memory));
            let (bypassed, _) = render(&mut engine, 24_000, sample_rate);

            let mut memory = buffer();
            let mut engine = Engine::default();
            assert!(engine.prepare_with(sample_rate as f64, &mut memory));
            assert!(engine.set_parameter(pedal.engaged, 1.0));
            let (engaged, _) = render(&mut engine, 24_000, sample_rate);

            let difference: Vec<f32> = bypassed
                .iter()
                .zip(&engaged)
                .map(|(off, on)| on - off)
                .collect();
            assert!(
                rms(&difference) > 1.0e-3,
                "{} changed nothing when engaged",
                pedal.id
            );
        }
    }

    #[test]
    fn the_board_order_follows_the_position_parameters() {
        let mut memory = buffer();
        let mut engine = Engine::default();
        assert!(engine.prepare_with(48_000.0, &mut memory));
        assert_eq!(engine.chain(), [0, 1, 2, 3, 4, 5, 6]);
        assert!(engine.set_parameter(REVERB_POSITION, 1.0));
        assert!(engine.set_parameter(COMP_POSITION, 2.0));
        assert_eq!(engine.chain()[0], SLOT_REVERB);
    }

    #[test]
    fn order_changes_the_sound_not_just_the_bookkeeping() {
        // Distortion after a delay smears the repeats; distortion before it
        // feeds clean repeats of a dirty signal. The two cannot be the same.
        let sample_rate = 48_000.0;
        let mut first_memory = buffer();
        let mut first = Engine::default();
        assert!(first.prepare_with(sample_rate as f64, &mut first_memory));
        for (index, value) in [
            (DIST_ENGAGED, 1.0),
            (DELAY_ENGAGED, 1.0),
            (DELAY_MIX, 0.5),
            (DELAY_FEEDBACK, 0.4),
            (DIST_POSITION, 1.0),
            (DELAY_POSITION, 2.0),
        ] {
            assert!(first.set_parameter(index, value));
        }
        let (dirt_first, _) = render(&mut first, 48_000, sample_rate);

        let mut second_memory = buffer();
        let mut second = Engine::default();
        assert!(second.prepare_with(sample_rate as f64, &mut second_memory));
        for (index, value) in [
            (DIST_ENGAGED, 1.0),
            (DELAY_ENGAGED, 1.0),
            (DELAY_MIX, 0.5),
            (DELAY_FEEDBACK, 0.4),
            (DELAY_POSITION, 1.0),
            (DIST_POSITION, 2.0),
        ] {
            assert!(second.set_parameter(index, value));
        }
        let (delay_first, _) = render(&mut second, 48_000, sample_rate);

        let difference: Vec<f32> = dirt_first
            .iter()
            .zip(&delay_first)
            .map(|(a, b)| a - b)
            .collect();
        assert!(
            rms(&difference) > 1.0e-3,
            "swapping two pedals produced the same audio"
        );
    }

    #[test]
    fn state_round_trips_through_bytes() {
        let mut memory = buffer();
        let mut engine = Engine::default();
        assert!(engine.prepare_with(48_000.0, &mut memory));
        assert!(engine.set_parameter(DRIVE_ENGAGED, 1.0));
        assert!(engine.set_parameter(DRIVE_DRIVE, 0.77));
        assert!(engine.set_parameter(DELAY_TIME, 512.0));

        let mut bytes = [0_u8; STATE_BYTES];
        assert_eq!(engine.save_state(&mut bytes), Some(STATE_BYTES));

        let mut other_memory = buffer();
        let mut restored = Engine::default();
        assert!(restored.prepare_with(48_000.0, &mut other_memory));
        assert!(restored.load_state(&bytes));
        assert_eq!(restored.parameter(DRIVE_DRIVE), Some(0.77_f32 as f64));
        assert_eq!(restored.parameter(DELAY_TIME), Some(512.0));
    }

    #[test]
    fn a_corrupt_state_is_rejected_atomically() {
        let mut memory = buffer();
        let mut engine = Engine::default();
        assert!(engine.prepare_with(48_000.0, &mut memory));
        assert!(engine.set_parameter(DRIVE_DRIVE, 0.6));

        let mut bytes = [0_u8; STATE_BYTES];
        assert_eq!(engine.save_state(&mut bytes), Some(STATE_BYTES));
        // Put an impossible value where the delay time lives.
        let offset = DELAY_TIME as usize * 4;
        bytes[offset..offset + 4].copy_from_slice(&99_999.0_f32.to_le_bytes());
        assert!(!engine.load_state(&bytes));
        assert_eq!(engine.parameter(DRIVE_DRIVE), Some(0.6_f32 as f64));

        assert!(!engine.load_state(&bytes[..STATE_BYTES - 1]));
    }

    #[test]
    fn every_factory_preset_loads_and_makes_sound() {
        let sample_rate = 48_000.0;
        for factory in PRESETS.iter() {
            let mut memory = buffer();
            let mut engine = Engine::default();
            assert!(engine.prepare_with(sample_rate as f64, &mut memory));
            assert!(engine.load_preset(factory.id), "{} refused", factory.id);
            let (left, right) = render(&mut engine, 24_000, sample_rate);
            let level = rms(&left);
            assert!(level > 1.0e-3, "{} produced silence ({level})", factory.id);
            assert!(
                peak(&left).is_finite() && peak(&right).is_finite(),
                "{} produced a non-finite sample",
                factory.id
            );
            assert!(peak(&left) < 8.0, "{} is far too loud", factory.id);
        }
    }

    #[test]
    fn an_unknown_preset_is_refused() {
        let mut memory = buffer();
        let mut engine = Engine::default();
        assert!(engine.prepare_with(48_000.0, &mut memory));
        assert!(!engine.load_preset("no-such-board"));
    }

    #[test]
    fn everything_switched_on_at_once_stays_finite() {
        let sample_rate = 48_000.0;
        let mut memory = buffer();
        let mut engine = Engine::default();
        assert!(engine.prepare_with(sample_rate as f64, &mut memory));
        for pedal in PEDALS.iter() {
            assert!(engine.set_parameter(pedal.engaged, 1.0));
        }
        for (index, value) in [
            (COMP_SUSTAIN, 1.0),
            (DRIVE_DRIVE, 1.0),
            (DIST_DISTORTION, 1.0),
            (FUZZ_SUSTAIN, 1.0),
            (CHORUS_DEPTH, 1.0),
            (CHORUS_MIX, 1.0),
            (DELAY_FEEDBACK, 1.0),
            (DELAY_MIX, 1.0),
            (DELAY_WIDTH, 1.0),
            (REVERB_DECAY, 1.0),
            (REVERB_MIX, 1.0),
            (RIG_INPUT, 24.0),
        ] {
            assert!(engine.set_parameter(index, value));
        }
        let (left, right) = render(&mut engine, 48_000 * 4, sample_rate);
        assert!(peak(&left).is_finite() && peak(&left) < 24.0);
        assert!(peak(&right).is_finite() && peak(&right) < 24.0);
    }
}
