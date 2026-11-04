//! Text representation of copied waveform rows.

use super::lane::{self, LaneData};
use super::model::{WaveModel, WaveRow};
use super::timeline::TimeBase;
use super::tree::{self, Entry};
use crate::data::text::{COPY_BYTES, append_exact};
use crate::document::Document;

impl WaveModel {
    pub(super) fn rows_tsv(&self, rows: &[Entry], doc: &Document) -> anyhow::Result<String> {
        let cursor = self.cursor(doc);
        let base = doc.time_base();
        anyhow::ensure!(
            base.unit.is_none_or(|unit| unit.len() <= COPY_BYTES),
            "Time unit exceeds the clipboard limit."
        );
        let time = cursor.map_or_else(|| "–".into(), |t| timestamp(t, base));
        let mut text = String::from("Time\t");
        append_cell(&mut text, &[&time])?;
        for &index in tree::visible(rows).iter() {
            let entry = &rows[index as usize];
            let row = &entry.row;
            append_exact(&mut text, "\n", COPY_BYTES)?;
            if row.scope().is_empty() {
                append_cell(&mut text, &[row.name()])?;
            } else {
                append_cell(&mut text, &[row.scope(), ".", row.name()])?;
            }
            append_exact(&mut text, "\t", COPY_BYTES)?;
            let remaining = COPY_BYTES.saturating_sub(text.len());
            let value = match row {
                WaveRow::Signal(signal) => {
                    let value = signal.cursor_value(cursor, remaining);
                    anyhow::ensure!(
                        !value.truncated,
                        "Values exceed the 64 KiB clipboard limit."
                    );
                    value.text
                }
                WaveRow::Lane(lane) => match lane.data(doc) {
                    LaneData::Ready(generator) => match cursor {
                        Some(t) => {
                            let index = lane
                                .track()
                                .and_then(|track| doc.transaction_index(track))
                                .and_then(|state| match state {
                                    crate::analysis::AnalysisLoad::Ready(index) => {
                                        Some(index.as_ref())
                                    }
                                    _ => None,
                                });
                            let value = lane::value_text(generator, index, t, remaining);
                            anyhow::ensure!(
                                !value.truncated,
                                "Values exceed the 64 KiB clipboard limit."
                            );
                            value.text
                        }
                        None => "–".into(),
                    },
                    LaneData::Loading => "loading…".into(),
                    LaneData::Unresolved => "not in trace".into(),
                    LaneData::Failed(_) | LaneData::Unavailable => "–".into(),
                },
                WaveRow::Clock(clock) => clock
                    .timeline(doc)
                    .zip(cursor)
                    .and_then(|(timeline, t)| {
                        timeline.cycle_at(t).map(|at| {
                            crate::clock::format_position(self.nav.clocks(), timeline, &at)
                        })
                    })
                    .unwrap_or_else(|| "–".into()),
                WaveRow::Group(_) => String::new(),
            };
            append_cell(&mut text, &[&value])?;
        }
        Ok(text)
    }
}

fn append_cell(text: &mut String, parts: &[&str]) -> anyhow::Result<()> {
    let quoted = parts
        .iter()
        .any(|part| part.contains(['\t', '\r', '\n', '"']));
    if quoted {
        append_exact(text, "\"", COPY_BYTES)?;
    }
    for part in parts {
        let mut chunks = part.split('"');
        append_exact(text, chunks.next().unwrap_or_default(), COPY_BYTES)?;
        for chunk in chunks {
            append_exact(text, "\"\"", COPY_BYTES)?;
            append_exact(text, chunk, COPY_BYTES)?;
        }
    }
    if quoted {
        append_exact(text, "\"", COPY_BYTES)?;
    }
    Ok(())
}

