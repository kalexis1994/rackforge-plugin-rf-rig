# Implementation plan

## v0.1.0 — done

* Contract: 41 parameters, seven pedal slots, chain order as parameter data,
  six factory boards.
* Engine: circuit-derived compressor, overdrive, distortion, fuzz, BBD chorus,
  BBD/digital echo, spring/plate reverb; oversampled clipping; allocation-free
  workspace.
* Adapter: `wasm-v1` component, sample-accurate automation, state save/load.
* Package: schema-2 manifest declaring a mono input and stereo output bus,
  generated metadata, generated branding, static PLAY surface.
* Tests: 82 covering the contract, each circuit block, each pedal's behaviour,
  the chain and the adapter — plus a guard that loads the built component in the
  host's own runtime.

## Next: make it playable

**1. The host has to offer effects in a Rack Slot.** RackForge's engine already
runs them — `rack_graph.rs` compiles a hardware audio input into a plugin node,
`live.rs` mixes it, and a test named `compiles_hardware_audio_input_into_an_effect`
builds exactly this graph on a rack it calls "pedalboard". The gap is in the Web
UI: `web/src/rackInstrumentSelection.ts` filters the picker to
`kind === "instrument"`, and `RackGraphEditor.tsx` offers "Instrument" and
"Audio Input" but no "Effect". Both need to change, plus whatever the Rack Slot
popover shows for a node with no MIDI input.

**2. Live knob response in a Rack Slot.** Slot-bound editing currently goes
through an isolated instance: a knob turn edits stored state rather than the
running voice ([rackforge#19](https://github.com/kalexis1994/rackforge/issues/19)).
For an instrument that is tolerable; for a pedalboard it is the difference
between a plugin you can play and one you can only configure. Worth measuring
before designing around it.

**3. Play it, then fix what the ear finds.** With a real guitar through the
Focusrite, at a real buffer size. Nothing below this line should be trusted over
that.

## Fidelity work, in order of expected audible return

1. **Tone stacks solved as networks.** The fuzz and distortion tone controls are
   the largest approximations left, and they are what a player hears first. A
   nodal solve of the two-branch junction, checked against the published
   transfer function of the circuit family.
2. **Loading between pedals.** A real chain interacts through input and output
   impedances; that is why a fuzz behaves differently after a buffered pedal.
   The chain would carry a source impedance alongside the sample.
3. **A proper transistor stage.** Ebers-Moll with emitter degeneration, instead
   of the asymmetric soft limit standing in for the booster.
4. **A clock-accurate bucket-brigade line.** Resampling at the clock rate rather
   than reading a fractional delay: the aliasing of a real BBD is part of its
   sound, and the current model band-limits it away.
5. **Component tolerance.** A seed per instance, drifting values within their
   stated tolerance, so two instances are not identical — the reason two units
   of the same pedal never quite match.

## Pedals not yet on the board

* **Tuner.** Deliberately deferred: the display needs live values from the
  running instance, and a Rack Slot surface reads an isolated one. It becomes
  straightforward once item 2 above is settled.
* **Phaser / tremolo.** An OTA or JFET all-pass ladder swept by an LFO; the same
  primitives are already here.
* **Wah.** A resonant band-pass with a real inductor model, and a pedal position
  parameter that a controller can sweep.
* **Amp and cabinet.** The user chose pedals first for v0.1. A preamp stage plus
  convolution against user-supplied impulse responses (through RackForge's
  `[[resources]]` mechanism, so the package ships no captured audio) is the
  natural next package — or a second plugin, since a rig is a chain and the host
  already knows how to chain.

## Performance

Not yet profiled. The clipping solve is the hot spot: four oversampled
sub-samples per sample per engaged dirt pedal, each with up to six Newton
iterations and one exponential apiece. Options, cheapest first: cap iterations
at three once convergence is measured, drop to 2× oversampling for the
soft-clipping stages, enable `+simd128` for the wasm build, or precompute the
diode branch as a table. Measure on the Raspberry Pi before choosing.
