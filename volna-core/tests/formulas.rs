//! Recorded-input formulas exercised through the application workflow.

use std::collections::BTreeSet;
use volna_core::{
    Action, App, Command, LoadRequest,
    computed::{Command as Derive, ComputationStatus},
    data::NumericKind,
    testing::a_all,
    trace::{TraceId, Traced},
    workspace::Workspace,
};
use volna_trace::{
    data::{Bit, SignalHistory, SignalShape, WaveValue},
    session::OpenSpec,
};
use vtr::Recorder;

fn fixture() -> (tempfile::NamedTempFile, App) {
    fixture_with_end(10)
}
fn fixture_with_end(end: u64) -> (tempfile::NamedTempFile, App) {
    let file = tempfile::Builder::new().suffix(".vtr").tempfile().unwrap();
    let mut writer = vtr::Writer::create_with(file.path(), {
        let mut options = vtr::WriterOptions::default();
        options.metadata.timescale = -9;
        options
    })
    .unwrap();
    let mut real = Vec::new();
    for name in ["a", "b", "c"] {
        real.push(
            writer
                .signal(
                    vtr::SourceOptions::new(name, None),
                    vtr::SignalKind::Real,
                    vtr::VarType::Real,
                    vtr::Direction::Output,
                )
                .unwrap(),
        );
    }
    let mut digital = Vec::new();
    for (name, width) in [("valid", 1), ("ready", 1), ("word", 8), ("float", 32)] {
        digital.push(
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
    writer.emit_real(real[0], 2.0).unwrap();
    writer.emit_real(real[1], 1.0).unwrap();
    writer.emit_real(real[2], 6.0).unwrap();
    writer.emit_packed(digital[0], 4, &[2]).unwrap();
    writer.emit_u64(digital[1], 0).unwrap();
    writer.emit_u64(digital[2], 254).unwrap();
    writer
        .emit_u64(digital[3], u64::from((-2.0_f32).to_bits()))
        .unwrap();
    writer.advance_time(2).unwrap();
    writer.emit_real(real[1], 4.0).unwrap();
    writer.emit_u64(digital[1], 1).unwrap();
    writer.advance_time(3).unwrap();
    writer.emit_real(real[0], 8.0).unwrap();
    writer.emit_u64(digital[0], 1).unwrap();
    writer.emit_u64(digital[2], 6).unwrap();
    writer
        .emit_u64(digital[3], u64::from(6.0_f32.to_bits()))
        .unwrap();
    writer.advance_time(5).unwrap();
    writer.emit_real(real[1], 0.0).unwrap();
    writer.emit_packed(digital[0], 4, &[2]).unwrap();
    writer.advance_time(7).unwrap();
    writer.emit_real(real[0], f64::NAN).unwrap();
    writer.emit_u64(digital[1], 0).unwrap();
    writer.advance_time(8).unwrap();
    writer.emit_real(real[0], 10.0).unwrap();
    writer.emit_u64(digital[0], 0).unwrap();
    writer.advance_time(end).unwrap();
    writer.emit_real(real[2], 9.0).unwrap();
    writer.close().unwrap();
    let mut app = App::new();
    app.set_session(OpenSpec::Path(file.path().into()).open().unwrap());
    app.handle(Command::AddVars(a_all([0])));
    pump(&mut app);
    (file, app)
}
fn command(app: &mut App, command: Derive) {
    app.handle(Command::Derived(app.panels.focused_id(), command));
}
fn pump(app: &mut App) {
    for _ in 0..30 {
        let requests = app.take_requests();
        if requests.is_empty() {
            return;
        }
        for request in requests {
            app.deliver(request.perform());
        }
    }
    panic!("work did not settle");
}
fn take_derived(app: &mut App) -> LoadRequest {
    for _ in 0..30 {
        let requests = app.take_requests();
        let mut sources = Vec::new();
        for request in requests {
            if matches!(request, LoadRequest::Derived { .. }) {
                return request;
            }
            sources.push(request);
        }
        for request in sources {
            app.deliver(request.perform());
        }
    }
    panic!("derived work was not scheduled");
}
fn formula(app: &mut App, expression: &str) {
    let mut draft = app.derived_editor.as_ref().unwrap().draft.clone();
    draft.name = "result".into();
    draft.formula = expression.into();
    command(app, Derive::Update(draft));
    command(app, Derive::Preview);
    pump(app);
}
fn add(app: &mut App, var: usize) {
    command(app, Derive::AddInput(Traced::new(TraceId::A, var)));
}
fn preview(app: &App) -> &dyn SignalHistory {
    let Some(ComputationStatus::Ready(h)) = app.derived_preview() else {
        panic!(
            "preview not ready: {:?}",
            app.derived_editor.as_ref().unwrap().error
        );
    };
    h.as_ref()
}
fn number(history: &dyn SignalHistory, t: u64) -> Option<f64> {
    match history.value(history.index_at(t)) {
        WaveValue::Real(v) => Some(v),
        _ => None,
    }
}

fn preview_with_budget(app: &mut App, budget: &volna_trace::remote::memory::MemoryBudget) {
    for _ in 0..30 {
        let requests = app.take_requests();
        if requests.is_empty() {
            return;
        }
        for mut request in requests {
            if let LoadRequest::Derived { budget: output, .. } = &mut request {
                *output = budget.clone();
            }
            app.deliver(request.perform());
        }
    }
    panic!("work did not settle");
}
fn saved(app: &App) -> Vec<u8> {
    Workspace::capture(app, volna_core::testing::paths("trace.vtr"), None)
        .unwrap()
        .to_bytes()
        .unwrap()
}
fn restore(app: &mut App, bytes: &[u8]) {
    Workspace::parse(bytes)
        .unwrap()
        .prepare(
            app,
            "file:///tmp/trace.vtr",
            "file:///tmp/trace.vtr.volna.json",
        )
        .unwrap()
        .commit(app)
        .unwrap();
    pump(app);
}
fn named_result<'a>(app: &'a App, name: &str) -> &'a dyn SignalHistory {
    app.panels
        .focused_waves()
        .unwrap()
        .items()
        .iter()
        .find_map(|e| e.signal().filter(|s| s.name == name))
        .unwrap()
        .history
        .as_deref()
        .unwrap()
}
fn result(app: &App) -> &dyn SignalHistory {
    named_result(app, "result")
}
fn rename_draft(app: &mut App, name: &str) {
    let mut draft = app.derived_editor.as_ref().unwrap().draft.clone();
    draft.name = name.into();
    command(app, Derive::Update(draft));
    command(app, Derive::Preview);
    pump(app);
}

// A user adds hidden recorded inputs and compares them; every input event must affect the held result without resampling.
#[test]
fn one_two_and_three_recorded_inputs_follow_the_union_of_events() {
    let (_file, mut app) = fixture();
    command(&mut app, Derive::Open(Some(0)));
    formula(&mut app, "abs(-a) * 2 + 1");
    assert_eq!(number(preview(&app), 0), Some(5.0));
    assert_eq!(number(preview(&app), 7), None);
    add(&mut app, 1);
    formula(&mut app, "a - b");
    let h = preview(&app);
    assert_eq!(
        (0..h.len()).map(|i| h.time(i)).collect::<Vec<_>>(),
        vec![0, 2, 3, 5, 7, 8, 11]
    );
    assert_eq!(
        [0, 2, 3, 5, 7, 8, 11].map(|t| number(h, t)),
        [
            Some(1.0),
            Some(-2.0),
            Some(4.0),
            Some(8.0),
            None,
            Some(10.0),
            None
        ]
    );
    add(&mut app, 2);
    formula(&mut app, "(a + b + c) / 3");
    assert_eq!(number(preview(&app), 3), Some(6.0));
    assert_eq!(number(preview(&app), 10), Some(19.0 / 3.0));
    let before = saved(&app);
    command(&mut app, Derive::Close);
    assert_eq!(saved(&app), before);
}

// A user writes predicates and explicit casts; unknown truth tables and unavailable arithmetic must remain visible gaps.
#[test]
fn boolean_outputs_use_three_valued_logic_and_explicit_numeric_casts() {
    let (_file, mut app) = fixture();
    command(&mut app, Derive::Open(Some(0)));
    add(&mut app, 3);
    add(&mut app, 4);
    command(&mut app, Derive::RemoveInput(0));
    formula(&mut app, "valid && ready");
    let h = preview(&app);
    assert_eq!(h.shape(), SignalShape::Bit);
    assert_eq!(
        [0, 2, 3, 5, 7, 8].map(|t| h.bit(h.index_at(t))),
        [
            Bit::Zero,
            Bit::Unavailable,
            Bit::One,
            Bit::Unavailable,
            Bit::Zero,
            Bit::Zero
        ]
    );
    formula(&mut app, "valid || ready");
    assert_eq!(preview(&app).bit(preview(&app).index_at(2)), Bit::One);
    formula(&mut app, "real(valid) + real(ready)");
    assert_eq!(number(preview(&app), 3), Some(2.0));
    assert_eq!(number(preview(&app), 0), None);
    formula(&mut app, "valid + ready");
    assert!(
        app.derived_editor
            .as_ref()
            .unwrap()
            .error
            .as_ref()
            .unwrap()
            .contains("requires numbers")
    );
    command(&mut app, Derive::Apply);
    assert!(app.derived_editor.is_some());
    formula(&mut app, "!valid == false");
    assert_eq!(preview(&app).bit(preview(&app).index_at(3)), Bit::One);
    command(&mut app, Derive::Apply);
    assert!(app.derived_editor.is_none());
    assert_eq!(result(&app).shape(), SignalShape::Bit);
    let bytes = saved(&app);
    restore(&mut app, &bytes);
    assert_eq!(result(&app).shape(), SignalShape::Bit);
}

