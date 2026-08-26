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

### What the surface has to survive

A Rack Slot edits its plugin through an isolated instance: every
`set_parameter` opens a plugin, loads state, applies one value and saves it
again. Every one of those moves the session revision, and every revision sends
a fresh context to every surface — including the one that caused it. So the
page is not talking to a fast, quiet host; it is talking to a slow one that
answers back louder than it was asked. Four rules follow.

**Build once, patch after.** The board's DOM is constructed when the schema
arrives and only ever patched. Rebuilding it on each context — which is to say,
on each of the page's own writes — would destroy the control under the hand
between one frame of a drag and the next.

**Pace the writes, and coalesce them.** Values are queued per parameter and
sent one at a time, at most sixteen a second, with the interval enforced
*between* writes rather than between flushes. Pacing off the host's replies
instead would make a fast host the worst case: answer in fifteen milliseconds
and the page would gratefully ask sixty-six times a second. Waiting also lets a
knob still in motion overwrite its own queued value, so eighty pointer moves
across one gesture become one write carrying the value the gesture ended on.

**A value being edited belongs to whoever is editing it.** While a control is
held, incoming values for that parameter are ignored, and reads are deferred
entirely. Gestures are registered so that any of them can be ended from
outside — a window that loses focus mid-drag would otherwise leave a control
held forever, and a held control never accepts another value from the host.

**Believe a reply only if it describes a later moment than the last write.**
Reads carry the write epoch they were issued at; a value written since is not
overwritten by an answer that predates it. Checking the queue is not enough,
because between leaving the queue and being acknowledged a write exists nowhere
a reply can see it. Every request also times out: there is no message that says
the plugin died, only silence, and a queue that waits forever on silence never
moves again.

`tools/ui-preview.html` reproduces all of it — latency, refusals, a context
after every write, and a host that stops answering — because a surface that
only works against an instant, perfect host has not been tested.

## Real-time behaviour

* No allocation, no locking, no I/O, no logging in `process`.
* Newton iterations are capped, so the worst case is bounded.
* Every feedback path is sanitised; a non-finite sample cannot become permanent.
* Parameter events are applied at their exact frame, so automation is
  sample-accurate.
