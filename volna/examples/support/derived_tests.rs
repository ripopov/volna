use super::*;
use std::collections::BTreeMap;
use volna_core::{
    App,
    computed::{MeasurementResult, MeasurementStatus},
    wave::model::DisplayedSignal,
    workspace::Workspace,
};
use volna_trace::data::{SignalShape, WaveValue};

fn example() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/derived.vtr")
}

fn open(path: &Path) -> Result<App> {
    workspace::open(
        path,
        &std::fs::read(path.with_extension("vtr.volna.example.json"))?,
    )
}

fn row<'a>(app: &'a App, name: &str) -> &'a DisplayedSignal {
    app.panels
        .iter()
        .filter_map(|p| p.kind.waves())
        .flat_map(|w| w.items())
        .filter_map(|entry| entry.signal())
        .find(|row| row.name == name)
        .unwrap_or_else(|| panic!("missing {name}"))
}

fn number(app: &App, name: &str, time: u64) -> Option<f64> {
    let history = row(app, name).history.as_ref().unwrap();
    match history.value(history.index_at(time)) {
        WaveValue::Real(value) => Some(value),
        WaveValue::Bits(value) if value == "1" => Some(1.0),
        WaveValue::Bits(value) if value == "0" => Some(0.0),
        _ => None,
    }
}

fn scalar(app: &App, name: &str) -> f64 {
    let Some(MeasurementResult::Scalar(value)) = &row(app, name).measurement else {
        panic!("{name} is not a measurement");
    };
    assert_eq!(value.status, MeasurementStatus::Complete, "{name}");
    assert_eq!(value.coverage(), 1.0, "{name}");
    value.value.unwrap()
}

fn recorded(reader: &Reader) -> BTreeMap<String, vtr::trace::SignalData> {
    reader
        .declarations()
        .nodes
        .iter()
        .enumerate()
        .filter_map(|(index, node)| {
            let vtr::declarations::NodeData::Signal { signal, .. } = node.data else {
                return None;
            };
            Some((
                reader.full_path(NodeId(index as u32)).unwrap(),
                reader.load_signal(signal).unwrap(),
            ))
        })
        .collect()
}

fn raw(recorded: &BTreeMap<String, vtr::trace::SignalData>, name: &str, time: u64) -> u64 {
    recorded[name].value_at(time).as_u64().unwrap()
}

fn real(recorded: &BTreeMap<String, vtr::trace::SignalData>, name: &str, time: u64) -> f64 {
    let vtr::SignalValue::Real(value) = recorded[name].value_at(time) else {
        panic!("not real: {name}");
    };
    value
}

fn close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() < 1e-9 * expected.abs().max(1.0),
        "{actual} != {expected}"
    );
}

// A presenter opens the delivered example: every view resolves and every saved measurement has a complete result.
#[test]
fn delivered_workspace_is_ready_for_a_demo() -> Result<()> {
    let path = example();
    assert!(std::fs::metadata(&path)?.len() < MAX_BYTES);
    let reader = Reader::open(&path)?;
    reader.validate()?;
    assert_eq!(reader.signal_count(), 20);
    assert_eq!(
        reader
            .declarations()
            .signals
            .iter()
            .filter(|&&kind| kind == SignalKind::Real)
            .count(),
        3
    );
    let app = open(&path)?;
    let titles = app.panels.iter().map(|p| p.title()).collect::<Vec<_>>();
    for title in [
        "Handshake & queue",
        "Power & sensors",
        "GPU statistics",
        "Bandwidth samples [GB/s]",
    ] {
        assert!(titles.iter().any(|t| t == title), "missing {title}");
    }
    for waves in app.panels.iter().filter_map(|p| p.kind.waves()) {
        for signal in waves.items().iter().filter_map(|entry| entry.signal()) {
            assert!(
                signal.error.is_none(),
                "{}: {:?}",
                signal.name,
                signal.error
            );
            assert!(signal.history.is_some(), "{} not loaded", signal.name);
            if signal.measurement.is_some() {
                assert!(scalar(&app, &signal.name).is_finite());
            }
        }
    }
    assert_eq!(row(&app, "ADC voltage [V]").shape, SignalShape::Real);
    let table = app.panels.iter().find_map(|p| p.kind.table()).unwrap();
    assert_eq!(table.len(), 64);
    assert_eq!(table.state, volna_core::table::TableState::Ready);
    Ok(())
}