// A user edits, cancels, undoes, saves, and removes displayed inputs; formula bindings outlive row placement and one Apply is one undo step.
#[test]
fn formulas_edit_cancel_undo_and_restore_with_hidden_inputs() {
    let (_file, mut app) = fixture();
    let undo = app.history.undo_steps().count();
    command(&mut app, Derive::Open(Some(0)));
    add(&mut app, 1);
    formula(&mut app, "a / b");
    assert_eq!(number(preview(&app), 5), None);
    command(&mut app, Derive::Apply);
    assert!(app.derived_editor.is_none());
    assert_eq!(app.history.undo_steps().count(), undo + 1);
    assert_eq!(number(result(&app), 3), Some(2.0));
    command(&mut app, Derive::Open(Some(1)));
    formula(&mut app, "min(a,b)");
    command(&mut app, Derive::Close);
    assert_eq!(number(result(&app), 3), Some(2.0));
    command(&mut app, Derive::Open(Some(1)));
    formula(&mut app, "max(a,b)");
    command(&mut app, Derive::Apply);
    assert_eq!(number(result(&app), 3), Some(8.0));
    app.handle(Command::Undo);
    pump(&mut app);
    assert_eq!(number(result(&app), 3), Some(2.0));
    app.handle(Command::Redo);
    pump(&mut app);
    assert_eq!(number(result(&app), 3), Some(8.0));
    app.panels.focused_waves_mut().unwrap().selected = BTreeSet::from([0]);
    app.handle(Command::Action(Action::RemoveSelected));
    pump(&mut app);
    let bytes = saved(&app);
    restore(&mut app, &bytes);
    assert_eq!(number(result(&app), 3), Some(8.0));
    assert_eq!(saved(&app), bytes);
    command(&mut app, Derive::Open(Some(0)));
    assert_eq!(app.derived_editor.as_ref().unwrap().bindings.len(), 2);
}

// A user repairs a missing saved binding; the definition stays editable and never silently resolves to another occurrence.
#[test]
fn missing_inputs_are_preserved_until_explicitly_rebound() {
    let (_file, mut app) = fixture();
    command(&mut app, Derive::Open(Some(0)));
    add(&mut app, 1);
    formula(&mut app, "a-b");
    command(&mut app, Derive::Apply);
    let bytes = saved(&app);
    let mut data: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    data["panels"][0]["rows"][1]["computed"]["inputs"][1]["path"] = serde_json::json!(["missing"]);
    restore(&mut app, &serde_json::to_vec(&data).unwrap());
    let row = app.panels.focused_waves().unwrap().signal(1).unwrap();
    assert!(row.history.is_none());
    assert!(row.error.as_ref().unwrap().contains("missing or ambiguous"));
    command(&mut app, Derive::Open(Some(1)));
    assert!(app.derived_editor.is_some());
    assert!(app.derived_editor.as_ref().unwrap().error.is_some());
    command(&mut app, Derive::Rebind(1, Traced::new(TraceId::A, 1)));
    command(&mut app, Derive::Preview);
    pump(&mut app);
    command(&mut app, Derive::Apply);
    assert_eq!(number(result(&app), 3), Some(4.0));
    let original: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    data["panels"][0]["rows"][1]["computed"]["inputs"][1]["path"] =
        original["panels"][0]["rows"][1]["computed"]["inputs"][1]["path"].clone();
    data["panels"][0]["rows"][1]["computed"]["inputs"][1]["trace"] = serde_json::json!("B");
    let mut missing_trace = data["traces"][0].clone();
    missing_trace["letter"] = serde_json::json!("B");
    missing_trace["path"] = serde_json::json!("missing.vtr");
    data["traces"].as_array_mut().unwrap().push(missing_trace);
    restore(&mut app, &serde_json::to_vec(&data).unwrap());
    command(&mut app, Derive::Open(Some(1)));
    assert!(app.derived_editor.is_some());
    command(&mut app, Derive::Rebind(1, Traced::new(TraceId::A, 1)));
    command(&mut app, Derive::Preview);
    pump(&mut app);
    command(&mut app, Derive::Apply);
    assert_eq!(number(result(&app), 3), Some(4.0));
}

// A user changes a formula while its worker is running; a superseded result must never become applicable.
#[test]
fn edited_and_rebound_formulas_reject_late_results() {
    let (_file, mut app) = fixture();
    command(&mut app, Derive::Open(Some(0)));
    pump(&mut app);
    add(&mut app, 1);
    formula(&mut app, "a");
    let mut draft = app.derived_editor.as_ref().unwrap().draft.clone();
    draft.formula = "a-b".into();
    command(&mut app, Derive::Update(draft.clone()));
    command(&mut app, Derive::Preview);
    assert!(
        app.derived_editor.as_ref().unwrap().error.is_none(),
        "{:?}",
        app.derived_editor.as_ref().unwrap().error
    );
    let old = take_derived(&mut app);
    draft.formula = "a+b".into();
    command(&mut app, Derive::Update(draft));
    command(&mut app, Derive::Preview);
    assert!(
        app.derived_editor.as_ref().unwrap().error.is_none(),
        "{:?}",
        app.derived_editor.as_ref().unwrap().error
    );
    let new = take_derived(&mut app);
    app.deliver(old.perform());
    assert!(matches!(
        app.derived_preview(),
        Some(ComputationStatus::Building)
    ));
    app.deliver(new.perform());
    assert_eq!(number(preview(&app), 3), Some(12.0));
    command(&mut app, Derive::Rebind(1, Traced::new(TraceId::A, 2)));
    command(&mut app, Derive::Apply);
    assert!(app.derived_editor.is_some());
    command(&mut app, Derive::Preview);
    pump(&mut app);
    assert_eq!(number(preview(&app), 3), Some(14.0));
}

// A user captures signed, sliced and IEEE interpretations; formula outputs and restored bindings must retain those choices.
#[test]
fn numeric_interpretations_slices_and_ieee_inputs_are_captured() {
    let (_file, mut app) = fixture();
    command(&mut app, Derive::Open(Some(0)));
    add(&mut app, 5);
    add(&mut app, 6);
    command(&mut app, Derive::RemoveInput(0));
    command(
        &mut app,
        Derive::SetInput {
            index: 0,
            alias: "word".into(),
            kind: NumericKind::Signed,
            slice: "6:0".into(),
        },
    );
    command(
        &mut app,
        Derive::SetInput {
            index: 1,
            alias: "float".into(),
            kind: NumericKind::Float,
            slice: String::new(),
        },
    );
    formula(&mut app, "word - float");
    assert_eq!(number(preview(&app), 0), Some(0.0));
    assert_eq!(number(preview(&app), 3), Some(0.0));
    command(&mut app, Derive::Apply);
    let bytes = saved(&app);
    restore(&mut app, &bytes);
    assert_eq!(number(result(&app), 0), Some(0.0));
    command(&mut app, Derive::Open(Some(1)));
    command(
        &mut app,
        Derive::SetInput {
            index: 0,
            alias: "word".into(),
            kind: NumericKind::Unsigned,
            slice: "6:0".into(),
        },
    );
    command(&mut app, Derive::Preview);
    pump(&mut app);
    assert_eq!(number(preview(&app), 0), Some(128.0));
}

// A user enters an oversized or ill-typed expression; errors must be bounded, recoverable, and keep Apply disabled.
#[test]
fn invalid_and_oversized_formulas_recover_without_applying_old_results() {
    let (_file, mut app) = fixture();
    command(&mut app, Derive::Open(Some(0)));
    for expression in [
        "a +",
        "a && true",
        "unknown + 1",
        "1e999",
        &"(".repeat(40),
        &"a+".repeat(2050),
        &std::iter::repeat_n("a", 140).collect::<Vec<_>>().join("+"),
    ] {
        formula(&mut app, expression);
        assert!(
            app.derived_editor.as_ref().unwrap().error.is_some(),
            "{expression}"
        );
        assert!(app.derived_preview().is_none());
        command(&mut app, Derive::Apply);
        assert!(app.derived_editor.is_some());
    }
    formula(&mut app, "a / 0");
    assert_eq!(number(preview(&app), 0), None);
    formula(&mut app, "bool(a) && a > 0");
    assert_eq!(preview(&app).bit(preview(&app).index_at(0)), Bit::One);
    formula(&mut app, "a * 1e308");
    assert_eq!(number(preview(&app), 0), None);
    formula(&mut app, "a + 1");
    assert_eq!(number(preview(&app), 0), Some(3.0));
}

// A user combines selections and a shorter second recording, then closes and restores that trace; formulas honor all source lifetimes and their common domain.
#[test]
fn selection_seeds_inputs_and_cross_trace_formulas_stop_at_the_shorter_recording() {
    let (_file, mut app) = fixture();
    app.handle(Command::AddVars(a_all([1, 2])));
    pump(&mut app);
    app.panels.focused_waves_mut().unwrap().selected = BTreeSet::from([0, 1, 2]);
    command(&mut app, Derive::Open(None));
    formula(&mut app, "(a+b+c)/3");
    assert_eq!(number(preview(&app), 3), Some(6.0));
    command(&mut app, Derive::Close);
    let second = tempfile::Builder::new().suffix(".vtr").tempfile().unwrap();
    let mut options = vtr::WriterOptions::default();
    options.metadata.timescale = -9;
    let mut writer = vtr::Writer::create_with(second.path(), options).unwrap();
    let signal = writer
        .signal(
            vtr::SourceOptions::new("a", None),
            vtr::SignalKind::Real,
            vtr::VarType::Real,
            vtr::Direction::Output,
        )
        .unwrap();
    writer.advance_time(2).unwrap();
    writer.emit_real(signal, 20.0).unwrap();
    writer.advance_time(6).unwrap();
    writer.emit_real(signal, 30.0).unwrap();
    writer.close().unwrap();
    let trace = app
        .add_session(OpenSpec::Path(second.path().into()).open().unwrap())
        .unwrap();
    pump(&mut app);
    app.panels.focused_waves_mut().unwrap().selected = BTreeSet::from([0]);
    command(&mut app, Derive::Open(None));
    command(&mut app, Derive::AddInput(Traced::new(trace, 0)));
    formula(&mut app, "a - a_2");
    let h = preview(&app);
    assert_eq!(
        [0, 2, 3, 6, 7].map(|t| number(h, t)),
        [None, Some(-18.0), Some(-12.0), Some(-22.0), None]
    );
    command(&mut app, Derive::Apply);
    let bytes = saved(&app);
    restore(&mut app, &bytes);
    assert_eq!(number(result(&app), 6), Some(-22.0));
    assert_eq!(number(result(&app), 7), None);
    app.handle(Command::RemoveTrace(trace));
    pump(&mut app);
    assert_eq!(app.panels.focused_waves().unwrap().items().len(), 3);
    app.handle(Command::Undo);
    pump(&mut app);
    assert_eq!(number(result(&app), 6), Some(-22.0));
    command(&mut app, Derive::Open(Some(1)));
    app.handle(Command::RemoveTrace(trace));
    pump(&mut app);
    assert!(app.derived_editor.is_none());
}

