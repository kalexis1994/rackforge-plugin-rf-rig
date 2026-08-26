# How a pedal becomes a model

This is the method RF-Rig follows, and the honest ledger of where it currently
stops.

## The starting point is a netlist, not an opinion

A synthesizer like the Prophet-5 or the Juno-106 keeps its personality in
firmware: voice assignment, auto-tune, envelope shapes computed by a CPU. That
is why reverse engineering one begins with a ROM dump.

An analog stompbox has no firmware to dump. Everything it does is in the
circuit — perhaps thirty components — and that circuit is public knowledge:
community traces of unpotted units, component-level analyses, expired patents,
and service documentation. The equivalent of "dump the ROM" is "read the
schematic", and the equivalent of "emulate the CPU" is "solve the circuit".

That is a well-established discipline, not an invention of this project. The
techniques it draws on are nodal state-space models (the DK method), wave
digital filters, and antiderivative antialiasing for memoryless shapers.
Sources are listed in [`REFERENCES.md`](REFERENCES.md).

## The one equation that does most of the work

Every clipping stage here reduces to a single node. Whatever current the stage
pushes into the clipping network has to leave through the resistor or through
the diodes:

```text
I_in = V/R + 2·Is·sinh(V / (n·Vt))
```

* `Is` — reverse saturation current, from the diode's datasheet
  (1N4148: 2.52 nA; germanium: microamps; a red LED: essentially zero until
  1.7 V).
* `n·Vt` — emission coefficient times thermal voltage, 45 mV for a 1N4148 at
  room temperature.
* `R` — the resistor the diodes sit across, from the schematic.

RF-Rig solves this with Newton's method, warm-started from the previous sample
(`circuit/nonlinear.rs`). Two iterations are normally enough; six is the cap, so
the audio callback has a bounded worst case. One exponential serves both
hyperbolic functions.

Two topologies fall out of the same solver:

* **Diodes across the feedback resistor** of an op-amp stage. The output is the
  input *plus* the drop the network cannot avoid, so the stage never stops
  following its input: a soft knee, and a pedal that cleans up when the guitar
  volume comes down. This is the overdrive.
* **Diodes from the node to ground** behind a series resistor. Once the node
  reaches the forward drop it cannot go further: a hard corner, dense odd
  harmonics, and a pedal whose dirt barely changes with input level. This is the
  distortion.

Nothing about those two descriptions is programmed. They are what the equation
does in each position.

## Where the frequency response comes from

A guitar pedal is mostly RC pairs, and the interesting ones are *inside* the
gain path rather than after it:

* The overdrive's inverting input sees 4.7 kΩ in series with 47 nF. The corner
  is 1/(2π·4.7k·47n) ≈ 720 Hz, and below it the stage barely amplifies at all.
  That single pair is the mid-hump the whole pedal is known for, and the reason
  a low E stays defined instead of turning to mud.
* The distortion's equivalent pair uses 0.47 µF, putting its corner near 72 Hz —
  which is why the same topology sounds so much bigger in the bass.
* A fuzz's clipping stages are coupled at ~160 Hz and rolled off at ~3.4 kHz by
  the 470 pF across the feedback resistor.

Every one of those numbers is a component value, and each appears as a named
constant next to the comment that derives it.

## Sampling: why oversampling is not optional

A hard knee generates harmonics far above the audio band. Evaluated at 48 kHz
they fold back as inharmonic tones no analog circuit produces — the single most
recognisable "digital distortion" artefact.

Every clipping stage therefore runs at four times the host rate through a
polyphase half-band interpolator (`circuit/oversample.rs`), with the filters
inside the oversampled path configured at the oversampled rate. A test measures
the alias that lands at 13 kHz from a 7 kHz tone and requires the oversampled
path to beat the naive one.

Where a Newton solve would be wasted — the soft limit inside a delay's feedback
loop, the compressor's recovery — the code uses first-order antiderivative
antialiasing instead, which removes most of the folding for the price of one
sample of state.

## Bucket-brigade lines

The chorus and the analog echo share a model of a clocked analog shift register
(`circuit/delay.rs`):

* the delay is set by the clock, so *moving the clock* moves the pitch — which
  is why an analog chorus sounds like a second player and a crossfading digital
  one does not;
* the register is band-limited on both sides;
* a compander surrounds it, because the device's own noise floor is otherwise
  audible on every repeat. The compressor and expander are not perfectly
  complementary, and that mismatch is part of the character.

The echo's feedback path is filtered and softly limited *inside* the loop, so
each repeat is darker than the last and a runaway howls instead of tearing.

## What is measured, and what is still assumed

Honest ledger, as of `v0.1.0`:

**Derived from circuit values**
* Both clipping topologies and all three diode options.
* The overdrive's and distortion's frequency-dependent gain networks.
* The fuzz's two-stage cascade and its interstage coupling.
* The BBD sweep, band-limiting and companding structure.
* The echo's in-loop filtering and its two modes.

**Structurally right, numerically approximate — and marked as such in the code**
* The fuzz tone stack is a two-path blend plus a fixed midrange cut. A real one
  is a resistive junction whose notch depth moves with the control; it wants a
  nodal solve.
* The distortion's tone network is a tilt plus a fixed scoop, for the same
  reason.
* The compressor's detector law is a plausible feedback loop rather than a
  modelled OTA control port.
* The transistor booster is a soft asymmetric limit rather than an Ebers-Moll
  stage.
* The spring reverb's dispersion is an all-pass chain chosen by ear-shaped
  reasoning, not fitted to a measured tank.

**Not attempted yet**
* Component tolerance and temperature drift.
* Loading between pedals: a real chain's input and output impedances interact,
  which is why a fuzz behaves differently after a buffer.
* Power supply sag.

## How to move an item up that list

Measure both sides of the same experiment. The bench procedure is in
[`MEASUREMENT.md`](MEASUREMENT.md); the trap it exists to avoid is comparing a
model and an instrument under different conditions and then chasing the
difference. Sweep against the render, not against the formula: what a transfer
function returns is not what is heard until the rest of the chain has multiplied
it.
