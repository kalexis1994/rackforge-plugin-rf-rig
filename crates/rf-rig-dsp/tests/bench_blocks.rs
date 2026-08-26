//! What the board costs, per pedal and together.
//!
//! Run it deliberately, because it is a measurement and not a check:
//!
//! ```text
//! cargo test --release -p rf-rig-dsp --test bench_blocks -- --ignored --nocapture
//! ```
//!
//! The budget is one audio block. At 48 kHz a 512-frame block must be finished
//! in 10.67 ms or the host underruns, so the percentages below are the fraction
//! of one core this board would take on this machine. A Raspberry Pi is several
//! times slower, which is the number that actually decides what ships — measure
//! there before believing anything here about headroom.

use std::time::Instant;

use rf_rig_contract::index::*;
use rf_rig_dsp::{Engine, WORKSPACE_SAMPLES};

const SAMPLE_RATE: f64 = 48_000.0;
const BLOCK: usize = 512;
const BLOCKS: usize = 400;

fn budget_micros() -> f64 {
    BLOCK as f64 / SAMPLE_RATE * 1.0e6
}

fn measure(name: &str, settings: &[(u32, f64)]) {
    let mut memory = vec![0.0_f32; WORKSPACE_SAMPLES];
    let mut engine = Engine::default();
    assert!(engine.prepare_with(SAMPLE_RATE, &mut memory));
    for (index, value) in settings {
        assert!(engine.set_parameter(*index, *value), "parameter {index}");
    }

    // A signal that keeps every detector and every clipper busy.
    let input: Vec<f32> = (0..BLOCK)
        .map(|index| {
            let phase = std::f32::consts::TAU * 196.0 * index as f32 / SAMPLE_RATE as f32;
            0.2 * phase.sin()
        })
        .collect();

    // Warm up: settle the detectors and let the branch predictor see the loop.
    for _ in 0..40 {
        for sample in &input {
            std::hint::black_box(engine.process(*sample));
        }
    }

    let start = Instant::now();
    for _ in 0..BLOCKS {
        for sample in &input {
            std::hint::black_box(engine.process(*sample));
        }
    }
    let elapsed = start.elapsed().as_secs_f64() * 1.0e6 / BLOCKS as f64;
    println!(
        "{name:<22} {elapsed:8.1} us/block   {:5.1} % of one core",
        100.0 * elapsed / budget_micros()
    );
}

#[test]
#[ignore = "a measurement, not a check"]
fn what_the_board_costs() {
    println!(
        "\n512-frame block at 48 kHz: {:.2} ms of budget\n",
        budget_micros() / 1000.0
    );

    measure("empty board", &[]);
    measure("compressor", &[(COMP_ENGAGED, 1.0), (COMP_SUSTAIN, 0.7)]);
    measure("overdrive", &[(DRIVE_ENGAGED, 1.0), (DRIVE_DRIVE, 0.7)]);
    measure("distortion", &[(DIST_ENGAGED, 1.0), (DIST_DISTORTION, 0.7)]);
    measure("fuzz", &[(FUZZ_ENGAGED, 1.0), (FUZZ_SUSTAIN, 0.8)]);
    measure("chorus", &[(CHORUS_ENGAGED, 1.0), (CHORUS_MIX, 0.6)]);
    measure(
        "delay (analog)",
        &[
            (DELAY_ENGAGED, 1.0),
            (DELAY_MIX, 0.5),
            (DELAY_FEEDBACK, 0.5),
        ],
    );
    measure(
        "delay (digital)",
        &[
            (DELAY_ENGAGED, 1.0),
            (DELAY_MODE, 1.0),
            (DELAY_MIX, 0.5),
            (DELAY_FEEDBACK, 0.5),
        ],
    );
    measure(
        "reverb (spring)",
        &[(REVERB_ENGAGED, 1.0), (REVERB_MIX, 0.5)],
    );
    measure(
        "reverb (plate)",
        &[(REVERB_ENGAGED, 1.0), (REVERB_MODE, 1.0), (REVERB_MIX, 0.5)],
    );
    measure(
        "everything engaged",
        &[
            (COMP_ENGAGED, 1.0),
            (DRIVE_ENGAGED, 1.0),
            (DIST_ENGAGED, 1.0),
            (FUZZ_ENGAGED, 1.0),
            (CHORUS_ENGAGED, 1.0),
            (DELAY_ENGAGED, 1.0),
            (REVERB_ENGAGED, 1.0),
        ],
    );
    println!();
}
