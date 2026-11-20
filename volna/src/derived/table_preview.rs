//! Virtualized interval rows from the current Measure preview.

use super::*;
use gpui_kit::uniform_list;
use volna_core::table::statistics::{Statistic, column_title};
use volna_trace::data::{SignalShape, value_view::ValueView};

fn row(
    id: impl Into<gpui_kit::ElementId>,
    text: [String; 3],
    header: bool,
    t: &crate::theme::Theme,
) -> gpui_kit::Stateful<gpui_kit::Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .role(gpui_kit::accesskit::Role::Row)
        .w_full()
        .h(t.px(22.))
        .border_b_1()
        .border_color(t.border_variant)
        .when(header, |el| el.bg(t.bar.bg).text_color(t.panel.text_muted))
        .when(!header, |el| el.font_family(t.mono_font))
        .children(text.into_iter().enumerate().map(|(column, value)| {
            div()
                .id(column)
                .role(if header {
                    gpui_kit::accesskit::Role::ColumnHeader
                } else {
                    gpui_kit::accesskit::Role::Cell
                })
                .min_w_0()
                .flex_1()
                .px(t.px(10.))
                .truncate()
                .child(value)
        }))
}

impl DerivedEditor {
    pub(super) fn table_preview(
        ws: &Workspace,
        name: &str,
        preview: Option<&ComputationStatus>,
        cx: &Context<Self>,
    ) -> AnyElement {
        let t = *theme(cx);
        let toggle = Button::new("measure-table-toggle")
            .debug_selector(|| "measure-table-toggle".into())
            .tooltip("Show or hide table preview")
            .ghost()
            .xsmall()
            .label(if ws.measure_table_collapsed {
                "▸ Table preview"
            } else {
                "▾ Table preview"
            })
            .on_click(cx.listener(|view, _, _, cx| {
                if let Some(owner) = view.owner.upgrade() {
                    owner.update(cx, |ws, _| {
                        ws.measure_table_collapsed = !ws.measure_table_collapsed;
                    });
                    cx.notify();
                }
            }));
        let pane = div()
            .id("measure-table-preview")
            .debug_selector(|| "measure-table-preview".into())
            .flex()
            .flex_col()
            .w_full()
            .min_h_0()
            .overflow_hidden()
            .border_t_1()
            .border_color(t.border)
            .child(
                div()
                    .flex()
                    .items_center()
                    .px(t.px(6.))
                    .h(t.px(23.))
                    .flex_none()
                    .child(toggle),
            );
        if ws.measure_table_collapsed {
            return pane.flex_none().into_any_element();
        }
        let pane = pane.flex_1().min_h(t.px(68.));
        let result = preview
            .and_then(ComputationStatus::output)
            .and_then(|output| output.measurement());
        let Some(result) = result else {
            return pane.into_any_element();
        };
        let translator = ws.app.doc.translators.default_for(SignalShape::Real);
        let list = uniform_list(
            "measure-table-list",
            result.readings().len(),
            move |range, _, _| {
                range
                    .map(|index| {
                        let reading = &result.readings()[index];
                        let stat = Statistic::from(reading);
                        let value = stat.value.map_or(ValueView::Unavailable, ValueView::Real);
                        let text = translator.translate_view(&value, usize::MAX).text;
                        row(
                            ("measure-table-row", index),
                            [stat.start.to_string(), text, reading.bounds.1.to_string()],
                            false,
                            &t,
                        )
                        .debug_selector(move || format!("measure-table-row-{}", stat.start))
                    })
                    .collect()
            },
        )
        .flex_1()
        .min_h_0()
        .w_full();
        let table = div()
            .id("measurement-rows")
            .role(gpui_kit::accesskit::Role::Table)
            .aria_label("Table preview")
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(
                row(
                    "measure-table-header",
                    std::array::from_fn(|i| column_title(name, true, i).to_owned()),
                    true,
                    &t,
                )
                .flex_none(),
            )
            .child(list);
        pane.child(table).into_any_element()
    }
}
