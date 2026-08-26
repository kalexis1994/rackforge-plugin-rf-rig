# Measuring, on the bench and in the model

The point of this file is that a claim about a circuit should be checkable. The
lab tool measures the model; the same procedure with an interface and a real
pedal measures the circuit. Comparing the two is only meaningful when both sides
are the *same* experiment.

## Two faults this project has already found in its own rulers

Both were found by disbelieving a number, and both had been quietly present in
every measurement taken before them.

**Spectral leakage.** The distortion analyser correlates the signal against each
harmonic. A tone rarely fits a whole number of cycles into the analysis window,
and the leftover leaks onto the harmonic probes: measured bare, a *perfect* sine
read 0.62 % distorted. That is the same order as a lightly driven overdrive, so
every clean-end reading taken before the fix was meaningless. A Hann window took
the floor to 0.0006 %.

**Single-sample probing of a stateful shaper.** The antialiased `tanh` returns
the average slope since the previous input, so asking it about one isolated
sample reads half the gain the cell actually has. Gain is measured with a tone,
the way a bench would.

The lesson both share: before believing a difference, check what the instrument
reports for a case where the answer is known.

## The trap this exists to avoid

Two signals normalised by different references are not a comparison. Neither is
a model measured at one input level against a pedal measured at another, nor a
frequency response taken with one tone control at noon and the other at
maximum. Before chasing a difference, prove the two measurements were the same
question.

Peak readings are a second trap: they measure phase as much as level whenever
two things are beating. Use RMS or a single-frequency correlation.

## Measuring the model

```bash
# Frequency response through the current board, 40 Hz to 10 kHz.
cargo run -p rf-rig-lab -- sweep --set drive.engaged=1 --set drive.tone=0.5

# Harmonic content and crest factor against input level.
cargo run -p rf-rig-lab -- thd --set dist.engaged=1 --frequency 220

# A file to listen to.
cargo run -p rf-rig-lab -- render --preset blues-drive --out artifacts/demo.wav
cargo run -p rf-rig-lab -- render --input di.wav --preset fuzz-lead --out artifacts/fuzz.wav
```

`--set` takes parameter identifiers (`drive.drive=0.7`) or indexes. The board
starts from the defaults, or from `--preset <id>`.

Levels are in volts at the input jack, with host full scale taken as 1 V. A hot
single coil peaks around 0.2 V, a humbucker higher; `rig.input` is the trim that
matches a real rig to that convention. Measurements should say what input level
they used.

## Measuring a real pedal

With an interface, a DI box and a load resistor:

1. **Static transfer.** Feed a slow sine (40–80 Hz, so no filter in the pedal is
   doing anything yet) at a series of levels. Plot output against input. This is
   the clipping curve, and it is the single most informative measurement there
   is — it tells you the knee voltage, the symmetry and roughly which diodes are
   in there.
2. **Harmonic profile.** A 220 Hz sine at several levels, then the amplitude of
   each harmonic. Feedback clipping and shunt clipping look different here in a
   way that survives any tone control.
3. **Tone stack.** Sweep at a level low enough that nothing clips, once per
   control position. That isolates the filter from the nonlinearity.
4. **Gain network.** The same sweep with the gain control at minimum shows the
   frequency-dependent gain — where the mid-hump lives.
5. **Time-domain behaviour.** For a compressor: a step and a decaying note, to
   read attack and recovery. For a delay: a click, to read the delay time and
   how each repeat darkens.

Record the input level, the control positions, the interface gain and the
temperature. A measurement without its conditions cannot be compared to
anything later.

## What the board costs

```bash
cargo test --release -p rf-rig-dsp --test bench_blocks -- --ignored --nocapture
```

Per 512-frame block at 48 kHz, on the development desktop, as a fraction of the
10.67 ms budget:

| | µs/block | one core |
| --- | ---: | ---: |
| empty board | 4 | 0.0 % |
| compressor | 30 | 0.3 % |
| overdrive | 170 | 1.6 % |
| distortion | 254 | 2.4 % |
| fuzz | 549 | 5.1 % |
| chorus | 55 | 0.5 % |
| delay | 44–48 | 0.4 % |
| reverb | 67 plate, 391 spring | 0.6 % / 3.7 % |
| everything engaged | 1196 | 11.2 % |

The dirt pedals dominate, because each one solves circuit equations four times
per sample. A Raspberry Pi is several times slower per core, so measure there
before believing anything here about headroom.

Two results worth keeping. Solving the op-amp stage as one loop — finite
bandwidth, both capacitors, the diodes — replaced an ideal amplifier plus a
separate clipping solver and came out *cheaper*: the overdrive went from 227 to
170 microseconds. More circuit for less arithmetic, because a well-posed loop
converges in two iterations.

And: replacing Cramer's rule with Gaussian elimination in
the transistor solve — a third of the arithmetic — made the fuzz *57 % slower*,
because the pivot search branches on data. Straight-line arithmetic wins when
the operations are this small.

## Judging a change

1. Measure before.
2. Change one thing.
3. Measure after, the same way.
4. Render both and listen.

If the metric improves and the render sounds worse, the metric is the suspect.
That has happened often enough in this codebase's sibling projects to be worth
writing down.
