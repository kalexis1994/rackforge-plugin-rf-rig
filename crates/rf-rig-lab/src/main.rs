//! The RF-Rig bench.
//!
//! Three jobs, all of them about keeping the project honest:
//!
//! * `metadata` renders the package's JSON from the contract, so the schema the
//!   host validates and the engine's behaviour come from one place;
//! * `render` puts a signal through the board and writes a file you can listen
//!   to, because listening has settled more arguments here than any metric;
//! * `sweep` and `thd` measure the things a bench meter would measure, so a
//!   claim about a circuit can be checked instead of asserted.

mod manifest;
mod metadata;

use std::path::{Path, PathBuf};

use rf_rig_contract::{PARAMETERS, PEDALS, PRESETS};
use rf_rig_dsp::{Engine, WORKSPACE_SAMPLES};

const SAMPLE_RATE: f32 = 48_000.0;

fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let command = arguments.first().map(String::as_str).unwrap_or("help");
    let result = match command {
        "metadata" => metadata_command(&arguments[1..]),
        "render" => render_command(&arguments[1..]),
        "sweep" => sweep_command(&arguments[1..]),
        "thd" => thd_command(&arguments[1..]),
        "presets" => presets_command(),
        "parameters" => parameters_command(),
        _ => {
            usage();
            Ok(())
        }
    };
    if let Err(error) = result {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

fn usage() {
    println!(
        "rf-rig-lab <command>

  metadata [--check]              render package metadata from the contract
  parameters                      list every parameter and its range
  presets                         list the factory boards
  render --out <file.wav>         render a signal through the board
         [--preset <id>] [--input <file.wav>] [--seconds <n>]
         [--set <id>=<value> ...]
  sweep  [--preset <id>] [--set <id>=<value> ...] [--points <n>]
  thd    [--preset <id>] [--set <id>=<value> ...] [--frequency <hz>]

Values may be given by parameter identifier or index:
  --set drive.engaged=1 --set drive.drive=0.7"
    );
}

fn repository_root() -> PathBuf {
    // The binary lives in target/<profile>/; the manifest directory is stable.
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .expect("the lab crate sits two levels below the repository root")
}

fn metadata_command(arguments: &[String]) -> Result<(), String> {
    let check = arguments.iter().any(|argument| argument == "--check");
    let package = repository_root().join("plugin").join("package");
    let identity = manifest::read(&package.join("rackforge-plugin.toml"))?;
    metadata::write(&package, &identity, check)?;
    if check {
        println!("metadata matches the contract");
    }
    Ok(())
}

fn parameters_command() -> Result<(), String> {
    for parameter in PARAMETERS.iter() {
        println!(
            "{:>3}  {:<18} {:<14} {}",
            parameter.index, parameter.id, parameter.page, parameter.name
        );
    }
    Ok(())
}

fn presets_command() -> Result<(), String> {
    for preset in PRESETS.iter() {
        println!(
            "{:<16} {:<18} {}",
            preset.id, preset.name, preset.description
        );
    }
    println!();
    for pedal in PEDALS.iter() {
        println!("{:<8} {:<12} {}", pedal.id, pedal.name, pedal.circuit);
    }
    Ok(())
}

struct Options {
    preset: Option<String>,
    overrides: Vec<(u32, f64)>,
    input: Option<PathBuf>,
    output: Option<PathBuf>,
    seconds: f32,
    points: usize,
    frequency: f32,
}

fn parse_options(arguments: &[String]) -> Result<Options, String> {
    let mut options = Options {
        preset: None,
        overrides: Vec::new(),
        input: None,
        output: None,
        seconds: 4.0,
        points: 24,
        frequency: 220.0,
    };
    let mut index = 0;
    while index < arguments.len() {
        let flag = arguments[index].as_str();
        let value = || next_value(arguments, index, flag);
        match flag {
            "--preset" => {
                options.preset = Some(value()?);
                index += 2;
            }
            "--set" => {
                let assignment = value()?;
                options.overrides.push(parse_assignment(&assignment)?);
                index += 2;
            }
            "--input" => {
                options.input = Some(PathBuf::from(value()?));
                index += 2;
            }
            "--out" => {
                options.output = Some(PathBuf::from(value()?));
                index += 2;
            }
            "--seconds" => {
                options.seconds = value()?.parse().map_err(|_| "--seconds wants a number")?;
                index += 2;
            }
            "--points" => {
                options.points = value()?.parse().map_err(|_| "--points wants a number")?;
                index += 2;
            }
            "--frequency" => {
                options.frequency = value()?.parse().map_err(|_| "--frequency wants a number")?;
                index += 2;
            }
            other => return Err(format!("unknown option {other}")),
        }
    }
    Ok(options)
}

fn next_value(arguments: &[String], index: usize, flag: &str) -> Result<String, String> {
    arguments
        .get(index + 1)
        .cloned()
        .ok_or_else(|| format!("{flag} needs a value"))
}

fn parse_assignment(assignment: &str) -> Result<(u32, f64), String> {
    let (name, value) = assignment
        .split_once('=')
        .ok_or_else(|| format!("expected id=value, got {assignment}"))?;
    let value: f64 = value
        .parse()
        .map_err(|_| format!("{value} is not a number"))?;
    if let Ok(index) = name.parse::<u32>() {
        return Ok((index, value));
    }
    PARAMETERS
        .iter()
        .find(|parameter| parameter.id == name)
        .map(|parameter| (parameter.index, value))
        .ok_or_else(|| format!("no parameter is called {name}"))
}

fn configure(engine: &mut Engine<'_>, options: &Options) -> Result<(), String> {
    if let Some(preset) = &options.preset
        && !engine.load_preset(preset)
    {
        return Err(format!("no factory board is called {preset}"));
    }
    for (index, value) in &options.overrides {
        if !engine.set_parameter(*index, *value) {
            return Err(format!(
                "{} does not accept {value}",
                PARAMETERS
                    .get(*index as usize)
                    .map(|parameter| parameter.id)
                    .unwrap_or("that parameter")
            ));
        }
    }
    Ok(())
}

/// A decaying harmonic stack with a noisy attack. Not a guitar, but enough of
/// one to hear what a pedal is doing to an envelope.
fn plucked(seconds: f32, sample_rate: f32) -> Vec<f32> {
    let count = (seconds * sample_rate) as usize;
    let mut samples = Vec::with_capacity(count);
    let mut noise_state = 0x1234_5678_u32;
    for index in 0..count {
        let time = index as f32 / sample_rate;
        // A new note every 800 ms.
        let position = time % 0.8;
        let note = ((time / 0.8) as usize) % 4;
        let fundamental = [82.41, 110.0, 146.83, 196.0][note];
        let envelope = (-position * 3.2).exp();
        let mut value = 0.0;
        for harmonic in 1..=8 {
            let amplitude = envelope / (harmonic as f32).powf(1.6);
            value += amplitude
                * (std::f32::consts::TAU * fundamental * harmonic as f32 * position).sin();
        }
        // Pick noise for the first few milliseconds.
        noise_state = noise_state
            .wrapping_mul(1_664_525)
            .wrapping_add(1_013_904_223);
        let noise = (noise_state >> 9) as f32 / (1 << 23) as f32 - 1.0;
        value += noise * (-position * 260.0).exp() * 0.3;
        samples.push(value * 0.25);
    }
    samples
}

fn read_wav(path: &Path) -> Result<Vec<f32>, String> {
    let mut reader =
        hound::WavReader::open(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let specification = reader.spec();
    let channels = specification.channels as usize;
    let samples: Vec<f32> = match specification.sample_format {
        hound::SampleFormat::Float => reader.samples::<f32>().filter_map(Result::ok).collect(),
        hound::SampleFormat::Int => {
            let scale = 1.0 / (1_i64 << (specification.bits_per_sample - 1)) as f32;
            reader
                .samples::<i32>()
                .filter_map(Result::ok)
                .map(|sample| sample as f32 * scale)
                .collect()
        }
    };
    if channels <= 1 {
        return Ok(samples);
    }
    Ok(samples
        .chunks(channels)
        .map(|frame| frame.iter().sum::<f32>() / channels as f32)
        .collect())
}

fn render_command(arguments: &[String]) -> Result<(), String> {
    let options = parse_options(arguments)?;
    let output = options
        .output
        .clone()
        .ok_or("render needs --out <file.wav>")?;

    let input = match &options.input {
        Some(path) => read_wav(path)?,
        None => plucked(options.seconds, SAMPLE_RATE),
    };

    let mut memory = vec![0.0_f32; WORKSPACE_SAMPLES];
    let mut engine = Engine::default();
    if !engine.prepare_with(SAMPLE_RATE as f64, &mut memory) {
        return Err("the engine refused to prepare".into());
    }
    configure(&mut engine, &options)?;

    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("{}: {error}", parent.display()))?;
    }
    let specification = hound::WavSpec {
        channels: 2,
        sample_rate: SAMPLE_RATE as u32,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut writer = hound::WavWriter::create(&output, specification)
        .map_err(|error| format!("{}: {error}", output.display()))?;

    let mut peak = 0.0_f32;
    for sample in &input {
        let (left, right) = engine.process(*sample);
        peak = peak.max(left.abs()).max(right.abs());
        writer
            .write_sample(left)
            .and_then(|_| writer.write_sample(right))
            .map_err(|error| format!("writing {}: {error}", output.display()))?;
    }
    // Let the tails ring out rather than cutting them at the last input sample.
    for _ in 0..(SAMPLE_RATE as usize * 3) {
        let (left, right) = engine.process(0.0);
        peak = peak.max(left.abs()).max(right.abs());
        writer
            .write_sample(left)
            .and_then(|_| writer.write_sample(right))
            .map_err(|error| format!("writing {}: {error}", output.display()))?;
    }
    writer
        .finalize()
        .map_err(|error| format!("closing {}: {error}", output.display()))?;

    println!("wrote {} (peak {:.3})", output.display(), peak);
    if peak > 1.0 {
        println!("note: the render clips a 0 dBFS file; trim rig.output or the pedal levels");
    }
    Ok(())
}

fn magnitude_at(samples: &[f32], frequency: f32, sample_rate: f32) -> f32 {
    let mut real = 0.0_f32;
    let mut imaginary = 0.0_f32;
    for (index, sample) in samples.iter().enumerate() {
        let phase = std::f32::consts::TAU * frequency * index as f32 / sample_rate;
        real += sample * phase.cos();
        imaginary += sample * phase.sin();
    }
    2.0 * (real * real + imaginary * imaginary).sqrt() / samples.len() as f32
}

fn steady_state(
    options: &Options,
    frequency: f32,
    amplitude: f32,
    samples: usize,
) -> Result<Vec<f32>, String> {
    let mut memory = vec![0.0_f32; WORKSPACE_SAMPLES];
    let mut engine = Engine::default();
    if !engine.prepare_with(SAMPLE_RATE as f64, &mut memory) {
        return Err("the engine refused to prepare".into());
    }
    configure(&mut engine, options)?;

    let settle = (SAMPLE_RATE * 0.5) as usize;
    let mut rendered = Vec::with_capacity(samples);
    for index in 0..settle + samples {
        let phase = std::f32::consts::TAU * frequency * index as f32 / SAMPLE_RATE;
        let (left, _) = engine.process(amplitude * phase.sin());
        if index >= settle {
            rendered.push(left);
        }
    }
    Ok(rendered)
}

fn sweep_command(arguments: &[String]) -> Result<(), String> {
    let options = parse_options(arguments)?;
    let amplitude = 0.05;
    println!("  Hz        dB   (input {amplitude:.3})");
    for point in 0..options.points {
        let fraction = point as f32 / (options.points - 1).max(1) as f32;
        let frequency = 40.0 * (10_000.0_f32 / 40.0).powf(fraction);
        let rendered = steady_state(&options, frequency, amplitude, 16_384)?;
        let level = magnitude_at(&rendered, frequency, SAMPLE_RATE) / amplitude;
        println!("{frequency:8.0}  {:8.2}", 20.0 * level.max(1e-6).log10());
    }
    Ok(())
}

fn thd_command(arguments: &[String]) -> Result<(), String> {
    let options = parse_options(arguments)?;
    let frequency = options.frequency;
    println!("  input       out      THD%   crest   (at {frequency:.0} Hz)");
    for step in 0..10 {
        let amplitude = 0.002 * 1.6_f32.powi(step);
        let rendered = steady_state(&options, frequency, amplitude, 16_384)?;
        let fundamental = magnitude_at(&rendered, frequency, SAMPLE_RATE);
        let mut harmonics = 0.0_f32;
        for order in 2..=8 {
            let harmonic = frequency * order as f32;
            if harmonic >= SAMPLE_RATE * 0.45 {
                break;
            }
            let magnitude = magnitude_at(&rendered, harmonic, SAMPLE_RATE);
            harmonics += magnitude * magnitude;
        }
        let distortion = if fundamental > 1e-9 {
            harmonics.sqrt() / fundamental
        } else {
            0.0
        };
        let peak = rendered.iter().fold(0.0_f32, |worst, s| worst.max(s.abs()));
        let rms = (rendered.iter().map(|s| s * s).sum::<f32>() / rendered.len() as f32).sqrt();
        let crest = if rms > 1e-9 { peak / rms } else { 0.0 };
        println!(
            "{amplitude:8.4}  {:8.4}  {:8.2}  {crest:6.3}",
            fundamental,
            distortion * 100.0
        );
    }
    Ok(())
}