// A saved duplicate-path binding loses its ordinal; the application must request repair rather than choosing one of the recordings' variables.
#[test]
fn ambiguous_paths_require_repair_and_valid_ordinals_survive_restore() {
    let file = tempfile::Builder::new().suffix(".vtr").tempfile().unwrap();
    let mut writer = vtr::Writer::create(file.path()).unwrap();
    let mut vars = Vec::new();
    for _ in 0..2 {
        vars.push(
            writer
                .signal(
                    vtr::SourceOptions::new("same", None),
                    vtr::SignalKind::Real,
                    vtr::VarType::Real,
                    vtr::Direction::Output,
                )
                .unwrap(),
        );
    }
    writer.emit_real(vars[0], 2.0).unwrap();
    writer.emit_real(vars[1], 7.0).unwrap();
    writer.advance_time(10).unwrap();
    writer.emit_real(vars[1], 9.0).unwrap();
    writer.close().unwrap();
    let mut app = App::new();
    app.set_session(OpenSpec::Path(file.path().into()).open().unwrap());
    app.handle(Command::AddVars(a_all([0])));
    pump(&mut app);
    command(&mut app, Derive::Open(Some(0)));
    add(&mut app, 1);
    formula(&mut app, "same_2 - same");
    assert_eq!(number(preview(&app), 0), Some(5.0));
    command(&mut app, Derive::Apply);
    let matches = volna_core::computed::RecordedSearch::new(&app.doc, "same")
        .run(&std::sync::atomic::AtomicBool::new(false));
    assert_eq!(matches.len(), 2);
    assert!(matches[0].label.ends_with("same (occurrence 1)"));
    assert!(matches[1].label.ends_with("same (occurrence 2)"));
    let bytes = saved(&app);
    restore(&mut app, &bytes);
    assert_eq!(number(result(&app), 0), Some(5.0));
    let mut data: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    data["panels"][0]["rows"][1]["computed"]["inputs"][1]["nth"] = serde_json::Value::Null;
    restore(&mut app, &serde_json::to_vec(&data).unwrap());
    let row = app.panels.focused_waves().unwrap().signal(1).unwrap();
    assert!(row.history.is_none());
    assert!(row.error.as_ref().unwrap().contains("missing or ambiguous"));
    command(&mut app, Derive::Open(Some(1)));
    command(&mut app, Derive::Rebind(1, Traced::new(TraceId::A, 1)));
    command(&mut app, Derive::Preview);
    pump(&mut app);
    assert_eq!(number(preview(&app), 0), Some(5.0));
}

// A user hits an output memory limit, then retries with enough memory; a refusal must not publish partial results or poison the draft.
#[test]
fn formula_memory_admission_failure_is_recoverable() {
    use volna_trace::remote::memory::MemoryBudget;
    let (_file, mut app) = fixture();
    command(&mut app, Derive::Open(Some(0)));
    pump(&mut app);
    let mut draft = app.derived_editor.as_ref().unwrap().draft.clone();
    draft.formula = "a+1".into();
    command(&mut app, Derive::Update(draft));
    command(&mut app, Derive::Preview);
    let mut request = app
        .take_requests()
        .into_iter()
        .find(|r| matches!(r, LoadRequest::Derived { .. }))
        .unwrap();
    if let LoadRequest::Derived { budget, .. } = &mut request {
        *budget = MemoryBudget::new(1);
    }
    app.deliver(request.perform());
    assert!(matches!(
        app.derived_preview(),
        Some(ComputationStatus::Failed(_))
    ));
    command(&mut app, Derive::Apply);
    assert!(app.derived_editor.is_some());
    command(&mut app, Derive::Preview);
    pump(&mut app);
    assert_eq!(number(preview(&app), 0), Some(3.0));
}

// A dense recorded input exceeds the formula event limit; the worker must refuse it without publishing a truncated waveform or retaining output memory.
#[test]
fn oversized_event_output_is_refused_and_releases_its_reservation() {
    use std::sync::{Arc, atomic::AtomicBool};
    use volna_core::computed::{Definition, Key, Operation};
    use volna_trace::remote::memory::MemoryBudget;
    struct DenseRecording;
    impl SignalHistory for DenseRecording {
        fn shape(&self) -> SignalShape {
            SignalShape::Real
        }
        fn len(&self) -> usize {
            1_000_000
        }
        fn time(&self, i: usize) -> u64 {
            i as u64
        }
        fn value(&self, i: Option<usize>) -> WaveValue {
            i.map_or(WaveValue::Unavailable, |i| WaveValue::Real(i as f64))
        }
        fn bit(&self, _: Option<usize>) -> Bit {
            Bit::Other
        }
    }
    let (_file, app) = fixture_with_end(1_000_000);
    let mut input = app.recorded_input(Traced::new(TraceId::A, 0)).unwrap();
    input.alias = "a".into();
    let key = Key::new(
        &app.doc,
        &[input],
        &Definition {
            id: volna_core::computed::FormulaId(0),
            revision: 0,
            name: "result".into(),
            operation: Operation::Formula {
                expression: "a+1".into(),
                settings: volna_core::computed::FormulaSettings {
                    default_step: 1,
                    saved_range: (0, 1_000_000),
                    scope: volna_core::computed::MeasurementScope::WholeRange,
                    unknown: volna_core::computed::UnknownPolicy::Gap,
                },
            },
        },
    )
    .unwrap();
    let budget = MemoryBudget::new(64 << 20);
    let result = LoadRequest::Derived {
        key,
        request: 1,
        sources: vec![Arc::new(DenseRecording)],
        budget: budget.clone(),
        cancelled: Arc::new(AtomicBool::new(false)),
    }
    .perform();
    let volna_core::LoadResult::Derived {
        result: Err(error), ..
    } = result
    else {
        panic!("oversized output was accepted");
    };
    assert!(error.to_string().contains("1,000,000 output events"));
    assert_eq!(budget.used(), 0);
}

// A user reopens a saved formula against a shorter recording; output must stop at the current recording's boundary.
#[test]
fn saved_formula_domain_follows_current_recording() {
    let (_original, mut app) = fixture();
    command(&mut app, Derive::Open(Some(0)));
    formula(&mut app, "a");
    command(&mut app, Derive::Apply);
    let bytes = saved(&app);
    let shortened = tempfile::Builder::new().suffix(".vtr").tempfile().unwrap();
    let mut options = vtr::WriterOptions::default();
    options.metadata.timescale = -9;
    let mut writer = vtr::Writer::create_with(shortened.path(), options).unwrap();
    let a = writer
        .signal(
            vtr::SourceOptions::new("a", None),
            vtr::SignalKind::Real,
            vtr::VarType::Real,
            vtr::Direction::Output,
        )
        .unwrap();
    writer.emit_real(a, 2.0).unwrap();
    writer.advance_time(3).unwrap();
    writer.emit_real(a, 4.0).unwrap();
    writer.close().unwrap();
    let mut reopened = App::new();
    reopened.set_session(OpenSpec::Path(shortened.path().into()).open().unwrap());
    restore(&mut reopened, &bytes);
    assert_eq!(reopened.doc.limits(), (0, 3));
    assert_eq!(number(result(&reopened), 3), Some(4.0));
    assert_eq!(
        number(result(&reopened), 8),
        None,
        "saved formula must not hold past the new recording's end"
    );
}

// A user edits a slice to invalid text, then removes that input; core validation must recover without applying an old result.
#[test]
fn invalid_slice_text_prevents_apply_and_removal_recovers() {
    let (_file, mut app) = fixture();
    command(&mut app, Derive::Open(Some(0)));
    add(&mut app, 5);
    formula(&mut app, "a+1");
    command(
        &mut app,
        Derive::SetInput {
            index: 1,
            alias: "word".into(),
            kind: NumericKind::Unsigned,
            slice: "bad".into(),
        },
    );
    command(&mut app, Derive::Apply);
    assert!(app.derived_editor.is_some());
    assert!(
        app.derived_editor
            .as_ref()
            .unwrap()
            .error
            .as_ref()
            .unwrap()
            .contains("slice")
    );
    command(&mut app, Derive::RemoveInput(1));
    command(&mut app, Derive::Preview);
    pump(&mut app);
    assert_eq!(number(preview(&app), 0), Some(3.0));
    command(&mut app, Derive::Apply);
    assert!(app.derived_editor.is_none());
}
// A user searches full recorded paths and duplicate names; bounded results must identify distinct recorded sources and support cancellation.
#[test]
fn recorded_search_matches_full_paths_and_cancels() {
    use std::sync::atomic::AtomicBool;
    use volna_core::computed::RecordedSearch;
    let (_file, app) = fixture();
    let matches = RecordedSearch::new(&app.doc, "word").run(&AtomicBool::new(false));
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].var.item, 5);
    assert!(matches[0].label.starts_with("A: "));
    let exact = RecordedSearch::new(&app.doc, &matches[0].label).run(&AtomicBool::new(false));
    assert_eq!(exact.len(), 1);
    assert_eq!(exact[0].var.item, 5);
    assert!(
        RecordedSearch::new(&app.doc, "not_a_signal")
            .run(&AtomicBool::new(false))
            .is_empty()
    );
    assert!(
        RecordedSearch::new(&app.doc, "")
            .run(&AtomicBool::new(true))
            .is_empty()
    );
}

// A user sums dense recordings, hits the memory limit, and retries; both intermediate storage and the retained result must be admitted and released.
#[test]
fn synchronized_inputs_admit_intermediate_storage_and_release_it_on_close() {
    use volna_trace::remote::memory::MemoryBudget;
    let file = tempfile::Builder::new().suffix(".vtr").tempfile().unwrap();
    let mut writer = vtr::Writer::create(file.path()).unwrap();
    let signals = ["a", "b", "c"].map(|name| {
        writer
            .signal(
                vtr::SourceOptions::new(name, None),
                vtr::SignalKind::Real,
                vtr::VarType::Real,
                vtr::Direction::Output,
            )
            .unwrap()
    });
    for time in 0..50_000 {
        writer.advance_time(time).unwrap();
        for (offset, signal) in signals.iter().enumerate() {
            writer
                .emit_real(*signal, time as f64 + offset as f64)
                .unwrap();
        }
    }
    writer.close().unwrap();
    let mut app = App::new();
    app.set_session(OpenSpec::Path(file.path().into()).open().unwrap());
    app.handle(Command::AddVars(a_all([0])));
    pump(&mut app);
    command(&mut app, Derive::Open(Some(0)));
    add(&mut app, 1);
    add(&mut app, 2);
    let mut draft = app.derived_editor.as_ref().unwrap().draft.clone();
    draft.formula = "a + b + c".into();
    command(&mut app, Derive::Update(draft));
    command(&mut app, Derive::Preview);
    let budget = MemoryBudget::new(2 << 20);
    preview_with_budget(&mut app, &budget);
    let Some(ComputationStatus::Failed(error)) = app.derived_preview() else {
        panic!("intermediate storage exceeded the budget without refusing the preview");
    };
    assert!(error.contains("memory budget exceeded"), "{error}");
    assert_eq!(budget.used(), 0);
    budget.set_limit(16 << 20);
    command(&mut app, Derive::Preview);
    preview_with_budget(&mut app, &budget);
    assert_eq!(number(preview(&app), 0), Some(3.0));
    assert_eq!(number(preview(&app), 49_999), Some(150_000.0));
    assert_eq!(number(preview(&app), 50_000), None);
    assert!(budget.used() > 0 && budget.used() <= budget.limit());
    command(&mut app, Derive::Close);
    assert_eq!(budget.used(), 0);
}

