# Architecture

## Crates

```text
crates/rf-rig-contract   parameters, pedal slots, chain order, factory boards
crates/rf-rig-dsp        circuit primitives, the seven pedals, the engine
crates/rf-rig-lab        metadata generator and measurement bench (native)
plugin/                  the RackForge wasm-v1 adapter and the package
```

`rf-rig-contract` and `rf-rig-dsp` are `no_std` and allocation-free; `libm`
supplies the maths so the desktop tests and the packaged component run the
*same* arithmetic. A measurement taken natively therefore says something about
the wasm build, which is not true if the two use different `sin`.

## One source of truth per fact

* **What a parameter is** — `rf-rig-contract`. The engine reads it, the packager
  renders `metadata/parameters.json` from it, and the web surface receives that
  same schema back from the host. Adding a knob in the contract makes it appear
  in the interface with no other change.
* **What version this is** — `plugin/package/rackforge-plugin.toml`. The runtime
  descriptor is generated from it, because RackForge rejects a package whose
  descriptor and manifest disagree and reports it in a dialog rather than on the
  console.
* **What a factory board is** — `rf-rig-contract::preset`. The engine loads it
  and the packager renders the catalog from the same table.

`cargo run -p rf-rig-lab -- metadata --check` fails when any generated document
has drifted; the build script regenerates before packing, and CI checks.

## Signal flow

```text
guitar in (mono)
   │  input trim, noise gate
   ▼
 chain, ordered by each pedal's `position` parameter
   │  compressor · overdrive · distortion · fuzz · chorus · delay · reverb
   ▼
 output level → stereo out
```

The frame carries a `stereo` flag. Until something widens the signal the chain
runs one channel, which halves the work of every clipper in front of it. A pedal
with no stereo behaviour sums the frame back down before it runs — which is what
a real one-input pedal does, so putting the fuzz after the delay collapses the
image on purpose.

## Memory

Nothing allocates after activation. The delay lines, the bucket-brigade
register and the reverb tank all borrow slices of one block that the plugin
hands the engine when the host activates it (`rf-rig-dsp::workspace`). The block
is sized for the highest sample rate the plugin accepts, and activation above
that is refused rather than silently shortening every delay.

The block is a `static` in the plugin crate rather than a field of the
processor. That is not a style choice: building a megabyte of buffers on
WebAssembly's shadow stack is how a component stops instantiating while every
native test still passes. `crates/rf-rig-lab/tests/component_instantiates.rs`
loads the built component in the host's own runtime and proves it still starts.

On native targets — the tests and the lab tool, where several engines live in
one process — each engine gets its own block instead.

## Host integration

The package declares itself an effect with a mono input bus and a stereo output
bus (Plugin API 1.9). RackForge owns the interface: the plugin never enumerates
a device, and the host maps whatever the player selected into that bus.

The PLAY surface is static HTML in the package, sandboxed in an iframe, talking
to the host over `rackforge.plugin.web@1`. It builds itself from the schema, so
it holds no private list of pedals — the only convention it relies on is that
`<pedal>.engaged` is the footswitch and `<pedal>.position` is the slot.

Reordering writes positions. There is no side channel: the host validates each
write, the values live in plugin state, and a controller could automate them.

## Real-time behaviour

* No allocation, no locking, no I/O, no logging in `process`.
* Newton iterations are capped, so the worst case is bounded.
* Every feedback path is sanitised; a non-finite sample cannot become permanent.
* Parameter events are applied at their exact frame, so automation is
  sample-accurate.
