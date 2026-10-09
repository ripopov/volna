//! Compact analog and stacked-group fixture for the Volna viewer.
//!
//! Regenerate from the workspace root:
//!     cargo run --locked -p volna --example analog_showcase
//!
//! An optional output path replaces `volna/examples/analog_showcase.vtr`.

use std::f64::consts::TAU;
use std::path::PathBuf;

use vtr::{
    Direction, Logic, LogicStates, NodeId, ReadOptions, Reader, ScopeType, SignalId, SignalKind,
    Timescale, VarType, Writer, WriterOptions,
};

const INTERVALS: u64 = 1024;
const STEP_NS: u64 = 100;
const MAX_BYTES: u64 = 128 * 1024;

fn group<const N: usize>(
    writer: &mut Writer,
    top: NodeId,
    name: &str,
    names: [&str; N],
    kind: SignalKind,
) -> vtr::Result<[SignalId; N]> {
    let scope = writer.add_scope(Some(top), name, ScopeType::Generic, "")?;
    let var_type = if kind == SignalKind::Real {
        VarType::Real
    } else {
        VarType::Wire
    };
    let mut signals = [SignalId(0); N];
    for (signal, name) in signals.iter_mut().zip(names) {
        *signal = writer
            .add_var(Some(scope), name, var_type, Direction::Output, kind)?
            .1;
    }
    Ok(signals)
}

/// Binary fractions keep smooth plots while compressing better than full
/// precision floating-point samples. Non-finite samples retain their meaning.
fn quantize(value: f64) -> f64 {
    (value * 4096.0).round() / 4096.0
}

fn emit_reals<const N: usize>(
    writer: &mut Writer,
    signals: &[SignalId; N],
    values: [f64; N],
) -> vtr::Result<()> {
    for (&signal, value) in signals.iter().zip(values) {
        writer.emit_real(signal, quantize(value))?;
    }
    Ok(())
}

