# Measuring, on the bench and in the model

The point of this file is that a claim about a circuit should be checkable. The
lab tool measures the model; the same procedure with an interface and a real
pedal measures the circuit. Comparing the two is only meaningful when both sides
are the *same* experiment.

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

## Judging a change

1. Measure before.
2. Change one thing.
3. Measure after, the same way.
4. Render both and listen.

If the metric improves and the render sounds worse, the metric is the suspect.
That has happened often enough in this codebase's sibling projects to be worth
writing down.
