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

## Four devices, four equations

Every nonlinear thing in RF-Rig is one of these, solved rather than shaped.

### The diode pair

```text
I_in = V/R + 2·Is·sinh(V / (n·Vt))
```

`Is` and `n` come from a datasheet (1N4148: 2.52 nA, 1.752), `R` from the
schematic. Newton's method, warm-started from the previous sample, in
`circuit/nonlinear.rs`.

Two topologies fall out of the same solver. Diodes **across the feedback
resistor** of a gain stage give a soft knee and a pedal that cleans up when the
guitar volume comes down — the overdrive. Diodes **from the node to ground**
behind a series resistor give a hard corner and a pedal whose dirt barely
changes with input level — the distortion. Neither behaviour is programmed;
they are the same equation with the diodes in different places, and the measured
distortion curves separate accordingly: at 2 mV in, the overdrive reads 0.07 %
and the distortion 1.35 %.

### The bipolar transistor

```text
Ic = Is·(exp(Vbe/Vt) − 1)
```

`circuit/transistor.rs` solves a common-emitter stage as three unknowns — base,
collector and emitter — against three node equations, with the clipping diodes
inside the same system so the transistor and the pair argue about the collector
voltage the way they do in the circuit. The input coupling capacitor is
integrated alongside, which is what makes the bias network able to hold the base
at a forward drop at all.

What this produces, none of it asked for: the fuzz's clipping stages bias at
0.97 V — the low collector voltage a service sheet reports for that family —
and clip asymmetrically as a result, swinging 0.375 V down but only 0.104 V up,
because that is how much room the collector has in each direction.

Two Newton iterations per sample. That is measured, not assumed: a test compares
one, two and three iterations against a twelve-iteration reference and requires
the chosen count to be within 1 %.

### The transconductance cell

```text
Iout = Iabc · tanh(Vin / (2·Vt))
```

The OTA in `circuit/ota.rs` is linear over about ±25 mV, so the compressor
attenuates by thirty before it and the cell's own knee is what thickens a pick
attack. The bias current is the control input, and the rectifier steals from it:
solving the loop, `out = in·G·(Iq − s·out)`, gives a ceiling of `Iq/s`, which is
what the sustain control actually sets.

### The tone network

Not nonlinear, but just as often faked. `circuit/tonestack.rs` solves the real
thing: two RC branches driven from the same stage and bridged by the pot, with
the next stage's input impedance as the load. Three nodes, eliminated by hand
into a biquad, discretised inside the oversampled block where the bilinear
transform barely warps it.

The midrange scoop is a consequence: at the middle of the pot the wiper sits
between a signal that has lost its treble and one that has lost its bass. It
measures 6 dB deep at 1.2 kHz on the fuzz network, and it *travels* — 1.6 kHz
with the control at 0.3, 680 Hz at 0.7 — which no fixed peaking filter
reproduces. A test checks the running filter against a direct complex solve of
the same three node equations at every position; it agrees within 0.15 dB.

### What one pedal does to the next

A pedal is not an isolated box: it presents an input impedance and drives from
an output impedance, and those two argue with each other.

Inside the board, a pedal whose input is a bare transistor stage takes the
previous pedal's output impedance into its own input resistor — the physical
truth, and it moves two things at once. Less signal reaches the base, so the
pedal saturates less; and the coupling capacitor now works against a larger
resistance, so its corner moves *down* and there is relatively more bass. That
second one is worth stating because it is the opposite of the usual intuition
about lossy cables, which is a story about capacitance to ground.

Pedals with buffered inputs see a ten-kilohm source against a half-megohm
input: under a fifth of a decibel. That is left out rather than modelled as a
multiply with no consequence — and the reason buffers exist is precisely that
the answer is allowed to be "almost nothing".

The declared impedances are checked, not asserted. The fuzz's is measured from
the model itself: drive the stage with a tone, divide by the current its input
branch draws, and require the published constant to agree.

### What the guitar does before any of it

The most audible loading of all happens before the first pedal. A pickup is a
coil — henries of inductance, kilohms of wire — feeding its own capacitance and
the cable, and that network resonates somewhere between two and five kilohertz.
How tall that resonance stands depends on what is plugged into it.

RF-Rig never sees a pickup: it receives a signal that already went through one,
loaded by whatever interface captured it. So what `circuit/source.rs` models is
not the pickup but the *difference* between two loading conditions,

```text
correction(s) = H(s, R_pedal) / H(s, R_captured)
```

which is exactly the part that is missing, and which collapses to a wire when
the two match — an identity the tests check. The `Source` control says which
guitar is plugged in, or `Buffered` for a signal that arrived through something
with a stiff output, where the honest correction is none at all.

With a single coil in front of a fuzz, the resonance is damped by several
decibels and the top goes with it. In front of a buffered pedal, nothing
measurable happens. Both of those are the same equation.

## Where the frequency response comes from

A guitar pedal is mostly RC pairs, and the interesting ones are *inside* the
gain path rather than after it:

* The overdrive's inverting input sees 4.7 kΩ in series with 47 nF. The corner
  is 1/(2π·4.7k·47n) ≈ 720 Hz, and below it the stage barely amplifies at all.
  That single pair is the mid-hump the whole pedal is known for — measured at
  8.6 dB with the tone open — and the reason a low E stays defined.
* The distortion's equivalent pair uses 0.47 µF, putting its corner near 72 Hz,
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
polyphase half-band interpolator (`circuit/oversample.rs`), with the filters and
tone networks inside the oversampled path configured at the oversampled rate. A
test measures the alias that lands at 13 kHz from a 7 kHz tone and requires the
oversampled path to beat the naive one.

Where a Newton solve would be wasted — the soft limit inside a delay's feedback
loop, the OTA's knee — the code uses first-order antiderivative antialiasing
instead. That comes with a numerical trap worth knowing about: the difference
quotient it computes is catastrophically ill-conditioned when consecutive
samples are close, so the fallback threshold has to be *relative*. Before it
was, a compressor fed a two-millivolt sine put out more noise than signal.

## What is measured, and what is still assumed

**Solved from component values**
* Both clipping topologies and all three diode options.
* The transistor stages: bias point, asymmetry, coupling, and the diodes inside
  the same solve.
* All three tone networks, checked against a direct solve of their netlists.
* The compressor's gain cell and the feedback loop around it.
* The overdrive's and distortion's frequency-dependent gain networks.
* Loading: pedal to pedal inside the board, and the pickup's resonance against
  whatever the first pedal presents.
* The BBD sweep, band-limiting and companding structure.
* The echo's in-loop filtering and its two modes.

**Structurally right, numerically approximate — and marked as such in the code**
* The distortion's and overdrive's tone networks use the correct topology with
  representative values; only the fuzz's are the published ones.
* The rectifier that drives the compressor's bias current is a linear law, not a
  modelled transistor current source.
* The op-amp gain stages are ideal apart from an explicit supply limit.
* The reverb is a plausible spring and plate, not a measured tank.
* The buffered pedals' input impedances are the family's published figures
  rather than solved from their buffer stages: an emitter follower's only
  audible job is that number, so it is declared instead of computed.

**Not attempted yet**
* Component tolerance and temperature drift.
* Power supply sag.
* A clock-accurate bucket-brigade line: the aliasing of a real BBD is part of
  its sound, and the current model band-limits it away.

## How to move an item up that list

Measure both sides of the same experiment. The bench procedure is in
[`MEASUREMENT.md`](MEASUREMENT.md), along with the two instrument faults this
project has already found in its own rulers. Sweep against the render, not the
formula: what a transfer function returns is not what is heard until the rest of
the chain has multiplied it.
