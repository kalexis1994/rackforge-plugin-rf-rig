# Contributing to RF-Rig

## The rule that matters most

A constant that shapes the sound should be traceable to something outside
somebody's taste: a component value, a datasheet number, a corner frequency, a
measurement. When that is not yet true, say so where the code is — the fuzz tone
stack and the transistor booster both carry that admission today, and
[`docs/CIRCUIT_MODELING.md`](docs/CIRCUIT_MODELING.md) keeps the ledger.

Raising an uncalibrated constant until something happens is how a model stops
being a model.

## Working on it

```bash
cargo test --workspace          # everything
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
```

Before opening a pull request, build the package once — it regenerates the
metadata, runs the tests, and proves the component still packs:

```bash
pwsh tools/build-package.ps1     # or: bash tools/build-package.sh
```

On this machine the Rust default toolchain is windows-gnu and the RackForge
runtime only builds under MSVC, so pass `+stable-x86_64-pc-windows-msvc` (the
build script does it for you).

## Generated files are generated

`plugin/package/metadata/*.json` and `plugin/package/branding/*.png` are outputs.
Edit the contract or the generator, never the JSON:

```bash
cargo run -p rf-rig-lab -- metadata
cargo run -p rf-rig-lab -- metadata --check    # what CI runs
python tools/generate-branding.py
```

The version lives in `plugin/package/rackforge-plugin.toml` and nowhere else;
the runtime descriptor is generated from it.

## Adding a pedal

1. Add its parameters to `rf-rig-contract` (`lib.rs`, `index.rs`, `pedal.rs`)
   and bump `PARAMETER_COUNT`. State is a flat block of `f32`s keyed by length,
   so adding parameters is a state-version change: bump `state_version` in the
   manifest and add the migration.
2. Write the pedal in `rf-rig-dsp/src/pedals/`, with the component values it
   derives from as named constants.
3. Wire it into `engine.rs`: a slot number, an arm in the chain match, and its
   `set_controls` call in `apply_settings`.
4. Give it memory in `workspace.rs` if it needs a delay line.
5. Tests: what the circuit is supposed to *do*, not what the code currently
   returns. "More drive means more harmonic content", "the tone control moves
   treble without moving the body", "it stays bounded on a hot input".
6. `cargo run -p rf-rig-lab -- metadata`.

The interface needs no change: it builds itself from the schema.

## Tests

Prefer a test that would fail if the physics were wrong over one that pins the
current output. Every pedal has at least: it changes the signal when engaged, a
control does what its name says, and it stays finite and bounded when everything
is at maximum.

Numbers in assertions should be justified in a comment — a crest factor of 1.414
is a sine, 1.0 is a square, and that is why the threshold is where it is.

## Style

The house voice is descriptive, never boastful. Comments explain *why* a value
is what it is, not what the line does. No trademarks, no brand names, no product
names — see [`docs/REFERENCES.md`](docs/REFERENCES.md).
