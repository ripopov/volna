//! Native rendering of waveform menus; selection and command handling stay in the core and workspace.

use std::rc::Rc;

use gpui_kit::component::{
    menu::{PopupMenu, PopupMenuItem},
    tooltip::Tooltip,
};
use gpui_kit::prelude::*;
use gpui_kit::{App, Entity, FocusHandle, ParentElement, Styled, Window, div, px};
use volna_core::wave::model::MenuAction;
use volna_core::wave::{MenuEntry, MenuItem, MenuItemState, WaveMenu, WaveMenuKind};

use crate::app::{
    CopySignals, CutSignals, GroupSelection, PasteSignals, RemoveSelected, RenameGroup,
    ToggleOverlaid, ToggleStack, Ungroup,
};
use crate::theme::{ThemePx, theme};
use crate::ui::{Icon, IconName};

type Select = dyn Fn(MenuAction, &mut Window, &mut App);

pub(crate) fn build(
    model: &WaveMenu,
    focus: FocusHandle,
    window: &mut Window,
    cx: &mut App,
    select: impl Fn(MenuAction, &mut Window, &mut App) + 'static,
) -> Entity<PopupMenu> {
    let select: Rc<Select> = Rc::new(select);
    let item = move |entry: &MenuItem| render_item(entry, select.clone());
    let entries = model.entries.clone();
    let min_w = theme(cx).px(200.0);
    let signal_width = (model.kind == WaveMenuKind::Signal).then(|| {
        theme(cx)
            .px(300.0)
            .min((window.viewport_size().width - theme(cx).px(16.0)).max(px(0.0)))
    });
    PopupMenu::build(window, cx, move |mut menu, window, cx| {
        for entry in &entries {
            menu = match entry {
                MenuEntry::Item(entry) => menu.item(item(entry)),
                MenuEntry::Separator => menu.separator(),
                MenuEntry::Label(label) if signal_width.is_some() => {
                    let label = label.clone();
                    let width = signal_width.unwrap();
                    menu.item(
                        PopupMenuItem::element(move |_, cx| {
                            div()
                                .id("wave-menu-heading")
                                .w(width - theme(cx).px(32.0))
                                .truncate()
                                .text_color(theme(cx).panel.text_muted)
                                .aria_label(label.clone())
                                .child(label.clone())
                                .tooltip_show_delay(crate::ui::tooltip::delay(cx))
                                .tooltip({
                                    let label = label.clone();
                                    move |window, cx| Tooltip::new(label.clone()).build(window, cx)
                                })
                        })
                        .disabled(true),
                    )
                }
                MenuEntry::Label(label) => menu.label(label.clone()),
                MenuEntry::Submenu {
                    label,
                    value,
                    description,
                    items,
                } => {
                    let (items, description, item, focus) = (
                        items.clone(),
                        description.clone(),
                        item.clone(),
                        focus.clone(),
                    );
                    let (label, description) = submenu_text(
                        label,
                        value.as_deref(),
                        description.as_deref(),
                        window.viewport_size().width < theme(cx).px(600.0),
                    );
                    menu.submenu(label, window, cx, move |mut sub, window, cx| {
                        let width = theme(cx)
                            .px(280.0)
                            .min((window.viewport_size().width - theme(cx).px(16.0)).max(px(0.0)));
                        sub = sub.min_w(width).max_w(width).scrollable(true).max_h(
                            (window.viewport_size().height - theme(cx).px(16.0)).max(px(0.0)),
                        );
                        if let Some(description) = &description {
                            let description = description.clone();
                            sub = sub.item(
                                PopupMenuItem::element(move |_, cx| {
                                    div()
                                        .id("wave-menu-description")
                                        .w(width - theme(cx).px(32.0))
                                        .whitespace_normal()
                                        .text_size(theme(cx).px(12.0))
                                        .text_color(theme(cx).panel.text_muted)
                                        .aria_label(description.clone())
                                        .child(description.clone())
                                })
                                .disabled(true),
                            );
                        }
                        for entry in &items {
                            sub = sub.item(item(entry));
                        }
                        sub.action_context(focus.clone())
                    })
                }
            };
        }
        menu = menu
            .scrollable(true)
            .max_h((window.viewport_size().height - theme(cx).px(16.0)).max(px(0.0)));
        // The toolkit uses its maximum width to choose the submenu side.
        // Capping it to the parent width makes submenus overflow narrow windows.
        menu = match signal_width {
            Some(width) => menu.min_w(width),
            None => menu.min_w(min_w),
        };
        menu.action_context(focus.clone())
    })
}

