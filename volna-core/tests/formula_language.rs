//! Formula-language examples exercised through parsed programs and published results.

use std::sync::atomic::AtomicBool;
use volna_core::{
    computed::{
        Evaluation, EvaluationOptions, FormulaHistory, FormulaType, Interpretation, Measurement,
        Program, ResultKind, Source, UnknownPolicy, evaluate_program,
    },
    data::NumericKind,
    wave::timeline::TimeBase,
};
use volna_trace::{
    data::{Bit, SignalHistory, SignalShape, WaveValue, history::VecHistory},
    remote::memory::MemoryBudget,
};

fn real_history(times: &[u64], values: &[Option<f64>]) -> VecHistory {
    VecHistory {
        shape: SignalShape::Real,
        times: times.to_vec(),
        values: values
            .iter()
            .map(|value| value.map_or(WaveValue::Unavailable, WaveValue::Real))
            .collect(),
        initial: WaveValue::Unavailable,
    }
}

fn bit_history(times: &[u64], values: &[Option<bool>]) -> VecHistory {
    VecHistory {
        shape: SignalShape::Bit,
        times: times.to_vec(),
        values: values
            .iter()
            .map(|value| {
                value.map_or(WaveValue::Unavailable, |value| {
                    WaveValue::Bits(if value { "1" } else { "0" }.into())
                })
            })
            .collect(),
        initial: WaveValue::Unavailable,
    }
}

fn real_source(history: &dyn SignalHistory, domain: (u64, u64)) -> Source<'_> {
    Source {
        history,
        interpretation: Interpretation {
            kind: NumericKind::Real,
            projection: None,
        },
        domain,
    }
}

fn bit_source(history: &dyn SignalHistory, domain: (u64, u64)) -> Source<'_> {
    Source {
        history,
        interpretation: Interpretation {
            kind: NumericKind::Unsigned,
            projection: None,
        },
        domain,
    }
}

fn options(range: (u64, u64)) -> EvaluationOptions<'static> {
    EvaluationOptions {
        saved_range: range,
        default_step: 1,
        time_base: TimeBase::si(-6),
        interval: None,
        unknown: UnknownPolicy::Gap,
    }
}