// A user adds a predicate with a busy secondary input; navigation must skip unchanged results but still stop at unknown gaps and the recording end.
#[test]
fn unchanged_formula_values_do_not_create_transitions_or_hide_gaps() {
    let (_file, mut app) = fixture();
    command(&mut app, Derive::Open(Some(0)));
    add(&mut app, 1);
    formula(&mut app, "a >= 0");
    let h = preview(&app);
    assert_eq!(h.next_change_after(0), Some(7));
    assert_eq!(h.next_change_after(7), Some(8));
    assert_eq!(h.next_change_after(8), Some(11));
    assert_eq!(
        [0, 6, 7, 8, 11].map(|t| h.bit(h.index_at(t))),
        [
            Bit::One,
            Bit::One,
            Bit::Unavailable,
            Bit::One,
            Bit::Unavailable
        ]
    );
    command(&mut app, Derive::Apply);
    let bytes = saved(&app);
    restore(&mut app, &bytes);
    assert_eq!(result(&app).next_change_after(0), Some(7));
}

// A user binds two fields of the same recorded bus with different interpretations; both fields must update independently, including reversed HDL slices.
#[test]
fn fields_from_one_bus_keep_independent_float_and_signed_interpretations() {
    let file = tempfile::Builder::new().suffix(".vtr").tempfile().unwrap();
    let mut writer = vtr::Writer::create(file.path()).unwrap();
    let signal = writer
        .signal(
            vtr::SourceOptions::new("packet[63:0]", None),
            vtr::SignalKind::Bits {
                width: 64,
                states: 4,
            },
            vtr::VarType::Wire,
            vtr::Direction::Output,
        )
        .unwrap();
    writer
        .emit_u64(signal, (u64::from((-2.0_f32).to_bits()) << 16) | 0b1101)
        .unwrap();
    writer.advance_time(5).unwrap();
    writer
        .emit_u64(signal, (u64::from(6.0_f32.to_bits()) << 16) | 2)
        .unwrap();
    writer.close().unwrap();
    let mut app = App::new();
    app.set_session(OpenSpec::Path(file.path().into()).open().unwrap());
    app.handle(Command::AddVars(a_all([0])));
    pump(&mut app);
    command(&mut app, Derive::Open(Some(0)));
    add(&mut app, 0);
    command(
        &mut app,
        Derive::SetInput {
            index: 0,
            alias: "sample".into(),
            kind: NumericKind::Float,
            slice: "47:16".into(),
        },
    );
    command(
        &mut app,
        Derive::SetInput {
            index: 1,
            alias: "offset".into(),
            kind: NumericKind::Signed,
            slice: "3:0".into(),
        },
    );
    formula(&mut app, "sample + offset");
    assert_eq!(number(preview(&app), 0), Some(-5.0));
    assert_eq!(number(preview(&app), 5), Some(8.0));
    command(
        &mut app,
        Derive::SetInput {
            index: 1,
            alias: "offset".into(),
            kind: NumericKind::Signed,
            slice: "0:3".into(),
        },
    );
    command(&mut app, Derive::Preview);
    pump(&mut app);
    assert_eq!(number(preview(&app), 0), Some(-7.0));
    assert_eq!(number(preview(&app), 5), Some(10.0));
    command(&mut app, Derive::Apply);
    let bytes = saved(&app);
    restore(&mut app, &bytes);
    assert_eq!(number(result(&app), 0), Some(-7.0));
    assert_eq!(number(result(&app), 5), Some(10.0));
}

// A user passes real values through a formula; compaction must preserve the visible sign of zero and the following unavailable boundary.
#[test]
fn real_formula_transitions_preserve_signed_zero() {
    let file = tempfile::Builder::new().suffix(".vtr").tempfile().unwrap();
    let mut writer = vtr::Writer::create(file.path()).unwrap();
    let signal = writer
        .signal(
            vtr::SourceOptions::new("a", None),
            vtr::SignalKind::Real,
            vtr::VarType::Real,
            vtr::Direction::Output,
        )
        .unwrap();
    writer.emit_real(signal, 0.0).unwrap();
    writer.advance_time(1).unwrap();
    writer.emit_real(signal, -0.0).unwrap();
    writer.advance_time(2).unwrap();
    writer.emit_real(signal, 0.0).unwrap();
    writer.close().unwrap();
    let mut app = App::new();
    app.set_session(OpenSpec::Path(file.path().into()).open().unwrap());
    app.handle(Command::AddVars(a_all([0])));
    pump(&mut app);
    command(&mut app, Derive::Open(Some(0)));
    formula(&mut app, "a");
    let h = preview(&app);
    assert_eq!(number(h, 0).unwrap().to_bits(), 0.0_f64.to_bits());
    assert_eq!(number(h, 1).unwrap().to_bits(), (-0.0_f64).to_bits());
    assert_eq!(number(h, 2).unwrap().to_bits(), 0.0_f64.to_bits());
    assert_eq!(h.next_change_after(0), Some(1));
    assert_eq!(h.next_change_after(1), Some(2));
    assert_eq!(number(h, 3), None);
}

// A user replaces a saved numeric formula's binding with a digital input; preparation must use the new output type and reject incompatible arithmetic.
#[test]
fn rebinding_a_saved_formula_changes_its_output_type() {
    let (_file, mut app) = fixture();
    command(&mut app, Derive::Open(Some(0)));
    formula(&mut app, "a");
    command(&mut app, Derive::Apply);
    command(&mut app, Derive::Open(Some(1)));
    command(&mut app, Derive::Rebind(0, Traced::new(TraceId::A, 3)));
    command(&mut app, Derive::Preview);
    pump(&mut app);
    let h = preview(&app);
    assert_eq!(h.shape(), SignalShape::Bit);
    assert_eq!(h.bit(h.index_at(3)), Bit::One);
    formula(&mut app, "a + 1");
    assert!(
        app.derived_editor
            .as_ref()
            .unwrap()
            .error
            .as_ref()
            .unwrap()
            .contains("requires numbers")
    );
    command(&mut app, Derive::Apply);
    assert!(app.derived_editor.is_some());
    formula(&mut app, "real(a) + 1");
    assert_eq!(number(preview(&app), 0), None);
    assert_eq!(number(preview(&app), 3), Some(2.0));
    command(&mut app, Derive::Rebind(0, Traced::new(TraceId::A, 0)));
    formula(&mut app, "a");
    assert_eq!(preview(&app).shape(), SignalShape::Real);
    assert_eq!(number(preview(&app), 0), Some(2.0));
}

// A user applies temporal sampling and edge capture in the waveform editor; saved grids and capture timestamps must survive the normal preview/Apply path.
#[test]
fn temporal_formulas_and_captures_work_through_the_application() {
    let (_file, mut app) = fixture();
    command(&mut app, Derive::Open(Some(0)));
    formula(
        &mut app,
        "moving_avg(a, 4 ns, step: 2 ns, warmup: \"available\", unknown: \"skip\")",
    );
    assert_eq!(number(preview(&app), 4), Some(3.5));
    command(&mut app, Derive::Apply);
    assert_eq!(number(result(&app), 4), Some(3.5));

    command(&mut app, Derive::Open(Some(0)));
    add(&mut app, 4);
    formula(&mut app, "a + 1 @posedge ready");
    assert_eq!(preview(&app).capture_times(), Some(&[2][..]));
    assert_eq!(number(preview(&app), 2), Some(3.0));
    assert_eq!(number(preview(&app), 8), Some(3.0));
    command(&mut app, Derive::Apply);
    let bytes = saved(&app);
    restore(&mut app, &bytes);
    assert_eq!(result(&app).capture_times(), Some(&[2][..]));
    assert_eq!(number(result(&app), 8), Some(3.0));
}

// A user removes all recorded bindings and generates a waveform; the document range and saved default step are sufficient on their own.
#[test]
fn generator_formulas_need_no_recorded_inputs_and_restore_their_step() {
    let (_file, mut app) = fixture();
    command(&mut app, Derive::Open(Some(0)));
    command(&mut app, Derive::RemoveInput(0));
    let mut draft = app.derived_editor.as_ref().unwrap().draft.clone();
    draft.name = "generated".into();
    draft.interval = "2 ns".into();
    draft.formula = "sine(period: 8 ns)".into();
    command(&mut app, Derive::Update(draft));
    command(&mut app, Derive::Preview);
    pump(&mut app);
    assert!((number(preview(&app), 2).unwrap() - 1.0).abs() < 1e-12);
    command(&mut app, Derive::Apply);
    let bytes = saved(&app);
    restore(&mut app, &bytes);
    let generated = app
        .panels
        .focused_waves()
        .unwrap()
        .items()
        .iter()
        .find_map(|entry| entry.signal().filter(|signal| signal.name == "generated"))
        .unwrap();
    assert!((number(generated.history.as_deref().unwrap(), 2).unwrap() - 1.0).abs() < 1e-12);
    let volna_core::computed::Operation::Formula { settings, .. } =
        &generated.source.computed().unwrap().operation;
    assert_eq!(settings.default_step, 2);
    assert_eq!(settings.saved_range, app.doc.limits());
}

