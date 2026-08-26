//! The packaged component must be able to start, prepare and produce audio.
//!
//! Native tests prove the engine; they do not prove the *component*. A wasm
//! module has a small shadow stack, and a processor that builds large state on
//! it traps inside `default()` with an out-of-bounds access — while every
//! native test still passes. RackForge has been bitten by exactly that, three
//! releases in a row, so RF-Rig keeps the delay memory in a `static` and keeps
//! this test to prove the arrangement still holds.
//!
//! It runs the real host runtime, so a pass here means the plugin starts on
//! desktop, Raspberry Pi and Android alike. It skips quietly when the wasm has
//! not been built, so it costs nothing during ordinary work.

use std::path::PathBuf;

use rackforge_plugin_runtime::{PortableEngine, RuntimeLimits};

fn wasm_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/wasm32-unknown-unknown/release/rackforge_rf_rig.wasm")
}

#[test]
fn the_packaged_board_can_start_and_pass_audio() {
    let path = wasm_path();
    if !path.is_file() {
        eprintln!(
            "skipping: no wasm at {}. Build it with\n  \
             cargo build --release --target wasm32-unknown-unknown -p rackforge-rf-rig",
            path.display()
        );
        return;
    }

    let runtime = PortableEngine::new(RuntimeLimits::default()).expect("runtime");
    let module = runtime
        .compile(&std::fs::read(&path).expect("read wasm"))
        .expect("the component must compile");

    let mut instance = module
        .instantiate()
        .expect("the board must instantiate; a trap here means the shadow stack overflowed");
    instance
        .prepare(48_000.0, 512, 1, 2)
        .expect("the board must prepare for a mono input and a stereo output");

    // Engage every pedal, then ask for audio. This is the pass that actually
    // touches the delay memory, which is where an undersized workspace or a
    // bad borrow would show up.
    for parameter in rf_rig_contract::PEDALS.iter() {
        instance
            .set_parameter(parameter.engaged, 1.0)
            .expect("engaging a pedal must be accepted");
    }

    let frames = 512;
    let input = vec![0.1_f32; frames];
    let mut output = vec![0.0_f32; frames * 2];
    for _ in 0..8 {
        instance
            .process_interleaved(&input, &mut output, frames as u32)
            .expect("the board must process a block");
    }
    assert!(
        output.iter().all(|sample| sample.is_finite()),
        "the component produced a non-finite sample"
    );
    assert!(
        output.iter().any(|sample| sample.abs() > 1.0e-6),
        "the component produced silence with every pedal engaged"
    );
}
