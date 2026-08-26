//! Factory boards.
//!
//! A preset here is nothing but a list of parameter values, which is what a
//! pedalboard is: which boxes are on, in what order, with the knobs where. The
//! packager renders the same table into `metadata/presets.json`, so the catalog
//! RackForge shows and the settings the engine loads cannot disagree.

use crate::Settings;
use crate::index::*;

pub struct Preset {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub values: &'static [(u32, f32)],
}

pub const PRESET_COUNT: usize = 6;

pub const PRESETS: [Preset; PRESET_COUNT] = [
    Preset {
        id: "clean-board",
        name: "Clean Board",
        description: "Compressor into a short spring. Everything else off.",
        values: &[
            (COMP_ENGAGED, 1.0),
            (COMP_SUSTAIN, 0.45),
            (COMP_ATTACK, 0.55),
            (COMP_LEVEL, 0.38),
            (REVERB_ENGAGED, 1.0),
            (REVERB_DECAY, 0.3),
            (REVERB_TONE, 0.5),
            (REVERB_MIX, 0.18),
            (REVERB_MODE, 0.0),
        ],
    },
    Preset {
        id: "blues-drive",
        name: "Blues Drive",
        description: "Overdrive just past the knee, compressor in front of it.",
        values: &[
            (COMP_ENGAGED, 1.0),
            (COMP_SUSTAIN, 0.35),
            (COMP_LEVEL, 0.36),
            (DRIVE_ENGAGED, 1.0),
            (DRIVE_DRIVE, 0.35),
            (DRIVE_TONE, 0.55),
            (DRIVE_LEVEL, 0.6),
            (REVERB_ENGAGED, 1.0),
            (REVERB_MIX, 0.15),
        ],
    },
    Preset {
        id: "rock-distortion",
        name: "Rock Distortion",
        description: "Hard clipping with the scoop open and a slapback behind it.",
        values: &[
            (DIST_ENGAGED, 1.0),
            (DIST_DISTORTION, 0.6),
            (DIST_TONE, 0.55),
            (DIST_LEVEL, 0.5),
            (DELAY_ENGAGED, 1.0),
            (DELAY_TIME, 120.0),
            (DELAY_FEEDBACK, 0.2),
            (DELAY_MIX, 0.18),
            (DELAY_MODE, 0.0),
        ],
    },
    Preset {
        id: "fuzz-lead",
        name: "Fuzz Lead",
        description: "Both clipping stages wide open, delay long enough to sing.",
        values: &[
            (FUZZ_ENGAGED, 1.0),
            (FUZZ_SUSTAIN, 0.85),
            (FUZZ_TONE, 0.55),
            (FUZZ_VOLUME, 0.35),
            (DELAY_ENGAGED, 1.0),
            (DELAY_TIME, 420.0),
            (DELAY_FEEDBACK, 0.4),
            (DELAY_MIX, 0.28),
            (REVERB_ENGAGED, 1.0),
            (REVERB_MODE, 1.0),
            (REVERB_DECAY, 0.55),
            (REVERB_MIX, 0.22),
        ],
    },
    Preset {
        id: "surf",
        name: "Surf",
        description: "Dry front end, spring tank most of the way up.",
        values: &[
            (COMP_ENGAGED, 1.0),
            (COMP_SUSTAIN, 0.3),
            (COMP_LEVEL, 0.35),
            (REVERB_ENGAGED, 1.0),
            (REVERB_MODE, 0.0),
            (REVERB_DECAY, 0.7),
            (REVERB_TONE, 0.7),
            (REVERB_MIX, 0.45),
        ],
    },
    Preset {
        id: "ambient-wash",
        name: "Ambient Wash",
        description: "Chorus into a wide analog echo into a long plate.",
        values: &[
            (CHORUS_ENGAGED, 1.0),
            (CHORUS_RATE, 0.5),
            (CHORUS_DEPTH, 0.6),
            (CHORUS_MIX, 0.5),
            (DELAY_ENGAGED, 1.0),
            (DELAY_TIME, 620.0),
            (DELAY_FEEDBACK, 0.55),
            (DELAY_MIX, 0.35),
            (DELAY_WIDTH, 0.8),
            (REVERB_ENGAGED, 1.0),
            (REVERB_MODE, 1.0),
            (REVERB_DECAY, 0.85),
            (REVERB_TONE, 0.4),
            (REVERB_MIX, 0.4),
        ],
    },
];

/// Builds the settings a preset describes, starting from the defaults.
/// Returns `None` if the preset names a value the contract rejects, which a
/// test turns into a build failure rather than a surprise on stage.
pub fn settings_for(id: &str) -> Option<Settings> {
    let preset = PRESETS.iter().find(|preset| preset.id == id)?;
    let mut settings = Settings::default();
    for (index, value) in preset.values {
        if !settings.set(*index, *value as f64) {
            return None;
        }
    }
    Some(settings)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_preset_is_inside_the_contract() {
        for preset in PRESETS.iter() {
            assert!(
                settings_for(preset.id).is_some(),
                "{} contains a value the contract rejects",
                preset.id
            );
        }
    }

    #[test]
    fn preset_identifiers_are_unique() {
        for (position, preset) in PRESETS.iter().enumerate() {
            for other in &PRESETS[position + 1..] {
                assert_ne!(preset.id, other.id);
            }
        }
    }

    #[test]
    fn every_preset_turns_at_least_one_pedal_on() {
        for preset in PRESETS.iter() {
            let settings = settings_for(preset.id).expect("valid preset");
            let engaged = crate::PEDALS
                .iter()
                .filter(|pedal| settings.engaged(pedal.engaged))
                .count();
            assert!(engaged > 0, "{} is a board with nothing on it", preset.id);
        }
    }
}
