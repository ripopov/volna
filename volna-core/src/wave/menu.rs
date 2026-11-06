//! Contextual commands for waveform rows and selections.

use super::analog::{AnalogDraw, AnalogRange};
use super::format::shared;
use super::model::{
    GroupStyle, MenuAction, MenuEntry, MenuItem, WaveMenu, WaveMenuKind, WaveModel,
};
use crate::document::Document;
use crate::geometry::{Modifiers, Point};

fn command(action: MenuAction, label: &str) -> MenuEntry {
    MenuEntry::Item(MenuItem::plain(action, label))
}

impl WaveModel {
    pub fn open_signal_menu(&mut self, doc: &Document, row: usize, position: Point) {
        if row >= self.items().len() {
            return;
        }
        if !self.selected.contains(&row) {
            self.select_row(row, Modifiers::default());
        }
        let targets = self.menu_rows(row);
        let signals = self.format_rows(row);
        let any_group = targets.iter().any(|&r| self.items()[r].is_group());
        let only_signals = targets.iter().all(|&r| self.signal(r).is_some());
        let heading = if targets.len() == 1 {
            let entry = &self.items()[row];
            if entry.is_group() {
                format!("Group: {}", entry.name())
            } else if entry.scope().is_empty() {
                entry.name().to_owned()
            } else {
                format!("{}.{}", entry.scope(), entry.name())
            }
        } else if only_signals {
            format!("{} signals selected", targets.len())
        } else {
            format!("{} rows selected", targets.len())
        };
        let mut entries = vec![MenuEntry::Label(heading), MenuEntry::Separator];
        entries.extend(self.signal_display_menu(doc, row));
        entries.extend(self.group_display_menu(&targets));
        entries.extend(self.appearance_menu(&targets));

        if !signals.is_empty() {
            let plural = signals.len() > 1;
            entries.push(MenuEntry::Separator);
            entries.push(MenuEntry::Item(MenuItem {
                disabled: signals.iter().any(|&r| {
                    self.signal(r)
                        .unwrap()
                        .cursor_value_view(self.cursor(doc))
                        .is_none()
                }),
                ..MenuItem::plain(
                    MenuAction::CopyValues,
                    if any_group {
                        "Copy contained values"
                    } else if plural {
                        "Copy values at cursor"
                    } else {
                        "Copy value at cursor"
                    },
                )
            }));
            entries.push(command(
                MenuAction::CopyPaths,
                if any_group {
                    "Copy contained paths"
                } else if plural {
                    "Copy full signal paths"
                } else {
                    "Copy full signal path"
                },
            ));
        }
        entries.push(MenuEntry::Separator);
        if let Some(signal) = self.signal(row) {
            if let Some(bus) = signal.bus {
                entries.push(command(
                    MenuAction::Fold(!bus.collapsed),
                    if bus.collapsed {
                        "Expand bits"
                    } else {
                        "Collapse bits"
                    },
                ));
                entries.push(command(MenuAction::AddSlice, "Add bit or slice…"));
                if bus.initialized {
                    entries.push(command(MenuAction::ReverseBits, "Reverse bit order"));
                }
            }
            if signal.projection.is_some() {
                entries.push(command(
                    MenuAction::AddIndependent,
                    "Add as independent trace",
                ));
            }
        }
        entries.push(MenuEntry::Item(MenuItem {
            badge: Some("G".into()),
            ..MenuItem::plain(MenuAction::Group, "Group selection")
        }));
        if any_group {
            entries.push(MenuEntry::Item(MenuItem {
                badge: Some("⇧G".into()),
                ..MenuItem::plain(MenuAction::Ungroup, "Ungroup")
            }));
        }
        if let Some(group) = self.items()[row].group() {
            entries.push(MenuEntry::Item(MenuItem {
                badge: Some("F2".into()),
                ..MenuItem::plain(MenuAction::Rename, "Rename group…")
            }));
            entries.push(command(
                MenuAction::Fold(!group.collapsed),
                if group.collapsed {
                    "Expand group"
                } else {
                    "Collapse group"
                },
            ));
        }
        if self.overlaid_of(row).is_some()
            && self.signal(row).is_some_and(|s| s.numeric_kind().is_some())
        {
            let hidden = self.signal(row).unwrap().overlay_hidden;
            entries.push(MenuEntry::Separator);
            entries.push(MenuEntry::Item(MenuItem::toggle(
                MenuAction::OverlayHidden(!hidden),
                "Show in overlay",
                !hidden,
            )));
            entries.push(command(MenuAction::IsolateOverlay, "Isolate in overlay"));
            entries.push(command(
                MenuAction::ShowAllOverlay,
                "Show all overlay lines",
            ));
        }
        entries.push(MenuEntry::Separator);
        if let Some(signal) = self
            .signal(row)
            .filter(|s| s.numeric_kind().is_some() || s.source.computed().is_some())
        {
            entries.push(command(
                MenuAction::Derived,
                if signal.source.computed().is_some() {
                    "Edit derived signal…"
                } else {
                    "Derive signal…"
                },
            ));
        }
        entries.push(command(
            MenuAction::OpenTable,
            if self
                .signal(row)
                .is_some_and(|s| s.source.computed().is_some())
            {
                "Open recorded inputs in table"
            } else if self.signal(row).is_some_and(|s| s.projection.is_some()) {
                "Open source signal in table"
            } else {
                "Open in table"
            },
        ));
        if let Some(lane) = self.items()[row].lane() {
            if let Some(track) = lane.track().filter(|&track| {
                !doc.clocks.is_clock_track(track)
                    && doc.session(track.trace).is_some_and(|session| {
                        session.capabilities().transactions
                            && session.tracks().iter().any(|t| t.id == track.item)
                    })
            }) {
                entries.push(command(
                    MenuAction::OpenPipeline(track),
                    "Open in Pipeline view",
                ));
            }
            if lane.generator(doc).is_some_and(|g| g.points().is_none()) {
                entries.push(MenuEntry::Item(MenuItem::toggle(
                    MenuAction::Occupancy(!lane.occupancy),
                    "Occupancy",
                    lane.occupancy,
                )));
                entries.push(command(MenuAction::FitHeight, "Fit height"));
            }
        }
        if self
            .signal(row)
            .is_some_and(|s| s.error.is_some() && s.source.signal().is_some())
        {
            entries.push(command(MenuAction::RetryLoad, "Retry loading"));
        }
        entries.push(MenuEntry::Separator);
        entries.push(command(
            MenuAction::CutSignals,
            if only_signals {
                "Cut signals"
            } else {
                "Cut rows"
            },
        ));
        entries.push(command(
            MenuAction::CopySignals,
            if only_signals {
                "Copy signals"
            } else {
                "Copy rows"
            },
        ));
        if !doc.copied_rows.is_empty() {
            entries.push(command(MenuAction::PasteSignals, "Paste rows"));
        }
        entries.push(MenuEntry::Separator);
        entries.push(command(
            MenuAction::RemoveSignals,
            if any_group && targets.len() == 1 {
                "Remove group and contents"
            } else if any_group {
                "Remove rows and contents"
            } else {
                "Remove from view"
            },
        ));
        self.menu = Some(WaveMenu {
            kind: WaveMenuKind::Signal,
            row,
            position,
            entries,
        });
    }