// A user saves a measurement, then moves its markers; the saved values and analog presentation survive restore.
#[test]
fn measurements_apply_restore_and_preserve_marker_intervals() {
    let (_file, mut app) = fixture();
    command(&mut app, Derive::Open(Some(0)));
    command(&mut app, Derive::Rebind(0, Traced::new(TraceId::A, 4)));
    formula(&mut app, "interval_mean(real(a))");
    let Some(ComputationStatus::ReadyMeasurement(measurement)) = app.derived_preview() else {
        panic!("scalar preview did not complete");
    };
    assert!(
        (measurement.value.unwrap() - 0.5).abs() < 1e-12,
        "{measurement:?}"
    );
    command(&mut app, Derive::Apply);
    let bytes = saved(&app);
    restore(&mut app, &bytes);
    let row = app.panels.focused_waves().unwrap().signal(1).unwrap();
    assert!(row.history.is_some());
    assert!(row.analog.is_some());
    let Some(volna_core::computed::MeasurementResult::Scalar(measurement)) = &row.measurement
    else {
        panic!("saved scalar did not recompute");
    };
    assert!((measurement.value.unwrap() - 0.5).abs() < 1e-12);

    for time in [0, 2, 7, 10] {
        app.doc.add_marker(time).unwrap();
    }
    command(&mut app, Derive::Open(Some(1)));
    let mut draft = app.derived_editor.as_ref().unwrap().draft.clone();
    draft.scope = volna_core::computed::MeasurementScope::MarkerSet {
        markers: app.doc.markers().iter().map(|marker| marker.id).collect(),
    };
    command(&mut app, Derive::Update(draft));
    command(&mut app, Derive::Preview);
    pump(&mut app);
    let Some(ComputationStatus::ReadyTable(rows)) = app.derived_preview() else {
        panic!("marker table preview did not complete");
    };
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0].result.as_ref().unwrap().value, Some(0.0));
    assert_eq!(rows[1].result.as_ref().unwrap().value, Some(1.0));
    assert_eq!(rows[2].result.as_ref().unwrap().value, Some(0.0));
    command(&mut app, Derive::Apply);
    let second = app.doc.markers()[1].id;
    app.doc.shared.cursor = Some(3);
    app.handle(Command::Action(Action::SetMarker(second)));
    pump(&mut app);
    let row = app.panels.focused_waves().unwrap().signal(1).unwrap();
    let Some(volna_core::computed::MeasurementResult::Table(rows)) = &row.measurement else {
        panic!("saved marker table did not recompute");
    };
    assert_eq!(app.doc.markers()[1].time, 3);
    assert_eq!(rows[0].result.as_ref().unwrap().value, Some(0.0));
    let bytes = saved(&app);
    restore(&mut app, &bytes);
    let row = app.panels.focused_waves().unwrap().signal(1).unwrap();
    let Some(volna_core::computed::MeasurementResult::Table(rows)) = &row.measurement else {
        panic!("restored marker table did not recompute");
    };
    assert_eq!(rows[0].result.as_ref().unwrap().value, Some(0.0));
}

// A user plots interval maxima across markers, including recording edges, then moves a marker and reopens the workspace.
#[test]
fn interval_max_plots_marker_partitions_and_unknown_gaps() {
    let (_file, mut app) = fixture();
    let first = app.doc.add_marker(3).unwrap();
    app.doc.add_marker(8).unwrap();
    command(&mut app, Derive::Open(Some(0)));
    formula(&mut app, "interval_max(a)");
    command(&mut app, Derive::Apply);
    let assert_plot = |app: &App, first_max| {
        let row = app.panels.focused_waves().unwrap().signal(1).unwrap();
        assert!(row.analog.is_some());
        let history = row.history.as_ref().unwrap();
        assert_eq!(number(history.as_ref(), 0), Some(first_max));
        assert_eq!(number(history.as_ref(), 5), None);
        assert_eq!(number(history.as_ref(), 8), Some(10.0));
        assert_eq!(number(history.as_ref(), 10), None);
    };
    assert_plot(&app, 2.0);
    app.doc.shared.cursor = Some(5);
    app.handle(Command::Action(Action::SetMarker(first)));
    pump(&mut app);
    assert_plot(&app, 2.0);
    let markers = app
        .doc
        .markers()
        .iter()
        .map(|marker| marker.id)
        .collect::<Vec<_>>();
    for id in markers {
        assert!(app.doc.remove_marker(id));
    }
    pump(&mut app);
    assert_plot(&app, 2.0);
    let bytes = saved(&app);
    restore(&mut app, &bytes);
    assert_plot(&app, 2.0);
}

// A user places, moves, and removes markers while previewing; only Apply freezes the resulting intervals, and Stop still pauses previews.
#[test]
fn interval_preview_follows_marker_edits_until_apply() {
    let (_file, mut app) = fixture();
    command(&mut app, Derive::Open(Some(0)));
    let mut draft = app.derived_editor.as_ref().unwrap().draft.clone();
    draft.skip_unknown = true;
    command(&mut app, Derive::Update(draft));
    formula(&mut app, "interval_max(a)");
    assert!(matches!(
        app.derived_preview(),
        Some(ComputationStatus::ReadyMeasurement(_))
    ));
    let first = app.doc.add_marker(3).unwrap();
    let second = app.doc.add_marker(8).unwrap();
    pump(&mut app);
    let intervals = |app: &App| {
        let Some(ComputationStatus::ReadyTable(rows)) = app.derived_preview() else {
            panic!("interval preview is not ready")
        };
        rows.iter()
            .map(|row| (row.bounds, row.result.as_ref().unwrap().value))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        intervals(&app),
        vec![
            ((0, 3), Some(2.0)),
            ((3, 8), Some(8.0)),
            ((8, 10), Some(10.0))
        ]
    );
    app.doc.shared.cursor = Some(5);
    app.handle(Command::Action(Action::SetMarker(first)));
    pump(&mut app);
    assert_eq!(
        intervals(&app),
        vec![
            ((0, 5), Some(8.0)),
            ((5, 8), Some(8.0)),
            ((8, 10), Some(10.0))
        ]
    );
    assert!(app.doc.remove_marker(second));
    pump(&mut app);
    assert_eq!(
        intervals(&app),
        vec![((0, 5), Some(8.0)), ((5, 10), Some(10.0))]
    );
    command(&mut app, Derive::Stop);
    app.doc.shared.cursor = Some(6);
    app.handle(Command::Action(Action::SetMarker(first)));
    pump(&mut app);
    assert!(app.derived_preview().is_none());
    command(&mut app, Derive::Preview);
    pump(&mut app);
    assert_eq!(
        intervals(&app),
        vec![((0, 6), Some(8.0)), ((6, 10), Some(10.0))]
    );
    command(&mut app, Derive::Apply);
    assert!(app.doc.remove_marker(first));
    pump(&mut app);
    let history = named_result(&app, "result");
    assert_eq!(number(history, 0), Some(8.0));
    assert_eq!(number(history, 6), Some(10.0));
}

// A user adds a clock row while editing, binds it, and sees the sampled interval result in the live waveform preview before applying.
#[test]
fn preview_remains_visible_when_rows_are_added_during_editing() {
    let (_file, mut app) = fixture();
    command(&mut app, Derive::Open(Some(0)));
    app.handle(Command::AddVars(a_all([4])));
    pump(&mut app);
    add(&mut app, 4);
    app.doc.add_marker(3).unwrap();
    formula(&mut app, "interval_sum(a @posedge ready)");
    let panel = app.panels.focused_id();
    let theme = volna_core::Theme::one_dark();
    let bounds = volna_core::geometry::Rect::from_xywh(0.0, 0.0, 1200.0, 600.0);
    app.layout_panel(
        panel,
        bounds,
        &theme,
        1.0,
        &mut volna_core::scene::MonoMeasure,
    )
    .unwrap();
    let scene = app.render_panel(panel, &theme, &mut volna_core::scene::MonoMeasure);
    assert!(scene.texts().any(|text| text == "Draft · Live preview"));
    assert!(scene.texts().any(|text| text == "result"));
    command(&mut app, Derive::Apply);
    assert!(app.derived_editor.is_none());
    assert_eq!(number(named_result(&app, "result"), 0), Some(2.0));
}

// Removing a row above the draft preserves its target; removing the target itself reports a repairable error instead of claiming a live preview.
#[test]
fn deleting_rows_keeps_the_correct_preview_target() {
    let (_file, mut app) = fixture();
    app.handle(Command::AddVars(a_all([1])));
    pump(&mut app);
    command(&mut app, Derive::Open(Some(1)));
    formula(&mut app, "b + 1");
    app.panels.focused_waves_mut().unwrap().selected = BTreeSet::from([0]);
    app.handle(Command::Action(Action::RemoveSelected));
    pump(&mut app);
    command(&mut app, Derive::Apply);
    assert!(app.derived_editor.is_none());
    assert_eq!(number(named_result(&app, "result"), 0), Some(2.0));
    command(&mut app, Derive::Open(Some(1)));
    app.panels.focused_waves_mut().unwrap().selected = BTreeSet::from([1]);
    app.handle(Command::Action(Action::RemoveSelected));
    pump(&mut app);
    assert!(app.derived_preview().is_none());
    assert!(
        app.derived_editor
            .as_ref()
            .unwrap()
            .error
            .as_ref()
            .unwrap()
            .contains("Reopen")
    );
}

// A user sends interval maxima to the existing table, copies a row, deletes markers, and reopens the saved values.
#[test]
fn interval_statistics_open_in_a_persistent_table() {
    let (_file, mut app) = fixture();
    app.doc.add_marker(3).unwrap();
    app.doc.add_marker(8).unwrap();
    command(&mut app, Derive::Open(Some(0)));
    formula(&mut app, "interval_max(a)");
    command(&mut app, Derive::ApplyTable);
    pump(&mut app);
    let table_id = app
        .panels
        .iter()
        .find(|panel| panel.kind.table().is_some())
        .unwrap()
        .id;
    let table = app
        .panels
        .get_mut(table_id)
        .unwrap()
        .kind
        .table_mut()
        .unwrap();
    assert_eq!(table.len(), 3);
    assert_eq!(table.row_time(0), Some(0));
    assert_eq!(table.row_time(1), Some(3));
    assert_eq!(table.row_time(2), Some(8));
    table.active = Some(volna_core::table::RowIdentity::SignalTime(8));
    assert_eq!(
        table.copy_tsv().unwrap(),
        "Start\tresult\tEnd (ticks)\n8\t10.0\t10"
    );
    let markers = app
        .doc
        .markers()
        .iter()
        .map(|marker| marker.id)
        .collect::<Vec<_>>();
    for id in markers {
        assert!(app.doc.remove_marker(id));
    }
    let bytes = saved(&app);
    restore(&mut app, &bytes);
    let table = app
        .panels
        .get_mut(table_id)
        .unwrap()
        .kind
        .table_mut()
        .unwrap();
    assert_eq!(table.len(), 3);
    table.active = Some(volna_core::table::RowIdentity::SignalTime(0));
    assert_eq!(
        table.copy_tsv().unwrap(),
        "Start\tresult\tEnd (ticks)\n0\t2.0\t3"
    );
}

