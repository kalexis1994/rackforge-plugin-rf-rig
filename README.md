# RF-Rig

A guitar pedalboard for [RackForge](https://github.com/kalexis1994/rackforge),
modelled at circuit level: compressor, overdrive, distortion, fuzz, chorus,
delay and reverb, in whatever order you cable them.

RF-Rig is one `.rfplugin` package rather than seven. A board is a single chain
with a single state, so a preset carries the whole rig — which pedals are on,
where each one sits, and where every knob is.

> `v0.1.0` is the first working version. The engine runs, the package installs,
> and the plan in [`docs/IMPLEMENTATION_PLAN.md`](docs/IMPLEMENTATION_PLAN.md)
> says plainly what is derived from a circuit and what is still an
> approximation.

## Why circuit level

The other RackForge instruments — RF-5, RF-106, RF-M1 — are reverse engineered
from firmware: the personality of those synthesizers lives in a ROM, so the ROM
is where you go. An analog stompbox has no firmware. Its personality is
*entirely* in the circuit, and the circuit is public: community traces,
component-level analyses, expired patents, and any unit you can measure
yourself.

So RF-Rig does not shape a curve until it sounds about right. It writes down
the equation the circuit obeys and solves it:

```text
I_in = V/R + 2·Is·sinh(V / (n·Vt))
```

That is Kirchhoff's current law at a clipping node. `Is` and `n` come from a
diode datasheet, `R` from the schematic. The famous soft knee of a feedback
clipper and the hard corner of a shunt clipper are the *solutions* of that same
line, differing only in where the diodes sit — which is exactly how the two
pedals differ. Change the diode to germanium and the pedal cleans up earlier,
because the datasheet says it does.

The same applies to the parts that are not clipping. A transistor stage is
solved as three node equations against Ebers-Moll, so its bias point — and the
asymmetry that follows from it — is a consequence rather than a setting. A tone
control is solved as the network it is: two RC branches bridged by a pot, whose
midrange scoop is 6 dB deep and *travels* from 1.6 kHz to 680 Hz as the knob
turns, because that is what the network does.

The method, and an honest list of what is still approximated, is in
[`docs/CIRCUIT_MODELING.md`](docs/CIRCUIT_MODELING.md).

## What is on the board

| Pedal | Circuit family |
| --- | --- |
| **Compressor** | An OTA gain cell, `Iout = Iabc·tanh(Vin/2Vt)`, with the rectifier stealing from its bias current: enormous ratio, soft knee, program-dependent recovery, and a cell that thickens transients because it is linear over only ±25 mV. |
| **Overdrive** | Op-amp stage with a diode pair across the feedback resistor, and an input network that stops amplifying below ~720 Hz. That is the mid-hump, and the reason it cleans up on the guitar's volume. |
| **Distortion** | Booster into a high-gain stage that hard-clips to ground, behind a scooped tone network. |
| **Fuzz** | A booster into two transistor clipping stages, each solved from Ebers-Moll with its diodes inside the same system. They bias at 0.97 V and clip asymmetrically because that is how much room the collector has. |
| **Chorus** | Bucket-brigade line with companding and a swept clock, so the delayed copy is genuinely pitch-shifted rather than crossfaded. |
| **Delay** | The same line as an echo: band-limited *inside* the feedback loop so each repeat darkens, plus a clean digital mode. |
| **Reverb** | A dispersive spring tank, or a plate built as a feedback delay network. |

Plus a rig strip: input trim, output level and a noise gate.

Order is part of the parameter space. Each pedal owns a `position` parameter, so
moving the delay in front of the fuzz survives in the preset, can be automated,
and is validated by the host like any other value.

## Build and install

The RackForge checkout must sit next to this one, because the packager lives
there.

```bash
pwsh tools/build-package.ps1
```

```bash
bash tools/build-package.sh
```

Either script regenerates the package metadata from the contract, runs the
tests, builds the WebAssembly component and packs
`artifacts/RF-Rig.rfplugin`. Install it the way you install any RackForge
plugin — the desktop's Plugin Manager, or:

```bash
./target/release/rackforge-desktop.exe --install-plugin ../rackforge-plugin-rf-rig/artifacts/RF-Rig.rfplugin
```

### One host change is still needed

RackForge's engine already runs effects: the Rack graph compiles a hardware
audio input into a plugin node and the audio thread mixes it, and this package
passes the host's validation as it stands. What the Web UI does not do yet is
*offer* an effect when you add a plugin to a Rack Slot — the picker filters for
`kind === "instrument"`. Until that lands, RF-Rig installs and validates but
cannot be placed on a board from the interface. See
[`docs/IMPLEMENTATION_PLAN.md`](docs/IMPLEMENTATION_PLAN.md) for the exact
change.

## Working on it

```bash
cargo test --workspace                        # engine, contract and adapter
cargo run -p rf-rig-lab -- metadata           # regenerate package metadata
cargo run -p rf-rig-lab -- render --preset blues-drive --out artifacts/demo.wav
cargo run -p rf-rig-lab -- render --input my-guitar-di.wav --preset fuzz-lead --out artifacts/fuzz.wav
cargo run -p rf-rig-lab -- sweep --set drive.engaged=1
cargo run -p rf-rig-lab -- thd --set dist.engaged=1
```

The surface can be worked on without building a package: serve the repository
and open `tools/ui-preview.html`, which speaks the same `rackforge.plugin.web@1`
protocol the host does.

```bash
python -m http.server 8131
```

More: [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md),
[`docs/MEASUREMENT.md`](docs/MEASUREMENT.md),
[`docs/REFERENCES.md`](docs/REFERENCES.md),
[`CONTRIBUTING.md`](CONTRIBUTING.md).

## Licence

GPL-3.0-only. RF-Rig models circuit topologies, which are not copyrightable and
whose patents expired decades ago. It contains no manufacturer's firmware,
artwork, trademark or brand name, and it names no product. See
[`NOTICE.md`](NOTICE.md).
