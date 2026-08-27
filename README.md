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
| **Compressor** | An OTA gain cell, `Iout = Iabc·tanh(Vin/2Vt)`, controlled by a diode charging a timing capacitor. That detector gives it a real threshold — below it the pedal is a clean gain stage whatever the sustain knob says — an attack that depends on how hard you hit it, and a ratio that runs from 1.08:1 to 8:1 across the control. |
| **Overdrive** | Op-amp stage with a diode pair across the feedback resistor, and an input network that stops amplifying below ~720 Hz — the mid-hump, measured at 8.8 dB. The amplifier is a real one: with the drive up its loop runs out of authority at 13 kHz, which is part of why this circuit sounds smooth rather than fizzy. |
| **Distortion** | Booster into a high-gain stage that hard-clips to ground, behind a scooped tone network. |
| **Fuzz** | A booster into two transistor clipping stages, each solved from Ebers-Moll with its diodes inside the same system. They bias at 0.97 V and clip asymmetrically because that is how much room the collector has. |
| **Chorus** | Bucket-brigade line with companding and a swept clock, so the delayed copy is genuinely pitch-shifted rather than crossfaded. |
| **Delay** | The same line as an echo: band-limited *inside* the feedback loop so each repeat darkens, plus a clean digital mode. |
| **Reverb** | A spring tank built as the dispersive transmission line it is — transit, dispersion, reflection, three springs — so one note comes back as a descending chirp and each pass adds another. Or a plate, built as a feedback delay network, because that is what a plate is. |

Plus a rig strip: a source selector, input trim, output level and a noise gate.

The source selector is not a tone control. It tells the rig what is plugged in —
a buffered signal, a single coil, a humbucker — and the first pedal's input
impedance then decides how much of that pickup's resonance survives. Put the
fuzz first with a single coil selected and several decibels come off the top,
because a bare transistor input loads a pickup and a buffered one does not.
Order changes the sound before the first diode conducts.

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

### Putting it on a board

Enable RF-Rig in Plugin Manager, then open a Rack and add an **Effect** node —
either from the button in the Slot settings header or by right-clicking the
graph canvas. The editor wires it from the hardware audio input, so whatever is
plugged into the interface runs through the board and out to the main output.

Move it wherever you like afterwards: the node is an ordinary Slot, so it can
sit behind an instrument instead of the input, or feed another Slot.

> RackForge's engine could always run effects — the Rack graph compiles a
> hardware audio input into a plugin node and the audio thread mixes it — but
> until August 2026 the Web UI's Slot picker filtered for
> `kind === "instrument"`, so an effect could be installed and validated and
> still never reach a board.

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

The preview is not a friendly host. It has controls for write latency, a
refusal rate, a context echoed after every write — which is what a Rack Slot
really does — and a switch that makes it stop answering altogether. The page is
expected to stay usable under all of them; `window.__preview` exposes the write
counters so that can be checked rather than eyeballed.

More: [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md),
[`docs/MEASUREMENT.md`](docs/MEASUREMENT.md),
[`docs/REFERENCES.md`](docs/REFERENCES.md),
[`CONTRIBUTING.md`](CONTRIBUTING.md).

## Licence

GPL-3.0-only. RF-Rig models circuit topologies, which are not copyrightable and
whose patents expired decades ago. It contains no manufacturer's firmware,
artwork, trademark or brand name, and it names no product. See
[`NOTICE.md`](NOTICE.md).