// A user composes an applied formula, then renames and edits its source; the dependent binding retains identity and recomputes the new revision.
#[test]
fn applied_formula_bindings_follow_identity_and_live_revisions() {
    let (_file, mut app) = fixture();
    command(&mut app, Derive::Open(Some(0)));
    formula(&mut app, "a + 1");
    command(&mut app, Derive::Apply);
    app.panels.focused_waves_mut().unwrap().selected = BTreeSet::from([0, 1]);
    command(&mut app, Derive::Open(Some(1)));
    formula(&mut app, "a + result * 2");
    assert_eq!(number(preview(&app), 0), Some(8.0));
    rename_draft(&mut app, "dependent");
    command(&mut app, Derive::Apply);
    assert_eq!(number(named_result(&app, "dependent"), 0), Some(8.0));

    command(&mut app, Derive::Open(Some(1)));
    let mut draft = app.derived_editor.as_ref().unwrap().draft.clone();
    draft.name = "renamed_source".into();
    draft.formula = "a * 2".into();
    command(&mut app, Derive::Update(draft));
    command(&mut app, Derive::Preview);
    pump(&mut app);
    command(&mut app, Derive::Apply);
    pump(&mut app);
    assert_eq!(number(named_result(&app, "renamed_source"), 0), Some(4.0));
    assert_eq!(number(named_result(&app, "dependent"), 0), Some(10.0));
    let bytes = saved(&app);
    restore(&mut app, &bytes);
    assert_eq!(number(named_result(&app, "dependent"), 0), Some(10.0));
}

// A user removes an applied formula's visible row; the dependent definition retains its serialized snapshot and remains usable.
#[test]
fn removing_an_applied_formula_row_preserves_dependents() {
    let (_file, mut app) = fixture();
    command(&mut app, Derive::Open(Some(0)));
    formula(&mut app, "a + 1");
    command(&mut app, Derive::Apply);
    app.panels.focused_waves_mut().unwrap().selected = BTreeSet::from([0, 1]);
    command(&mut app, Derive::Open(Some(1)));
    formula(&mut app, "result * 3");
    rename_draft(&mut app, "dependent");
    command(&mut app, Derive::Apply);
    assert_eq!(number(named_result(&app, "dependent"), 0), Some(9.0));
    app.panels.focused_waves_mut().unwrap().selected = BTreeSet::from([1]);
    app.handle(Command::Action(Action::RemoveSelected));
    pump(&mut app);
    assert_eq!(number(named_result(&app, "dependent"), 0), Some(9.0));
    let bytes = saved(&app);
    restore(&mut app, &bytes);
    let dependents = app
        .panels
        .focused_waves()
        .unwrap()
        .items()
        .iter()
        .filter_map(|entry| entry.signal().filter(|signal| signal.name == "dependent"))
        .collect::<Vec<_>>();
    assert_eq!(
        dependents.len(),
        1,
        "{:?}",
        dependents.iter().map(|row| &row.source).collect::<Vec<_>>()
    );
    assert_eq!(
        number(dependents[0].history.as_deref().unwrap(), 0),
        Some(9.0)
    );
}

// A user sums captures from an applied captured formula; direct references retain capture timestamps, unlike arithmetic on the held waveform.
#[test]
fn applied_captured_formulas_retain_capture_metadata() {
    let (_file, mut app) = fixture();
    command(&mut app, Derive::Open(Some(0)));
    add(&mut app, 4);
    formula(&mut app, "a @posedge ready");
    command(&mut app, Derive::Apply);
    app.panels.focused_waves_mut().unwrap().selected = BTreeSet::from([0, 1]);
    command(&mut app, Derive::Open(Some(1)));
    formula(&mut app, "interval_sum(result)");
    let Some(ComputationStatus::ReadyMeasurement(measurement)) = app.derived_preview() else {
        panic!("applied capture measurement did not complete");
    };
    assert_eq!(measurement.value, Some(2.0));
    assert_eq!(measurement.captures.unwrap().total, 1);
    command(&mut app, Derive::Apply);
    let row = app.panels.focused_waves().unwrap().signal(2).unwrap();
    assert!(row.measurement.is_some());
}

// A user creates a dependency cycle between two applied formulas; preparation rejects the complete path instead of recursing or publishing stale output.
#[test]
fn formula_cycles_are_rejected_with_the_complete_path() {
    let (_file, mut app) = fixture();
    command(&mut app, Derive::Open(Some(0)));
    formula(&mut app, "a + 1");
    command(&mut app, Derive::Apply);
    app.panels.focused_waves_mut().unwrap().selected = BTreeSet::from([0, 1]);
    command(&mut app, Derive::Open(Some(1)));
    formula(&mut app, "result + a");
    rename_draft(&mut app, "dependent");
    command(&mut app, Derive::Apply);

    let second = app
        .panels
        .focused_waves()
        .unwrap()
        .items()
        .iter()
        .find_map(|entry| entry.signal().filter(|signal| signal.name == "dependent"))
        .unwrap();
    let second_definition = second.source.computed().unwrap().clone();
    let second_inputs = match &second.source {
        volna_core::wave::model::RowSource::Computed { inputs, .. } => inputs.clone(),
        _ => unreachable!(),
    };
    command(&mut app, Derive::Open(Some(1)));
    let mut draft = app.derived_editor.as_ref().unwrap().draft.clone();
    draft.formula = "second + a".into();
    command(&mut app, Derive::Update(draft));
    command(&mut app, Derive::AddInput(Traced::new(TraceId::A, 1)));
    let second_binding = app.derived_editor.as_ref().unwrap().bindings[1].clone();
    command(&mut app, Derive::RemoveInput(1));
    let mut bindings = app.derived_editor.as_ref().unwrap().bindings.clone();
    bindings.push(volna_core::computed::InputDraft {
        alias: "second".into(),
        source: volna_core::wave::model::RowSource::Computed {
            inputs: second_inputs,
            definition: second_definition,
        },
        ..second_binding
    });
    app.derived_editor.as_mut().unwrap().bindings = bindings;
    command(&mut app, Derive::Preview);
    let error = app
        .derived_editor
        .as_ref()
        .unwrap()
        .error
        .clone()
        .unwrap_or_default();
    assert!(
        error.contains("dependency cycle"),
        "{error} {:?}",
        app.derived_editor.as_ref().unwrap().bindings
    );
    assert!(error.contains("->"), "{error}");
}

// A user restores a formula with an unused missing binding; only referenced sources are required for domain resolution and evaluation.
#[test]
fn unused_missing_bindings_do_not_block_evaluation() {
    let (_file, mut app) = fixture();
    command(&mut app, Derive::Open(Some(0)));
    add(&mut app, 1);
    formula(&mut app, "a + 1");
    command(&mut app, Derive::Apply);
    let bytes = saved(&app);
    let mut data: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    data["panels"][0]["rows"][1]["computed"]["inputs"][1]["path"] = serde_json::json!(["missing"]);
    restore(&mut app, &serde_json::to_vec(&data).unwrap());
    let row = app.panels.focused_waves().unwrap().signal(1).unwrap();
    assert!(row.error.is_none(), "{:?}", row.error);
    assert_eq!(number(row.history.as_deref().unwrap(), 0), Some(3.0));
}

// A user adds a formula after restoring a workspace; new identities advance beyond restored IDs instead of making dependencies ambiguous.
#[test]
fn new_formulas_after_restore_allocate_fresh_identities() {
    let (_file, mut app) = fixture();
    command(&mut app, Derive::Open(Some(0)));
    formula(&mut app, "a + 1");
    command(&mut app, Derive::Apply);
    let bytes = saved(&app);
    restore(&mut app, &bytes);
    command(&mut app, Derive::Open(Some(0)));
    formula(&mut app, "a * 3");
    rename_draft(&mut app, "second");
    command(&mut app, Derive::Apply);
    pump(&mut app);
    let waves = app.panels.focused_waves().unwrap();
    let first = waves
        .items()
        .iter()
        .find_map(|entry| entry.signal().filter(|signal| signal.name == "result"))
        .unwrap();
    let second = waves
        .items()
        .iter()
        .find_map(|entry| entry.signal().filter(|signal| signal.name == "second"))
        .unwrap();
    assert_ne!(
        first.source.computed().unwrap().id,
        second.source.computed().unwrap().id
    );
    assert_eq!(number(second.history.as_deref().unwrap(), 0), Some(6.0));
}

// A user samples a long recording for an interval sum, hits a memory limit, and retries with more memory; failed scratch allocations must be released.
#[test]
fn interval_measurements_admit_scratch_and_recover_after_refusal() {
    use volna_trace::remote::memory::MemoryBudget;
    let (_file, mut app) = fixture_with_end(100_000);
    command(&mut app, Derive::Open(Some(0)));
    let mut draft = app.derived_editor.as_ref().unwrap().draft.clone();
    draft.formula = "interval_sum(a @ 1 ns)".into();
    draft.skip_unknown = true;
    command(&mut app, Derive::Update(draft));
    command(&mut app, Derive::Preview);
    let budget = MemoryBudget::new(8 << 10);
    preview_with_budget(&mut app, &budget);
    let Some(ComputationStatus::Failed(error)) = app.derived_preview() else {
        panic!("measurement scratch storage was not admitted");
    };
    assert!(error.contains("memory budget exceeded"), "{error}");
    assert_eq!(budget.used(), 0);
    command(&mut app, Derive::Apply);
    assert!(app.derived_editor.is_some());
    budget.set_limit(16 << 20);
    command(&mut app, Derive::Preview);
    preview_with_budget(&mut app, &budget);
    let Some(ComputationStatus::ReadyMeasurement(value)) = app.derived_preview() else {
        panic!("measurement did not recover after increasing the budget");
    };
    assert_eq!(value.value, Some(999_958.0));
    assert_eq!(value.captures.unwrap().total, 100_000);
    assert_eq!(budget.used(), 0);
}

// A user requests many saved intervals with a costly scalar expression; the request-wide work limit must refuse the entire preview rather than publish a partial table.
#[test]
fn interval_table_enforces_one_work_limit_for_the_request() {
    use volna_core::computed::MeasurementScope;
    use volna_trace::remote::memory::MemoryBudget;
    let (_file, mut app) = fixture_with_end(100_000);
    command(&mut app, Derive::Open(Some(0)));
    let mut draft = app.derived_editor.as_ref().unwrap().draft.clone();
    let mut expression = "interval_max(a)".to_owned();
    for _ in 0..6 {
        expression = format!("({expression}+{expression})");
    }
    draft.formula = expression;
    draft.scope = MeasurementScope::IntervalSnapshot {
        intervals: (0..100_000).map(|start| (start, start + 1)).collect(),
    };
    command(&mut app, Derive::Update(draft));
    command(&mut app, Derive::Preview);
    let budget = MemoryBudget::new(64 << 20);
    preview_with_budget(&mut app, &budget);
    let Some(ComputationStatus::Failed(error)) = app.derived_preview() else {
        panic!("interval table bypassed the request work limit");
    };
    assert!(error.contains("evaluation steps"), "{error}");
    assert_eq!(budget.used(), 0);
    command(&mut app, Derive::ApplyTable);
    assert!(app.derived_editor.is_some());
    assert!(app.panels.iter().all(|panel| panel.kind.table().is_none()));
}