// A user counts rising-edge handshakes and bytes, then checks queue occupancy against accepted and completed packets.
#[test]
fn handshake_measurements_and_signed_adc_match_recorded_behavior() -> Result<()> {
    let path = example();
    let reader = Reader::open(&path)?;
    let recorded = recorded(&reader);
    let app = open(&path)?;
    let end = reader.waveform_time_range().unwrap().last;
    let mut packets = 0;
    let mut bytes = 0;
    let mut stalled = 0;
    let mut peak = 0;
    for time in (CLOCK_PS / 2..end).step_by(CLOCK_PS as usize) {
        let valid = raw(&recorded, "stream.reset_n", time) == 1
            && raw(&recorded, "stream.valid", time) == 1;
        let ready = raw(&recorded, "stream.ready", time) == 1;
        packets += u64::from(valid && ready);
        if valid && ready {
            bytes += raw(&recorded, "stream.payload_bytes", time);
        }
        stalled += u64::from(valid && !ready);
        let depth = raw(&recorded, "stream.queue_depth", time);
        peak = peak.max(depth);
        assert!(depth <= 8);
        assert_eq!(
            depth,
            raw(&recorded, "stream.accepted_count", time)
                - raw(&recorded, "stream.completed_count", time)
        );
        assert_eq!(
            number(&app, "In flight [packets]", time),
            Some(depth as f64)
        );
        assert_eq!(
            number(&app, "Transfer · rising edge", time),
            Some(f64::from(valid && ready))
        );
    }
    assert!(stalled > 0 && packets > 0);
    assert_eq!(scalar(&app, "Transferred packets"), packets as f64);
    assert_eq!(scalar(&app, "Transferred bytes"), bytes as f64);
    assert_eq!(scalar(&app, "Backpressured cycles"), stalled as f64);
    assert_eq!(scalar(&app, "Peak queue [packets]"), peak as f64);
    assert_eq!(raw(&recorded, "stream.accepted_count", end), packets);
    assert_eq!(raw(&recorded, "stream.accepted_bytes", end), bytes);
    assert_eq!(number(&app, "ADC voltage [V]", 0), None);
    assert_eq!(number(&app, "ADC voltage [V]", 100 * CLOCK_PS), Some(1.5));
    assert_eq!(number(&app, "ADC voltage [V]", 300 * CLOCK_PS), Some(-1.5));
    Ok(())
}

// A user integrates measured supply power; the displayed waveform and scalar energy must agree with held sensor values in SI units.
#[test]
fn power_measurements_have_correct_physical_units() -> Result<()> {
    let path = example();
    let reader = Reader::open(&path)?;
    let recorded = recorded(&reader);
    let app = open(&path)?;
    let end = reader.waveform_time_range().unwrap().last;
    let mut energy = 0.0;
    for time in (0..end).step_by((10 * CLOCK_PS) as usize) {
        let power =
            real(&recorded, "sensors.supply_v", time) * real(&recorded, "sensors.load_a", time);
        assert_eq!(number(&app, "Power [W]", time), Some(power));
        energy += power * (10 * CLOCK_PS).min(end - time) as f64 * 1e-12;
    }
    close(number(&app, "Energy [nJ]", end).unwrap(), energy * 1e9);
    close(scalar(&app, "Total energy [nJ]"), energy * 1e9);
    close(
        scalar(&app, "Mean power [W]"),
        energy / (end as f64 * 1e-12),
    );
    Ok(())
}

// A user samples interval counters by sample number, so unchanged counts still contribute once per window and the final window is included.
#[test]
fn gpu_metrics_count_all_source_windows_and_preserve_bandwidth() -> Result<()> {
    let path = example();
    let reader = Reader::open(&path)?;
    let recorded = recorded(&reader);
    let app = open(&path)?;
    assert_eq!(scalar(&app, "Total LDS bytes"), 299_344.0);
    assert_eq!(scalar(&app, "Total LDS requests"), 2_206.0);
    close(
        scalar(&app, "Weighted extra cycles/request"),
        2640.0 / 2206.0,
    );
    assert_eq!(scalar(&app, "Mean LDS traffic [bytes/clock]"), 46.7725);
    let mut transferred_bytes = 0.0;
    for sample in 1..=64 {
        let clocks = sample * 100;
        let time = clocks * CLOCK_PS;
        assert_eq!(raw(&recorded, "stats.sample_number", time), sample);
        assert_eq!(raw(&recorded, "stats.clock_count", time), clocks);
        let bandwidth = number(&app, "LDS traffic [GB/s]", time).unwrap();
        transferred_bytes += bandwidth * 1e9 * (100 * CLOCK_PS) as f64 * 1e-12;
        assert_eq!(
            number(&app, "LDS traffic [GB/s]", time + CLOCK_PS / 2),
            Some(bandwidth)
        );
    }
    close(transferred_bytes, 299_344.0);
    let requests = row(&app, "Total LDS requests");
    let Some(MeasurementResult::Scalar(value)) = &requests.measurement else {
        unreachable!()
    };
    assert_eq!(value.captures.unwrap().total, 64);
    Ok(())
}

// Regenerating the demo preserves a user's autosaved workspace; the example still reopens with the same visible results.
#[test]
fn regeneration_and_workspace_round_trip_keep_the_demo_complete() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("derived.vtr");
    generate(&path, &stats::Samples::bundled()?)?;
    let personal = path.with_extension("vtr.volna.json");
    std::fs::write(&personal, b"personal workspace")?;
    workspace::write(&path)?;
    assert_eq!(std::fs::read(personal)?, b"personal workspace");
    let app = open(&path)?;
    let captured = Workspace::capture(&app, |_| Ok(Some("derived.vtr".into())), None)?;
    let restored = workspace::open(&path, &captured.to_bytes()?)?;
    assert_eq!(scalar(&restored, "Total LDS bytes"), 299_344.0);
    assert_eq!(
        scalar(&restored, "Transferred bytes"),
        scalar(&open(&example())?, "Transferred bytes")
    );
    assert_eq!(
        restored
            .panels
            .iter()
            .filter(|p| p.kind.table().is_some())
            .count(),
        1
    );
    assert_eq!(
        captured.to_bytes()?,
        Workspace::capture(&restored, |_| Ok(Some("derived.vtr".into())), None)?.to_bytes()?
    );
    Ok(())
}
