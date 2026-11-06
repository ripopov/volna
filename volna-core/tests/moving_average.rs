//! Moving-average numerics and formula preview, persistence, and undo.

use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use volna_core::{
    Action, App, Command, LoadRequest,
    computed::{Command as Average, ComputationStatus, Interpretation},
    data::NumericKind,
    moving_average::{MeanHistory, Settings},
    testing::a_all,
    workspace::Workspace,
};
use volna_trace::{
    data::{Bit, SignalHistory, SignalShape, WaveValue, history::VecHistory},
    remote::memory::MemoryBudget,
    session::OpenSpec,
};
use vtr::Recorder;

fn real() -> Interpretation {
    Interpretation {
        kind: NumericKind::Real,
        projection: None,
    }
}

fn settings() -> Settings {
    Settings {
        window: 4,
        interval: 1,
        origin: 0,
        end: 20,
        partial: false,
        skip_unknown: false,
    }
}

fn source(origin: u64) -> VecHistory {
    VecHistory {
        shape: SignalShape::Real,
        times: [0, 3, 7, 11, 13].map(|t| origin + t).into(),
        values: vec![
            WaveValue::Real(2.0),
            WaveValue::Real(8.0),
            WaveValue::Unavailable,
            WaveValue::Real(-4.0),
            WaveValue::Real(6.0),
        ],
        initial: WaveValue::Unavailable,
    }
}

fn number(history: &dyn SignalHistory, time: u64) -> Option<f64> {
    match history.value(history.index_at(time)) {
        WaveValue::Real(v) if v.is_finite() => Some(v),
        _ => None,
    }
}

#[test]
fn numeric_inputs_use_the_analog_interpretation_and_gap_policies() {
    let encodings = [
        (
            SignalShape::Vector { width: 8 },
            NumericKind::Unsigned,
            "11111110".into(),
            "00000110".into(),
            192.0,
        ),
        (
            SignalShape::Vector { width: 8 },
            NumericKind::Signed,
            "11111110".into(),
            "00000110".into(),
            0.0,
        ),
        (
            SignalShape::Vector { width: 16 },
            NumericKind::Float,
            format!("{:016b}", 0xc000_u16),
            format!("{:016b}", 0x4600_u16),
            0.0,
        ),
        (
            SignalShape::Vector { width: 32 },
            NumericKind::Float,
            format!("{:032b}", (-2.0_f32).to_bits()),
            format!("{:032b}", 6.0_f32.to_bits()),
            0.0,
        ),
        (
            SignalShape::Vector { width: 64 },
            NumericKind::Float,
            format!("{:064b}", (-2.0_f64).to_bits()),
            format!("{:064b}", 6.0_f64.to_bits()),
            0.0,
        ),
        (
            SignalShape::Bit,
            NumericKind::Unsigned,
            "1".into(),
            "0".into(),
            0.75,
        ),
        (
            SignalShape::Bit,
            NumericKind::Unsigned,
            "H".into(),
            "L".into(),
            0.75,
        ),
        (
            SignalShape::Vector { width: 96 },
            NumericKind::Unsigned,
            format!("{:096b}", 2_u128),
            format!("{:096b}", 6_u128),
            3.0,
        ),
    ];
    for (shape, kind, first, second, expected) in encodings {
        for unknown in ['x', 'z'] {
            let input = VecHistory {
                shape,
                times: vec![0, 3, 7, 11],
                values: vec![
                    WaveValue::Bits(first.clone()),
                    WaveValue::Bits(second.clone()),
                    WaveValue::Bits(unknown.to_string().repeat(shape.width() as usize)),
                    WaveValue::Bits(second.clone()),
                ],
                initial: WaveValue::Unavailable,
            };
            let interpretation = Interpretation {
                kind,
                projection: None,
            };
            let result = MeanHistory::build(
                &input,
                interpretation,
                settings(),
                &MemoryBudget::new(1 << 20),
                &AtomicBool::new(false),
            )
            .unwrap();
            assert_eq!(number(&result, 4), Some(expected), "{shape:?} {kind:?}");
            assert_eq!(number(&result, 8), None);
            let skip = MeanHistory::build(
                &input,
                interpretation,
                Settings {
                    skip_unknown: true,
                    ..settings()
                },
                &MemoryBudget::new(1 << 20),
                &AtomicBool::new(false),
            )
            .unwrap();
            let volna_core::wave::analog::Sample::Value(known) =
                volna_core::wave::analog::sample(&input, kind, Some(1))
            else {
                panic!("known input");
            };
            assert_eq!(number(&skip, 8), Some(known));
            assert_eq!(number(&skip, 11), None);
        }
    }
}