fn render_item(item: &MenuItem, select: Rc<Select>) -> PopupMenuItem {
    let id = item.action.clone();
    let label = item.label.clone();
    let shortcut: Option<Box<dyn gpui_kit::Action>> = match item.action {
        MenuAction::CopySignals => Some(Box::new(CopySignals)),
        MenuAction::CutSignals => Some(Box::new(CutSignals)),
        MenuAction::PasteSignals => Some(Box::new(PasteSignals)),
        MenuAction::RemoveSignals => Some(Box::new(RemoveSelected)),
        MenuAction::Group => Some(Box::new(GroupSelection)),
        MenuAction::Ungroup => Some(Box::new(Ungroup)),
        MenuAction::Rename => Some(Box::new(RenameGroup)),
        MenuAction::Stack(true) => Some(Box::new(ToggleStack)),
        MenuAction::Overlaid => Some(Box::new(ToggleOverlaid)),
        _ => None,
    };
    let entry = match &item.action {
        MenuAction::Derived => PopupMenuItem::element(move |_, _| {
            div()
                .id("wave-menu-derived")
                .debug_selector(|| "wave-menu-derived".into())
                .aria_label(label.clone())
                .child(label.clone())
        }),
        MenuAction::Draw(draw) => {
            let draw = *draw;
            let (selector, icon, hint) = match draw {
                None => (
                    "waveform-digital",
                    IconName::WaveDigital,
                    "Show digital states and values",
                ),
                Some(volna_core::wave::analog::AnalogDraw::Step) => (
                    "waveform-steps",
                    IconName::WaveStep,
                    "Hold each sample until the next sample",
                ),
                Some(volna_core::wave::analog::AnalogDraw::Linear) => (
                    "waveform-lines",
                    IconName::WaveLinear,
                    "Interpolate between samples with straight lines",
                ),
            };
            PopupMenuItem::element(move |_, cx| {
                div()
                    .id(selector)
                    .debug_selector(move || selector.into())
                    .flex()
                    .items_center()
                    .gap(theme(cx).px(8.0))
                    .aria_label(label.clone())
                    .aria_description(hint)
                    .child(Icon::new(icon).inherit_color())
                    .child(label.clone())
                    .tooltip_show_delay(crate::ui::tooltip::delay(cx))
                    .tooltip(move |window, cx| Tooltip::new(hint).build(window, cx))
            })
        }
        MenuAction::Tint(tint) => {
            let tint = *tint;
            PopupMenuItem::element(move |_, cx| {
                let t = theme(cx);
                let swatch = t.px(10.0);
                div()
                    .flex()
                    .items_center()
                    .gap(t.px(8.0))
                    .child(div().size(swatch).rounded(t.px(2.0)).bg(t.ink(tint)))
                    .child(label.clone())
            })
        }
        _ if shortcut.is_none() && item.badge.is_some() => {
            let badge = item.badge.clone().unwrap();
            PopupMenuItem::element(move |_, cx| {
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(theme(cx).px(16.0))
                    .id(gpui_kit::SharedString::from(format!(
                        "wave-menu-hint-{label}"
                    )))
                    .aria_label(format!("{label}, {badge}"))
                    .child(label.clone())
                    .child(
                        div()
                            .text_color(theme(cx).panel.text_muted)
                            .child(badge.clone()),
                    )
            })
        }
        _ => PopupMenuItem::new(label),
    };
    let entry = match shortcut {
        Some(action) => entry.action(action),
        None => entry,
    };
    let entry = match item.state {
        MenuItemState::Choice { selected } => {
            entry.icon(gpui_kit::component::Icon::empty().path(if selected {
                IconName::CircleDot.path()
            } else {
                IconName::Circle.path()
            }))
        }
        MenuItemState::Command | MenuItemState::Toggle { .. } => entry,
    };
    entry
        .disabled(item.disabled)
        .checked(item.is_checked())
        .on_click(move |_, window, cx| select(id.clone(), window, cx))
}

fn submenu_text(
    label: &str,
    value: Option<&str>,
    description: Option<&str>,
    narrow: bool,
) -> (String, Option<String>) {
    match (value, narrow) {
        (Some(value), true) => {
            let current = format!("Current: {value}");
            let description =
                Some(description.map_or(current.clone(), |reason| format!("{current}. {reason}")));
            (label.into(), description)
        }
        (Some(value), false) => (format!("{label} · {value}"), description.map(str::to_owned)),
        (None, _) => (label.into(), description.map(str::to_owned)),
    }
}
