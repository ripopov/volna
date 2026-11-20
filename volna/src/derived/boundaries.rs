//! Selectable measurement boundaries in timeline order.

use super::*;
use volna_core::computed::{Boundary, MeasurementScope};

fn selections(scope: &MeasurementScope, domain: (u64, u64)) -> Vec<Boundary> {
    let fixed = |tick| match tick {
        t if t == domain.0 => Boundary::Start,
        t if t == domain.1 => Boundary::End,
        t => Boundary::Tick(t),
    };
    match scope {
        MeasurementScope::WholeRange => vec![Boundary::Start, Boundary::End],
        MeasurementScope::Explicit { start, end } => vec![fixed(*start), fixed(*end)],
        MeasurementScope::IntervalSnapshot { intervals } => {
            let mut points: Vec<_> = intervals
                .iter()
                .flat_map(|&(a, b)| [fixed(a), fixed(b)])
                .collect();
            points.sort_unstable();
            points.dedup();
            points
        }
        MeasurementScope::Selected { boundaries } => boundaries.clone(),
    }
}

impl DerivedEditor {
    pub(super) fn boundary_picker(
        &self,
        doc: &volna_core::Document,
        domain: (u64, u64),
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let selected: std::collections::HashSet<_> =
            selections(&self.scope, domain).into_iter().collect();
        let mut choices = vec![
            (Boundary::Start, Some(domain.0), "Start".to_owned()),
            (Boundary::End, Some(domain.1), "End".to_owned()),
        ];
        choices.extend(doc.markers().iter().map(|marker| {
            let name = match &marker.label {
                Some(label) => format!("M{} {label}", marker.id),
                None => format!("M{}", marker.id),
            };
            (Boundary::Marker(marker.id), Some(marker.time), name)
        }));
        for &tick in &self.saved_ticks {
            if selected.contains(&Boundary::Tick(tick)) || (tick != domain.0 && tick != domain.1) {
                choices.push((Boundary::Tick(tick), Some(tick), "Saved".to_owned()));
            }
        }
        let available: std::collections::HashSet<_> =
            choices.iter().map(|(point, _, _)| *point).collect();
        for boundary in selected.difference(&available) {
            if let Boundary::Marker(id) = boundary {
                choices.push((*boundary, None, format!("M{id} · removed")));
            }
        }
        choices.sort_unstable_by_key(|(boundary, time, _)| (time.unwrap_or(u64::MAX), *boundary));
        let mut buttons = div().flex().flex_wrap().gap_1();
        for (boundary, time, name) in choices {
            let label = time.map_or_else(
                || name.clone(),
                |t| {
                    format!(
                        "{name} · {}",
                        volna_core::wave::timeline::format_time(t as f64, doc.time_base())
                    )
                },
            );
            let id = match boundary {
                Boundary::Start => "scope-start".to_owned(),
                Boundary::End => "scope-end".to_owned(),
                Boundary::Tick(t) => format!("scope-tick-{t}"),
                Boundary::Marker(id) => format!("scope-marker-{id}"),
            };
            let active = selected.contains(&boundary);
            buttons = buttons.child(
                Button::new(id.clone())
                    .debug_selector(move || id)
                    .small()
                    .label(label)
                    .selected(active)
                    .disabled(!active && time.is_some_and(|t| !(domain.0..=domain.1).contains(&t)))
                    .on_click(cx.listener(move |view, _, window, cx| {
                        let mut boundaries = selections(&view.scope, domain);
                        if let Some(i) = boundaries.iter().position(|&b| b == boundary) {
                            boundaries.remove(i);
                        } else {
                            boundaries.push(boundary);
                        }
                        view.set_scope(MeasurementScope::Selected { boundaries }, window, cx);
                    })),
            );
        }
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child("Measure between selected times")
            .child(buttons)
    }
}

pub(super) fn saved_ticks(scope: &MeasurementScope) -> Vec<u64> {
    let mut ticks = match scope {
        MeasurementScope::Explicit { start, end } => vec![*start, *end],
        MeasurementScope::IntervalSnapshot { intervals } => {
            intervals.iter().flat_map(|&(a, b)| [a, b]).collect()
        }
        MeasurementScope::Selected { boundaries } => boundaries
            .iter()
            .filter_map(|point| match point {
                Boundary::Tick(tick) => Some(*tick),
                _ => None,
            })
            .collect(),
        MeasurementScope::WholeRange => Vec::new(),
    };
    ticks.sort_unstable();
    ticks.dedup();
    ticks
}
