//! The public contract of RF-Rig: every parameter the host can see, the pedal
//! slots those parameters belong to, and the flat settings block that becomes
//! plugin state.
//!
//! Nothing here performs audio work. The engine reads this table, the packager
//! renders `metadata/parameters.json` from it, and the web surface receives the
//! same schema back from the host, so the three can never drift apart.

#![no_std]

pub mod index;
pub mod pedal;
pub mod preset;

pub use pedal::{PEDAL_COUNT, PEDALS, PedalSpec, chain_order};
pub use preset::{PRESET_COUNT, PRESETS, Preset, settings_for};

/// Number of public parameters. Also the length of the state block in `f32`s.
pub const PARAMETER_COUNT: usize = 41;

/// Editor pages. RackForge renders them in `order`; the web surface uses the
/// same identifiers to group its pedal panels.
pub struct PageSpec {
    pub id: &'static str,
    pub name: &'static str,
    pub order: i32,
}

pub const PAGES: [PageSpec; 8] = [
    PageSpec {
        id: "rig",
        name: "Rig",
        order: 0,
    },
    PageSpec {
        id: "comp",
        name: "Compressor",
        order: 1,
    },
    PageSpec {
        id: "drive",
        name: "Overdrive",
        order: 2,
    },
    PageSpec {
        id: "dist",
        name: "Distortion",
        order: 3,
    },
    PageSpec {
        id: "fuzz",
        name: "Fuzz",
        order: 4,
    },
    PageSpec {
        id: "chorus",
        name: "Chorus",
        order: 5,
    },
    PageSpec {
        id: "delay",
        name: "Delay",
        order: 6,
    },
    PageSpec {
        id: "reverb",
        name: "Reverb",
        order: 7,
    },
];

/// The parameter kinds RF-Rig uses. This is a deliberate subset of the
/// RackForge parameter schema: no triggers and no meters, because a Rack Slot
/// edits its plugin through an isolated instance and cannot poll live values.
#[derive(Clone, Copy)]
pub enum Kind {
    Float {
        minimum: f32,
        maximum: f32,
        default: f32,
        step: f32,
        unit: Option<&'static str>,
    },
    Boolean {
        default: bool,
    },
    Integer {
        minimum: i32,
        maximum: i32,
        default: i32,
    },
    Enum {
        default: u32,
        choices: &'static [&'static str],
    },
}

/// Control hint published to RackForge surfaces (LITTLE, controller mappings).
#[derive(Clone, Copy)]
pub enum Control {
    Knob,
    Toggle,
    List,
}

impl Control {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Knob => "knob",
            Self::Toggle => "toggle",
            Self::List => "list",
        }
    }
}

pub struct ParameterSpec {
    pub index: u32,
    pub id: &'static str,
    pub name: &'static str,
    pub page: &'static str,
    pub order: i32,
    pub kind: Kind,
    pub control: Control,
}

impl ParameterSpec {
    pub const fn default_value(&self) -> f32 {
        match self.kind {
            Kind::Float { default, .. } => default,
            Kind::Boolean { default } => {
                if default {
                    1.0
                } else {
                    0.0
                }
            }
            Kind::Integer { default, .. } => default as f32,
            Kind::Enum { default, .. } => default as f32,
        }
    }

    /// Accepts a host value and returns the canonical stored value, or `None`
    /// when the value is outside the declared contract. RackForge validates
    /// first; this is the engine's own guard so a broken caller cannot poison
    /// the audio thread.
    pub fn canonicalize(&self, value: f64) -> Option<f32> {
        if !value.is_finite() {
            return None;
        }
        match self.kind {
            Kind::Float {
                minimum, maximum, ..
            } => {
                let value = value as f32;
                (value >= minimum && value <= maximum).then_some(value)
            }
            Kind::Boolean { .. } => {
                if value == 0.0 {
                    Some(0.0)
                } else if value == 1.0 {
                    Some(1.0)
                } else {
                    None
                }
            }
            Kind::Integer {
                minimum, maximum, ..
            } => {
                let rounded = value as i64;
                if value != rounded as f64 {
                    return None;
                }
                (rounded >= minimum as i64 && rounded <= maximum as i64).then_some(rounded as f32)
            }
            Kind::Enum { choices, .. } => {
                let rounded = value as i64;
                if value != rounded as f64 || rounded < 0 {
                    return None;
                }
                (rounded < choices.len() as i64).then_some(rounded as f32)
            }
        }
    }
}

