//! Shared formatting controls for a signal selection.

use super::analog::{AnalogDraw, AnalogRange};
use super::model::{MenuAction, MenuEntry, MenuItem, RowHeight, WaveMenu, WaveMenuKind, WaveModel};
use super::tint::Tint;
use crate::document::Document;
use crate::geometry::Point;

fn shared<T: PartialEq>(mut values: impl Iterator<Item = T>) -> Option<T> {
    let first = values.next()?;
    values.all(|value| value == first).then_some(first)
}

fn setting_label(setting: &str, value: Option<&str>) -> String {
    format!("{setting} · {}", value.unwrap_or("Mixed"))
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

    pub fn open_format_menu(&mut self, doc: &Document, row: usize, position: Point) {
        let rows = self.format_rows(row);
        let signals: Vec<_> = rows.iter().filter_map(|&r| self.signal(r)).collect();
        let Some(first) = signals.first() else { return };
        let heading = if signals.len() == 1 {
            "Format signal".into()
        } else {
            format!("Format {} signals", signals.len())
        };
        let mut entries = vec![MenuEntry::Label(heading)];
        let plottable: Vec<_> = signals
            .iter()
            .copied()
            .filter(|s| super::analog::supports(s.shape, s.translator.as_ref()))
            .collect();
        if !plottable.is_empty() {
            let draw = shared(plottable.iter().map(|s| s.analog.as_ref().map(|a| a.draw)));
            let label = draw.map(|draw| match draw {
                None => "Digital",
                Some(AnalogDraw::Step) => "Analog steps",
                Some(AnalogDraw::Linear) => "Analog lines",
            });
            entries.extend([
                MenuEntry::Separator,
                MenuEntry::Label(setting_label("Waveform", label)),
            ]);
            if plottable.len() != signals.len() {
                entries.push(MenuEntry::Label(format!(
                    "Applies to {} numeric signal{}",
                    plottable.len(),
                    if plottable.len() == 1 { "" } else { "s" }
                )));
            }
            for (mode, label) in [
                (None, "Digital"),
                (Some(AnalogDraw::Step), "Analog steps"),
                (Some(AnalogDraw::Linear), "Analog lines"),
            ] {
                entries.push(MenuEntry::Item(MenuItem {
                    checked: draw == Some(mode),
                    ..MenuItem::plain(MenuAction::Draw(mode), label)
                }));
            }
            let analog_signals: Vec<_> = plottable
                .iter()
                .copied()
                .filter_map(|signal| signal.analog.as_ref().map(|analog| (signal, analog)))
                .collect();
            if !analog_signals.is_empty() {
                let range = shared(analog_signals.iter().map(|(_, analog)| analog.range));
                let mut ranges = vec![
                    (AnalogRange::Trace, "Whole trace"),
                    (AnalogRange::Window, "Visible window"),
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
                    label: setting_label("Vertical scale", label),
                    items: ranges
                        .into_iter()
                        .map(|(r, label)| MenuItem {
                            checked: range == Some(r),
                            ..MenuItem::plain(MenuAction::Range(r), label)
                        })
                        .collect(),
                });
            }
        }
        let format = shared(signals.iter().map(|s| s.translator.id()));
        let formats: Vec<_> = doc
            .translators
            .applicable(first.shape)
            .into_iter()
            .filter(|t| signals.iter().all(|s| t.applies(s.shape)))
            .map(|t| MenuItem {
                action: MenuAction::Format(t.id().into()),
                label: t.name().into(),
                badge: Some(t.badge().into()),
                checked: format == Some(t.id()),
            })
            .collect();
        if formats.len() != 1 {
            let label = setting_label("Value format", format.map(|_| first.translator.name()));
            entries.push(MenuEntry::Separator);
            if formats.is_empty() {
                entries.push(MenuEntry::Label(label));
                entries.push(MenuEntry::Label("No shared value format".into()));
            } else {
                entries.push(MenuEntry::Submenu {
                    label,
                    items: formats,
                });
            }
        }
        entries.extend([MenuEntry::Separator, MenuEntry::Label("Appearance".into())]);
        entries.extend(self.appearance_menu(&rows, true));
        if self
            .signal(row)
            .is_some_and(|s| s.error.is_some() && s.source.signal().is_some())
        {
            entries.push(MenuEntry::Separator);
            entries.push(MenuEntry::Item(MenuItem::plain(
                MenuAction::RetryLoad,
                "Retry loading",
            )));
        }
        self.menu = Some(WaveMenu {
            kind: WaveMenuKind::Format,
            row,
            position,
            entries,
        });
    }

    pub(super) fn appearance_menu(&self, rows: &[usize], summary: bool) -> [MenuEntry; 2] {
        let tint = shared(rows.iter().map(|&r| self.items()[r].tint()));
        let height = shared(rows.iter().map(|&r| self.items()[r].height()));
        let height_label = height.map(|h| format!("{}×", h.multiple()));
        [
            MenuEntry::Submenu {
                label: if summary {
                    setting_label("Color", tint.map(Tint::label))
                } else {
                    "Color".into()
                },
                items: std::iter::once(None)
                    .chain(Tint::ALL.map(Some))
                    .map(|t| MenuItem {
                        checked: tint == Some(t),
                        ..MenuItem::plain(MenuAction::Tint(t), Tint::label(t))
                    })
                    .collect(),
            },
            MenuEntry::Submenu {
                label: if summary {
                    setting_label("Row height", height_label.as_deref())
                } else {
                    "Height".into()
                },
                items: (if rows.iter().any(|&r| self.items()[r].lane().is_some()) {
                    RowHeight::ALL.as_slice()
                } else {
                    RowHeight::PRESETS.as_slice()
                })
                .iter()
                .copied()
                .map(|h| MenuItem {
                    action: MenuAction::RowHeight(h),
                    label: if h.is_default() {
                        "1× (Default)".into()
                    } else {
                        format!("{}×", h.multiple())
                    },
                    badge: None,
                    checked: height == Some(h),
                })
                .collect(),
            },
        ]
    }
}