    fn group_display_menu(&self, rows: &[usize]) -> Vec<MenuEntry> {
        let groups: Vec<_> = rows
            .iter()
            .filter_map(|&r| self.items()[r].group())
            .collect();
        if groups.is_empty() {
            return Vec::new();
        }
        let style = shared(groups.iter().map(|g| {
            if g.is_stacked() {
                "Stacked area"
            } else if g.is_overlaid() {
                "Overlaid lines"
            } else {
                "Activity"
            }
        }));
        let mut entries = vec![MenuEntry::Submenu {
            label: "Group display".into(),
            value: Some(style.unwrap_or("Mixed").into()),
            description: (groups.len() != rows.len()).then(|| {
                format!(
                    "Applies to {} of {} selected rows",
                    groups.len(),
                    rows.len()
                )
            }),
            items: vec![
                MenuItem::choice(
                    MenuAction::Stack(false),
                    "Activity",
                    style == Some("Activity"),
                ),
                MenuItem {
                    badge: Some("⇧A".into()),
                    ..MenuItem::choice(
                        MenuAction::Stack(true),
                        "Stacked area",
                        style == Some("Stacked area"),
                    )
                },
                MenuItem {
                    badge: Some("⇧O".into()),
                    ..MenuItem::choice(
                        MenuAction::Overlaid,
                        "Overlaid lines",
                        style == Some("Overlaid lines"),
                    )
                },
            ],
        }];
        let overlays: Vec<_> = groups
            .iter()
            .filter_map(|g| match g.style {
                GroupStyle::Overlaid { draw, range } => Some((draw, range)),
                _ => None,
            })
            .collect();
        if !overlays.is_empty() {
            let draw = shared(overlays.iter().map(|&(draw, _)| draw));
            let range = shared(overlays.iter().map(|&(_, range)| range));
            let applicability = (overlays.len() != rows.len())
                .then(|| format!("Applies to {} overlaid groups", overlays.len()));
            entries.push(MenuEntry::Submenu {
                label: "Interpolation".into(),
                value: Some(
                    draw.map_or("Mixed", |d| match d {
                        AnalogDraw::Step => "Steps",
                        AnalogDraw::Linear => "Interpolated",
                    })
                    .into(),
                ),
                description: applicability.clone(),
                items: [
                    (AnalogDraw::Step, "Steps"),
                    (AnalogDraw::Linear, "Interpolated"),
                ]
                .into_iter()
                .map(|(d, label)| {
                    MenuItem::choice(MenuAction::OverlaidDraw(d), label, draw == Some(d))
                })
                .collect(),
            });
            entries.push(MenuEntry::Submenu {
                label: "Group vertical scale".into(),
                value: Some(
                    range
                        .map_or("Mixed", |r| match r {
                            AnalogRange::Trace => "Whole trace",
                            AnalogRange::Window => "Visible time window",
                            AnalogRange::Type => "Type limits",
                        })
                        .into(),
                ),
                description: applicability,
                items: [
                    (AnalogRange::Trace, "Whole trace"),
                    (AnalogRange::Window, "Visible time window"),
                    (AnalogRange::Type, "Type limits"),
                ]
                .into_iter()
                .map(|(r, label)| {
                    MenuItem::choice(MenuAction::OverlaidRange(r), label, range == Some(r))
                })
                .collect(),
            });
            entries.push(command(
                MenuAction::ShowAllOverlay,
                "Show all overlay lines",
            ));
        }
        let stacked: Vec<_> = groups.iter().filter(|g| g.is_stacked()).collect();
        if !stacked.is_empty() {
            let checked = stacked.iter().all(|g| g.shows_peak());
            entries.push(MenuEntry::Item(MenuItem::toggle(
                MenuAction::Peak(!checked),
                "Peak of total",
                checked,
            )));
        }
        entries
    }
}