const fn knob(
    index: u32,
    id: &'static str,
    name: &'static str,
    page: &'static str,
    order: i32,
    default: f32,
) -> ParameterSpec {
    ParameterSpec {
        index,
        id,
        name,
        page,
        order,
        kind: Kind::Float {
            minimum: 0.0,
            maximum: 1.0,
            default,
            step: 0.001,
            unit: None,
        },
        control: Control::Knob,
    }
}

const fn footswitch(index: u32, id: &'static str, page: &'static str) -> ParameterSpec {
    ParameterSpec {
        index,
        id,
        name: "Engaged",
        page,
        order: 0,
        kind: Kind::Boolean { default: false },
        control: Control::Toggle,
    }
}

const fn position(index: u32, id: &'static str, page: &'static str, default: i32) -> ParameterSpec {
    ParameterSpec {
        index,
        id,
        name: "Board Position",
        page,
        order: 1,
        kind: Kind::Integer {
            minimum: 1,
            maximum: PEDAL_COUNT as i32,
            default,
        },
        control: Control::List,
    }
}

pub const PARAMETERS: [ParameterSpec; PARAMETER_COUNT] = [
    // ---- Rig -----------------------------------------------------------
    ParameterSpec {
        index: 0,
        id: "rig.input",
        name: "Input Trim",
        page: "rig",
        order: 0,
        kind: Kind::Float {
            minimum: -24.0,
            maximum: 24.0,
            default: 0.0,
            step: 0.1,
            unit: Some("dB"),
        },
        control: Control::Knob,
    },
    ParameterSpec {
        index: 1,
        id: "rig.output",
        name: "Output Level",
        page: "rig",
        order: 1,
        kind: Kind::Float {
            minimum: -24.0,
            maximum: 24.0,
            default: 0.0,
            step: 0.1,
            unit: Some("dB"),
        },
        control: Control::Knob,
    },
    ParameterSpec {
        index: 2,
        id: "rig.gate",
        name: "Noise Gate",
        page: "rig",
        order: 2,
        kind: Kind::Float {
            minimum: -90.0,
            maximum: -20.0,
            default: -90.0,
            step: 0.5,
            unit: Some("dBFS"),
        },
        control: Control::Knob,
    },
    // ---- Compressor ----------------------------------------------------
    footswitch(3, "comp.engaged", "comp"),
    position(4, "comp.position", "comp", 1),
    knob(5, "comp.sustain", "Sustain", "comp", 2, 0.5),
    knob(6, "comp.attack", "Attack", "comp", 3, 0.5),
    knob(7, "comp.level", "Level", "comp", 4, 0.5),
    // ---- Overdrive -----------------------------------------------------
    footswitch(8, "drive.engaged", "drive"),
    position(9, "drive.position", "drive", 2),
    knob(10, "drive.drive", "Drive", "drive", 2, 0.5),
    knob(11, "drive.tone", "Tone", "drive", 3, 0.5),
    knob(12, "drive.level", "Level", "drive", 4, 0.5),
    // ---- Distortion ----------------------------------------------------
    footswitch(13, "dist.engaged", "dist"),
    position(14, "dist.position", "dist", 3),
    knob(15, "dist.distortion", "Distortion", "dist", 2, 0.5),
    knob(16, "dist.tone", "Tone", "dist", 3, 0.5),
    knob(17, "dist.level", "Level", "dist", 4, 0.5),
    // ---- Fuzz ----------------------------------------------------------
    footswitch(18, "fuzz.engaged", "fuzz"),
    position(19, "fuzz.position", "fuzz", 4),
    knob(20, "fuzz.sustain", "Sustain", "fuzz", 2, 0.6),
    knob(21, "fuzz.tone", "Tone", "fuzz", 3, 0.5),
    knob(22, "fuzz.volume", "Volume", "fuzz", 4, 0.4),
    // ---- Chorus --------------------------------------------------------
    footswitch(23, "chorus.engaged", "chorus"),
    position(24, "chorus.position", "chorus", 5),
    ParameterSpec {
        index: 25,
        id: "chorus.rate",
        name: "Rate",
        page: "chorus",
        order: 2,
        kind: Kind::Float {
            minimum: 0.05,
            maximum: 8.0,
            default: 0.8,
            step: 0.01,
            unit: Some("Hz"),
        },
        control: Control::Knob,
    },
    knob(26, "chorus.depth", "Depth", "chorus", 3, 0.5),
    knob(27, "chorus.mix", "Mix", "chorus", 4, 0.5),
    // ---- Delay ---------------------------------------------------------
    footswitch(28, "delay.engaged", "delay"),
    position(29, "delay.position", "delay", 6),
    ParameterSpec {
        index: 30,
        id: "delay.time",
        name: "Time",
        page: "delay",
        order: 2,
        kind: Kind::Float {
            minimum: 20.0,
            maximum: 1200.0,
            default: 380.0,
            step: 1.0,
            unit: Some("ms"),
        },
        control: Control::Knob,
    },
    knob(31, "delay.feedback", "Repeats", "delay", 3, 0.35),
    knob(32, "delay.mix", "Mix", "delay", 4, 0.3),
    ParameterSpec {
        index: 33,
        id: "delay.mode",
        name: "Mode",
        page: "delay",
        order: 5,
        kind: Kind::Enum {
            default: 0,
            choices: &["Analog (BBD)", "Digital"],
        },
        control: Control::List,
    },
    knob(34, "delay.width", "Width", "delay", 6, 0.0),
    // ---- Reverb --------------------------------------------------------
    footswitch(35, "reverb.engaged", "reverb"),
    position(36, "reverb.position", "reverb", 7),
    knob(37, "reverb.decay", "Decay", "reverb", 2, 0.4),
    knob(38, "reverb.tone", "Tone", "reverb", 3, 0.5),
    knob(39, "reverb.mix", "Mix", "reverb", 4, 0.25),
    ParameterSpec {
        index: 40,
        id: "reverb.mode",
        name: "Mode",
        page: "reverb",
        order: 5,
        kind: Kind::Enum {
            default: 0,
            choices: &["Spring", "Plate"],
        },
        control: Control::List,
    },
];

