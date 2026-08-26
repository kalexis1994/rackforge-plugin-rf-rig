//! The pedals on the board and the rule that turns their `position`
//! parameters into a signal chain.
//!
//! Order is part of the parameter space on purpose. It survives in plugin
//! state, RackForge validates it like any other parameter, a controller can
//! automate it, and the web surface reorders the board by writing positions
//! instead of inventing a private side channel.

use crate::Settings;

pub const PEDAL_COUNT: usize = 7;

/// A pedal slot: its identity, the circuit family it models, and the
/// parameters that belong to it.
pub struct PedalSpec {
    pub id: &'static str,
    pub name: &'static str,
    /// The circuit family this pedal is derived from. Descriptive, never a
    /// trademark: RF-Rig models topologies, not products.
    pub circuit: &'static str,
    pub page: &'static str,
    pub engaged: u32,
    pub position: u32,
    pub controls: &'static [u32],
}

pub const PEDALS: [PedalSpec; PEDAL_COUNT] = [
    PedalSpec {
        id: "comp",
        name: "Compressor",
        circuit: "OTA feedback compressor with a peak detector",
        page: "comp",
        engaged: 3,
        position: 4,
        controls: &[5, 6, 7],
    },
    PedalSpec {
        id: "drive",
        name: "Overdrive",
        circuit: "Op-amp stage with symmetric diode clipping in the feedback loop",
        page: "drive",
        engaged: 8,
        position: 9,
        controls: &[10, 11, 12],
    },
    PedalSpec {
        id: "dist",
        name: "Distortion",
        circuit: "Transistor booster into an op-amp stage with hard clipping to ground",
        page: "dist",
        engaged: 13,
        position: 14,
        controls: &[15, 16, 17],
    },
    PedalSpec {
        id: "fuzz",
        name: "Fuzz",
        circuit: "Two cascaded feedback-clipping stages with a lowpass/highpass blend tone stack",
        page: "fuzz",
        engaged: 18,
        position: 19,
        controls: &[20, 21, 22],
    },
    PedalSpec {
        id: "chorus",
        name: "Chorus",
        circuit: "Bucket-brigade delay line with companding and a swept clock",
        page: "chorus",
        engaged: 23,
        position: 24,
        controls: &[25, 26, 27],
    },
    PedalSpec {
        id: "delay",
        name: "Delay",
        circuit: "Bucket-brigade echo with band-limited repeats, or a clean digital line",
        page: "delay",
        engaged: 28,
        position: 29,
        controls: &[30, 31, 32, 33, 34],
    },
    PedalSpec {
        id: "reverb",
        name: "Reverb",
        circuit: "Dispersive spring tank or a feedback delay network plate",
        page: "reverb",
        engaged: 35,
        position: 36,
        controls: &[37, 38, 39, 40],
    },
];

/// Resolves the board order. Pedals are sorted by their `position` parameter;
/// equal positions keep declaration order, so a collision degrades into a
/// stable chain rather than an ambiguous one.
pub fn chain_order(settings: &Settings) -> [usize; PEDAL_COUNT] {
    let mut order = [0_usize; PEDAL_COUNT];
    for (slot, entry) in order.iter_mut().enumerate() {
        *entry = slot;
    }
    // Insertion sort: PEDAL_COUNT is tiny, the pass is allocation-free, and it
    // is naturally stable.
    let mut index = 1;
    while index < PEDAL_COUNT {
        let candidate = order[index];
        let candidate_key = settings.value(PEDALS[candidate].position);
        let mut scan = index;
        while scan > 0 {
            let previous = order[scan - 1];
            if settings.value(PEDALS[previous].position) <= candidate_key {
                break;
            }
            order[scan] = previous;
            scan -= 1;
        }
        order[scan] = candidate;
        index += 1;
    }
    order
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PARAMETER_COUNT, PARAMETERS};

    #[test]
    fn every_parameter_belongs_to_the_rig_page_or_to_exactly_one_pedal() {
        for spec in PARAMETERS.iter() {
            if spec.page == "rig" {
                continue;
            }
            let owners = PEDALS
                .iter()
                .filter(|pedal| {
                    pedal.engaged == spec.index
                        || pedal.position == spec.index
                        || pedal.controls.contains(&spec.index)
                })
                .count();
            assert_eq!(owners, 1, "{} is owned by {owners} pedals", spec.id);
        }
    }

    #[test]
    fn pedal_parameter_indexes_stay_inside_the_contract() {
        for pedal in PEDALS.iter() {
            assert!((pedal.engaged as usize) < PARAMETER_COUNT);
            assert!((pedal.position as usize) < PARAMETER_COUNT);
            assert_eq!(PARAMETERS[pedal.engaged as usize].page, pedal.page);
            for control in pedal.controls {
                assert_eq!(PARAMETERS[*control as usize].page, pedal.page);
            }
        }
    }

    #[test]
    fn default_positions_produce_the_declaration_order() {
        let settings = Settings::default();
        assert_eq!(chain_order(&settings), [0, 1, 2, 3, 4, 5, 6]);
    }

    #[test]
    fn moving_the_delay_to_the_front_reorders_the_chain() {
        let mut settings = Settings::default();
        // The compressor already sits at position 1, so the delay has to take
        // that slot and push the compressor back for the move to be
        // unambiguous.
        assert!(settings.set(PEDALS[5].position, 1.0));
        assert!(settings.set(PEDALS[0].position, 2.0));
        let order = chain_order(&settings);
        assert_eq!(order, [5, 0, 1, 2, 3, 4, 6]);
    }

    #[test]
    fn a_tie_keeps_the_earlier_pedal_first() {
        let mut settings = Settings::default();
        // Delay onto the compressor's slot, without moving the compressor.
        assert!(settings.set(PEDALS[5].position, 1.0));
        let order = chain_order(&settings);
        assert_eq!(order, [0, 5, 1, 2, 3, 4, 6]);
    }

    #[test]
    fn colliding_positions_fall_back_to_declaration_order() {
        let mut settings = Settings::default();
        for pedal in PEDALS.iter() {
            assert!(settings.set(pedal.position, 3.0));
        }
        assert_eq!(chain_order(&settings), [0, 1, 2, 3, 4, 5, 6]);
    }
}