fn run(
    expression: &str,
    inputs: &[(String, FormulaType)],
    sources: &[Source<'_>],
    options: EvaluationOptions<'_>,
) -> Evaluation {
    let program = Program::parse(expression, inputs).unwrap();
    evaluate_program(
        &program,
        sources,
        options,
        &MemoryBudget::new(64 << 20),
        &AtomicBool::new(false),
    )
    .unwrap()
}

fn waveform(evaluation: Evaluation) -> FormulaHistory {
    match evaluation {
        Evaluation::Waveform(history) => history,
        Evaluation::Measurement(_) => panic!("expected waveform"),
    }
}

fn measurement(evaluation: Evaluation) -> Measurement {
    match evaluation {
        Evaluation::Measurement(measurement) => measurement,
        Evaluation::Waveform(_) => panic!("expected measurement"),
    }
}

fn number(history: &dyn SignalHistory, time: u64) -> Option<f64> {
    match history.value(history.index_at(time)) {
        WaveValue::Real(value) => Some(value),
        _ => None,
    }
}

fn aliases(types: &[(&str, FormulaType)]) -> Vec<(String, FormulaType)> {
    types
        .iter()
        .map(|(name, ty)| ((*name).into(), *ty))
        .collect()
}

// A user evaluates the tutorial moving average and subtracts it from the raw input; full warm-up and the raw simultaneous transition must remain visible.
#[test]
fn moving_average_matches_the_spec_fixture_and_mixes_with_raw_events() {
    let a = real_history(&[0, 3, 7], &[Some(0.0), Some(10.0), Some(2.0)]);
    let inputs = aliases(&[("a", FormulaType::Real)]);
    let sources = [real_source(&a, (0, 10))];
    let history = waveform(run(
        "moving_avg(a, 4 us, step: 2 us)",
        &inputs,
        &sources,
        options((0, 10)),
    ));
    assert_eq!(
        [0, 2, 4, 6, 8, 10].map(|time| number(&history, time)),
        [None, None, Some(2.5), Some(7.5), Some(8.0), Some(4.0)]
    );
    let residual = waveform(run(
        "a - moving_avg(a, 4 us, step: 2 us)",
        &inputs,
        &sources,
        options((0, 10)),
    ));
    assert_eq!(number(&residual, 6), Some(2.5));
    assert_eq!(number(&residual, 7), Some(-5.5));
}

// A user captures on known rising edges; simultaneous source updates apply first and equal captured values are not compacted away.
#[test]
fn rising_edge_captures_apply_simultaneous_updates_and_retain_repeats() {
    let a = real_history(&[0, 3, 7], &[Some(0.0), Some(10.0), Some(2.0)]);
    let b = real_history(&[0], &[Some(1.0)]);
    let clk = bit_history(
        &[0, 2, 5, 7, 9, 10],
        &[
            Some(false),
            Some(true),
            Some(false),
            Some(true),
            Some(false),
            Some(true),
        ],
    );
    let inputs = aliases(&[
        ("a", FormulaType::Real),
        ("b", FormulaType::Real),
        ("clk", FormulaType::Boolean),
    ]);
    let sources = [
        real_source(&a, (0, 10)),
        real_source(&b, (0, 10)),
        bit_source(&clk, (0, 10)),
    ];
    let history = waveform(run(
        "a - b @posedge clk",
        &inputs,
        &sources,
        options((0, 10)),
    ));
    assert_eq!(history.capture_times(), &[2, 7, 10]);
    assert_eq!(
        [2, 7, 10].map(|time| number(&history, time)),
        [Some(-1.0), Some(1.0), Some(1.0)]
    );
    assert_eq!(number(&history, 3), Some(-1.0));
    let sum = measurement(run(
        "interval_sum(a - b @posedge clk)",
        &inputs,
        &sources,
        EvaluationOptions {
            interval: Some((1, 9)),
            ..options((0, 10))
        },
    ));
    assert_eq!(sum.value, Some(0.0));
    assert_eq!(sum.captures.unwrap().total, 2);
    assert_eq!(sum.captures.unwrap().known, 2);
}

// A user samples a flat value on a periodic grid; every requested timestamp remains available to sample-count measurements.
#[test]
fn periodic_capture_keeps_equal_values_and_starts_at_the_domain_start() {
    let a = real_history(&[0], &[Some(3.0)]);
    let inputs = aliases(&[("a", FormulaType::Real)]);
    let sources = [real_source(&a, (2, 9))];
    let history = waveform(run("a @ 3 us", &inputs, &sources, options((0, 20))));
    assert_eq!(history.capture_times(), &[2, 5, 8]);
    assert_eq!(
        [2, 4, 5, 8, 9].map(|time| number(&history, time)),
        [Some(3.0); 5]
    );
    let sum = measurement(run(
        "interval_sum(a @ 3 us)",
        &inputs,
        &sources,
        EvaluationOptions {
            interval: Some((2, 9)),
            ..options((0, 20))
        },
    ));
    assert_eq!(sum.value, Some(9.0));
    assert_eq!(sum.captures.unwrap().total, 3);
}

// A user generates signals without recorded bindings; the saved range anchors the sine grid and exact clock transitions.
#[test]
fn generators_use_saved_ranges_and_exact_clock_edges() {
    let sine = waveform(run(
        "sine(period: 20 us, amplitude: 1, offset: 0, phase: 0, step: 1 us)",
        &[],
        &[],
        options((0, 40)),
    ));
    assert_eq!(sine.len(), 42);
    assert!((number(&sine, 5).unwrap() - 1.0).abs() < 1e-12);
    assert!((number(&sine, 15).unwrap() + 1.0).abs() < 1e-12);
    let clock = waveform(run(
        "clock(period: 10 us, high: 4 us, phase: 0 us)",
        &[],
        &[],
        options((0, 40)),
    ));
    assert_eq!(
        (0..clock.len()).map(|i| clock.time(i)).collect::<Vec<_>>(),
        [0, 4, 10, 14, 20, 24, 30, 34, 40, 41]
    );
    assert_eq!(clock.bit(clock.index_at(0)), Bit::One);
    assert_eq!(clock.bit(clock.index_at(4)), Bit::Zero);
}

// A user filters and integrates a held signal; integration excludes the value arriving exactly at each output grid instant.
#[test]
fn lowpass_and_integral_follow_held_segments() {
    let a = real_history(&[0, 4], &[Some(2.0), Some(4.0)]);
    let inputs = aliases(&[("a", FormulaType::Real)]);
    let sources = [real_source(&a, (0, 8))];
    let lowpass = waveform(run(
        "lowpass(a, 2 us, step: 2 us)",
        &inputs,
        &sources,
        options((0, 8)),
    ));
    assert_eq!(number(&lowpass, 0), Some(2.0));
    assert_eq!(number(&lowpass, 2), Some(2.0));
    let expected = 4.0 - 2.0 * (-1.0_f64).exp();
    assert!((number(&lowpass, 6).unwrap() - expected).abs() < 1e-12);
    let integral = waveform(run(
        "integrate(a, initial: 1, step: 2 us)",
        &inputs,
        &sources,
        options((0, 8)),
    ));
    assert_eq!(number(&integral, 0), Some(1.0));
    assert!((number(&integral, 4).unwrap() - (1.0 + 8e-6)).abs() < 1e-12);
    assert!((number(&integral, 6).unwrap() - (1.0 + 16e-6)).abs() < 1e-12);
}

// A user measures global and per-marker duty from the same expression; right endpoints contribute no held duration.
#[test]
fn interval_measurements_support_global_and_adjacent_marker_scopes() {
    let valid = bit_history(
        &[0, 1, 7, 11, 16],
        &[
            Some(false),
            Some(true),
            Some(false),
            Some(true),
            Some(false),
        ],
    );
    let inputs = aliases(&[("valid", FormulaType::Boolean)]);
    let sources = [bit_source(&valid, (0, 20))];
    let expression = "100 * interval_mean(real(valid))";
    let global = measurement(run(expression, &inputs, &sources, options((0, 20))));
    assert!((global.value.unwrap() - 55.0).abs() < 1e-12);
    for (bounds, expected) in [((0, 10), 60.0), ((10, 20), 50.0)] {
        let row = measurement(run(
            expression,
            &inputs,
            &sources,
            EvaluationOptions {
                interval: Some(bounds),
                ..options((0, 20))
            },
        ));
        assert_eq!(row.value, Some(expected));
        assert_eq!(row.bounds, bounds);
    }
}

// A user chooses skip-unknown measurements; known support and coverage survive while unknown time is excluded rather than treated as zero.
#[test]
fn skip_unknown_reports_known_support_and_coverage() {
    let a = real_history(&[0, 4, 8], &[Some(2.0), None, Some(4.0)]);
    let inputs = aliases(&[("a", FormulaType::Real)]);
    let sources = [real_source(&a, (0, 10))];
    let skipped = measurement(run(
        "interval_mean(a)",
        &inputs,
        &sources,
        EvaluationOptions {
            unknown: UnknownPolicy::Skip,
            ..options((0, 10))
        },
    ));
    assert_eq!(skipped.value, Some(8.0 / 3.0));
    assert_eq!(skipped.known_ticks, 6);
    assert_eq!(skipped.coverage(), 0.6);
    let strict = measurement(run("interval_mean(a)", &inputs, &sources, options((0, 10))));
    assert_eq!(strict.value, None);
    assert_eq!(strict.known_ticks, 6);
}

// A user relies on exact type and grammar diagnostics; malformed calls and ambiguous capture or comparison chains never evaluate.
#[test]
fn invalid_syntax_and_types_are_rejected_before_evaluation() {
    let inputs = aliases(&[
        ("a", FormulaType::Real),
        ("b", FormulaType::Real),
        ("clk", FormulaType::Boolean),
    ]);
    for expression in [
        "a < b < a",
        "a == b == a",
        "a @ 1 us @clk",
        "a @posedge (clk)",
        "interval_sum(a)",
        "interval_mean(a) @ 1 us",
        "interval_mean(a) + a",
        "min(a)",
        "min(a, b,)",
        "moving_avg(a, 1 us, step: 1 us, step: 2 us)",
        "sine(period: 1 us) + 1 ns",
        "a // comment",
        "2 a",
    ] {
        assert!(Program::parse(expression, &inputs).is_err(), "{expression}");
    }
    for expression in [
        "a @ 1 us @ 2 us",
        "interval_sum((a @ 1 us) + 0)",
        "a @posedge clk",
    ] {
        let result = Program::parse(expression, &inputs);
        assert_eq!(
            result.is_ok(),
            expression == "a @posedge clk",
            "{expression}"
        );
    }
}

// A user has an unused binding from a shorter recording; only referenced inputs determine the visible result domain.
#[test]
fn unused_bindings_do_not_narrow_the_result_domain() {
    let a = real_history(&[0], &[Some(2.0)]);
    let b = real_history(&[2], &[Some(9.0)]);
    let inputs = aliases(&[("a", FormulaType::Real), ("b", FormulaType::Real)]);
    let sources = [real_source(&a, (0, 10)), real_source(&b, (2, 3))];
    let history = waveform(run("a + 1", &inputs, &sources, options((0, 20))));
    assert_eq!(number(&history, 10), Some(3.0));
    assert_eq!(number(&history, 11), None);
}

// A user compares signed zero; min and max preserve the required sign even though numeric equality treats the values as equal.
#[test]
fn min_and_max_preserve_signed_zero() {
    let min = waveform(run("min(0, -0)", &[], &[], options((0, 1))));
    let max = waveform(run("max(0, -0)", &[], &[], options((0, 1))));
    assert!(number(&min, 0).unwrap().is_sign_negative());
    assert!(!number(&max, 0).unwrap().is_sign_negative());
    let program =
        Program::parse("interval_mean(a)", &aliases(&[("a", FormulaType::Real)])).unwrap();
    assert_eq!(program.kind, ResultKind::ScalarMeasurement);
}

// A user captures every published clock state change; known/unknown transitions count, while unknown-to-known is not a rising edge.
#[test]
fn clock_change_capture_includes_unknown_but_posedge_requires_known_edges() {
    let a = real_history(&[0], &[Some(3.0)]);
    let clk = bit_history(
        &[0, 1, 2, 3, 4, 5],
        &[Some(false), Some(true), None, Some(false), None, Some(true)],
    );
    let inputs = aliases(&[("a", FormulaType::Real), ("clk", FormulaType::Boolean)]);
    let sources = [real_source(&a, (0, 6)), bit_source(&clk, (0, 6))];
    let changes = waveform(run("a @clk", &inputs, &sources, options((0, 6))));
    assert_eq!(changes.capture_times(), &[1, 2, 3, 4, 5]);
    let rising = waveform(run("a @posedge clk", &inputs, &sources, options((0, 6))));
    assert_eq!(rising.capture_times(), &[1]);
}

// A user transforms a signal captured after the range start; the result remains unknown before the first capture and readable afterward.
#[test]
fn pointwise_consumers_keep_time_order_when_captures_start_late() {
    let a = real_history(&[0], &[Some(3.0)]);
    let clk = bit_history(&[0, 2, 4], &[Some(false), Some(true), Some(false)]);
    let inputs = aliases(&[("a", FormulaType::Real), ("clk", FormulaType::Boolean)]);
    let sources = [real_source(&a, (0, 6)), bit_source(&clk, (0, 6))];
    for expression in ["-(a @posedge clk)", "(a @posedge clk) + (a @posedge clk)"] {
        let history = waveform(run(expression, &inputs, &sources, options((0, 6))));
        assert!(
            (0..history.len().saturating_sub(1)).all(|i| history.time(i) < history.time(i + 1))
        );
        assert_eq!(number(&history, 0), None);
        assert_eq!(number(&history, 1), None);
        assert_eq!(
            number(&history, 2),
            Some(if expression.starts_with('-') {
                -3.0
            } else {
                6.0
            })
        );
    }
}

// A user starts a value recording on a published rising edge of an older clock; that edge captures the value at the shared start.
#[test]
fn capture_includes_clock_edge_at_intersected_domain_start() {
    let a = real_history(&[2], &[Some(3.0)]);
    let clk = bit_history(&[0, 2, 4], &[Some(false), Some(true), Some(false)]);
    let inputs = aliases(&[("a", FormulaType::Real), ("clk", FormulaType::Boolean)]);
    let sources = [real_source(&a, (2, 6)), bit_source(&clk, (0, 6))];
    let rising = waveform(run("a @posedge clk", &inputs, &sources, options((0, 6))));
    assert_eq!(rising.capture_times(), &[2]);
    assert_eq!(number(&rising, 2), Some(3.0));
    let changes = waveform(run("a @clk", &inputs, &sources, options((0, 6))));
    assert_eq!(changes.capture_times(), &[2, 4]);
}

// A user binds a saved captured Boolean formula as a clock; its first published value is a state change from unknown.
#[test]
fn first_published_captured_clock_value_triggers_change_capture() {
    let bit = bit_history(&[0], &[Some(true)]);
    let raw_clock = bit_history(&[0, 2], &[Some(false), Some(true)]);
    let clock_inputs = aliases(&[("bit", FormulaType::Boolean), ("raw", FormulaType::Boolean)]);
    let clock_sources = [bit_source(&bit, (0, 6)), bit_source(&raw_clock, (0, 6))];
    let captured_clock = waveform(run(
        "bit @posedge raw",
        &clock_inputs,
        &clock_sources,
        options((0, 6)),
    ));
    assert_eq!(captured_clock.capture_times(), &[2]);
    let a = real_history(&[0], &[Some(3.0)]);
    let inputs = aliases(&[
        ("a", FormulaType::Real),
        ("sample_clock", FormulaType::Boolean),
    ]);
    let sources = [real_source(&a, (0, 6)), bit_source(&captured_clock, (0, 6))];
    let history = waveform(run("a @sample_clock", &inputs, &sources, options((0, 6))));
    assert_eq!(history.capture_times(), &[2]);
    assert_eq!(number(&history, 2), Some(3.0));
}

// A user restarts a low-pass filter after an unknown interval; the first known sample establishes the new state immediately.
#[test]
fn lowpass_restarts_at_the_first_known_value_after_a_gap() {
    let a = real_history(&[0, 2, 4], &[Some(1.0), None, Some(3.0)]);
    let inputs = aliases(&[("a", FormulaType::Real)]);
    let sources = [real_source(&a, (0, 6))];
    let history = waveform(run(
        "lowpass(a, 2 us, step: 1 us)",
        &inputs,
        &sources,
        options((0, 6)),
    ));
    assert_eq!(number(&history, 3), None);
    assert_eq!(number(&history, 4), Some(3.0));
    assert_eq!(number(&history, 5), Some(3.0));
}

// A user sums unknown captures under both policies; strict mode propagates the gap and skip mode reports known captures out of the total.
#[test]
fn interval_sum_never_treats_unknown_captures_as_zero() {
    let a = real_history(&[0, 1, 2], &[Some(1.0), None, Some(2.0)]);
    let inputs = aliases(&[("a", FormulaType::Real)]);
    let sources = [real_source(&a, (0, 3))];
    let strict = measurement(run(
        "interval_sum(a @ 1 us)",
        &inputs,
        &sources,
        options((0, 3)),
    ));
    assert_eq!(strict.value, None);
    assert_eq!(strict.captures.unwrap().total, 3);
    let skipped = measurement(run(
        "interval_sum(a @ 1 us)",
        &inputs,
        &sources,
        EvaluationOptions {
            unknown: UnknownPolicy::Skip,
            ..options((0, 3))
        },
    ));
    assert_eq!(skipped.value, Some(3.0));
    assert_eq!(skipped.captures.unwrap().known, 2);
    assert_eq!(skipped.captures.unwrap().total, 3);
}

// A user evaluates extrema and time-weighted area; only positive-duration held values contribute and the physical time unit is applied once.
#[test]
fn interval_extrema_and_integral_use_positive_held_durations() {
    let a = real_history(&[0, 3, 5], &[Some(2.0), Some(-1.0), Some(4.0)]);
    let inputs = aliases(&[("a", FormulaType::Real)]);
    let sources = [real_source(&a, (0, 8))];
    assert_eq!(
        measurement(run("interval_min(a)", &inputs, &sources, options((0, 8)))).value,
        Some(-1.0)
    );
    assert_eq!(
        measurement(run("interval_max(a)", &inputs, &sources, options((0, 8)))).value,
        Some(4.0)
    );
    assert_eq!(
        measurement(run("interval_mean(a)", &inputs, &sources, options((0, 8)))).value,
        Some(2.0)
    );
    let integral = measurement(run(
        "interval_integral(a)",
        &inputs,
        &sources,
        options((0, 8)),
    ));
    assert!((integral.value.unwrap() - 16e-6).abs() < 1e-15);
}

// A user works on physical and logical timelines; durations are exact ticks, logical units do not accept SI, and omitted steps use the saved default.
#[test]
fn durations_are_exact_and_default_steps_come_from_saved_settings() {
    let program = Program::parse("sine(period: 8 us)", &[]).unwrap();
    let history = match evaluate_program(
        &program,
        &[],
        EvaluationOptions {
            default_step: 2,
            ..options((0, 8))
        },
        &MemoryBudget::new(1 << 20),
        &AtomicBool::new(false),
    )
    .unwrap()
    {
        Evaluation::Waveform(history) => history,
        Evaluation::Measurement(_) => unreachable!(),
    };
    assert!((number(&history, 2).unwrap() - 1.0).abs() < 1e-12);
    for expression in [
        "sine(period: 1 ns, step: 0.1 ns)",
        "clock(period: 1 ns, high: 0 ns)",
        "sine(period: 1 cycles)",
    ] {
        let program = Program::parse(expression, &[]).unwrap();
        assert!(
            evaluate_program(
                &program,
                &[],
                options((0, 4)),
                &MemoryBudget::new(1 << 20),
                &AtomicBool::new(false),
            )
            .is_err(),
            "{expression}"
        );
    }
    let program = Program::parse("sine(period: 8 cycles)", &[]).unwrap();
    let logical = EvaluationOptions {
        time_base: TimeBase {
            timescale: 0,
            unit: Some("cycles"),
        },
        default_step: 2,
        ..options((0, 8))
    };
    assert!(
        evaluate_program(
            &program,
            &[],
            logical,
            &MemoryBudget::new(1 << 20),
            &AtomicBool::new(false),
        )
        .is_ok()
    );
    let warning = Program::parse("sine(period: 10 us, step: 1 us)", &[]).unwrap();
    let history = match evaluate_program(
        &warning,
        &[],
        options((0, 10)),
        &MemoryBudget::new(1 << 20),
        &AtomicBool::new(false),
    )
    .unwrap()
    {
        Evaluation::Waveform(history) => history,
        Evaluation::Measurement(_) => unreachable!(),
    };
    assert_eq!(history.warnings().len(), 1);
}

// A user skips unknown time in a rolling average and requests capture fidelity details; coverage and timestamp-coalescing limits remain visible with the result.
#[test]
fn moving_average_reports_coverage_and_capture_histories_expose_fidelity_warnings() {
    let a = real_history(&[0, 2], &[Some(2.0), None]);
    let inputs = aliases(&[("a", FormulaType::Real)]);
    let sources = [real_source(&a, (0, 4))];
    let history = waveform(run(
        "moving_avg(a, 4 us, step: 4 us, warmup: \"available\", unknown: \"skip\")",
        &inputs,
        &sources,
        options((0, 4)),
    ));
    assert_eq!(number(&history, 4), Some(2.0));
    assert_eq!(history.coverage_at(4), Some(0.5));
    let capture = waveform(run("a @ 1 us", &inputs, &sources, options((0, 4))));
    assert!(
        capture
            .warnings()
            .iter()
            .any(|warning| warning.contains("delta-cycle"))
    );
}

// A user writes literal-only Boolean formulas and a Boolean measurement; literal waveforms evaluate on the saved domain, while scalar outputs must be Real.
#[test]
fn literal_boolean_waveforms_evaluate_and_boolean_measurements_are_rejected() {
    let false_history = waveform(run("true && false", &[], &[], options((0, 2))));
    assert_eq!(false_history.bit(false_history.index_at(0)), Bit::Zero);
    let not_history = waveform(run("!true", &[], &[], options((0, 2))));
    assert_eq!(not_history.bit(not_history.index_at(0)), Bit::Zero);
    let comparison = waveform(run("1 < 2", &[], &[], options((0, 2))));
    assert_eq!(comparison.bit(comparison.index_at(0)), Bit::One);
    let inputs = aliases(&[("a", FormulaType::Real)]);
    assert!(Program::parse("interval_mean(a) > 0", &inputs).is_err());
}
