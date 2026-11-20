//! Session readings retain their values while the user changes the live interval.

use super::*;

pub(super) fn readout(
    reading: &volna_core::computed::Measurement,
    base: volna_core::wave::timeline::TimeBase<'_>,
) -> String {
    let value = reading
        .value
        .map_or_else(|| "Unavailable".to_owned(), |v| v.to_string());
    use volna_core::computed::MeasurementSupport;
    match reading.support {
        MeasurementSupport::Samples(samples) => format!(
            "{value} · {} of {} samples known",
            samples.known, samples.total
        ),
        MeasurementSupport::Time { known, total } if known < total => {
            let missing = total - known;
            let duration = volna_core::wave::timeline::format_time(missing as f64, base);
            format!("{value} · {duration} unknown")
        }
        MeasurementSupport::Combined { fully_known: false } => {
            format!("{value} · combined coverage unavailable")
        }
        _ => format!("{value} · 100% known"),
    }
}

impl DerivedEditor {
    pub(super) fn pinned_readings(
        &self,
        measurement: bool,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        let owner = self.owner.upgrade()?;
        let ws = owner.read(cx);
        let pins = &ws.app.panels.waves(self.panel)?.pinned_readings;
        if pins.is_empty() {
            return None;
        }
        let time =
            |tick| volna_core::wave::timeline::format_time(tick as f64, ws.app.doc.time_base());
        Some(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child("Pinned readings")
                .children(pins.iter().enumerate().rev().map(|(index, pin)| {
                    let (start, end) = pin.reading.bounds;
                    div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap_1()
                        .child(div().font_family(theme(cx).mono_font).child(format!(
                            "{} = {} · {} – {}",
                            pin.expression,
                            readout(&pin.reading, ws.app.doc.time_base()),
                            time(start),
                            time(end)
                        )))
                        .when(measurement, |el| {
                            el.child(
                                Button::new(("pin-bounds", index))
                                    .xsmall()
                                    .label("Use bounds")
                                    .tooltip(pin.sources.clone())
                                    .on_click(cx.listener(move |view, _, window, cx| {
                                        view.set_scope(
                                            volna_core::computed::MeasurementScope::Explicit {
                                                start,
                                                end,
                                            },
                                            window,
                                            cx,
                                        );
                                    })),
                            )
                        })
                        .child(
                            Button::new(("unpin", index))
                                .xsmall()
                                .ghost()
                                .label("Unpin")
                                .on_click(cx.listener(move |view, _, window, cx| {
                                    view.send(DerivedCommand::Unpin(index), window, cx);
                                })),
                        )
                }))
                .into_any_element(),
        )
    }
}