// A user sends a generated clock to a table and reopens it; every timestamped sample, including the recording endpoint, must remain copyable.
#[test]
fn waveform_table_preserves_the_terminal_sample_after_restore() {
    let (_file, mut app) = fixture();
    command(&mut app, Derive::Open(Some(0)));
    command(&mut app, Derive::RemoveInput(0));
    formula(&mut app, "clock(period: 4 ns, high: 2 ns)");
    assert_eq!(preview(&app).time(5), 10);
    command(&mut app, Derive::ApplyTable);
    pump(&mut app);
    let table_id = app
        .panels
        .iter()
        .find(|panel| panel.kind.table().is_some())
        .unwrap()
        .id;
    for restored in [false, true] {
        if restored {
            let bytes = saved(&app);
            restore(&mut app, &bytes);
        }
        let table = app
            .panels
            .get_mut(table_id)
            .unwrap()
            .kind
            .table_mut()
            .unwrap();
        assert_eq!(table.len(), 6);
        for index in 0..6 {
            assert_eq!(table.row_time(index), Some(index * 2));
        }
        table.active = Some(volna_core::table::RowIdentity::SignalTime(10));
        assert_eq!(table.copy_tsv().unwrap(), "Time\tresult\n10\t0.0");
    }
}

// A user previews captured sums between markers and closes the editor; each interval keeps its own samples and closing releases the retained results.
#[test]
fn interval_preview_retains_only_result_storage_until_closed() {
    use volna_trace::remote::memory::MemoryBudget;
    let (_file, mut app) = fixture_with_end(100_000);
    app.doc.add_marker(3).unwrap();
    app.doc.add_marker(8).unwrap();
    command(&mut app, Derive::Open(Some(0)));
    let mut draft = app.derived_editor.as_ref().unwrap().draft.clone();
    draft.formula = "interval_sum(a @ 1 ns)".into();
    draft.skip_unknown = true;
    command(&mut app, Derive::Update(draft));
    command(&mut app, Derive::Preview);
    let budget = MemoryBudget::new(16 << 20);
    preview_with_budget(&mut app, &budget);
    let Some(ComputationStatus::ReadyTable(rows)) = app.derived_preview() else {
        panic!("interval preview did not complete");
    };
    let values: Vec<_> = rows
        .iter()
        .map(|row| row.result.as_ref().unwrap().value)
        .collect();
    assert_eq!(values, [Some(6.0), Some(32.0), Some(999_920.0)]);
    assert!(budget.used() > 0);
    command(&mut app, Derive::Close);
    assert_eq!(budget.used(), 0);
}

// A user enters a scientific literal; its editor highlighting and numerical
// result must agree that the exponent belongs to the same number.
#[test]
fn scientific_literals_share_editor_tokens_and_evaluation() {
    use volna_core::computed::language::{self, TokenKind};
    let (_file, mut app) = fixture();
    command(&mut app, Derive::Open(Some(0)));
    let expression = "2e-3 + .5";
    let syntax = language::Syntax::new(expression);
    let tokens = syntax.tokens();
    let number_token = tokens
        .iter()
        .find(|token| token.range.contains(&2))
        .unwrap();
    assert_eq!(&expression[number_token.range.clone()], "2e-3");
    assert_eq!(number_token.kind, TokenKind::Number);
    formula(&mut app, expression);
    assert_eq!(number(preview(&app), 0), Some(0.502));
}

// A user repairs invalid input while editing; diagnostics must point to the
// offending source text, and incomplete syntax must still provide function help.
#[test]
fn editor_diagnostics_and_hints_follow_the_current_expression() {
    use volna_core::computed::{
        FormulaType, Program,
        language::{self, Diagnostic, TokenKind},
    };
    let inputs = [("a".into(), FormulaType::Real)];
    for expression in ["missing", "1 + missing", "min(a, missing)"] {
        let error = Program::parse(expression, &inputs).unwrap_err();
        let diagnostic = Diagnostic::from_error(expression, &error);
        assert_eq!(&expression[diagnostic.range], "missing");
        assert!(diagnostic.message.contains("Available aliases: a"));
    }
    let expression = "mim(a, 1)";
    let diagnostic = Diagnostic::from_error(
        expression,
        &Program::parse(expression, &inputs).unwrap_err(),
    );
    assert_eq!(&expression[diagnostic.range], "mim");
    let expression = "moving_avg(a, 2e-3 ns, warmup: \"available\"";
    let hint = language::Syntax::new(expression)
        .signature(expression.len())
        .unwrap();
    assert!(hint.starts_with("moving_avg("));
    assert!(hint.contains("warmup:"));
    let expression = "min(a, λ)";
    let diagnostic = Diagnostic::from_error(
        expression,
        &Program::parse(expression, &inputs).unwrap_err(),
    );
    assert_eq!(&expression[diagnostic.range], "λ");
    let expression = "a + ? + 3";
    assert!(language::Syntax::new(expression).tokens().iter().any(|token| token.kind == TokenKind::Number && &expression[token.range.clone()] == "3"));
}

// A user registers a clock from its waveform menu, saves a measurement, and changes
// the default to a counter. Only new formulas use the new default, including after reopening.
#[test]
fn sampling_defaults_preserve_saved_measurements_and_survive_reopening() {
    use volna_core::sampling::{SamplingChoice, SamplingCommand, SamplingMode};
    let (_file, mut app) = fixture();
    app.handle(Command::AddVars(a_all([4])));
    let panel = app.panels.focused_id();
    app.handle(Command::OpenSignalMenu(panel));
    let menu = app.panels.focused_waves().unwrap().menu.as_ref().unwrap();
    let action = menu
        .items()
        .find(|item| item.label == "Rising edge")
        .unwrap()
        .action
        .clone();
    app.handle(Command::MenuSelect(panel, action));
    command(&mut app, Derive::Open(Some(0)));
    let expression = app.derived_editor.as_ref().unwrap().draft.formula.clone();
    formula(&mut app, &format!("interval_sum({expression})"));
    let Some(ComputationStatus::ReadyMeasurement(measurement)) = app.derived_preview() else {
        panic!("clock measurement unavailable")
    };
    assert_eq!(measurement.value, Some(2.0));
    command(&mut app, Derive::Apply);
    let counter = SamplingChoice::new(
        &app.doc,
        Traced::new(TraceId::A, 5),
        SamplingMode::ValueChanges,
    )
    .unwrap();
    app.handle(Command::Sampling(SamplingCommand::Register(
        Traced::new(TraceId::A, 5),
        SamplingMode::ValueChanges,
    )));
    command(&mut app, Derive::Open(Some(0)));
    pump(&mut app);
    assert_eq!(preview(&app).capture_times(), Some([2].as_slice()));
    command(&mut app, Derive::Close);
    app.handle(Command::Sampling(SamplingCommand::MakeDefault(counter)));
    let bytes = saved(&app);
    restore(&mut app, &bytes);
    let saved = app
        .panels
        .focused_waves()
        .unwrap()
        .items()
        .iter()
        .filter_map(|e| e.signal())
        .find(|s| s.name == "result")
        .unwrap();
    let Some(volna_core::computed::MeasurementResult::Scalar(value)) = &saved.measurement else {
        panic!("saved measurement unavailable")
    };
    assert_eq!(value.value, Some(2.0));
    command(&mut app, Derive::Open(Some(0)));
    pump(&mut app);
    assert_eq!(preview(&app).capture_times(), Some([3].as_slice()));
    assert_eq!(number(preview(&app), 3), Some(8.0));
    let expression = app.derived_editor.as_ref().unwrap().draft.formula.clone();
    formula(&mut app, &format!("interval_sum({expression})"));
    let Some(ComputationStatus::ReadyMeasurement(measurement)) = app.derived_preview() else {
        panic!("counter measurement unavailable")
    };
    assert_eq!(measurement.value, Some(8.0));
}

// A user picks a counter while composing a measurement, then switches to a clock.
// Sampling belongs inside the reduction and rejected bus edges leave the result intact.
#[test]
fn sampling_picker_updates_measurements_without_changing_their_meaning() {
    use volna_core::sampling::SamplingMode;
    let (_file, mut app) = fixture();
    command(&mut app, Derive::Open(Some(0)));
    formula(&mut app, "interval_sum(a + 1)");
    command(
        &mut app,
        Derive::Sample {
            var: Traced::new(TraceId::A, 5),
            mode: SamplingMode::ValueChanges,
            make_default: true,
        },
    );
    pump(&mut app);
    let Some(ComputationStatus::ReadyMeasurement(value)) = app.derived_preview() else {
        panic!("counter measurement unavailable")
    };
    assert_eq!(value.value, Some(9.0));
    command(
        &mut app,
        Derive::Sample {
            var: Traced::new(TraceId::A, 4),
            mode: SamplingMode::RisingEdge,
            make_default: false,
        },
    );
    pump(&mut app);
    let Some(ComputationStatus::ReadyMeasurement(value)) = app.derived_preview() else {
        panic!("clock measurement unavailable")
    };
    assert_eq!(value.value, Some(3.0));
    command(
        &mut app,
        Derive::Sample {
            var: Traced::new(TraceId::A, 5),
            mode: SamplingMode::RisingEdge,
            make_default: true,
        },
    );
    assert!(
        app.derived_editor
            .as_ref()
            .unwrap()
            .error
            .as_deref()
            .unwrap()
            .contains("Choose a bit")
    );
    assert_eq!(
        app.derived_editor
            .as_ref()
            .unwrap()
            .sampling(&app.doc)
            .unwrap()
            .mode,
        SamplingMode::RisingEdge
    );
    command(&mut app, Derive::Preview);
    pump(&mut app);
    let Some(ComputationStatus::ReadyMeasurement(value)) = app.derived_preview() else {
        panic!("previous measurement was lost")
    };
    assert_eq!(value.value, Some(3.0));
}

