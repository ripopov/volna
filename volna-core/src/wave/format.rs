//! Shared formatting controls for a signal selection.

use super::analog::{AnalogDraw, AnalogRange};
use super::model::{MenuAction, MenuEntry, MenuItem, RowHeight, WaveModel};
use super::tint::Tint;
use crate::document::Document;

pub(super) fn shared<T: PartialEq>(mut values: impl Iterator<Item = T>) -> Option<T> {
    let first = values.next()?;
    values.all(|value| value == first).then_some(first)
}

impl WaveModel {
    pub(super) fn format_rows(&self, row: usize) -> Vec<usize> {
        if row >= self.items().len() {
            return Vec::new();
        }
        self.leaves_of(&self.menu_rows(row))
            .into_iter()
            .filter(|&r| self.signal(r).is_some())
            .collect()
    }

    pub(super) fn waveform_rows(&self, row: usize) -> Vec<usize> {
        if row >= self.items().len() {
            return Vec::new();
        }
        let rows = self.menu_rows(row);
        if rows.iter().any(|&r| self.items()[r].is_group()) {
            // Group selection must not implicitly apply the separate signal controls to its contents.
            rows.into_iter()
                .filter(|&r| self.signal(r).is_some())
                .collect()
        } else {
            self.format_rows(row)
        }
    }

    pub(super) fn signal_display_menu(&self, doc: &Document, row: usize) -> Vec<MenuEntry> {
        let rows = self.format_rows(row);
        let signals: Vec<_> = rows.iter().filter_map(|&r| self.signal(r)).collect();
        let Some(first) = signals.first() else {
            return Vec::new();
        };
        let mut entries = Vec::new();
        let includes_groups = self
            .menu_rows(row)
            .iter()
            .any(|&r| self.items()[r].is_group());
        let format = shared(signals.iter().map(|s| s.translator.id()));
        let available: Vec<_> = signals.iter().map(|s| s.formats(doc)).collect();
        let formats: Vec<_> = available[0]
            .iter()
            .filter(|t| {
                available
                    .iter()
                    .all(|formats| formats.iter().any(|candidate| candidate.id() == t.id()))
            })
            .map(|t| {
                MenuItem::choice(
                    MenuAction::Format(t.id().into()),
                    t.name(),
                    format == Some(t.id()),
                )
            })
            .collect();
        if formats.len() != 1 {
            let description = if formats.is_empty() {
                Some("No value format supports every selected signal. Select signals of the same type to change their format.".into())
            } else {
                includes_groups.then(|| {
                    format!(
                        "Applies to {} signals, including group contents",
                        signals.len()
                    )
                })
            };
            entries.push(MenuEntry::Submenu {
                label: "Value format".into(),
                value: Some(format.map_or("Mixed", |_| first.translator.name()).into()),
                description,
                items: formats,
            });
        }
        let waveform_rows = self.waveform_rows(row);
        let waveform_signals: Vec<_> = waveform_rows
            .iter()
            .filter_map(|&r| self.signal(r))
            .collect();
        let plottable: Vec<_> = waveform_signals
            .iter()
            .copied()
            .filter(|s| super::analog::supports(s.shape, s.translator.as_ref()))
            .collect();
        if !plottable.is_empty() {
            let draw = shared(plottable.iter().map(|s| s.analog.as_ref().map(|a| a.draw)));
            let label = draw.map(|draw| match draw {
                None => "Digital",
                Some(AnalogDraw::Step) => "Analog — steps",
                Some(AnalogDraw::Linear) => "Analog — interpolated",
            });
            entries.push(MenuEntry::Submenu {
                label: if includes_groups {
                    "Signal waveform"
                } else {
                    "Waveform"
                }
                .into(),
                value: Some(label.unwrap_or("Mixed").into()),
                description: (includes_groups || plottable.len() != waveform_signals.len()).then(
                    || {
                        format!(
                            "Applies to {} of {} selected signals",
                            plottable.len(),
                            waveform_signals.len()
                        )
                    },
                ),
                items: [
                    (None, "Digital"),
                    (Some(AnalogDraw::Step), "Analog — steps"),
                    (Some(AnalogDraw::Linear), "Analog — interpolated"),
                ]
                .into_iter()
                .map(|(mode, label)| {
                    MenuItem::choice(MenuAction::Draw(mode), label, draw == Some(mode))
                })
                .collect(),
            });
            let analog_signals: Vec<_> = plottable
                .iter()
                .copied()
                .filter_map(|signal| signal.analog.as_ref().map(|analog| (signal, analog)))
                .collect();
            if !analog_signals.is_empty() {
                let range = shared(analog_signals.iter().map(|(_, analog)| analog.range));
                let mut ranges = vec![
                    (AnalogRange::Trace, "Whole trace"),
                    (AnalogRange::Window, "Visible time window"),
                ];
                if analog_signals
                    .iter()
                    .all(|(signal, _)| signal.translator.limits(signal.shape).is_some())
                {
                    ranges.push((AnalogRange::Type, "Type limits"));
                }
                let label = range.and_then(|range| {
                    ranges
                        .iter()
                        .find(|(r, _)| *r == range)
                        .map(|(_, label)| *label)
                });
                entries.push(MenuEntry::Submenu {
                    label: if includes_groups {
                        "Signal vertical scale"
                    } else {
                        "Vertical scale"
                    }
                    .into(),
                    value: Some(label.unwrap_or("Mixed").into()),
                    description: (includes_groups
                        || analog_signals.len() != waveform_signals.len())
                    .then(|| {
                        format!(
                            "Applies to {} of {} selected signals",
                            analog_signals.len(),
                            waveform_signals.len()
                        )
                    }),
                    items: ranges
                        .into_iter()
                        .map(|(r, label)| {
                            MenuItem::choice(MenuAction::Range(r), label, range == Some(r))
                        })
                        .collect(),
                });
            }
        }
        entries
    }

    pub(super) fn appearance_menu(&self, rows: &[usize]) -> [MenuEntry; 2] {
        let tint = shared(rows.iter().map(|&r| self.items()[r].tint()));
        let height = shared(rows.iter().map(|&r| self.items()[r].height()));
        let height_label = height.map(|h| format!("{}×", h.multiple()));
        [
            MenuEntry::Submenu {
                label: "Color".into(),
                value: Some(tint.map_or("Mixed", Tint::label).into()),
                description: None,
                items: std::iter::once(None)
                    .chain(Tint::ALL.map(Some))
                    .map(|t| MenuItem::choice(MenuAction::Tint(t), Tint::label(t), tint == Some(t)))
                    .collect(),
            },
            MenuEntry::Submenu {
                label: "Row height".into(),
                value: Some(height_label.unwrap_or_else(|| "Mixed".into())),
                description: None,
                items: (if rows.iter().any(|&r| self.items()[r].lane().is_some()) {
                    RowHeight::ALL.as_slice()
                } else {
                    RowHeight::PRESETS.as_slice()
                })
                .iter()
                .copied()
                .map(|h| {
                    MenuItem::choice(
                        MenuAction::RowHeight(h),
                        &if h.is_default() {
                            "1× (Default)".into()
                        } else {
                            format!("{}×", h.multiple())
                        },
                        height == Some(h),
                    )
                })
                .collect(),
            },
        ]
    }
}