fn generate(path: &std::path::Path) -> vtr::Result<()> {
    let mut writer = Writer::builder()
        .timescale(Timescale::Nanoseconds)
        .writer_name("Volna analog showcase")
        .comment("Analog shapes, numeric encodings, and related contributions for stacked groups")
        .options(WriterOptions {
            background: false,
            ..WriterOptions::default()
        })
        .create(path)?;
    let top = writer.add_scope(None, "analog", ScopeType::Generic, "")?;
    let waves = group(
        &mut writer,
        top,
        "waves",
        [
            "sine",
            "cosine",
            "triangle",
            "sawtooth",
            "square",
            "chirp",
            "ringing",
            "gaussian",
            "staircase",
            "clipped",
            "noise",
            "dc",
        ],
        SignalKind::Real,
    )?;
    let phases = group(
        &mut writer,
        top,
        "phases",
        ["phase_a", "phase_b", "phase_c"],
        SignalKind::Real,
    )?;
    let power = group(
        &mut writer,
        top,
        "power",
        ["cpu_mw", "gpu_mw", "memory_mw", "io_mw"],
        SignalKind::Real,
    )?;
    let flows = group(
        &mut writer,
        top,
        "flows",
        ["solar_ma", "load_ma", "battery_ma", "regeneration_ma"],
        SignalKind::Real,
    )?;
    let queues = group(
        &mut writer,
        top,
        "queues",
        ["rx_count", "tx_count", "dma_count"],
        SignalKind::bits(8, LogicStates::Four),
    )?;
    let activity = group(
        &mut writer,
        top,
        "activity",
        ["cpu_busy", "gpu_busy", "dma_busy", "io_busy"],
        SignalKind::bits(1, LogicStates::Four),
    )?;
    let edges = group(
        &mut writer,
        top,
        "edge_cases",
        ["dropout", "nonfinite", "spike", "late_start"],
        SignalKind::Real,
    )?;
    let numeric = writer.add_scope(Some(top), "numeric", ScopeType::Generic, "")?;
    let mut numeric_signal = |name, width| {
        writer
            .add_var(
                Some(numeric),
                name,
                VarType::Wire,
                Direction::Output,
                SignalKind::bits(width, LogicStates::Four),
            )
            .map(|(_, signal)| signal)
    };
    let signed = numeric_signal("signed_adc", 16)?;
    let unsigned = numeric_signal("unsigned_adc", 12)?;
    let float32 = numeric_signal("float32_bits", 32)?;
    let float64 = numeric_signal("float64_bits", 64)?;

    // Fixed seed and sample schedule, with no wall-clock metadata or periodic
    // commits: identical runs produce identical bytes on the same toolchain.
    let mut noise = 0x5eed_1234u32;
    for i in 0..=INTERVALS {
        writer.set_time(i * STEP_NS)?;
        let u = i as f64 / INTERVALS as f64;
        let phase = TAU * 8.0 * u;
        let cycle = (8.0 * u).fract();
        let sine = phase.sin();
        let pulse = if (i % 256) < 64 { 1.0 } else { 0.0 };
        noise ^= noise << 13;
        noise ^= noise >> 17;
        noise ^= noise << 5;
        let random = f64::from(noise & 0xffff) / 32768.0 - 1.0;
        emit_reals(
            &mut writer,
            &waves,
            [
                sine,
                phase.cos(),
                1.0 - 4.0 * (cycle - 0.5).abs(),
                2.0 * cycle - 1.0,
                if cycle < 0.5 { 1.0 } else { -1.0 },
                (TAU * (2.0 * u + 14.0 * u * u)).sin(),
                (-6.0 * u).exp() * (TAU * 24.0 * u).sin(),
                (-((u - 0.5) / 0.08).powi(2)).exp(),
                (cycle * 8.0).floor() / 7.0,
                (1.8 * sine).clamp(-0.65, 0.65),
                0.3 * random,
                0.375,
            ],
        )?;
        emit_reals(
            &mut writer,
            &phases,
            [sine, (phase - TAU / 3.0).sin(), (phase + TAU / 3.0).sin()],
        )?;
        emit_reals(
            &mut writer,
            &power,
            [
                12.0 + 8.0 * (1.0 + sine),
                5.0 + 25.0 * pulse,
                6.0 + 4.0 * (1.0 + (phase * 0.5).cos()),
                2.0 + 3.0 * (1.0 + (phase * 2.0).sin()),
            ],
        )?;
        emit_reals(
            &mut writer,
            &flows,
            [
                25.0 * (1.0 + (TAU * u).sin()),
                -35.0 - 20.0 * pulse,
                40.0 * (TAU * 2.0 * u).sin(),
                12.0 * (1.0 + sine),
            ],
        )?;
        // Slower integer and bit channels make both individual steps and
        // dense column averages visible when their scopes are stacked.
        if i % 8 == 0 {
            for (k, &signal) in queues.iter().enumerate() {
                if k == 2 && (512..560).contains(&i) {
                    writer.emit_logic_str(signal, b"xxxxxxxx")?;
                } else if k == 1 && (768..800).contains(&i) {
                    writer.emit_logic_str(signal, b"zzzzzzzz")?;
                } else {
                    writer.emit_u64(signal, (i / 8 + k as u64 * 5) % 17)?;
                }
            }
            for (k, &signal) in activity.iter().enumerate() {
                let period = 16 << k;
                writer.emit_bit(signal, Logic::from(i % period < period / 2))?;
            }
        }
        writer.emit_real(
            edges[0],
            if (384..448).contains(&i) {
                f64::NAN
            } else {
                quantize(sine)
            },
        )?;
        writer.emit_real(
            edges[1],
            match i {
                640..672 => f64::INFINITY,
                672..704 => f64::NEG_INFINITY,
                _ => quantize(sine),
            },
        )?;
        writer.emit_real(edges[2], if i == 513 { 3.0 } else { quantize(sine) })?;
        if i >= 128 && i % 16 == 0 {
            writer.emit_real(edges[3], quantize(0.5 + 0.25 * sine))?;
        }
        if i % 4 == 0 {
            if i < 16 {
                writer.emit_logic_str(signed, b"xxxxxxxxxxxxxxxx")?;
            } else {
                let adc = (sine * 24_000.0).round() as i16;
                writer.emit_u64(signed, u64::from(adc as u16))?;
            }
            writer.emit_u64(unsigned, ((sine + 1.0) * 2047.5).round() as u64)?;
            writer.emit_u64(float32, u64::from((quantize(sine) as f32).to_bits()))?;
            writer.emit_u64(float64, quantize(sine).to_bits())?;
        }
    }
    writer.close()
}

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args_os().skip(1);
    let path = args.next().map(PathBuf::from).unwrap_or_else(|| {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/analog_showcase.vtr")
    });
    anyhow::ensure!(args.next().is_none(), "usage: analog_showcase [OUTPUT.vtr]");
    generate(&path)?;
    let reader = Reader::open_with(
        &path,
        ReadOptions {
            verify_crc: true,
            ..ReadOptions::default()
        },
    )?;
    let bytes = std::fs::metadata(&path)?.len();
    anyhow::ensure!(
        bytes <= MAX_BYTES,
        "trace exceeds the {MAX_BYTES}-byte budget: {bytes} bytes"
    );
    println!(
        "{}: {} signals, {} ns, {} bytes",
        path.display(),
        reader.signal_count(),
        INTERVALS * STEP_NS,
        bytes,
    );
    Ok(())
}
