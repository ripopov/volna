//! Opaque transaction fills with distinct borders and readable labels.

use super::{Theme, fallback_text, over, stroke};
use crate::color::Color;
use volna_trace::data::transactions::Status;

#[derive(Clone, Copy, Debug)]
pub struct TransactionStyle {
    pub fill: Color,
    pub edge: Color,
    pub text: Color,
}

impl Theme {
    #[must_use]
    pub fn transaction_style(&self, status: Status) -> TransactionStyle {
        let ink = match status {
            Status::Error => self.editor.error,
            Status::Aborted => self.wave_weak,
            _ => self.wave_signal,
        };
        let fill = over(ink.with_alpha(0.20), self.editor.bg);
        TransactionStyle {
            fill,
            edge: stroke(
                ink.with_alpha(1.0),
                &[
                    self.editor.bg,
                    over(self.wave_row_selected, self.editor.bg),
                    over(self.wave_row_hover, self.editor.bg),
                    fill,
                ],
            ),
            text: fallback_text(self.editor.text, fill),
        }
    }
}
