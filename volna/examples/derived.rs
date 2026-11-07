//! A reproducible derived-signal playground with a streaming queue and sampled GPU statistics.

#[path = "support/derived_stats.rs"]
mod stats;
#[cfg(test)]
#[path = "support/derived_tests.rs"]
mod tests;
#[path = "support/derived_workspace.rs"]
mod workspace;

use anyhow::{Context, Result, ensure};
use std::path::{Path, PathBuf};
use vtr::{
    Attribute, Direction, ExecutionMode, LogicStates, NodeId, Reader, Recorder, SignalId,
    SignalKind, SourceOptions, Value, VarType, Writer, WriterOptions,
};

const CLOCK_PS: u64 = 476;
const SAMPLE_CLOCKS: u64 = 100;
const MAX_BYTES: u64 = 100_000;
const QUEUE_CAPACITY: u64 = 8;

fn signal(
    writer: &mut Writer,
    scope: NodeId,
    name: &str,
    width: u32,
    unit: &str,
    description: &str,
) -> Result<SignalId> {
    let attributes = [("unit", unit), ("description", description)]
        .into_iter()
        .map(|(key, text)| {
            Ok(Attribute {
                key: writer.intern(key)?,
                value: Value::Text(text.into()),
            })
        })
        .collect::<vtr::Result<_>>()?;
    let real = width == 0;
    Ok(writer.signal(
        SourceOptions {
            name: name.into(),
            parent: Some(scope),
            attributes,
        },
        if real {
            SignalKind::Real
        } else {
            SignalKind::bits(width, LogicStates::Four)
        },
        if real { VarType::Real } else { VarType::Wire },
        Direction::Output,
    )?)
}

fn quantize(value: f64) -> f64 {
    (value * 4096.0).round() / 4096.0
}