/// The flat parameter block. This *is* the plugin state: RF-Rig serialises it
/// as little-endian `f32`s, and migrations key off the byte length exactly as
/// the other RackForge plugins do.
#[derive(Clone, Copy)]
pub struct Settings {
    values: [f32; PARAMETER_COUNT],
}

impl Default for Settings {
    fn default() -> Self {
        let mut values = [0.0_f32; PARAMETER_COUNT];
        let mut index = 0;
        while index < PARAMETER_COUNT {
            values[index] = PARAMETERS[index].default_value();
            index += 1;
        }
        Self { values }
    }
}

impl Settings {
    pub fn set(&mut self, index: u32, value: f64) -> bool {
        let Some(spec) = PARAMETERS.get(index as usize) else {
            return false;
        };
        let Some(canonical) = spec.canonicalize(value) else {
            return false;
        };
        self.values[index as usize] = canonical;
        true
    }

    pub fn get(&self, index: u32) -> Option<f64> {
        self.values.get(index as usize).map(|value| *value as f64)
    }

    pub fn value(&self, index: u32) -> f32 {
        self.values[index as usize]
    }

    pub fn engaged(&self, index: u32) -> bool {
        self.values[index as usize] >= 0.5
    }

    pub fn selection(&self, index: u32) -> u32 {
        let value = self.values[index as usize];
        if value <= 0.0 { 0 } else { value as u32 }
    }

    pub fn as_array(&self) -> [f32; PARAMETER_COUNT] {
        self.values
    }

    pub fn from_array(values: [f32; PARAMETER_COUNT]) -> Option<Self> {
        let mut settings = Self::default();
        for (index, value) in values.into_iter().enumerate() {
            if !settings.set(index as u32, value as f64) {
                return None;
            }
        }
        Some(settings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_parameter_declares_its_own_index_and_a_known_page() {
        for (position, spec) in PARAMETERS.iter().enumerate() {
            assert_eq!(spec.index as usize, position, "{}", spec.id);
            assert!(
                PAGES.iter().any(|page| page.id == spec.page),
                "{} refers to an undeclared page",
                spec.id
            );
        }
    }

    #[test]
    fn parameter_identifiers_are_unique_and_host_legal() {
        for (position, spec) in PARAMETERS.iter().enumerate() {
            assert!(
                spec.id.bytes().all(|byte| byte.is_ascii_lowercase()
                    || byte.is_ascii_digit()
                    || b".-_".contains(&byte)),
                "{} is not a legal RackForge identifier",
                spec.id
            );
            for other in &PARAMETERS[position + 1..] {
                assert_ne!(spec.id, other.id);
            }
        }
    }

    #[test]
    fn defaults_round_trip_through_validation() {
        let settings = Settings::default();
        let restored = Settings::from_array(settings.as_array()).expect("defaults are valid");
        assert_eq!(settings.as_array(), restored.as_array());
    }

    #[test]
    fn out_of_contract_values_are_rejected() {
        let mut settings = Settings::default();
        assert!(!settings.set(0, 200.0));
        assert!(!settings.set(3, 0.5));
        assert!(!settings.set(33, 7.0));
        assert!(!settings.set(PARAMETER_COUNT as u32, 0.0));
        assert!(settings.set(33, 1.0));
    }
}
