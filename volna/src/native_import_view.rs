//! Conversion status uses the same progress records as the terminal host.

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::prelude::*;
use gpui_kit::{
    Animation, AnimationExt, AnyElement, Context, IntoElement, SharedString, div, relative,
};
use volna_core::app::Command;
use volna_core::trace::TraceId;
use vtr_progress::{Phase, format_bytes};

use crate::app::Workspace;
use crate::theme::{ThemePx, theme};

impl Workspace {
    pub(crate) fn render_import_progress(
        &self,
        trace: TraceId,
        compact: bool,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let progress = self.native_imports.snapshot(trace)?;
        let t = *theme(cx);
        let phase = if progress.phase == Phase::Complete {
            "Opening VTR"
        } else {
            progress.phase.label()
        };
        let percent = progress
            .fraction()
            .map_or_else(String::new, |f| format!("{:.0}%", f * 100.0));
        let fill = div()
            .debug_selector(move || format!("conversion-fill-{trace}"))
            .h_full()
            .bg(t.editor.icon_accent);
        let fill = if let Some(fraction) = progress.fraction() {
            fill.w(relative(fraction)).into_any_element()
        } else if cx.reduce_motion() {
            fill.w(relative(0.25)).into_any_element()
        } else {
            fill.w(relative(0.25))
                .with_animation(
                    "conversion-progress",
                    Animation::new(std::time::Duration::from_secs(1)).repeat(),
                    |bar, delta| bar.ml(relative(delta * 0.75)),
                )
                .into_any_element()
        };
        let size = |label, bytes| {
            div()
                .flex()
                .justify_between()
                .gap_3()
                .child(div().text_color(t.editor.text_muted).child(label))
                .child(
                    div()
                        .font_family(t.mono_font)
                        .child(SharedString::from(format_bytes(bytes))),
                )
        };
        let name = self
            .app
            .trace_chip(trace)
            .map_or_else(|| format!("Trace {trace}"), |chip| chip.name);
        Some(
            div()
                .id(("conversion", trace.index()))
                .debug_selector(move || format!("conversion-{trace}"))
                .flex_none()
                .min_w_0()
                .max_w_full()
                .flex()
                .flex_col()
                .gap_2()
                .when(compact, |view| {
                    view.w_full()
                        .px_3()
                        .py_2()
                        .border_b_1()
                        .border_color(t.border)
                })
                .when(!compact, |view| view.w(t.px(360.0)).px_4())
                .when(compact, |view| {
                    view.child(
                        div()
                            .whitespace_normal()
                            .child(SharedString::from(format!("Converting {name} to VTR…"))),
                    )
                })
                .child(
                    div()
                        .flex()
                        .justify_between()
                        .gap_3()
                        .text_color(t.editor.text_muted)
                        .child(phase)
                        .child(SharedString::from(percent)),
                )
                .child(
                    div()
                        .debug_selector(move || format!("conversion-bar-{trace}"))
                        .w_full()
                        .h(t.px(4.0))
                        .overflow_hidden()
                        .bg(t.border)
                        .child(fill),
                )
                .child(
                    div()
                        .text_size(gpui_kit::px(t.ui_size_small))
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(size("Input file", progress.input_bytes))
                        .child(size("Output VTR", progress.output_bytes)),
                )
                .child(
                    Button::new(SharedString::from(format!("cancel-conversion-{trace}")))
                        .label("Cancel conversion")
                        .ghost()
                        .on_click(cx.listener(move |this, _, window, cx| {
                            let command = if trace.is_a() {
                                Command::CloseTrace
                            } else {
                                Command::RemoveTrace(trace)
                            };
                            this.dispatch(command, Some(window), cx);
                        })),
                )
                .into_any_element(),
        )
    }
}
