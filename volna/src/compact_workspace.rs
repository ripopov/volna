//! Narrow Volna windows show one workspace region at a time.

use super::Workspace;
use crate::theme::{ThemePx, theme};
use gpui_kit::component::{
    Selectable, Sizable,
    button::{Button, ButtonVariants},
};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, Context, Window, div};
use volna_core::app::Command;
use volna_core::panels::{PanelId, PanelsCommand, Placement};

pub(super) const REGION_BAR_HEIGHT: f32 = 32.;

impl Workspace {
    pub(super) fn focus_compact_hierarchy(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.compact_hierarchy = true;
        self.panel_focus_pending = false;
        window.focus(&self.scopes_focus, cx);
        cx.notify();
    }

    pub(super) fn compact_workspace(
        &self,
        sidebar: Option<AnyElement>,
        dock: AnyElement,
        window: &Window,
        cx: &Context<Self>,
    ) -> AnyElement {
        let maximized = self
            .dock
            .as_ref()
            .is_some_and(|host| host.read(cx).area.read(cx).is_zoomed());
        if maximized && !self.compact_hierarchy {
            return div().flex_1().min_h_0().child(dock).into_any_element();
        }
        let t = theme(cx);
        let width = window.viewport_size().width;
        let layout = self.app.panels.layout();
        let focus = self.app.panels.focused_id();
        let target = |tree: &volna_core::panels::Layout| {
            tree.active_for(focus).or_else(|| {
                if maximized {
                    None
                } else {
                    tree.visible().first().copied()
                }
            })
        };
        let center = layout.center.as_ref().and_then(target);
        let right = layout
            .right
            .as_ref()
            .filter(|r| r.open)
            .and_then(|r| target(&r.layout));
        let show_right = right.is_some()
            && (center.is_none() || layout.placement(focus) == Some(Placement::Right));
        let hierarchy = sidebar.is_some();
        let dock_pages = if center.is_some() && right.is_some() {
            2.
        } else {
            1.
        };
        let offset = if self.compact_hierarchy {
            0.
        } else {
            f32::from(hierarchy) + f32::from(show_right && center.is_some())
        };
        let page_button =
            |id: &'static str, label: &'static str, panel: Option<PanelId>, selected: bool| {
                Button::new(id)
                    .ghost()
                    .small()
                    .label(label)
                    .selected(selected)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if let Some(panel) = panel {
                            this.panel_focus_pending = true;
                            this.dispatch(
                                Command::Panels(PanelsCommand::Focus(panel)),
                                Some(window),
                                cx,
                            );
                        } else {
                            this.focus_compact_hierarchy(window, cx);
                        }
                    }))
            };
        let switcher = div()
            .flex_none()
            .flex()
            .items_center()
            .gap(t.px(4.))
            .px(t.px(6.))
            .h(t.px(REGION_BAR_HEIGHT))
            .when(hierarchy, |bar| {
                bar.child(page_button(
                    "compact-hierarchy",
                    "Hierarchy",
                    None,
                    self.compact_hierarchy,
                ))
            })
            .when_some(center, |bar, id| {
                bar.child(page_button(
                    "compact-center",
                    "Workspace",
                    Some(id),
                    !self.compact_hierarchy && !show_right,
                ))
            })
            .when_some(right, |bar, id| {
                let label = if self
                    .app
                    .panels
                    .get(id)
                    .is_some_and(|p| p.kind.transaction().is_some())
                {
                    "Details"
                } else {
                    "Right dock"
                };
                bar.child(page_button(
                    "compact-right",
                    label,
                    Some(id),
                    !self.compact_hierarchy && show_right,
                ))
            });
        // Keep the dock mounted so region navigation preserves tab groups and their scroll positions.
        let pages = div()
            .absolute()
            .top_0()
            .bottom_0()
            .left(width * -offset)
            .w(width * (dock_pages + f32::from(hierarchy)))
            .flex()
            .children(sidebar)
            .child(div().w(width * dock_pages).h_full().flex_none().child(dock));
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .when(hierarchy || dock_pages > 1., |el| el.child(switcher))
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .child(pages),
            )
            .into_any_element()
    }
}
