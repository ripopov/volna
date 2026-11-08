//! Immutable clipboard export over a captured row view, selection, and column formats.

use super::columns::TransactionColumn;
use super::format::TableFormats;
use super::selection::{RowOrder, RowSelection};
use super::source::Rows;
use crate::data::text::{COPY_BYTES, append_exact};
use crate::data::translator::{Translated, Translators};

pub(super) struct CopyColumn {
    pub source_index: usize,
    pub title: String,
    pub transaction: Option<TransactionColumn>,
}

pub(super) struct CopyRequest {
    pub rows: Rows,
    pub order: RowOrder,
    pub selection: RowSelection,
    pub active: Option<u64>,
    pub columns: Vec<CopyColumn>,
    pub formats: TableFormats,
    pub translators: Translators,
}

impl CopyRequest {
    pub fn run(self) -> anyhow::Result<String> {
        // Each row needs at least one newline, so the byte limit also bounds candidate storage.
        let ranks = self
            .selection
            .visible_ranks(&self.order, self.active, COPY_BYTES)
            .ok_or_else(|| anyhow::anyhow!("Rows exceed the 64 KiB clipboard limit."))?;
        anyhow::ensure!(!ranks.is_empty(), "Select a visible row to copy.");
        let mut text = String::new();
        for (position, column) in self.columns.iter().enumerate() {
            if position > 0 {
                append_exact(&mut text, "\t", COPY_BYTES)?;
            }
            append_tsv_field(&mut text, &column.title)?;
        }
        for rank in ranks {
            let source = self
                .order
                .source_row(rank)
                .and_then(|row| usize::try_from(row).ok())
                .ok_or_else(|| anyhow::anyhow!("Selected row is unavailable"))?;
            append_exact(&mut text, "\n", COPY_BYTES)?;
            for (position, column) in self.columns.iter().enumerate() {
                if position > 0 {
                    append_exact(&mut text, "\t", COPY_BYTES)?;
                }
                let value = self.cell(source, column, COPY_BYTES.saturating_sub(text.len()))?;
                anyhow::ensure!(!value.truncated, "Rows exceed the 64 KiB clipboard limit.");
                append_tsv_field(&mut text, &value.text)?;
            }
        }
        Ok(text)
    }

    fn cell(&self, source: usize, column: &CopyColumn, limit: usize) -> anyhow::Result<Translated> {
        match &self.rows {
            Rows::Generator(generator) => {
                let transaction = generator
                    .transactions()
                    .get(source)
                    .ok_or_else(|| anyhow::anyhow!("Selected row is unavailable"))?;
                let value = column
                    .transaction
                    .as_ref()
                    .expect("transaction column")
                    .value(transaction);
                // Attribute previews reserve space for an ellipsis; export checks the full row budget.
                Ok(self.formats.format_cell(
                    &self.translators,
                    value,
                    column.source_index,
                    COPY_BYTES,
                    None,
                ))
            }
            Rows::Signals {
                histories, axis, ..
            } => {
                let time = axis
                    .as_ref()
                    .map_or_else(|| histories[0].time(source), |axis| axis[source]);
                if column.source_index == 0 {
                    return Ok(Translated::bounded(
                        &time.to_string(),
                        volna_trace::data::ValueKind::Normal,
                        limit,
                    ));
                }
                let history = &histories[column.source_index - 1];
                Ok(self.formats.format_signal(
                    &self.translators,
                    column.source_index,
                    history.as_ref(),
                    history.index_at(time),
                    limit,
                ))
            }
            Rows::None => Err(anyhow::anyhow!("Source is not ready.")),
        }
    }
}

fn append_tsv_field(text: &mut String, value: &str) -> anyhow::Result<()> {
    if !value.contains(['\t', '\n', '\r', '"']) {
        return append_exact(text, value, COPY_BYTES);
    }
    append_exact(text, "\"", COPY_BYTES)?;
    for (index, part) in value.split('"').enumerate() {
        if index > 0 {
            append_exact(text, "\"\"", COPY_BYTES)?;
        }
        append_exact(text, part, COPY_BYTES)?;
    }
    append_exact(text, "\"", COPY_BYTES)
}
