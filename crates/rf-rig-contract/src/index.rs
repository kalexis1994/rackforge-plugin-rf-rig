//! Named parameter indexes.
//!
//! The engine and the packager both address parameters by number. Naming them
//! here — and asserting in tests that each name still points at the identifier
//! it claims — keeps a renumbering from quietly rewiring a knob.

pub const RIG_INPUT: u32 = 0;
pub const RIG_OUTPUT: u32 = 1;
pub const RIG_GATE: u32 = 2;
/// Appended after the pedals; see the note in `lib.rs`.
pub const RIG_SOURCE: u32 = 41;

pub const COMP_ENGAGED: u32 = 3;
pub const COMP_POSITION: u32 = 4;
pub const COMP_SUSTAIN: u32 = 5;
pub const COMP_ATTACK: u32 = 6;
pub const COMP_LEVEL: u32 = 7;

pub const DRIVE_ENGAGED: u32 = 8;
pub const DRIVE_POSITION: u32 = 9;
pub const DRIVE_DRIVE: u32 = 10;
pub const DRIVE_TONE: u32 = 11;
pub const DRIVE_LEVEL: u32 = 12;

pub const DIST_ENGAGED: u32 = 13;
pub const DIST_POSITION: u32 = 14;
pub const DIST_DISTORTION: u32 = 15;
pub const DIST_TONE: u32 = 16;
pub const DIST_LEVEL: u32 = 17;

pub const FUZZ_ENGAGED: u32 = 18;
pub const FUZZ_POSITION: u32 = 19;
pub const FUZZ_SUSTAIN: u32 = 20;
pub const FUZZ_TONE: u32 = 21;
pub const FUZZ_VOLUME: u32 = 22;

pub const CHORUS_ENGAGED: u32 = 23;
pub const CHORUS_POSITION: u32 = 24;
pub const CHORUS_RATE: u32 = 25;
pub const CHORUS_DEPTH: u32 = 26;
pub const CHORUS_MIX: u32 = 27;

pub const DELAY_ENGAGED: u32 = 28;
pub const DELAY_POSITION: u32 = 29;
pub const DELAY_TIME: u32 = 30;
pub const DELAY_FEEDBACK: u32 = 31;
pub const DELAY_MIX: u32 = 32;
pub const DELAY_MODE: u32 = 33;
pub const DELAY_WIDTH: u32 = 34;

pub const REVERB_ENGAGED: u32 = 35;
pub const REVERB_POSITION: u32 = 36;
pub const REVERB_DECAY: u32 = 37;
pub const REVERB_TONE: u32 = 38;
pub const REVERB_MIX: u32 = 39;
pub const REVERB_MODE: u32 = 40;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PARAMETERS;

    fn identifier(index: u32) -> &'static str {
        PARAMETERS[index as usize].id
    }

    #[test]
    fn every_name_points_at_the_parameter_it_claims() {
        assert_eq!(identifier(RIG_INPUT), "rig.input");
        assert_eq!(identifier(RIG_OUTPUT), "rig.output");
        assert_eq!(identifier(RIG_GATE), "rig.gate");
        assert_eq!(identifier(RIG_SOURCE), "rig.source");
        assert_eq!(identifier(COMP_ENGAGED), "comp.engaged");
        assert_eq!(identifier(COMP_POSITION), "comp.position");
        assert_eq!(identifier(COMP_SUSTAIN), "comp.sustain");
        assert_eq!(identifier(COMP_ATTACK), "comp.attack");
        assert_eq!(identifier(COMP_LEVEL), "comp.level");
        assert_eq!(identifier(DRIVE_ENGAGED), "drive.engaged");
        assert_eq!(identifier(DRIVE_POSITION), "drive.position");
        assert_eq!(identifier(DRIVE_DRIVE), "drive.drive");
        assert_eq!(identifier(DRIVE_TONE), "drive.tone");
        assert_eq!(identifier(DRIVE_LEVEL), "drive.level");
        assert_eq!(identifier(DIST_ENGAGED), "dist.engaged");
        assert_eq!(identifier(DIST_POSITION), "dist.position");
        assert_eq!(identifier(DIST_DISTORTION), "dist.distortion");
        assert_eq!(identifier(DIST_TONE), "dist.tone");
        assert_eq!(identifier(DIST_LEVEL), "dist.level");
        assert_eq!(identifier(FUZZ_ENGAGED), "fuzz.engaged");
        assert_eq!(identifier(FUZZ_POSITION), "fuzz.position");
        assert_eq!(identifier(FUZZ_SUSTAIN), "fuzz.sustain");
        assert_eq!(identifier(FUZZ_TONE), "fuzz.tone");
        assert_eq!(identifier(FUZZ_VOLUME), "fuzz.volume");
        assert_eq!(identifier(CHORUS_ENGAGED), "chorus.engaged");
        assert_eq!(identifier(CHORUS_POSITION), "chorus.position");
        assert_eq!(identifier(CHORUS_RATE), "chorus.rate");
        assert_eq!(identifier(CHORUS_DEPTH), "chorus.depth");
        assert_eq!(identifier(CHORUS_MIX), "chorus.mix");
        assert_eq!(identifier(DELAY_ENGAGED), "delay.engaged");
        assert_eq!(identifier(DELAY_POSITION), "delay.position");
        assert_eq!(identifier(DELAY_TIME), "delay.time");
        assert_eq!(identifier(DELAY_FEEDBACK), "delay.feedback");
        assert_eq!(identifier(DELAY_MIX), "delay.mix");
        assert_eq!(identifier(DELAY_MODE), "delay.mode");
        assert_eq!(identifier(DELAY_WIDTH), "delay.width");
        assert_eq!(identifier(REVERB_ENGAGED), "reverb.engaged");
        assert_eq!(identifier(REVERB_POSITION), "reverb.position");
        assert_eq!(identifier(REVERB_DECAY), "reverb.decay");
        assert_eq!(identifier(REVERB_TONE), "reverb.tone");
        assert_eq!(identifier(REVERB_MIX), "reverb.mix");
        assert_eq!(identifier(REVERB_MODE), "reverb.mode");
    }
}