// A user removes the clock default from the signal menu. The remaining counter
// becomes the default; removing all choices restores an unsampled new formula.
#[test]
fn removing_sampling_choices_changes_only_new_formula_defaults() {
    use volna_core::sampling::{SamplingChoice, SamplingCommand, SamplingMode};
    let (_file, mut app) = fixture();
    let clock = SamplingChoice::new(
        &app.doc,
        Traced::new(TraceId::A, 4),
        SamplingMode::RisingEdge,
    )
    .unwrap();
    let counter = SamplingChoice::new(
        &app.doc,
        Traced::new(TraceId::A, 5),
        SamplingMode::ValueChanges,
    )
    .unwrap();
    app.handle(Command::Sampling(SamplingCommand::Register(
        Traced::new(TraceId::A, 4),
        SamplingMode::RisingEdge,
    )));
    app.handle(Command::Sampling(SamplingCommand::Register(
        Traced::new(TraceId::A, 5),
        SamplingMode::ValueChanges,
    )));
    app.handle(Command::Sampling(SamplingCommand::Remove(clock)));
    command(&mut app, Derive::Open(Some(0)));
    pump(&mut app);
    assert_eq!(preview(&app).capture_times(), Some([3].as_slice()));
    command(&mut app, Derive::Close);
    app.handle(Command::Sampling(SamplingCommand::Remove(counter)));
    command(&mut app, Derive::Open(Some(0)));
    pump(&mut app);
    assert!(preview(&app).capture_times().is_none());
    assert_eq!(number(preview(&app), 0), Some(2.0));
}

// A user samples by a 64-bit counter with adjacent large values, skipped values,
// resets and wrap. Each observed change contributes exactly one sample to the sum.
#[test]
fn counter_sampling_preserves_exact_transitions_above_float_precision() {
    use volna_core::sampling::SamplingMode;
    let file = tempfile::Builder::new().suffix(".vtr").tempfile().unwrap();
    let mut writer = vtr::Writer::create(file.path()).unwrap();
    let a = writer
        .signal(
            vtr::SourceOptions::new("a", None),
            vtr::SignalKind::Real,
            vtr::VarType::Real,
            vtr::Direction::Output,
        )
        .unwrap();
    let counter = writer
        .signal(
            vtr::SourceOptions::new("sample_number", None),
            vtr::SignalKind::Bits {
                width: 64,
                states: 4,
            },
            vtr::VarType::Wire,
            vtr::Direction::Output,
        )
        .unwrap();
    for (time, value, sample) in [
        (0, 2.0, 1_u64 << 53),
        (2, 4.0, (1 << 53) + 1),
        (3, 6.0, (1 << 53) + 3),
        (4, 8.0, 0),
        (5, 10.0, u64::MAX),
        (6, 12.0, 0),
        (7, 99.0, 0),
    ] {
        writer.advance_time(time).unwrap();
        writer.emit_real(a, value).unwrap();
        writer.emit_u64(counter, sample).unwrap();
    }
    writer.advance_time(8).unwrap();
    writer.close().unwrap();
    let mut app = App::new();
    app.set_session(OpenSpec::Path(file.path().into()).open().unwrap());
    app.handle(Command::AddVars(a_all([0])));
    command(&mut app, Derive::Open(Some(0)));
    command(
        &mut app,
        Derive::Sample {
            var: Traced::new(TraceId::A, 1),
            mode: SamplingMode::ValueChanges,
            make_default: true,
        },
    );
    pump(&mut app);
    assert_eq!(
        preview(&app).capture_times(),
        Some([2, 3, 4, 5, 6].as_slice())
    );
    let expression = app.derived_editor.as_ref().unwrap().draft.formula.clone();
    formula(&mut app, &format!("interval_sum({expression})"));
    let Some(ComputationStatus::ReadyMeasurement(value)) = app.derived_preview() else {
        panic!("counter measurement unavailable")
    };
    assert_eq!(value.value, Some(40.0));
}

// A user switches a two-input formula from counter sampling back to input changes.
// Both operands must drive updates again, and saving must preserve this choice despite a trace default.
#[test]
fn input_change_sampling_restores_updates_from_each_operand() {
    use volna_core::sampling::SamplingMode;
    let (_file, mut app) = fixture();
    command(&mut app, Derive::Open(Some(0)));
    add(&mut app, 1);
    formula(&mut app, "a + b");
    command(
        &mut app,
        Derive::Sample {
            var: Traced::new(TraceId::A, 5),
            mode: SamplingMode::ValueChanges,
            make_default: true,
        },
    );
    pump(&mut app);
    assert_eq!(number(preview(&app), 2), None);
    command(&mut app, Derive::SampleOnInputChanges);
    pump(&mut app);
    assert_eq!(number(preview(&app), 2), Some(6.0));
    assert_eq!(number(preview(&app), 3), Some(12.0));
    assert_eq!(number(preview(&app), 5), Some(8.0));
    command(&mut app, Derive::Apply);
    let bytes = saved(&app);
    restore(&mut app, &bytes);
    assert_eq!(number(result(&app), 2), Some(6.0));
    assert_eq!(number(result(&app), 5), Some(8.0));
    command(&mut app, Derive::Open(Some(1)));
    pump(&mut app);
    assert_eq!(number(preview(&app), 2), Some(6.0));
    assert!(preview(&app).capture_times().is_none());
}

// A user changes the sampling source after filling the input slots. Replacing a
// trigger-only binding must not require an extra slot or alter the sampled value expression.
#[test]
fn sampling_can_be_replaced_when_all_input_slots_are_occupied() {
    use volna_core::sampling::SamplingMode;
    let (_file, mut app) = fixture();
    command(&mut app, Derive::Open(Some(0)));
    for _ in 0..14 {
        add(&mut app, 1);
    }
    formula(&mut app, "a + 1");
    command(
        &mut app,
        Derive::Sample {
            var: Traced::new(TraceId::A, 4),
            mode: SamplingMode::RisingEdge,
            make_default: false,
        },
    );
    pump(&mut app);
    assert_eq!(number(preview(&app), 3), Some(3.0));
    command(
        &mut app,
        Derive::Sample {
            var: Traced::new(TraceId::A, 5),
            mode: SamplingMode::ValueChanges,
            make_default: false,
        },
    );
    pump(&mut app);
    assert_eq!(preview(&app).capture_times(), Some([3].as_slice()));
    assert_eq!(number(preview(&app), 3), Some(9.0));
    command(&mut app, Derive::Apply);
    assert_eq!(number(result(&app), 3), Some(9.0));
}

// A user reopens a formula whose sampling signal has disappeared and selects a
// replacement. The missing trigger must be repairable without manually editing input bindings.
#[test]
fn unavailable_sampling_can_be_repaired_from_the_sampling_picker() {
    use volna_core::sampling::SamplingMode;
    let (_file, mut app) = fixture();
    command(&mut app, Derive::Open(Some(0)));
    formula(&mut app, "a");
    command(
        &mut app,
        Derive::Sample {
            var: Traced::new(TraceId::A, 4),
            mode: SamplingMode::RisingEdge,
            make_default: true,
        },
    );
    pump(&mut app);
    command(&mut app, Derive::Apply);
    let mut data: serde_json::Value = serde_json::from_slice(&saved(&app)).unwrap();
    data["panels"][0]["rows"][1]["computed"]["inputs"][1]["path"] =
        serde_json::json!(["missing_clock"]);
    data["sampling"]["choices"][0]["path"] = serde_json::json!(["missing_clock"]);
    restore(&mut app, &serde_json::to_vec(&data).unwrap());
    command(&mut app, Derive::Open(Some(1)));
    command(
        &mut app,
        Derive::Sample {
            var: Traced::new(TraceId::A, 5),
            mode: SamplingMode::ValueChanges,
            make_default: true,
        },
    );
    pump(&mut app);
    assert_eq!(preview(&app).capture_times(), Some([3].as_slice()));
    assert_eq!(number(preview(&app), 3), Some(8.0));
    command(&mut app, Derive::Apply);
    assert_eq!(number(result(&app), 3), Some(8.0));
}

// A user binds an optional signal from a shorter recording, leaves it out of the
// expression, and sends the result to a table. The table must include the full used recording.
#[test]
fn unused_short_trace_does_not_truncate_derived_table_output() {
    let (_file, mut app) = fixture();
    let second = tempfile::Builder::new().suffix(".vtr").tempfile().unwrap();
    let mut options = vtr::WriterOptions::default();
    options.metadata.timescale = -9;
    let mut writer = vtr::Writer::create_with(second.path(), options).unwrap();
    let signal = writer
        .signal(
            vtr::SourceOptions::new("optional", None),
            vtr::SignalKind::Real,
            vtr::VarType::Real,
            vtr::Direction::Output,
        )
        .unwrap();
    writer.emit_real(signal, 1.0).unwrap();
    writer.advance_time(3).unwrap();
    writer.emit_real(signal, 2.0).unwrap();
    writer.close().unwrap();
    let trace = app
        .add_session(OpenSpec::Path(second.path().into()).open().unwrap())
        .unwrap();
    pump(&mut app);
    command(&mut app, Derive::Open(Some(0)));
    command(&mut app, Derive::AddInput(Traced::new(trace, 0)));
    formula(&mut app, "a");
    command(&mut app, Derive::ApplyTable);
    pump(&mut app);
    let table = app
        .panels
        .iter_mut()
        .find_map(|panel| panel.kind.table_mut())
        .unwrap();
    table.active = Some(volna_core::table::RowIdentity::SignalTime(8));
    assert_eq!(table.copy_tsv().unwrap(), "Time\tresult\n8\t10.0");
}

// A user captures only one operand while leaving another live. The sampling
// control must describe this as custom, since changing the live operand still updates the result.
#[test]
fn partially_captured_expressions_do_not_claim_one_sampling_rule() {
    use volna_core::sampling::SamplingMode;
    let (_file, mut app) = fixture();
    command(&mut app, Derive::Open(Some(0)));
    add(&mut app, 1);
    add(&mut app, 4);
    formula(&mut app, "a + (b @posedge ready)");
    assert_eq!(number(preview(&app), 2), Some(6.0));
    assert_eq!(number(preview(&app), 3), Some(12.0));
    assert!(
        app.derived_editor
            .as_ref()
            .unwrap()
            .sampling(&app.doc)
            .is_none()
    );
    formula(&mut app, "(a + b) @posedge ready");
    assert_eq!(number(preview(&app), 3), Some(6.0));
    assert_eq!(
        app.derived_editor
            .as_ref()
            .unwrap()
            .sampling(&app.doc)
            .unwrap()
            .mode,
        SamplingMode::RisingEdge
    );
}
