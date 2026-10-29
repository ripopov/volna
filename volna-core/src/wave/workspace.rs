//! The wave panel's workspace schema, capture, validation and resolution.

use super::{
    Tint,
    analog::{Analog, AnalogDraw, AnalogRange},
    lane::TxLane,
    model::{
        DisplayedSignal, GroupRow, GroupStyle, Link, RowHeight, RowSource, WaveModel, WaveRow,
    },
    tree::{self, Entry},
    viewport::Viewport,
};
use crate::clock::ClockView;

use crate::panels::workspace::valid_viewport;
use crate::panels::{Panel, PanelId, PanelKind, workspace::RestoreContext};
use crate::trace::{TraceId, Traced};
use crate::workspace::admit_rows;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use std::collections::BTreeSet;
use volna_trace::data::source::Lookup;

#[derive(Debug, Serialize, Deserialize)]
struct Columns {
    names: f32,
    values: f32,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Row {
    Signal {
        shape: volna_trace::data::SignalShape,
        projection: Option<crate::data::projection::BitSlice>,
        bus: Option<super::bus::BusExpansion>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        rows: Vec<Row>,
        #[serde(default, skip_serializing_if = "TraceId::is_a")]
        trace: TraceId,
        signal: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        nth: Option<usize>,
        format: String,
        #[serde(default, skip_serializing_if = "RowHeight::is_default")]
        height: RowHeight,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        analog: Option<SavedAnalog>,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        overlay_hidden: bool,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "crate::wave::tint::serde_name"
        )]
        tint: Option<Tint>,
    },
    Lane {
        #[serde(default, skip_serializing_if = "TraceId::is_a")]
        trace: TraceId,
        generator: Vec<String>,
        #[serde(default, skip_serializing_if = "RowHeight::is_default")]
        height: RowHeight,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "crate::wave::tint::serde_name"
        )]
        tint: Option<Tint>,
    },
    Clock {
        #[serde(default, skip_serializing_if = "TraceId::is_a")]
        trace: TraceId,
        clock: String,
        #[serde(default, skip_serializing_if = "RowHeight::is_default")]
        height: RowHeight,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "crate::wave::tint::serde_name"
        )]
        tint: Option<Tint>,
    },
    Group {
        name: String,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        collapsed: bool,
        #[serde(default, skip_serializing_if = "RowHeight::is_default")]
        height: RowHeight,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "crate::wave::tint::serde_name"
        )]
        tint: Option<Tint>,
        #[serde(default, skip_serializing_if = "SavedStyle::is_activity")]
        style: SavedStyle,
        #[serde(default = "shows_peak", skip_serializing_if = "is_true")]
        peak: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        overlaid: Option<SavedAnalog>,
        rows: Vec<Row>,
    },
}

fn shows_peak() -> bool {
    true
}

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "Serde requires this callback to borrow the field."
)]
fn is_true(value: &bool) -> bool {
    *value
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum SavedStyle {
    #[default]
    Activity,
    Stack,
    Overlaid,
}

impl SavedStyle {
    #[expect(
        clippy::trivially_copy_pass_by_ref,
        reason = "Serde requires this callback to borrow the field."
    )]
    fn is_activity(&self) -> bool {
        *self == Self::Activity
    }
}

impl SavedStyle {
    fn of(style: GroupStyle) -> (Self, bool) {
        match style {
            GroupStyle::Activity => (Self::Activity, true),
            GroupStyle::Stack { peak } => (Self::Stack, peak),
            GroupStyle::Overlaid { .. } => (Self::Overlaid, true),
        }
    }

    fn style(self, peak: bool, overlaid: Option<SavedAnalog>) -> Result<GroupStyle> {
        Ok(match self {
            Self::Activity => GroupStyle::Activity,
            Self::Stack => GroupStyle::Stack { peak },
            Self::Overlaid => {
                let settings = overlaid.context("overlaid group missing drawing settings")?;
                GroupStyle::Overlaid {
                    draw: settings.draw,
                    range: settings.range,
                }
            }
        })
    }
}