fn generate(path: &Path, samples: &stats::Samples) -> Result<()> {
    let mut options = WriterOptions {
        mode: ExecutionMode::Inline,
        ..Default::default()
    };
    options.metadata.timescale = -12;
    options.metadata.writer = "Volna derived-signal playground".into();
    options.metadata.comment = format!(
        "Synthetic streaming queue and sensors; stats are an excerpt of {}. \
         Independent workloads share a rebased educational timeline. Recording {}. {}",
        samples.source, samples.recording_id, samples.description
    );
    let mut writer = Writer::create_with(path, options)?;
    let stream = writer.scope("stream", None)?;
    let sensors = writer.scope("sensors", None)?;
    let stats_scope = writer.scope("stats", None)?;
    let clk = signal(
        &mut writer,
        stream,
        "clk",
        1,
        "",
        "476 ps period, matching the source GPU shader clock; rising edges at 238 + 476*n ps",
    )?;
    let reset_n = signal(
        &mut writer,
        stream,
        "reset_n",
        1,
        "",
        "Active-low reset during the first four clocks",
    )?;
    let valid = signal(
        &mut writer,
        stream,
        "valid",
        1,
        "",
        "Producer holds valid and payload until a rising-edge handshake",
    )?;
    let ready = signal(
        &mut writer,
        stream,
        "ready",
        1,
        "",
        "Eight-entry queue has space; no combinational bypass",
    )?;
    let payload = signal(
        &mut writer,
        stream,
        "payload_bytes",
        8,
        "bytes",
        "Size of the offered packet: 16, 32, 64, or 128 bytes; zero when idle",
    )?;
    let depth = signal(
        &mut writer,
        stream,
        "queue_depth",
        4,
        "packets",
        "Accepted packets minus completed packets, bounded by eight",
    )?;
    let accepted = signal(
        &mut writer,
        stream,
        "accepted_count",
        16,
        "packets",
        "Cumulative rising-edge handshakes since reset",
    )?;
    let completed = signal(
        &mut writer,
        stream,
        "completed_count",
        16,
        "packets",
        "Cumulative serviced packets since reset",
    )?;
    let bytes = signal(
        &mut writer,
        stream,
        "accepted_bytes",
        32,
        "bytes",
        "Cumulative payload bytes accepted since reset",
    )?;
    let adc = signal(
        &mut writer,
        sensors,
        "adc_code",
        12,
        "codes",
        "Signed two's-complement ADC, 1024 codes per volt; unknown until the first conversion at clock 10",
    )?;
    let voltage = signal(
        &mut writer,
        sensors,
        "supply_v",
        0,
        "V",
        "Supply droop follows synthetic queue occupancy",
    )?;
    let current = signal(
        &mut writer,
        sensors,
        "load_a",
        0,
        "A",
        "Idle current plus queue-dependent load and periodic ripple",
    )?;
    let temperature = signal(
        &mut writer,
        sensors,
        "temperature_c",
        0,
        "degC",
        "First-order thermal response to the synthetic load",
    )?;
    let sample = signal(
        &mut writer,
        stats_scope,
        "sample_number",
        16,
        "samples",
        "Interval number; changes once per 100 clocks, including intervals with unchanged counters",
    )?;
    let clocks = signal(
        &mut writer,
        stats_scope,
        "clock_count",
        32,
        "clocks",
        "Elapsed clocks in the excerpt; increments by 100 per sample",
    )?;
    let counters = samples.declare(&mut writer, stats_scope)?;

    let mut queue = 0_u64;
    let mut accepted_count = 0_u64;
    let mut completed_count = 0_u64;
    let mut accepted_bytes = 0_u64;
    let mut pending = None;
    let mut temp = 35.0;
    let cycles = samples.rows.len() as u64 * SAMPLE_CLOCKS;
    for cycle in 0..=cycles {
        writer.advance_time(cycle * CLOCK_PS)?;
        writer.emit_u64(clk, 0)?;
        let running = cycle >= 4 && cycle < cycles;
        if pending.is_none() && running && cycle % 8 != 0 {
            pending = Some([16, 32, 64, 128][accepted_count as usize % 4]);
        }
        let offering = running && pending.is_some();
        let space = running && queue < QUEUE_CAPACITY;
        writer.emit_u64(reset_n, u64::from(cycle >= 4))?;
        writer.emit_u64(valid, u64::from(offering))?;
        writer.emit_u64(ready, u64::from(space))?;
        writer.emit_u64(payload, if offering { pending.unwrap() } else { 0 })?;
        if cycle == 0 {
            for id in [depth, accepted, completed, bytes] {
                writer.emit_u64(id, 0)?;
            }
            writer.emit_packed(adc, 4, &[0xaa; 3])?;
            writer.emit_u64(sample, 0)?;
            writer.emit_u64(clocks, 0)?;
            stats::Samples::emit_baseline(&mut writer, &counters)?;
        }
        if cycle % 10 == 0 {
            let phase = std::f64::consts::TAU * cycle as f64 / 400.0;
            let amps = 0.1 + 0.075 * queue as f64 + 0.025 * phase.sin();
            let volts = 1.2 - 0.02 * queue as f64;
            temp += (35.0 + amps * volts * 12.0 - temp) / 24.0;
            writer.emit_real(voltage, quantize(volts))?;
            writer.emit_real(current, quantize(amps))?;
            writer.emit_real(temperature, quantize(temp))?;
            if cycle >= 4 {
                let code = (1536.0 * phase.sin()).round() as i64;
                writer.emit_u64(adc, (code & 0xfff) as u64)?;
            }
        }
        if cycle > 0 && cycle % SAMPLE_CLOCKS == 0 {
            let interval = cycle / SAMPLE_CLOCKS;
            writer.emit_u64(sample, interval)?;
            writer.emit_u64(clocks, cycle)?;
            samples.emit(&mut writer, &counters, interval as usize - 1)?;
        }
        if cycle == cycles {
            break;
        }
        writer.advance_time(cycle * CLOCK_PS + CLOCK_PS / 2)?;
        writer.emit_u64(clk, 1)?;
        let paused = (160..200).contains(&(cycle % 400));
        let service_period = [2, 4, 7, 3][(cycle / 800) as usize % 4];
        let serviced = running && queue > 0 && !paused && cycle % service_period == 0;
        let transfer = offering && space;
        queue = queue + u64::from(transfer) - u64::from(serviced);
        if transfer {
            accepted_count += 1;
            accepted_bytes += pending.take().unwrap();
        }
        completed_count += u64::from(serviced);
        writer.emit_u64(depth, queue)?;
        writer.emit_u64(accepted, accepted_count)?;
        writer.emit_u64(completed, completed_count)?;
        writer.emit_u64(bytes, accepted_bytes)?;
    }
    // Whole-range reductions are half-open; hold the final statistics sample for one clock.
    writer.advance_time(cycles * CLOCK_PS + CLOCK_PS / 2)?;
    writer.emit_u64(clk, 1)?;
    writer.advance_time((cycles + 1) * CLOCK_PS)?;
    writer.emit_u64(clk, 0)?;
    writer.close()?;
    Reader::open(path)?.validate()?;
    let size = std::fs::metadata(path)?.len();
    ensure!(
        size < MAX_BYTES,
        "trace is {size} bytes; limit is {MAX_BYTES}"
    );
    println!(
        "{}: {size} bytes, {cycles} measured clocks + one clock post-roll, {} statistics intervals",
        path.display(),
        samples.rows.len()
    );
    Ok(())
}

fn main() -> Result<()> {
    let mut args = std::env::args_os().skip(1);
    let first = args.next();
    if first.as_deref() == Some(std::ffi::OsStr::new("--extract-stats")) {
        let source = args.next().context("source VTR required")?;
        let output = args.next().context("output JSON required")?;
        let start = args
            .next()
            .map(|v| v.to_string_lossy().parse::<u64>())
            .transpose()?
            .unwrap_or(79_400);
        ensure!(
            args.next().is_none(),
            "usage: derived --extract-stats SOURCE.vtr OUTPUT.json [START_GPU_CYCLE]"
        );
        return stats::Samples::extract(Path::new(&source), Path::new(&output), start);
    }
    let path = first.map_or_else(
        || PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/derived.vtr"),
        PathBuf::from,
    );
    ensure!(args.next().is_none(), "usage: derived [OUTPUT.vtr]");
    let samples = stats::Samples::bundled()?;
    generate(&path, &samples)?;
    workspace::write(&path).context("derived workspace")
}