fn timestamp(time: u64, base: TimeBase<'_>) -> String {
    if let Some(unit) = base.unit {
        return format!("{time} {unit}");
    }
    let (exponent, unit) = match base.timescale {
        -15..=-13 => (-15, "fs"),
        -12..=-10 => (-12, "ps"),
        -9..=-7 => (-9, "ns"),
        -6..=-4 => (-6, "µs"),
        -3..=-1 => (-3, "ms"),
        0..=2 => (0, "s"),
        scale => return format!("{time}e{scale} s"),
    };
    let zeroes = if time == 0 {
        0
    } else {
        base.timescale - exponent
    };
    format!("{time}{} {unit}", "0".repeat(zeroes as usize))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{ProceduralTrace, a_all};
    use crate::{Action, App, Command, Event};
    use std::sync::Arc;
    use volna_trace::data::value_view::ValueView;
    use volna_trace::data::{SignalHistory, SignalShape, WaveValue};

    fn app() -> App {
        let mut app = App::new();
        app.set_session(Arc::new(ProceduralTrace::new(100)));
        app.handle(Command::AddVars(a_all([0])));
        app.doc.shared.cursor = Some(7);
        app.panels.focused_waves_mut().unwrap().selected = [0].into();
        app.take_events();
        app
    }

    #[test]
    fn raw_slice_readouts_match_indexed_readouts_across_values_directions_and_limits() {
        use crate::data::projection::{BitSlice, ProjectedHistory};
        use volna_trace::data::history::VecHistory;
        use volna_trace::remote::memory::MemoryBudget;

        let app = app();
        let mut source = app
            .panels
            .focused_waves()
            .unwrap()
            .signal(0)
            .unwrap()
            .clone();
        source.shape = SignalShape::Vector { width: 4 };
        source.source_shape = source.shape;
        source.history = Some(Arc::new(VecHistory {
            shape: source.shape,
            times: vec![5, 10, 15, 20],
            values: ["0011", "0010", "xz01", "1010"]
                .map(|bits| WaveValue::Bits(bits.into()))
                .into(),
            initial: WaveValue::Unavailable,
        }));
        let budget = MemoryBudget::new(4096);
        for slice in [
            BitSlice { first: 0, last: 0 },
            BitSlice { first: 1, last: 3 },
            BitSlice { first: 3, last: 1 },
        ] {
            let raw = source.projected(slice, &app.doc.translators);
            let mut indexed = raw.clone();
            indexed.history = Some(Arc::new(
                ProjectedHistory::build(raw.raw_history().unwrap().clone(), slice, &budget)
                    .unwrap(),
            ));
            for time in [
                None,
                Some(0),
                Some(5),
                Some(10),
                Some(15),
                Some(20),
                Some(25),
            ] {
                for limit in [0, 1, 2, 256] {
                    assert_eq!(
                        raw.cursor_value(time, limit),
                        indexed.cursor_value(time, limit)
                    );
                }
            }
        }
    }

    #[test]
    fn oversized_borrowed_values_report_export_failure_and_preserve_copy_and_cut() {
        struct BorrowOnly(String);
        impl SignalHistory for BorrowOnly {
            fn shape(&self) -> SignalShape {
                SignalShape::Text
            }
            fn len(&self) -> usize {
                1
            }
            fn time(&self, _: usize) -> u64 {
                0
            }
            fn value(&self, _: Option<usize>) -> WaveValue {
                panic!("clipboard formatting must borrow recorded values")
            }
            fn value_view(&self, _: Option<usize>) -> ValueView<'_> {
                ValueView::Text(self.0.as_str().into())
            }
            fn bit(&self, _: Option<usize>) -> volna_trace::data::Bit {
                volna_trace::data::Bit::Other
            }
        }
        let mut app = app();
        let translator = app.doc.translators.default_for(SignalShape::Text);
        let signal = app.panels.focused_waves_mut().unwrap().items[0]
            .signal_mut()
            .unwrap();
        signal.shape = SignalShape::Text;
        signal.translator = translator;
        signal.history = Some(Arc::new(BorrowOnly("x".repeat(COPY_BYTES + 1))));
        let preview = signal.cursor_value(Some(7), 256);
        assert!(preview.truncated && preview.text.len() <= 256);

        for action in [Action::CopySignals, Action::CutSignals] {
            app.handle(Command::Action(action));
            let events = app.take_events();
            assert!(
                events
                    .iter()
                    .any(|e| matches!(e, Event::Notice(text) if text.contains("64 KiB")))
            );
            assert!(!events.iter().any(|e| matches!(e, Event::CopyText(_))));
            assert_eq!(app.doc.copied_rows.len(), 1);
        }
        assert!(app.panels.focused_waves().unwrap().items().is_empty());
        app.handle(Command::Action(Action::PasteSignals));
        assert_eq!(app.panels.focused_waves().unwrap().items().len(), 1);
    }

    #[test]
    fn event_values_are_pulses_at_the_cursor_in_both_readout_and_export() {
        let mut app = app();
        let signal = app.panels.focused_waves_mut().unwrap().items[0]
            .signal_mut()
            .unwrap();
        signal.shape = SignalShape::Event;
        signal.history = Some(Arc::new(volna_trace::data::history::VecHistory {
            shape: SignalShape::Event,
            times: vec![5],
            values: vec![WaveValue::Bits("1".into())],
            initial: WaveValue::Unavailable,
        }));
        for (time, expected) in [(4, "0"), (5, "1"), (6, "0")] {
            app.doc.shared.cursor = Some(time);
            let waves = app.panels.focused_waves().unwrap();
            assert_eq!(
                waves.signal(0).unwrap().cursor_value(Some(time), 256).text,
                expected
            );
            let text = waves.rows_tsv(waves.items(), &app.doc).unwrap();
            assert_eq!(
                text.lines()
                    .last()
                    .unwrap()
                    .split('\t')
                    .next_back()
                    .unwrap(),
                expected
            );
        }
    }

    #[test]
    fn aggregate_export_limit_keeps_all_internal_rows() {
        let mut app = app();
        let waves = app.panels.focused_waves_mut().unwrap();
        waves.items = (0..1000)
            .map(|_| {
                Entry::new(
                    0,
                    WaveRow::Group(super::super::model::GroupRow::new("x".repeat(100))),
                )
            })
            .collect();
        waves.selected = (0..1000).collect();
        app.handle(Command::Action(Action::CopySignals));
        assert_eq!(app.doc.copied_rows.len(), 1000);
        assert!(
            app.take_events()
                .iter()
                .any(|e| matches!(e, Event::Notice(_)))
        );
        app.handle(Command::Action(Action::PasteSignals));
        assert_eq!(app.panels.focused_waves().unwrap().items().len(), 2000);
    }

    #[test]
    fn timestamps_keep_all_ticks_and_scale_without_integer_overflow() {
        assert_eq!(
            timestamp(u64::MAX, TimeBase::si(-8)),
            "184467440737095516150 ns"
        );
        assert_eq!(timestamp(0, TimeBase::si(-8)), "0 ns");
        assert_eq!(timestamp(123_456_789, TimeBase::si(-9)), "123456789 ns");
        assert_eq!(timestamp(7, TimeBase::si(-18)), "7e-18 s");
        assert_eq!(
            timestamp(
                42,
                TimeBase {
                    timescale: -9,
                    unit: Some("cycle")
                }
            ),
            "42 cycle"
        );
    }

    #[test]
    fn tsv_cells_quote_embedded_delimiters_and_double_quotes() {
        let mut text = String::new();
        append_cell(&mut text, &["top.", "\"a\tb\"\n"]).unwrap();
        assert_eq!(text, "\"top.\"\"a\tb\"\"\n\"");
        text.clear();
        append_cell(&mut text, &["top", ".", "signal"]).unwrap();
        assert_eq!(text, "top.signal");
        text = "x".repeat(COPY_BYTES - 2);
        assert!(append_cell(&mut text, &["\""]).is_err());
        assert!(text.len() <= COPY_BYTES);
    }
}