fn nest(items: &[Entry], row: &dyn Fn(&WaveRow) -> Row) -> Vec<Row> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < items.len() {
        let end = tree::subtree_end(items, i);
        out.push(match &items[i].row {
            WaveRow::Group(g) => Row::Group {
                name: g.name.clone(),
                collapsed: g.collapsed,
                height: g.height,
                tint: g.tint,
                style: SavedStyle::of(g.style).0,
                peak: SavedStyle::of(g.style).1,
                overlaid: match g.style {
                    GroupStyle::Overlaid { draw, range } => Some(SavedAnalog { draw, range }),
                    _ => None,
                },
                rows: nest(&items[i + 1..end], row),
            },
            other => {
                let mut saved = row(other);
                if let Row::Signal { rows, .. } = &mut saved {
                    *rows = nest(&items[i + 1..end], row);
                }
                saved
            }
        });
        i = end;
    }
    out
}

fn flatten(rows: Vec<Row>, depth: u8, out: &mut Vec<(u8, Row)>) -> Result<()> {
    for row in rows {
        ensure!(
            depth < tree::MAX_DEPTH,
            "groups nested deeper than {}",
            tree::MAX_DEPTH
        );
        match row {
            Row::Group {
                name,
                collapsed,
                height,
                tint,
                style,
                peak,
                overlaid,
                rows,
            } => {
                ensure!(!name.trim().is_empty(), "empty group name");
                out.push((
                    depth,
                    Row::Group {
                        name,
                        collapsed,
                        height,
                        tint,
                        style,
                        peak,
                        overlaid,
                        rows: Vec::new(),
                    },
                ));
                flatten(rows, depth + 1, out)?;
            }
            mut row => {
                let children = match &mut row {
                    Row::Signal { rows, bus, .. } => {
                        ensure!(
                            rows.is_empty() || bus.is_some_and(|b| b.initialized),
                            "bits below an unexpanded signal"
                        );
                        std::mem::take(rows)
                    }
                    _ => Vec::new(),
                };
                out.push((depth, row));
                flatten(children, depth + 1, out)?;
            }
        }
    }
    Ok(())
}

#[derive(Debug, Serialize, Deserialize)]
struct SavedAnalog {
    draw: AnalogDraw,
    range: AnalogRange,
}

pub(crate) const VERSION: u32 = 8;

#[derive(Serialize, Deserialize)]
struct WavePanel {
    id: PanelId,
    kind: String,
    version: u32,
    title: Option<String>,
    link: Link,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    viewport: Option<Viewport>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::panels::workspace::present_raw"
    )]
    cursor: Option<Box<RawValue>>,
    scroll_y: f32,
    columns: Columns,
    rows: Vec<Row>,
    selected: BTreeSet<usize>,
    #[serde(default, skip_serializing_if = "ClockView::is_default")]
    clocks: ClockView,
}

pub(crate) fn save(
    w: &WaveModel,
    id: PanelId,
    title: Option<String>,
    doc: &crate::Document,
) -> Result<Box<RawValue>> {
    let rows = nest(w.items(), &|row| match row {
        WaveRow::Signal(item) => {
            let (signal, nth) = item.source.locator(doc);
            Row::Signal {
                shape: item.source_shape,
                projection: item.projection,
                bus: item.bus,
                rows: Vec::new(),
                trace: item.source.trace(),
                signal,
                nth,
                format: item.format_id(),
                height: item.height,
                analog: item.analog.as_ref().map(|a| SavedAnalog {
                    draw: a.draw,
                    range: a.range,
                }),
                overlay_hidden: item.overlay_hidden,
                tint: item.tint,
            }
        }
        WaveRow::Lane(lane) => Row::Lane {
            trace: lane.source.trace(),
            generator: lane.source.path().to_vec(),
            height: lane.height,
            tint: lane.tint,
        },
        WaveRow::Clock(clock) => Row::Clock {
            trace: clock.key.trace,
            clock: clock.key.item.clone(),
            height: clock.height,
            tint: clock.tint,
        },
        WaveRow::Group(_) => unreachable!("nest writes groups"),
    });
    Ok(serde_json::value::to_raw_value(&WavePanel {
        id,
        kind: "waves".into(),
        version: VERSION,
        title,
        link: w.nav.link,
        viewport: (!w.nav.link.viewport).then(|| w.nav.local_viewport.target()),
        cursor: (!w.nav.link.cursor)
            .then(|| serde_json::value::to_raw_value(&w.nav.local_cursor))
            .transpose()?,
        scroll_y: w.scroll_y,
        columns: Columns {
            names: w.names_width,
            values: w.values_width,
        },
        rows,
        selected: w.selected.clone(),
        clocks: w.nav.clocks().clone(),
    })?)
}

