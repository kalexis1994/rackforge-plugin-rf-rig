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
* Tests: 137 covering the contract, each circuit block, each pedal's behaviour,
  the chain and the adapter — plus a guard that loads the built component in the
  host's own runtime, and a bench that reports what the board costs.

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

## Fidelity work

### Done, and what it bought

1. **Tone stacks solved as networks** (`circuit/tonestack.rs`). Three nodes, the
   pot's two halves and the following stage's load, eliminated into a biquad and
   checked against a direct complex solve of the same netlist at every control
   position (within 0.15 dB). The midrange scoop and its travel across the
   control are now consequences of component values. The fuzz's values are the
   published ones; the distortion's and overdrive's are the right topology with
   representative values, which is a six-number change away from canonical.
2. **The compressor's gain cell** (`circuit/ota.rs`). The real transconductance
   equation, with the loop's ceiling derived rather than tuned. Before this, the
   pedal produced four decibels of gain reduction across a thirty-seven decibel
   input range; it now compresses roughly 4.5:1 at the top of the sustain
   control and stays under 0.2 % distorted until a transient reaches the cell's
   knee.
3. **Transistor stages from Ebers-Moll** (`circuit/transistor.rs`). Three
   unknowns, three node equations, the clipping diodes inside the same solve,
   and the input coupling capacitor integrated alongside. The fuzz's clipping
   stages now bias at 0.97 V — the low collector voltage this family's service
   notes report — and clip asymmetrically as a consequence.
4. **Two instrument faults fixed**, both of which had silently coloured every
   earlier measurement: spectral leakage giving a 0.62 % distortion floor, and
   single-sample probing of a stateful shaper reading half the gain. See
   [`MEASUREMENT.md`](MEASUREMENT.md).
5. **A numerical bug in the antialiased shaper**: its fallback threshold was
   absolute, so small signals divided two nearly equal antiderivatives and
   produced noise. A two-millivolt sine through the compressor was more noise
   than signal.

6. **Loading, both kinds** (`circuit/source.rs`, and impedance carried along
   the chain). Inside the board, a transistor input absorbs the previous
   pedal's output impedance, so a lossy source costs the fuzz gain and moves
   its bass corner down. Ahead of the board, a `Source` control says what is
   plugged in, and the pickup's resonance is damped by whatever the first
   pedal presents — modelled as the *difference* between that load and the one
   the signal was captured through, so it collapses to a wire when they match.

7. **The compressor's detector** (`circuit/rectifier.rs`). A diode charging a
   timing capacitor, solved, in place of an envelope follower's coefficient
   pair. It brought a real threshold — under it the pedal is a clean gain stage
   whatever the sustain control says — a level-dependent attack, and an RC
   release. The ratio now runs from 1.08:1 at the bottom of the control to 8:1
   at the top, where before the whole travel was one shape.

8. **The op-amp stages** (`circuit/opamp.rs`). Finite gain-bandwidth, slew
   rate, supply, and both capacitors around the loop, solved together with the
   diodes. The overdrive now loses its top as the drive comes up, because the
   loop runs out of authority at 13 kHz — which is where a megahertz of
   gain-bandwidth lands against a noise gain of 76. It also found a slow
   instability: solving for the inverting node let the amplifier's per-step
   gain magnify the solver's error into the next sample, and the output grew
   over seconds. Solving for the output fixed it and was faster.

### Next, in order of expected audible return

1. **Component tolerance.** A seed per instance, values drifting inside their
   stated tolerance, so two instances are not identical — the reason two units
   of the same pedal never quite match.
2. **A clock-accurate bucket-brigade line.** Resampling at the clock rate rather
   than reading a fractional delay: the aliasing of a real BBD is part of its
   sound, and the current model band-limits it away.
3. **Canonical values for the remaining two tone networks**, from a trace or a
   published analysis.

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

Measured, per 512-frame block at 48 kHz on the development desktop: the whole
board engaged costs 11.2 % of one core, and the fuzz — three transistor stages,
each solved four times per sample — is 5.0 % of that. The table is in
[`MEASUREMENT.md`](MEASUREMENT.md), and the bench that produces it is
`cargo test --release -p rf-rig-dsp --test bench_blocks -- --ignored --nocapture`.

Two decisions already came out of it. Newton runs two iterations per sample, not
three, because a test against a twelve-iteration reference puts the difference
under 1 %; that took the fuzz from 808 to 535 microseconds. And the 3x3 solve
stays on Cramer's rule, because Gaussian elimination — a third of the arithmetic
— was 57 % *slower* on this machine: its pivot search branches on data.

What is left, if a Raspberry Pi asks for more: a faster `exp` (the solvers are
dominated by it, and a diode model does not need `libm` accuracy), 2x
oversampling for the soft-clipping stages, or `+simd128` for the wasm build.
Measure on the Pi before choosing.