#[test]
fn projected_inputs_ignore_unselected_unknown_bits_and_release_scratch() {
    use volna_core::data::projection::BitSlice;
    let input = VecHistory {
        shape: SignalShape::Vector { width: 8 },
        times: vec![0, 3, 7],
        values: vec![
            WaveValue::Bits("x1111110".into()),
            WaveValue::Bits("z0000110".into()),
            WaveValue::Bits("00000x00".into()),
        ],
        initial: WaveValue::Unavailable,
    };
    let interpretation = Interpretation {
        kind: NumericKind::Signed,
        projection: Some(BitSlice { first: 1, last: 7 }),
    };
    let budget = MemoryBudget::new(1 << 20);
    let result = MeanHistory::build(
        &input,
        interpretation,
        settings(),
        &budget,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(number(&result, 4), Some(0.0));
    assert_eq!(number(&result, 8), None);
    assert_eq!(budget.used(), result.resident_bytes());
    drop(result);
    assert_eq!(budget.used(), 0);
    let reversed = Interpretation {
        kind: NumericKind::Unsigned,
        projection: Some(BitSlice { first: 7, last: 4 }),
    };
    let result = MeanHistory::build(
        &input,
        reversed,
        settings(),
        &budget,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(number(&result, 4), Some(6.75));
    assert!(
        MeanHistory::build(
            &input,
            Interpretation {
                projection: Some(BitSlice { first: 0, last: 8 }),
                ..interpretation
            },
            settings(),
            &budget,
            &AtomicBool::new(false)
        )
        .is_err()
    );
}

#[test]
fn held_time_windows_match_an_independent_tick_integral_at_large_origins() {
    for origin in [0, (1_u64 << 60) + 17] {
        for window in [1, 4, 7, 25] {
            for interval in [1, 3, 9] {
                for partial in [false, true] {
                    for skip_unknown in [false, true] {
                        let settings = Settings {
                            window,
                            interval,
                            origin,
                            end: origin + 20,
                            partial,
                            skip_unknown,
                        };
                        let input = source(origin);
                        let result = MeanHistory::build(
                            &input,
                            real(),
                            settings,
                            &MemoryBudget::new(1 << 20),
                            &AtomicBool::new(false),
                        )
                        .unwrap();
                        for i in 0..result.len() {
                            let t = result.time(i);
                            let start = t.saturating_sub(window).max(origin);
                            let values: Vec<_> =
                                (start..t).filter_map(|t| number(&input, t)).collect();
                            let expected = if values.is_empty()
                                || (!partial && t - origin < window)
                                || (!skip_unknown && values.len() as u64 != t - start)
                            {
                                None
                            } else {
                                Some(values.iter().sum::<f64>() / values.len() as f64)
                            };
                            match (number(&result, t), expected) {
                                (Some(a), Some(b)) => {
                                    assert!((a - b).abs() < 1e-11, "{settings:?}, {t}: {a} != {b}");
                                }
                                (a, b) => assert_eq!(a, b, "{settings:?}, {t}"),
                            }
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn fractional_durations_are_exact_and_subtick_windows_are_rejected() {
    use volna_core::{moving_average::duration, wave::timeline::TimeBase};
    let base = TimeBase::si(-9);
    assert_eq!(duration("4 us", base).unwrap(), 4000);
    assert_eq!(duration("0.25 µs", base).unwrap(), 250);
    assert_eq!(duration("1.5 ns", TimeBase::si(-12)).unwrap(), 1500);
    for text in [
        "0",
        "NaN",
        "-1 us",
        "1.5 ns",
        "0.1 ps",
        "1e3 ns",
        "1.2.3 ns",
        "99999999999999999999999999999999999999999999 s",
    ] {
        assert!(duration(text, base).is_err(), "{text}");
    }
    let custom = TimeBase {
        timescale: 0,
        unit: Some("cycles"),
    };
    assert_eq!(duration("4 cycles", custom).unwrap(), 4);
    assert!(duration("4 ns", custom).is_err());
}

struct Observed {
    reads: AtomicUsize,
    cancel: Arc<AtomicBool>,
    budget: MemoryBudget,
    cancel_after: usize,
}
impl SignalHistory for Observed {
    fn shape(&self) -> SignalShape {
        SignalShape::Real
    }
    fn len(&self) -> usize {
        100_000
    }
    fn time(&self, i: usize) -> u64 {
        assert!(self.budget.used() > 0, "reserve before scanning input");
        if self.reads.fetch_add(1, Ordering::Relaxed) >= self.cancel_after {
            self.cancel.store(true, Ordering::Relaxed);
        }
        i as u64
    }
    fn value(&self, _: Option<usize>) -> WaveValue {
        WaveValue::Real(1.0)
    }
    fn bit(&self, _: Option<usize>) -> Bit {
        Bit::Other
    }
}

#[test]
fn refusal_precedes_reads_and_cancellation_releases_admitted_output() {
    let cancel = Arc::new(AtomicBool::new(false));
    let budget = MemoryBudget::new(0);
    let input = Observed {
        reads: AtomicUsize::new(0),
        cancel: cancel.clone(),
        budget: budget.clone(),
        cancel_after: 100,
    };
    assert!(MeanHistory::build(&input, real(), settings(), &budget, &cancel).is_err());
    assert_eq!(input.reads.load(Ordering::Relaxed), 0);
    budget.set_limit(8 << 20);
    budget.set_object_limit(1);
    assert!(MeanHistory::build(&input, real(), settings(), &budget, &cancel).is_err());
    assert_eq!(input.reads.load(Ordering::Relaxed), 0);
    budget.set_object_limit(8 << 20);
    assert!(
        MeanHistory::build(
            &input,
            real(),
            Settings {
                end: 100_000,
                ..settings()
            },
            &budget,
            &cancel
        )
        .is_err()
    );
    assert!(cancel.load(Ordering::Relaxed));
    assert!(input.reads.load(Ordering::Relaxed) < 200);
    assert_eq!(budget.used(), 0);
    let bytes;
    {
        let result = MeanHistory::build(
            &source(0),
            real(),
            settings(),
            &budget,
            &AtomicBool::new(false),
        )
        .unwrap();
        bytes = result.resident_bytes();
        assert!(bytes > 0);
        assert_eq!(budget.used(), bytes);
    }
    assert_eq!(budget.used(), 0);
    assert!(
        MeanHistory::build(
            &source(0),
            real(),
            Settings {
                end: 1_000_001,
                ..settings()
            },
            &budget,
            &AtomicBool::new(false)
        )
        .is_err()
    );
}

fn fixture() -> (tempfile::NamedTempFile, App) {
    let file = tempfile::NamedTempFile::new().unwrap();
    let mut options = vtr::WriterOptions::default();
    options.metadata.timescale = -9;
    let mut writer = vtr::Writer::create_with(file.path(), options).unwrap();
    let real = writer
        .signal(
            vtr::SourceOptions::new("adc", None),
            vtr::SignalKind::Real,
            vtr::VarType::Real,
            vtr::Direction::Output,
        )
        .unwrap();
    for (time, value) in [
        (0, 2.0),
        (3000, 8.0),
        (7000, f64::NAN),
        (11000, -4.0),
        (13000, 6.0),
        (20000, 6.0),
    ] {
        writer.advance_time(time).unwrap();
        writer.emit_real(real, value).unwrap();
    }
    writer.close().unwrap();
    let mut app = App::new();
    app.set_session(OpenSpec::Path(file.path().into()).open().unwrap());
    app.handle(Command::AddVars(a_all([0])));
    pump(&mut app);
    (file, app)
}

fn pump(app: &mut App) {
    for _ in 0..20 {
        let requests = app.take_requests();
        if requests.is_empty() {
            return;
        }
        for request in requests {
            app.deliver(request.perform());
        }
    }
    panic!("load loop did not settle");
}
fn command(app: &mut App, command: Average) {
    app.handle(Command::Derived(app.panels.focused_id(), command));
}
fn draft(app: &mut App, window: &str) {
    let mut draft = app.derived_editor.as_ref().unwrap().draft.clone();
    draft.name = "smoothed".into();
    draft.formula = format!(
        "moving_avg({}, {window})",
        app.derived_editor.as_ref().unwrap().bindings[0].alias
    );
    draft.interval = "250 ns".into();
    command(app, Average::Update(draft));
    command(app, Average::Preview);
}
fn output(app: &App) -> &dyn SignalHistory {
    app.panels
        .focused_waves()
        .unwrap()
        .signal(1)
        .unwrap()
        .history
        .as_deref()
        .unwrap()
}

#[test]
fn create_edit_invalid_draft_restore_remove_and_undo_remain_usable() {
    let (_file, mut app) = fixture();
    let capture =
        |app: &App| Workspace::capture(app, volna_core::testing::paths("trace.vtr"), None).unwrap();
    let before = capture(&app).to_bytes().unwrap();
    let undo_steps = app.history.undo_steps().count();
    command(&mut app, Average::Open(Some(0)));
    pump(&mut app);
    assert!(matches!(
        app.derived_preview(),
        Some(ComputationStatus::Ready(_))
    ));
    assert_eq!(capture(&app).to_bytes().unwrap(), before);
    assert_eq!(app.history.undo_steps().count(), undo_steps);
    command(&mut app, Average::Close);
    assert_eq!(capture(&app).to_bytes().unwrap(), before);
    command(&mut app, Average::Open(Some(0)));
    draft(&mut app, "4 us");
    pump(&mut app);
    assert!(matches!(
        app.derived_preview(),
        Some(ComputationStatus::Ready(_))
    ));
    command(&mut app, Average::Apply);
    assert_eq!(app.history.undo_steps().count(), undo_steps + 1);
    assert_eq!(number(output(&app), 4000), Some(3.5));
    let history = app
        .panels
        .focused_waves()
        .unwrap()
        .signal(1)
        .unwrap()
        .history
        .clone()
        .unwrap();
    command(&mut app, Average::Open(Some(1)));
    draft(&mut app, "8 us");
    pump(&mut app);
    command(&mut app, Average::Close);
    assert!(Arc::ptr_eq(
        &history,
        app.panels
            .focused_waves()
            .unwrap()
            .signal(1)
            .unwrap()
            .history
            .as_ref()
            .unwrap()
    ));
    assert_eq!(app.history.undo_steps().count(), undo_steps + 1);
    command(&mut app, Average::Open(Some(1)));
    draft(&mut app, "0 us");
    command(&mut app, Average::Apply);
    assert!(app.derived_editor.as_ref().unwrap().error.is_some());
    assert!(Arc::ptr_eq(
        &history,
        app.panels
            .focused_waves()
            .unwrap()
            .signal(1)
            .unwrap()
            .history
            .as_ref()
            .unwrap()
    ));
    draft(&mut app, "8 us");
    pump(&mut app);
    command(&mut app, Average::Apply);
    assert_eq!(
        output(&app).value(output(&app).index_at(4000)),
        WaveValue::Unavailable
    );
    assert_eq!(number(output(&app), 20000), Some(4.75));

    let saved = capture(&app).to_bytes().unwrap();
    let parsed = Workspace::parse(&saved).unwrap();
    let plan = parsed
        .prepare(
            &app,
            "file:///tmp/trace.vtr",
            "file:///tmp/trace.vtr.volna.json",
        )
        .unwrap();
    assert!(plan.report().notices.is_empty());
    plan.commit(&mut app).unwrap();
    pump(&mut app);
    assert_eq!(number(output(&app), 20000), Some(4.75));
    assert_eq!(capture(&app).to_bytes().unwrap(), saved);
    command(&mut app, Average::Open(Some(1)));
    command(&mut app, Average::Remove);
    assert_eq!(app.panels.focused_waves().unwrap().items().len(), 1);
    app.handle(Command::Undo);
    pump(&mut app);
    assert_eq!(number(output(&app), 20000), Some(4.75));
    app.handle(Command::Redo);
    pump(&mut app);
    assert_eq!(app.panels.focused_waves().unwrap().items().len(), 1);
    assert!(
        app.panels
            .focused_waves()
            .unwrap()
            .signal(0)
            .unwrap()
            .history
            .is_some()
    );
}

#[test]
fn cancelled_and_superseded_previews_cannot_publish_late_results() {
    let (_file, mut app) = fixture();
    command(&mut app, Average::Open(Some(0)));
    draft(&mut app, "4 us");
    let stale = app
        .take_requests()
        .into_iter()
        .find(|r| matches!(r, LoadRequest::Derived { .. }))
        .unwrap();
    let completed = stale.perform();
    command(&mut app, Average::Stop);
    draft(&mut app, "4 us");
    let current = app
        .take_requests()
        .into_iter()
        .find(|r| matches!(r, LoadRequest::Derived { .. }))
        .unwrap();
    app.deliver(completed);
    assert!(matches!(
        app.derived_preview(),
        Some(ComputationStatus::Building)
    ));
    app.deliver(current.perform());
    assert!(matches!(
        app.derived_preview(),
        Some(ComputationStatus::Ready(_))
    ));
    draft(&mut app, "8 us");
    let running = app
        .take_requests()
        .into_iter()
        .find(|r| matches!(r, LoadRequest::Derived { .. }))
        .unwrap();
    command(&mut app, Average::Close);
    if let LoadRequest::Derived { cancelled, .. } = &running {
        assert!(cancelled.load(Ordering::Relaxed));
    }
    app.deliver(running.perform());
    assert!(app.derived_preview().is_none());
    assert_eq!(app.panels.focused_waves().unwrap().items().len(), 1);
}

#[test]
fn a_changed_row_list_rejects_apply_without_replacing_the_wrong_row() {
    let (_file, mut app) = fixture();
    command(&mut app, Average::Open(Some(0)));
    draft(&mut app, "4 us");
    pump(&mut app);
    app.handle(Command::Action(Action::RemoveSelected));
    command(&mut app, Average::Apply);
    assert!(
        app.derived_editor
            .as_ref()
            .unwrap()
            .error
            .as_ref()
            .unwrap()
            .contains("Rows changed")
    );
    assert!(app.panels.focused_waves().unwrap().items().is_empty());
}

#[test]
fn computed_row_survives_removing_its_displayed_input_and_reopens_without_it() {
    let (_file, mut app) = fixture();
    command(&mut app, Average::Open(Some(0)));
    draft(&mut app, "4 us");
    pump(&mut app);
    command(&mut app, Average::Apply);
    app.panels.focused_waves_mut().unwrap().selected = std::collections::BTreeSet::from([0]);
    app.handle(Command::Action(Action::RemoveSelected));
    pump(&mut app);
    assert_eq!(app.panels.focused_waves().unwrap().items().len(), 1);
    let saved = Workspace::capture(&app, volna_core::testing::paths("trace.vtr"), None)
        .unwrap()
        .to_bytes()
        .unwrap();
    let parsed = Workspace::parse(&saved).unwrap();
    parsed
        .prepare(
            &app,
            "file:///tmp/trace.vtr",
            "file:///tmp/trace.vtr.volna.json",
        )
        .unwrap()
        .commit(&mut app)
        .unwrap();
    pump(&mut app);
    let row = app.panels.focused_waves().unwrap().signal(0).unwrap();
    assert_eq!(number(row.history.as_deref().unwrap(), 4000), Some(3.5));
    let mut missing: serde_json::Value = serde_json::from_slice(&saved).unwrap();
    missing["panels"][0]["rows"][0]["computed"]["inputs"][0]["path"] =
        serde_json::json!(["missing"]);
    Workspace::parse(&serde_json::to_vec(&missing).unwrap())
        .unwrap()
        .prepare(
            &app,
            "file:///tmp/trace.vtr",
            "file:///tmp/trace.vtr.volna.json",
        )
        .unwrap()
        .commit(&mut app)
        .unwrap();
    pump(&mut app);
    let row = app.panels.focused_waves().unwrap().signal(0).unwrap();
    assert!(row.history.is_none());
    assert!(row.error.as_ref().unwrap().contains("missing or ambiguous"));
}

#[test]
fn changing_the_timeline_timescale_preserves_definitions_and_undo() {
    let (_file, mut app) = fixture();
    command(&mut app, Average::Open(Some(0)));
    draft(&mut app, "4 us");
    pump(&mut app);
    command(&mut app, Average::Apply);
    let fine = tempfile::NamedTempFile::new().unwrap();
    let mut options = vtr::WriterOptions::default();
    options.metadata.timescale = -12;
    let mut writer = vtr::Writer::create_with(fine.path(), options).unwrap();
    let signal = writer
        .signal(
            vtr::SourceOptions::new("fine", None),
            vtr::SignalKind::Real,
            vtr::VarType::Real,
            vtr::Direction::Output,
        )
        .unwrap();
    writer.emit_real(signal, 1.0).unwrap();
    writer.advance_time(20_000_000).unwrap();
    writer.emit_real(signal, 2.0).unwrap();
    writer.close().unwrap();
    app.add_session(OpenSpec::Path(fine.path().into()).open().unwrap())
        .unwrap();
    pump(&mut app);
    assert_eq!(app.doc.timescale(), -12);
    assert_eq!(number(output(&app), 4_000_000), Some(3.5));
    app.handle(Command::Undo);
    pump(&mut app);
    assert_eq!(number(output(&app), 4_000_000), Some(3.5));
    app.handle(Command::Undo);
    pump(&mut app);
    assert_eq!(app.panels.focused_waves().unwrap().items().len(), 1);
    app.handle(Command::Redo);
    pump(&mut app);
    assert_eq!(number(output(&app), 4_000_000), Some(3.5));
}

fn numeric_fixture() -> (tempfile::NamedTempFile, App) {
    let file = tempfile::NamedTempFile::new().unwrap();
    let mut options = vtr::WriterOptions::default();
    options.metadata.timescale = -9;
    let mut writer = vtr::Writer::create_with(file.path(), options).unwrap();
    let mut signals = Vec::new();
    for (name, width) in [("word", 8), ("float_word", 32), ("flag", 1)] {
        signals.push(
            writer
                .signal(
                    vtr::SourceOptions::new(name, None),
                    vtr::SignalKind::Bits { width, states: 4 },
                    vtr::VarType::Wire,
                    vtr::Direction::Output,
                )
                .unwrap(),
        );
    }
    for (time, word, float) in [(0, 254, -2.0_f32), (3, 6, 6.0), (20, 6, 6.0)] {
        writer.advance_time(time).unwrap();
        writer.emit_u64(signals[0], word).unwrap();
        writer
            .emit_u64(signals[1], u64::from(float.to_bits()))
            .unwrap();
        writer.emit_u64(signals[2], u64::from(word > 6)).unwrap();
    }
    writer.close().unwrap();
    let mut app = App::new();
    app.set_session(OpenSpec::Path(file.path().into()).open().unwrap());
    app.handle(Command::AddVars(a_all(0..3)));
    pump(&mut app);
    (file, app)
}

fn format_row(app: &mut App, row: usize, format: &str) {
    let panel = app.panels.focused_id();
    app.panels.waves_mut(panel).unwrap().selected = std::collections::BTreeSet::from([row]);
    app.panels.waves_mut(panel).unwrap().open_signal_menu(
        &app.doc,
        row,
        volna_core::geometry::point(0.0, 0.0),
    );
    app.handle(Command::MenuSelect(
        panel,
        volna_core::wave::model::MenuAction::Format(format.into()),
    ));
}

fn add_numeric_mean(app: &mut App, row: usize, name: &str) {
    command(app, Average::Open(Some(row)));
    let mut draft = app.derived_editor.as_ref().unwrap().draft.clone();
    draft.name = name.into();
    let binding = &app.derived_editor.as_ref().unwrap().bindings[0];
    let alias = if binding.effective_shape(&app.doc) == SignalShape::Bit {
        format!("real({})", binding.alias)
    } else {
        binding.alias.clone()
    };
    draft.formula = format!("moving_avg({alias}, 4 ns)");
    draft.interval = "1 ns".into();
    command(app, Average::Update(draft));
    command(app, Average::Preview);
    pump(app);
    command(app, Average::Apply);
    assert!(app.derived_editor.is_none());
}

fn named_mean<'a>(app: &'a App, name: &str) -> &'a volna_core::wave::model::DisplayedSignal {
    app.panels
        .focused_waves()
        .unwrap()
        .items()
        .iter()
        .filter_map(|e| e.signal())
        .find(|s| s.name == name)
        .unwrap()
}

#[test]
fn signed_unsigned_and_float_definitions_keep_distinct_results_and_restore_their_interpretation() {
    let (_file, mut app) = numeric_fixture();
    format_row(&mut app, 0, "sdec");
    assert!(app.can_derive_signal(app.panels.focused_id()));
    add_numeric_mean(&mut app, 0, "signed_mean");
    format_row(&mut app, 0, "udec");
    add_numeric_mean(&mut app, 0, "unsigned_mean");
    let float_row = app
        .panels
        .focused_waves()
        .unwrap()
        .items()
        .iter()
        .position(|e| e.name() == "float_word")
        .unwrap();
    format_row(&mut app, float_row, "float");
    add_numeric_mean(&mut app, float_row, "float_mean");
    for (name, expected) in [
        ("signed_mean", 0.0),
        ("unsigned_mean", 192.0),
        ("float_mean", 0.0),
    ] {
        let row = named_mean(&app, name);
        assert_eq!(row.shape, SignalShape::Real);
        assert!(row.bus.is_none() && row.projection.is_none());
        assert_eq!(row.translator.id(), "real");
        assert_eq!(number(row.history.as_deref().unwrap(), 4), Some(expected));
    }
    let saved = Workspace::capture(&app, volna_core::testing::paths("trace.vtr"), None)
        .unwrap()
        .to_bytes()
        .unwrap();
    Workspace::parse(&saved)
        .unwrap()
        .prepare(
            &app,
            "file:///tmp/trace.vtr",
            "file:///tmp/trace.vtr.volna.json",
        )
        .unwrap()
        .commit(&mut app)
        .unwrap();
    pump(&mut app);
    assert_eq!(
        number(
            named_mean(&app, "signed_mean").history.as_deref().unwrap(),
            4
        ),
        Some(0.0)
    );
    assert_eq!(
        number(
            named_mean(&app, "unsigned_mean")
                .history
                .as_deref()
                .unwrap(),
            4
        ),
        Some(192.0)
    );
    let signed_row = app
        .panels
        .focused_waves()
        .unwrap()
        .items()
        .iter()
        .position(|e| e.name() == "signed_mean")
        .unwrap();
    command(&mut app, Average::Open(Some(signed_row)));
    assert_eq!(
        app.derived_editor.as_ref().unwrap().bindings[0].kind,
        NumericKind::Signed
    );
    command(&mut app, Average::Preview);
    pump(&mut app);
    command(&mut app, Average::Apply);
    assert_eq!(
        number(
            named_mean(&app, "signed_mean").history.as_deref().unwrap(),
            4
        ),
        Some(0.0)
    );
}

#[test]
fn averages_of_slices_and_expanded_bits_are_real_siblings_and_keep_their_input_binding() {
    use volna_core::wave::model::MenuAction;
    let (_file, mut app) = numeric_fixture();
    let panel = app.panels.focused_id();
    app.panels
        .waves_mut(panel)
        .unwrap()
        .add_slice(&app.doc, 0, "3:0")
        .unwrap();
    pump(&mut app);
    add_numeric_mean(&mut app, 1, "nibble_mean");
    assert_eq!(
        number(
            named_mean(&app, "nibble_mean").history.as_deref().unwrap(),
            4
        ),
        Some(12.0)
    );
    let w = app.panels.waves_mut(panel).unwrap();
    w.open_signal_menu(&app.doc, 0, volna_core::geometry::point(0.0, 0.0));
    app.handle(Command::MenuSelect(panel, MenuAction::Fold(false)));
    pump(&mut app);
    add_numeric_mean(&mut app, 1, "msb_mean");
    let w = app.panels.waves(panel).unwrap();
    assert_eq!(w.items()[9].name(), "msb_mean");
    assert_eq!(w.items()[9].depth, 0);
    assert_eq!(
        number(named_mean(&app, "msb_mean").history.as_deref().unwrap(), 4),
        Some(0.75)
    );
    let saved = Workspace::capture(&app, volna_core::testing::paths("trace.vtr"), None)
        .unwrap()
        .to_bytes()
        .unwrap();
    Workspace::parse(&saved)
        .unwrap()
        .prepare(
            &app,
            "file:///tmp/trace.vtr",
            "file:///tmp/trace.vtr.volna.json",
        )
        .unwrap()
        .commit(&mut app)
        .unwrap();
    pump(&mut app);
    assert_eq!(
        number(
            named_mean(&app, "nibble_mean").history.as_deref().unwrap(),
            4
        ),
        Some(12.0)
    );
    assert_eq!(
        number(named_mean(&app, "msb_mean").history.as_deref().unwrap(), 4),
        Some(0.75)
    );
}

#[test]
fn text_and_event_sources_do_not_offer_a_numeric_average() {
    let session = OpenSpec::Bytes {
        name: "values.vtr".into(),
        bytes: include_bytes!("../../volna-trace/tests/fixtures/values.vtr").to_vec(),
    }
    .open()
    .unwrap();
    let vars: Vec<_> = session
        .hierarchy()
        .vars()
        .enumerate()
        .filter_map(|(i, v)| matches!(v.shape, SignalShape::Text | SignalShape::Event).then_some(i))
        .collect();
    assert!(!vars.is_empty());
    let mut app = App::new();
    app.set_session(session);
    app.handle(Command::AddVars(a_all(vars)));
    let panel = app.panels.focused_id();
    let count = app.panels.waves(panel).unwrap().items().len();
    for row in 0..count {
        app.panels.waves_mut(panel).unwrap().selected = std::collections::BTreeSet::from([row]);
        assert!(!app.can_derive_signal(panel));
        command(&mut app, Average::Open(Some(row)));
        assert!(app.derived_editor.is_none());
    }
}

fn preview_frame(app: &mut App) {
    let panel = app.panels.focused_id();
    let theme = volna_core::Theme::volna(true);
    app.layout_panel(
        panel,
        volna_core::geometry::Rect::from_xywh(0.0, 0.0, 1200.0, 400.0),
        &theme,
        1.0,
        &mut volna_core::scene::MonoMeasure,
    );
    app.render_panel(panel, &theme, &mut volna_core::scene::MonoMeasure);
}

#[test]
fn live_preview_rows_keep_bidirectional_layout_positions() {
    let (_file, mut app) = numeric_fixture();
    for source in 0..3 {
        command(&mut app, Average::Open(Some(source)));
        pump(&mut app);
        preview_frame(&mut app);
        let waves = app.panels.focused_waves().unwrap();
        let layout = waves.last_layout();
        let mut expected = vec![0, 1, 2];
        expected.insert(source + 1, 3);
        assert_eq!(layout.visible.as_ref(), expected);
        for (pos, &entry) in expected.iter().enumerate() {
            assert_eq!(layout.position(entry as usize), Some(pos));
            let (top, height) = layout.entry_span(entry as usize).unwrap();
            assert_eq!(layout.entry_at(top + height / 2.0), Some(entry as usize));
            assert_eq!(top, layout.row_y(pos));
        }
        assert_eq!(layout.position(4), None);
        assert_eq!(waves.items().len(), 3);
        assert!(
            app.scene()
                .texts()
                .any(|text| text == "Draft · Live preview")
        );
        command(&mut app, Average::Close);
        preview_frame(&mut app);
        assert_eq!(
            app.panels
                .focused_waves()
                .unwrap()
                .last_layout()
                .position(3),
            None
        );
    }
}

fn drag_between_rows(app: &mut App, from: usize, to: usize, fraction: f32) {
    use volna_core::geometry::{Modifiers, MouseButton, point};
    use volna_core::wave::{Drag, PointerEvent};
    preview_frame(app);
    let panel = app.panels.focused_id();
    let layout = app.panels.waves(panel).unwrap().last_layout();
    let (top, height) = layout.entry_span(from).unwrap();
    let press = point(layout.name_left + 60.0, top + height * 0.5);
    let (top, height) = layout.entry_span(to).unwrap();
    let target = point(press.x, top + height * fraction);
    app.handle(Command::Pointer(
        panel,
        PointerEvent::Down {
            position: press,
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        },
    ));
    assert!(matches!(
        app.panels.waves(panel).unwrap().drag,
        Some(Drag::Rows { .. })
    ));
    app.handle(Command::Pointer(
        panel,
        PointerEvent::Move { position: target },
    ));
    preview_frame(app);
    assert!(matches!(
        app.panels.waves(panel).unwrap().drag,
        Some(Drag::Rows { gap: Some(_), .. })
    ));
    app.handle(Command::Pointer(panel, PointerEvent::Up));
    preview_frame(app);
}

#[test]
fn dragging_rows_across_a_live_preview_preserves_its_input() {
    let (_file, mut app) = numeric_fixture();
    format_row(&mut app, 0, "sdec");
    let input = app
        .panels
        .focused_waves()
        .unwrap()
        .signal(0)
        .unwrap()
        .source
        .clone();
    command(&mut app, Average::Open(Some(0)));
    let mut draft = app.derived_editor.as_ref().unwrap().draft.clone();
    let binding = &app.derived_editor.as_ref().unwrap().bindings[0];
    let alias = if binding.effective_shape(&app.doc) == SignalShape::Bit {
        format!("real({})", binding.alias)
    } else {
        binding.alias.clone()
    };
    draft.formula = format!("moving_avg({alias}, 4 ns)");
    draft.interval = "1 ns".into();
    command(&mut app, Average::Update(draft));
    command(&mut app, Average::Preview);
    pump(&mut app);

    drag_between_rows(&mut app, 2, 3, 0.25);
    assert_eq!(
        app.panels.focused_waves().unwrap().signal(1).unwrap().name,
        "flag"
    );
    assert_eq!(app.derived_editor.as_ref().unwrap().row, 0);

    drag_between_rows(&mut app, 0, 2, 0.9);
    let editor = app.derived_editor.as_ref().unwrap();
    assert_eq!(editor.row, 2);
    assert_eq!(
        editor.revision,
        app.panels.focused_waves().unwrap().revision()
    );
    assert_eq!(editor.bindings[0].source, input);
    command(&mut app, Average::Apply);
    assert!(app.derived_editor.is_none());
    let mean = app.panels.focused_waves().unwrap().signal(3).unwrap();
    assert_eq!(mean.source.input_signal(), input.signal());
    assert_eq!(
        {
            let volna_core::wave::model::RowSource::Computed { inputs, .. } = &mean.source else {
                panic!()
            };
            inputs[0].interpretation.kind
        },
        NumericKind::Signed
    );
    assert_eq!(number(mean.history.as_deref().unwrap(), 4), Some(0.0));

    command(&mut app, Average::Open(Some(3)));
    drag_between_rows(&mut app, 0, 3, 0.9);
    assert_eq!(app.derived_editor.as_ref().unwrap().row, 2);
    command(&mut app, Average::Apply);
    assert!(app.derived_editor.is_none());
    assert_eq!(app.panels.focused_waves().unwrap().items().len(), 4);
    assert!(
        app.panels
            .focused_waves()
            .unwrap()
            .signal(2)
            .unwrap()
            .source
            .computed()
            .is_some()
    );
    assert_eq!(
        app.panels.focused_waves().unwrap().signal(3).unwrap().name,
        "flag"
    );
}

#[test]
fn calculation_revision_changes_only_when_calculation_fields_change() {
    let (_file, mut app) = fixture();
    command(&mut app, Average::Open(Some(0)));
    pump(&mut app);
    let Some(volna_core::computed::ComputationStatus::Ready(history)) = app.derived_preview()
    else {
        panic!("initial preview did not complete");
    };
    let expected = [0, 4, 8].map(|time| number(history.as_ref(), time));
    let editor = app.derived_editor.as_ref().unwrap();
    let revision = editor.calculation_revision;
    let key = editor.preview.clone();
    let mut draft = editor.draft.clone();
    for name in ["renamed", "", "renamed_again"] {
        draft.name = name.into();
        command(&mut app, Average::Update(draft.clone()));
        let editor = app.derived_editor.as_ref().unwrap();
        assert_eq!(editor.calculation_revision, revision);
        assert_eq!(editor.preview, key);
        assert_eq!(editor.error.is_some(), name.is_empty());
        pump(&mut app);
        if !name.is_empty() {
            let Some(volna_core::computed::ComputationStatus::Ready(history)) =
                app.derived_preview()
            else {
                panic!("renamed preview did not recover");
            };
            assert_eq!(
                [0, 4, 8].map(|time| number(history.as_ref(), time)),
                expected
            );
        }
    }
    draft.formula = "moving_avg(adc, 8 us)".into();
    command(&mut app, Average::Update(draft.clone()));
    let editor = app.derived_editor.as_ref().unwrap();
    assert_eq!(editor.calculation_revision, revision + 1);
    assert!(editor.preview.is_none());
    command(&mut app, Average::Update(draft));
    assert_eq!(
        app.derived_editor.as_ref().unwrap().calculation_revision,
        revision + 1
    );
}
