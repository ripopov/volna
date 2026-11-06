//! In-editor guide to the saved formula language.

use crate::theme::{Theme, ThemePx};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{
    Sizable,
    button::{Button, ButtonVariants},
};
use gpui_kit::prelude::*;
use gpui_kit::{Entity, IntoElement, Pixels, div, px};

struct Example {
    formula: &'static str,
    explanation: &'static str,
}

struct Section {
    heading: &'static str,
    guidance: &'static str,
    examples: &'static [Example],
}

const SECTIONS: &[Section] = &[
    Section {
        heading: "Start with bindings",
        guidance: "Add recorded or applied waveforms above, then use their aliases in one expression. Rename an alias in the input list if needed. Here a and b are Real; valid, ready, and clk are Boolean.",
        examples: &[
            Example {
                formula: "a - b",
                explanation: "Subtract the latest held values whenever either input changes.",
            },
            Example {
                formula: "valid && ready",
                explanation: "True only while both inputs are true.",
            },
            Example {
                formula: "abs(a - b)",
                explanation: "Keep the magnitude of a difference.",
            },
        ],
    },
    Section {
        heading: "Values and operators",
        guidance: "Use finite decimal numbers such as 2, .5, or 2e-3, and Boolean true or false. Parentheses group expressions. From tightest to loosest: unary + - !; * / %; + -; comparisons; == !=; &&; ||; capture @. Comparisons cannot chain. Unknown inputs produce gaps except where three-valued && or || determines a result.",
        examples: &[
            Example {
                formula: "min(a, b) + max(a, b)",
                explanation: "min and max compare two Real values.",
            },
            Example {
                formula: "a >= 0 && bool(b)",
                explanation: "Comparisons return Boolean; bool converts finite nonzero Real to true.",
            },
            Example {
                formula: "!valid || ready",
                explanation: "Negate and combine Boolean signals with three-valued logic.",
            },
            Example {
                formula: "a % b",
                explanation: "Real remainder; division or remainder by zero produces a gap.",
            },
            Example {
                formula: "100 * real(valid)",
                explanation: "real converts true to 1, false to 0, and unknown to a gap.",
            },
        ],
    },
    Section {
        heading: "Capture at instants",
        guidance: "@ has the lowest precedence. A captured waveform holds its last captured value between triggers. The clock after @ must be one bound Boolean alias; save a Boolean expression as a formula first if needed. A capture at a source event sees the new value. Repeated equal captures remain countable. Arithmetic on a captured waveform discards its capture timestamps; put the arithmetic inside @ when its result needs counting.",
        examples: &[
            Example {
                formula: "a - b @ 1 ns",
                explanation: "Capture on a grid starting at the resolved domain start.",
            },
            Example {
                formula: "a - b @posedge clk",
                explanation: "Capture on known low-to-high clock edges.",
            },
            Example {
                formula: "a @clk",
                explanation: "Capture on every published clock state change, including changes to or from unknown.",
            },
            Example {
                formula: "a - (b @ 1 ns)",
                explanation: "Capture only b before subtracting; this differs from (a - b) @ 1 ns.",
            },
        ],
    },
    Section {
        heading: "Sampled functions",
        guidance: "Steps and windows are exact timeline durations. Omit step to use the visible Default step above. Each function starts its grid at its resolved input-domain start; zoom does not alter it. A temporal child publishes only on its own grid.",
        examples: &[
            Example {
                formula: "moving_avg(a - b, 4 us, step: 2 us)",
                explanation: "Duration-weighted trailing mean over [t − window, t).",
            },
            Example {
                formula: "moving_avg(a, 4 us, warmup: \"available\", unknown: \"skip\")",
                explanation: "Use partial warm-up and divide by known duration during gaps. Defaults are \"full\" and \"gap\".",
            },
            Example {
                formula: "lowpass(a, 2 us, step: 1 us)",
                explanation: "First-order low-pass with time constant tau; restart after an unknown interval.",
            },
            Example {
                formula: "integrate(a, initial: 0, step: 1 us)",
                explanation: "Accumulate held area from the domain start; an unknown interval poisons the later result.",
            },
        ],
    },
    Section {
        heading: "Generate without inputs",
        guidance: "A generator uses the saved document range. Sine needs at least two samples per period. Clock publishes exact edges and has no step. A literal-only waveform also uses the saved range.",
        examples: &[
            Example {
                formula: "sine(period: 20 us, amplitude: 1, offset: 0, phase: 0, step: 1 us)",
                explanation: "Sample a sine; phase is in radians. Only period is required.",
            },
            Example {
                formula: "clock(period: 10 ns, high: 4 ns, phase: 0 ns)",
                explanation: "Generate a Boolean clock; period and high are required.",
            },
        ],
    },
    Section {
        heading: "Measure an interval",
        guidance: "Measurements plot one held value per interval. Each marker interval is the default: it includes the recording edges. Applying saves the boundaries, so markers can then be removed. Whole range plots one value over the recording. Other scopes use explicit tick bounds, a marker snapshot, linked markers, or adjacent pairs in a selected marker set. Intervals are [start, end), so an event at end is excluded. The missing-values control chooses strict gaps or skip-unknown support. Scale a measurement with literals or combine measurements with the same scope; a scalar cannot mix with a waveform or be captured with @.",
        examples: &[
            Example {
                formula: "100 * interval_mean(real(valid))",
                explanation: "Measure duty percentage. Whole range stays global even when markers exist.",
            },
            Example {
                formula: "interval_integral(a)",
                explanation: "Time-weighted area inside each resolved interval.",
            },
            Example {
                formula: "interval_min(a)",
                explanation: "Minimum held Real value with positive duration.",
            },
            Example {
                formula: "interval_max(a)",
                explanation: "Maximum held Real value with positive duration.",
            },
            Example {
                formula: "interval_sum(a - b @posedge clk)",
                explanation: "Capture a - b at each rising edge, then add those values. Equal repeated captures still count; an interval with no captures returns 0.",
            },
        ],
    },
    Section {
        heading: "Syntax and time rules",
        guidance: "Write one expression, with no assignment, semicolon, comment, implicit multiplication, or custom function. Identifiers use ASCII letters, digits, and underscores, starting with a letter or underscore; built-in names are reserved. Positional arguments precede named ones. Duration units are fs, ps, ns, us, ms, and s; a logical timeline accepts whole ticks or its declared unit. Durations must convert exactly to timeline ticks. A waveform result uses the intersection of referenced input domains; unused bindings do not narrow it.",
        examples: &[
            Example {
                formula: "a - b @ 1ns",
                explanation: "Spaces between a number and its unit are optional.",
            },
            Example {
                formula: "real(valid) + a",
                explanation: "Convert Boolean explicitly before Real arithmetic.",
            },
        ],
    },
];