pub(crate) fn restore(raw: &RawValue, ctx: &mut RestoreContext<'_>) -> Result<Panel> {
    let mut saved: WavePanel = serde_json::from_str(raw.get()).context("invalid waveform panel")?;
    let mut flat = Vec::new();
    flatten(std::mem::take(&mut saved.rows), 0, &mut flat)?;
    admit_rows(&mut ctx.row_count, flat.len())?;
    ensure!(
        saved.scroll_y.is_finite() && saved.scroll_y >= 0.0,
        "invalid row scroll"
    );
    ensure!(
        [saved.columns.names, saved.columns.values]
            .iter()
            .all(|v| v.is_finite() && *v >= crate::wave::layout::MIN_COLUMN),
        "invalid column width"
    );
    ensure!(
        saved.selected.iter().all(|i| *i < flat.len()),
        "invalid selected row"
    );
    ensure!(
        saved.link.viewport == saved.viewport.is_none(),
        "local viewport must exist exactly when unlinked"
    );
    ensure!(
        saved.link.cursor == saved.cursor.is_none(),
        "local cursor must exist exactly when unlinked"
    );
    let mut w = WaveModel::new();
    w.nav.link = saved.link;
    w.nav.restore_clocks(saved.clocks);
    if let Some(v) = saved.viewport {
        valid_viewport(v)?;
        w.nav.local_viewport.set(v);
    }
    if let Some(c) = saved.cursor {
        w.nav.local_cursor = serde_json::from_str(c.get()).context("invalid local cursor")?;
    }
    w.scroll_y = saved.scroll_y;
    w.names_width = saved.columns.names;
    w.values_width = saved.columns.values;
    let mut items = Vec::with_capacity(flat.len());
    for (depth, row) in flat {
        let (
            trace,
            signal,
            nth,
            format,
            height,
            analog,
            overlay_hidden,
            tint,
            saved_shape,
            projection,
            bus,
        ) = match row {
            Row::Signal {
                shape,
                projection,
                bus,
                rows: _,
                trace,
                signal,
                nth,
                format,
                height,
                analog,
                overlay_hidden,
                tint,
            } => (
                trace,
                signal,
                nth,
                format,
                height,
                analog,
                overlay_hidden,
                tint,
                shape,
                projection,
                bus,
            ),
            Row::Lane {
                trace,
                generator,
                height,
                tint,
            } => {
                ensure!(!generator.is_empty(), "empty lane generator path");
                let track = ctx
                    .traces
                    .tracks(trace)
                    .iter()
                    .find(|t| t.path == generator);
                let lane = if let Some(track) = track {
                    TxLane {
                        height,
                        auto_height: false,
                        ..TxLane::new(ctx.doc, Traced::new(trace, track.id))
                            .context("lane generator vanished")?
                    }
                } else {
                    ctx.report
                        .push(format!("Missing lane generator: {generator:?}"));
                    TxLane::unresolved(trace, generator, height)
                };
                let lane = TxLane { tint, ..lane };
                items.push(Entry::new(depth, WaveRow::Lane(lane)));
                continue;
            }
            Row::Clock {
                trace,
                clock,
                height,
                tint,
            } => {
                ensure!(!clock.is_empty(), "empty clock path");
                let key = Traced::new(trace, clock);
                if ctx.doc.clocks.find(&key).is_none() {
                    ctx.report.push(format!("Missing clock: {}", key.item));
                }
                items.push(Entry::new(
                    depth,
                    WaveRow::Clock(crate::wave::model::ClockRow {
                        height,
                        tint,
                        ..crate::wave::model::ClockRow::new(key)
                    }),
                ));
                continue;
            }
            Row::Group {
                name,
                collapsed,
                height,
                tint,
                style,
                peak,
                overlaid,
                ..
            } => {
                items.push(Entry::new(
                    depth,
                    WaveRow::Group(GroupRow {
                        name,
                        collapsed,
                        height,
                        tint,
                        style: style.style(peak, overlaid)?,
                        restore_height: None,
                    }),
                ));
                continue;
            }
        };
        let row_signal = signal;
        ensure!(!row_signal.is_empty(), "empty signal path");
        let h = ctx.traces.hierarchy(trace);
        let found = h.map_or(Lookup::Missing, |h| h.find_var(&row_signal, nth));
        let (source, mut name, scope, shape) = match (found, h) {
            (Lookup::Found(var), Some(h)) => {
                let v = h.var(var);
                (
                    RowSource::Resolved {
                        trace,
                        var,
                        signal: v.signal,
                    },
                    v.name.to_owned(),
                    h.scope_path(v.scope).join("."),
                    v.shape,
                )
            }
            (found, _) => {
                let ambiguous = found == Lookup::Ambiguous;
                ctx.report.push(format!(
                    "{} signal: {:?}",
                    if ambiguous { "Ambiguous" } else { "Missing" },
                    row_signal
                ));
                let name = row_signal.last().unwrap().clone();
                let scope = row_signal[..row_signal.len() - 1].join(".");
                (
                    RowSource::Unresolved {
                        trace,
                        path: row_signal,
                        nth,
                        ambiguous,
                    },
                    name,
                    scope,
                    volna_trace::data::SignalShape::Bit,
                )
            }
        };
        ensure!(
            !matches!(
                saved_shape,
                volna_trace::data::SignalShape::Vector { width: 0 | 1 }
            ),
            "invalid source shape"
        );
        let resolved = source.signal().is_some();
        let source_shape = if resolved { shape } else { saved_shape };
        ensure!(source_shape == saved_shape, "signal shape changed");
        let shape = if let Some(slice) = projection {
            slice.validate(source_shape)?;
            name = super::bus::BusIndices::new(&name, source_shape.width()).label(slice);
            slice.shape()
        } else {
            source_shape
        };
        ensure!(
            bus.is_some() == matches!(shape, volna_trace::data::SignalShape::Vector { width: 2.. }),
            "invalid bus expansion"
        );
        ensure!(
            bus.is_none_or(|b| b.initialized || b.collapsed),
            "uninitialized bus must be collapsed"
        );
        let requested = ctx.doc.translators.get(&format);
        let translator = requested
            .clone()
            .unwrap_or_else(|| ctx.doc.translators.default_for(shape));
        if requested.is_none() {
            ctx.report.push(format!("Unknown translator: {format}"));
        }
        items.push(Entry::new(
            depth,
            WaveRow::from(DisplayedSignal {
                source_shape,
                projection,
                bus,
                projection_source: None,
                source,
                requested_format: requested.is_none().then_some(format),
                name,
                scope,
                shape,
                translator,
                history: None,
                error: None,
                height,
                analog: analog.map(|a| Analog::new(a.draw, a.range)),
                overlay_hidden,
                tint,
            }),
        ));
    }
    tree::validate(&items).map_err(anyhow::Error::msg)?;
    w.restore_rows(items);
    w.selected = saved.selected;
    w.anchor = w.selected.first().copied();
    w.fix_hidden_selection();
    Ok(Panel {
        id: saved.id,
        title: crate::history::Journaled::new(saved.title),
        kind: PanelKind::Waves(Box::new(w)),
    })
}