pub(super) fn render(
    t: &Theme,
    height: Pixels,
    editor: &Entity<super::DerivedEditor>,
    query: &str,
) -> impl IntoElement {
    let query = query.trim().to_lowercase();
    let mut content = div().flex().flex_col().gap_3();
    let mut found = false;
    for (index, section) in SECTIONS.iter().enumerate() {
        if !query.is_empty()
            && !section.heading.to_lowercase().contains(&query)
            && !section.guidance.to_lowercase().contains(&query)
            && !section.examples.iter().any(|example| {
                example.formula.to_lowercase().contains(&query)
                    || example.explanation.to_lowercase().contains(&query)
            })
        {
            continue;
        }
        found = true;
        let mut body = div()
            .id(format!("formula-reference-section-{index}"))
            .debug_selector(move || format!("formula-reference-section-{index}"))
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                    .text_color(t.panel.text)
                    .child(section.heading),
            )
            .child(
                div()
                    .text_color(t.panel.text_muted)
                    .whitespace_normal()
                    .child(section.guidance),
            );
        for (example_index, example) in section.examples.iter().enumerate() {
            let owner = editor.downgrade();
            let formula = example.formula;
            body = body.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .p(t.px(10.0))
                    .rounded(t.px(6.0))
                    .border_1()
                    .border_color(t.border)
                    .bg(t.editor.bg)
                    .child(
                        div()
                            .w_full()
                            .overflow_x_scrollbar()
                            .whitespace_nowrap()
                            .font_family(t.mono_font)
                            .text_color(t.panel.text)
                            .bg(t.editor.bg)
                            .p(t.px(5.0))
                            .child(gpui_kit::base::SelectableText::new(
                                format!("formula-example-{index}-{example_index}"),
                                example.formula,
                            )),
                    )
                    .child(
                        div()
                            .text_color(t.panel.text_muted)
                            .whitespace_normal()
                            .child(example.explanation),
                    )
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap_1()
                            .child(
                                Button::new(format!("copy-example-{index}-{example_index}"))
                                    .debug_selector(move || {
                                        format!("copy-example-{index}-{example_index}")
                                    })
                                    .label("Copy")
                                    .ghost()
                                    .xsmall()
                                    .on_click(move |_, _, cx| {
                                        #[cfg(target_family = "wasm")]
                                        {
                                            crate::clipboard::copy(formula);
                                            let _ = cx;
                                        }
                                        #[cfg(not(target_family = "wasm"))]
                                        cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(
                                            formula.to_owned(),
                                        ));
                                    }),
                            )
                            .child(
                                Button::new(format!("use-example-{index}-{example_index}"))
                                    .debug_selector(move || {
                                        format!("use-example-{index}-{example_index}")
                                    })
                                    .label("Use example")
                                    .small()
                                    .on_click(move |_, window, cx| {
                                        drop(owner.update(cx, |view, cx| {
                                            view.use_example(formula, window, cx);
                                        }));
                                    }),
                            ),
                    ),
            );
        }
        content = content.child(body);
    }
    if !found {
        content = content.child(div().text_color(t.panel.text_muted).child(
            "No matching examples. Try a function name, such as moving_avg, or clear the search.",
        ));
    }
    div()
        .id("formula-reference")
        .debug_selector(|| "formula-reference".into())
        .h(height)
        .max_h(height)
        .overflow_hidden()
        .child(
            div()
                .id("formula-reference-scroll")
                .debug_selector(|| "formula-reference-scroll".into())
                .h_full()
                .min_h_0()
                .w_full()
                .overflow_y_scrollbar()
                .p(t.px(8.0))
                .text_size(px(t.ui_size_small))
                .child(content),
        )
}
