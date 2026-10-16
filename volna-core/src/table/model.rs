use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::collections::HashMap;
use std::sync::Arc;

use web_time::Instant;

use super::columns::{ColumnSet, TransactionColumn};
use super::format::{self, FormatChoice, TableFormats};
use super::layout::{
    ColumnHeader, HEADER_HEIGHT, MAX_COLUMN_WIDTH, MAX_PREPARED_ROWS, ROW_HEIGHT, RowViewport,
    TableLayout,
};
use super::source::TableSource;
use crate::data::loaded_tracks::LoadedGenerator;
use crate::data::text::{append_exact, push_limited};
use crate::data::translator::{Translated, Translator, Translators};
use crate::document::{Document, TrackLoadState};
use crate::geometry::{MouseButton, Point, Rect};
use crate::nav::{Link, NavState};
use crate::scene::{FontRole, TextMeasure};
use crate::theme::Theme;
use crate::trace::Traced;
use crate::wave::model::PointerEvent;
use volna_trace::data::transactions::{AttributeType, AttributeValue, Transaction, TransactionRef};
use volna_trace::data::value_view::ValueView;
use volna_trace::data::{SignalHistory, SignalRef};
use volna_trace::remote::memory::{MemoryBudget, Reservation};

pub use crate::data::text::{COPY_BYTES, PREVIEW_BYTES};

pub const PANEL_BYTES: u64 = 4 * 1024 * 1024;
const FIT_PREVIEW_BYTES: usize = 16 * 1024;
// Leave half the panel reservation for row metadata and painting, even at the maximum row count.
const ROW_PREVIEW_BYTES: usize = PANEL_BYTES as usize / 2 / MAX_PREPARED_ROWS as usize;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RowIdentity {
    Transaction(TransactionRef),
    SignalTime(u64),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedCell {
    pub text: String,
    pub changed: bool,
    pub missing: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedRow {
    pub ordinal: u64,
    pub identity: RowIdentity,
    pub time: u64,
    pub cells: Vec<PreparedCell>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PreparedWindow {
    pub start: u64,
    pub end: u64,
    pub rows: Vec<PreparedRow>,
    pub bytes: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AccessibleRow {
    pub ordinal: u64,
    pub label: String,
    pub selected: bool,
    pub bounds: Rect,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TableState {
    Loading,
    Ready,
    Empty,
    Failed(String),
    Refused(String),
    Unavailable,
}

enum Rows {
    None,
    Generator(Arc<LoadedGenerator>),
    Signals {
        histories: Vec<Arc<dyn SignalHistory>>,
        axis: Option<Arc<[u64]>>,
        axis_reservation: Option<Arc<Reservation>>,
    },
}

struct SignalAxisBuild {
    histories: Vec<Arc<dyn SignalHistory>>,
    cursor: Vec<usize>,
    heap: BinaryHeap<Reverse<(u64, usize)>>,
    axis: Vec<u64>,
    upper_bytes: u64,
    reservation: Reservation,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum TableDrag {
    Vertical {
        grab: f32,
    },
    Horizontal {
        grab: f32,
    },
    Column {
        source_index: usize,
        start_x: f32,
        width: f32,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum TableCommand {
    ToggleFilters,
    SetFilter {
        source_index: usize,
        filter: Option<super::filter::ColumnFilter>,
    },
    ClearFilters,
    SortColumn(usize),
    NumberFormat(String),
    ColumnFormat {
        source_index: usize,
        translator: Option<String>,
    },
    Scroll(f32),
    Select(u64),
    GoTo(u64),
    First,
    Last,
    Previous,
    Next,
    Page(i8),
    ToggleTransactionColumn(TransactionColumn),
    ShowTransactionColumn(TransactionColumn),
    ToggleSignalColumn(usize),
    MoveColumn {
        source_index: usize,
        before: Option<usize>,
    },
    ResizeColumn {
        source_index: usize,
        width: f32,
    },
    ResetColumns,
    ClearSelection,
    Cancel,
    Retry,
}

pub struct TableModel {
    pub source: TableSource,
    pub state: TableState,
    pub columns: crate::history::Journaled<ColumnSet>,
    pub widths: crate::history::Journaled<Vec<Option<f32>>>,
    pub formats: crate::history::Journaled<TableFormats>,
    translators: Translators,
    pub filters_visible: bool,
    pub filters: Vec<Option<super::filter::ColumnFilter>>,
    filter_unit: crate::trace::Unit,
    pub viewport: RowViewport,
    pub selected: Option<RowIdentity>,
    pub nav: NavState,
    pub window: PreparedWindow,
    pub horizontal: f32,
    pub layout: TableLayout,
    pub request: u64,
    pub sort_requested: Option<super::sort::Sort>,
    pub query_error: Option<String>,
    query_order: Option<super::query::Order>,
    query_job: Option<(u64, super::query::Job)>,
    query_revision: u64,
    reveal_column: Option<usize>,
    drag: Option<TableDrag>,
    rows: Rows,
    axis_build: Option<SignalAxisBuild>,
    signal_results: HashMap<Traced<SignalRef>, Result<Arc<dyn SignalHistory>, String>>,
    attached: bool,
    budget: MemoryBudget,
    panel_reservation: Option<Reservation>,
}

impl TableModel {
    #[must_use]
    pub fn new(
        source: TableSource,
        link: Link,
        budget: MemoryBudget,
        translators: Translators,
    ) -> Self {
        let columns = match &source {
            TableSource::Generator(_) => ColumnSet::transactions_default(),
            TableSource::Signals(signals) => ColumnSet::signals(signals.len()),
        };
        let count = match &source {
            TableSource::Generator(_) => TransactionColumn::ALL.len(),
            TableSource::Signals(signals) => signals.len() + 1,
        };
        let mut nav = NavState::new();
        nav.link = link;
        Self {
            source,
            state: TableState::Loading,
            columns: crate::history::Journaled::new(columns),
            widths: crate::history::Journaled::new(vec![None; count]),
            formats: crate::history::Journaled::new(TableFormats::new(count)),
            translators,
            filters_visible: false,
            filters: vec![None; count],
            filter_unit: crate::trace::Unit {
                timescale: -9,
                name: None,
            },
            viewport: RowViewport::default(),
            selected: None,
            nav,
            window: PreparedWindow::default(),
            horizontal: 0.0,
            layout: TableLayout::default(),
            request: 0,
            sort_requested: None,
            query_error: None,
            query_order: None,
            query_job: None,
            query_revision: 0,
            reveal_column: None,
            drag: None,
            rows: Rows::None,
            axis_build: None,
            signal_results: HashMap::new(),
            attached: false,
            budget,
            panel_reservation: None,
        }
    }

    #[must_use]
    pub fn clone_view(&self) -> Self {
        let mut clone = Self::new(
            self.source.clone(),
            self.nav.link,
            self.budget.clone(),
            self.translators.clone(),
        );
        clone.columns = self.columns.clone();
        clone.widths = self.widths.clone();
        clone.formats = self.formats.clone();
        clone.sort_requested = self.sort_requested;
        clone.filters.clone_from(&self.filters);
        clone.filters_visible = self.filters_visible;
        clone
    }

    pub fn attach(
        &mut self,
        doc: &mut Document,
        resident: &crate::wave::model::Resident,
    ) -> anyhow::Result<()> {
        if self.attached {
            return Ok(());
        }
        if self.panel_reservation.is_none() {
            match self.budget.reserve(PANEL_BYTES) {
                Ok(reservation) => {
                    self.panel_reservation = Some(reservation);
                    if matches!(self.state, TableState::Refused(_)) {
                        self.state = TableState::Loading;
                    }
                }
                Err(error) => {
                    self.state = TableState::Refused(format!(
                        "Table needs {PANEL_BYTES} bytes; admission failed: {error}"
                    ));
                }
            }
        }
        if matches!(self.state, TableState::Refused(_)) {
            return Ok(());
        }
        self.attached = true;
        self.nav.reset(Some(doc.limits()));
        match &mut self.source {
            TableSource::Generator(source) => {
                let Some(track) = source.track() else {
                    self.state = TableState::Unavailable;
                    return Ok(());
                };
                doc.retain_track(track)?;
                self.refresh(doc);
            }
            TableSource::Signals(sources) => {
                let unresolved = sources.iter_mut().any(|source| {
                    !doc.hierarchy(source.trace)
                        .is_some_and(|hierarchy| source.resolve(hierarchy))
                });
                if unresolved {
                    self.state = TableState::Unavailable;
                    return Ok(());
                }
                for signal in sources.iter().filter_map(|source| source.signal) {
                    if let Some(history) = resident.get(&signal) {
                        self.signal_results.insert(signal, Ok(history.clone()));
                    } else {
                        doc.request_signal(signal);
                    }
                }
                self.refresh(doc);
            }
        }
        Ok(())
    }

    pub fn detach(&mut self, doc: &mut Document) {
        self.reset_query();
        if !self.attached {
            return;
        }
        if let TableSource::Generator(source) = &self.source
            && let Some(track) = source.track()
        {
            doc.release_track(track);
        }
        self.attached = false;
        self.rows = Rows::None;
        self.axis_build = None;
        self.signal_results.clear();
        self.window = PreparedWindow::default();
        self.request = self.request.wrapping_add(1);
    }

    pub(crate) fn park(&mut self, doc: &mut Document) {
        self.detach(doc);
        self.panel_reservation = None;
    }

    pub fn signal_demand(&self) -> Box<dyn Iterator<Item = Traced<SignalRef>> + '_> {
        if !self.attached {
            return Box::new(std::iter::empty());
        }
        match &self.source {
            TableSource::Signals(sources) => Box::new(sources.iter().filter_map(|s| s.signal)),
            TableSource::Generator(_) => Box::new(std::iter::empty()),
        }
    }

    #[must_use]
    pub fn histories(&self) -> Vec<(Traced<SignalRef>, Arc<dyn SignalHistory>)> {
        self.signal_results
            .iter()
            .filter_map(|(&signal, result)| result.as_ref().ok().map(|h| (signal, h.clone())))
            .collect()
    }

    pub fn finish_signal(
        &mut self,
        signal: Traced<SignalRef>,
        result: Result<Arc<dyn SignalHistory>, String>,
    ) {
        if self.attached && self.signal_demand().any(|wanted| wanted == signal) {
            self.signal_results.insert(signal, result);
        }
    }

    pub fn refresh(&mut self, doc: &Document) {
        let base = doc.time_base();
        let unit = crate::trace::Unit {
            timescale: base.timescale,
            name: base.unit.map(str::to_owned),
        };
        if self.filter_unit != unit {
            self.filter_unit = unit;
            self.query_changed();
        }
        if matches!(self.state, TableState::Refused(_)) || !self.attached {
            return;
        }
        match &self.source {
            TableSource::Generator(source) => {
                let Some(track) = source.track() else {
                    self.state = TableState::Unavailable;
                    return;
                };
                match doc.track(track) {
                    Some(TrackLoadState::Loading) => self.state = TableState::Loading,
                    Some(TrackLoadState::Failed(error)) => {
                        self.state = TableState::Failed(error.clone());
                    }
                    Some(TrackLoadState::Ready(loaded)) => {
                        let Some(generator) = loaded
                            .generators
                            .iter()
                            .find(|g| g.generator() == track.item)
                            .cloned()
                        else {
                            self.state = TableState::Unavailable;
                            return;
                        };
                        if let Rows::Generator(previous) = &self.rows {
                            if Arc::ptr_eq(previous, &generator) {
                                return;
                            }
                            self.reset_query();
                        }
                        let empty = generator.transactions().is_empty();
                        self.rows = Rows::Generator(generator);
                        self.state = if empty {
                            TableState::Empty
                        } else {
                            TableState::Ready
                        };
                        self.invalidate_window();
                    }
                    None => self.state = TableState::Unavailable,
                }
            }
            TableSource::Signals(sources) => {
                if self.axis_build.is_some() {
                    self.state = TableState::Loading;
                    return;
                }
                if matches!(self.rows, Rows::Signals { .. })
                    && matches!(self.state, TableState::Ready | TableState::Empty)
                {
                    return;
                }
                let mut histories = Vec::with_capacity(sources.len());
                for source in sources {
                    let Some(signal) = source.signal else {
                        self.state = TableState::Unavailable;
                        return;
                    };
                    match self.signal_results.get(&signal) {
                        Some(Ok(history)) => histories.push(history.clone()),
                        Some(Err(error)) => {
                            self.state = TableState::Failed(error.clone());
                            return;
                        }
                        None => {
                            self.state = TableState::Loading;
                            return;
                        }
                    }
                }
                let started = self.begin_signal_rows(histories).and_then(|ready| {
                    if ready.is_none() {
                        self.advance_signal_axis(4096)
                    } else {
                        Ok(ready)
                    }
                });
                match started {
                    Ok(Some(empty)) => {
                        self.state = if empty {
                            TableState::Empty
                        } else {
                            TableState::Ready
                        }
                    }
                    Ok(None) => self.state = TableState::Loading,
                    Err(error) => self.state = TableState::Refused(format!("{error:#}")),
                }
                self.invalidate_window();
            }
        }
    }

    #[cfg(test)]
    fn build_signal_rows(
        &mut self,
        histories: Vec<Arc<dyn SignalHistory>>,
    ) -> anyhow::Result<bool> {
        if let Some(empty) = self.begin_signal_rows(histories)? {
            return Ok(empty);
        }
        loop {
            if let Some(empty) = self.advance_signal_axis(usize::MAX)? {
                return Ok(empty);
            }
        }
    }

    fn begin_signal_rows(
        &mut self,
        histories: Vec<Arc<dyn SignalHistory>>,
    ) -> anyhow::Result<Option<bool>> {
        if histories.len() <= 1 {
            let empty = histories.first().is_none_or(|h| h.is_empty());
            self.rows = Rows::Signals {
                histories,
                axis: None,
                axis_reservation: None,
            };
            return Ok(Some(empty));
        }
        let upper = histories.iter().try_fold(0u64, |sum, history| {
            sum.checked_add(history.len() as u64)
                .ok_or_else(|| anyhow::anyhow!("signal axis size overflow"))
        })?;
        let upper_bytes = upper
            .checked_mul(8)
            .ok_or_else(|| anyhow::anyhow!("signal axis size overflow"))?;
        let reservation = self.budget.reserve(upper_bytes).map_err(|error| {
            anyhow::anyhow!(
                "Signal axis needs at most {upper_bytes} bytes before merge; admission failed: {error}"
            )
        })?;
        let cursor = vec![0usize; histories.len()];
        let mut heap = BinaryHeap::new();
        for (index, history) in histories.iter().enumerate() {
            if !history.is_empty() {
                heap.push(Reverse((history.time(0), index)));
            }
        }
        let mut axis = Vec::new();
        axis.try_reserve_exact(usize::try_from(upper)?)?;
        self.axis_build = Some(SignalAxisBuild {
            histories,
            cursor,
            heap,
            axis,
            upper_bytes,
            reservation,
        });
        Ok(None)
    }

    fn advance_signal_axis(&mut self, limit: usize) -> anyhow::Result<Option<bool>> {
        let Some(build) = self.axis_build.as_mut() else {
            return Ok(None);
        };
        for _ in 0..limit {
            let Some(Reverse((next, signal))) = build.heap.pop() else {
                let SignalAxisBuild {
                    histories,
                    mut axis,
                    upper_bytes,
                    mut reservation,
                    ..
                } = self.axis_build.take().expect("active axis build");
                let actual = (axis.len() as u64)
                    .checked_mul(8)
                    .ok_or_else(|| anyhow::anyhow!("signal axis size overflow"))?;
                reservation.shrink(upper_bytes - actual)?;
                axis.shrink_to_fit();
                let empty = axis.is_empty();
                self.rows = Rows::Signals {
                    histories,
                    axis: Some(axis.into()),
                    axis_reservation: Some(Arc::new(reservation)),
                };
                return Ok(Some(empty));
            };
            if build.axis.last().copied() != Some(next) {
                build.axis.push(next);
            }
            let history = &build.histories[signal];
            let index = &mut build.cursor[signal];
            while *index < history.len() && history.time(*index) == next {
                *index += 1;
            }
            if *index < history.len() {
                build.heap.push(Reverse((history.time(*index), signal)));
            }
        }
        Ok(None)
    }

    pub fn tick(&mut self, now: Instant) -> bool {
        let changed = self.nav.tick(now) | self.poll_query();
        if self.axis_build.is_none() {
            return changed || self.querying();
        }
        match self.advance_signal_axis(4096) {
            Ok(Some(empty)) => {
                self.state = if empty {
                    TableState::Empty
                } else {
                    TableState::Ready
                };
                self.invalidate_window();
                true
            }
            Ok(None) => true,
            Err(error) => {
                self.axis_build = None;
                self.state = TableState::Refused(format!("{error:#}"));
                true
            }
        }
    }

    #[must_use]
    pub fn is_animating(&self) -> bool {
        self.axis_build.is_some() || self.nav.is_animating() || self.querying()
    }

    fn invalidate_window(&mut self) {
        self.request = self.request.wrapping_add(1);
        self.window = PreparedWindow::default();
    }

    fn source_index(&self, ordinal: u64) -> Option<usize> {
        if let Some(order) = &self.query_order {
            return usize::try_from(order.source_row(ordinal)?).ok();
        }
        usize::try_from(ordinal).ok()
    }

    #[must_use]
    pub fn applied_sort(&self) -> Option<super::sort::Sort> {
        self.query_order.as_ref().and_then(|o| o.sort)
    }

    #[must_use]
    pub fn querying(&self) -> bool {
        if self.query_job.is_some() {
            return true;
        }
        self.query_error.is_none()
            && (self.sort_requested != self.applied_sort() || !self.filters_applied())
            && matches!(self.state, TableState::Ready | TableState::Empty)
    }

    fn reset_query(&mut self) {
        self.query_revision = self.query_revision.wrapping_add(1);
        self.sort_requested = None;
        self.query_error = None;
        if let Some((_, job)) = &self.query_job {
            job.cancel();
        }
        if let Some(order) = self.query_order.take() {
            super::query::retire(order);
        }
    }

    #[must_use]
    pub fn active_filter_count(&self) -> usize {
        self.filters.iter().filter(|f| f.is_some()).count()
    }

    #[must_use]
    pub fn applied_filter(&self, column: usize) -> Option<&super::filter::ColumnFilter> {
        self.query_order.as_ref()?.filters.get(column)?.as_ref()
    }

    fn filters_applied(&self) -> bool {
        self.query_order.as_ref().map_or_else(
            || self.active_filter_count() == 0,
            |o| {
                super::filter::same_filters(&o.filters, &self.filters)
                    && (self.active_filter_count() == 0 || o.unit == self.filter_unit)
            },
        )
    }

    fn query_changed(&mut self) {
        self.query_revision = self.query_revision.wrapping_add(1);
        self.query_error = None;
        if let Some((_, job)) = &self.query_job {
            job.cancel();
        }
        if self.sort_requested.is_none() && self.active_filter_count() == 0 {
            if let Some(order) = self.query_order.take() {
                super::query::retire(order);
            }
            self.invalidate_window();
        }
    }

    #[must_use]
    pub fn default_filter_mode(&self, column: usize) -> super::filter::FilterMode {
        use super::filter::FilterMode;
        match self.transaction_column(column) {
            Some(
                TransactionColumn::Begin
                | TransactionColumn::End
                | TransactionColumn::Duration
                | TransactionColumn::Id,
            ) => FilterMode::Expression,
            Some(TransactionColumn::Attribute(key))
                if self.loaded_generator().is_some_and(|g| {
                    g.attribute_catalogue().iter().any(|a| {
                        &a.key == key
                            && matches!(
                                a.value_type,
                                AttributeType::I64
                                    | AttributeType::U64
                                    | AttributeType::F64
                                    | AttributeType::Pointer
                                    | AttributeType::Fixed
                                    | AttributeType::UFixed
                                    | AttributeType::Time
                            )
                    })
                }) =>
            {
                FilterMode::Expression
            }
            None if column == 0 => FilterMode::Expression,
            _ => FilterMode::Contains,
        }
    }

    #[must_use]
    pub fn can_sort_column(&self, column: usize) -> bool {
        if column >= self.widths.len()
            || !matches!(self.state, TableState::Ready | TableState::Empty)
        {
            return false;
        }
        let clearing = self
            .sort_requested
            .is_some_and(|sort| sort.column == column && sort.descending);
        let filtered = self.active_filter_count() != 0;
        if clearing && !filtered {
            return true;
        }
        if self.query_error.is_some() {
            return false;
        }
        let matches = if self.filters_applied() {
            self.len()
        } else {
            self.source_len()
        };
        let sort_bytes = if clearing {
            Some(0)
        } else {
            super::query::index_bytes(self.source_len(), matches)
        };
        let bitmap_bytes = if filtered {
            super::filter::index::required_bytes(self.source_len())
        } else {
            Some(0)
        };
        sort_bytes.zip(bitmap_bytes).is_some_and(|(sort, bitmap)| {
            sort <= self.budget.object_limit()
                && bitmap <= self.budget.object_limit()
                && sort.checked_add(bitmap).is_some_and(|bytes| {
                    bytes <= self.budget.limit().saturating_sub(self.budget.used())
                })
        })
    }

    fn cycle_sort(&mut self, column: usize) -> bool {
        if !self.can_sort_column(column) {
            return false;
        }
        self.sort_requested = match self.sort_requested {
            Some(sort) if sort.column == column && sort.descending => None,
            Some(sort) if sort.column == column => Some(super::sort::Sort {
                column,
                descending: true,
            }),
            _ => Some(super::sort::Sort {
                column,
                descending: false,
            }),
        };
        self.query_changed();
        true
    }

    fn poll_query(&mut self) -> bool {
        let mut changed = false;
        if let Some((revision, result)) = self
            .query_job
            .as_mut()
            .and_then(|(r, job)| job.poll().map(|result| (*r, result)))
        {
            self.query_job = None;
            changed = true;
            if revision == self.query_revision {
                match result {
                    Ok(order) => {
                        if let Some(old) = self.query_order.replace(order) {
                            super::query::retire(old);
                        }
                        if let Some(rank) = self.selected_ordinal() {
                            self.viewport.reveal(
                                rank,
                                self.len(),
                                self.layout.body.height(),
                                self.row_px(),
                            );
                        }
                        self.invalidate_window();
                    }
                    Err(error) => self.query_error = Some(format!("{error:#}")),
                }
            } else if let Ok(order) = result {
                super::query::retire(order);
            }
        }
        if self.query_job.is_none() && self.querying() {
            let sort = self.sort_requested;
            let source = match &self.rows {
                Rows::Generator(g) => super::sort::Source::Generator(
                    g.clone(),
                    self.transaction_column(sort.map_or(0, |s| s.column))
                        .expect("query column")
                        .clone(),
                ),
                Rows::Signals {
                    histories,
                    axis,
                    axis_reservation,
                } => super::sort::Source::Signals {
                    histories: histories.clone(),
                    axis: axis.clone(),
                    _reservation: axis_reservation.clone(),
                },
                Rows::None => return changed,
            };
            let columns = self
                .filters
                .iter()
                .enumerate()
                .filter_map(|(i, f)| {
                    let column = match &self.source {
                        TableSource::Generator(_) => {
                            super::filter::Column::Transaction(self.transaction_column(i)?.clone())
                        }
                        TableSource::Signals(_) if i == 0 => super::filter::Column::Time,
                        TableSource::Signals(_) => super::filter::Column::Signal {
                            index: i - 1,
                            name: self.column_title(i).to_owned(),
                        },
                    };
                    Some((column, f.as_ref()?.clone()))
                })
                .collect();
            let query = super::query::Query {
                sort,
                filters: self.filters.clone(),
                columns,
                unit: self.filter_unit.clone(),
            };
            match super::query::Job::start(source, self.source_len(), query, self.budget.clone()) {
                Ok(job) => self.query_job = Some((self.query_revision, job)),
                Err(error) => self.query_error = Some(format!("{error:#}")),
            }
            changed = true;
        }
        changed
    }

    #[must_use]
    pub fn len(&self) -> u64 {
        self.query_order
            .as_ref()
            .map_or_else(|| self.source_len(), super::query::Order::len)
    }

    #[must_use]
    pub fn source_len(&self) -> u64 {
        match &self.rows {
            Rows::None => 0,
            Rows::Generator(generator) => generator.transactions().len() as u64,
            Rows::Signals {
                histories, axis, ..
            } => axis.as_ref().map_or_else(
                || histories.first().map_or(0, |h| h.len() as u64),
                |a| a.len() as u64,
            ),
        }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[must_use]
    pub fn status(&self) -> String {
        if let Some(error) = &self.query_error {
            return format!("Query failed: {error} · previous results retained");
        }
        if self.querying() && !self.filters_applied() {
            return format!(
                "{} of {} rows · filtering… previous results shown",
                self.len(),
                self.source_len()
            );
        }
        if self.querying() {
            return format!("{} rows · sorting… previous order shown", self.len());
        }
        match &self.state {
            TableState::Loading => "Loading source…".into(),
            TableState::Ready
                if self
                    .query_order
                    .as_ref()
                    .is_some_and(|o| o.filters.iter().any(Option::is_some)) =>
            {
                format!("{} of {} rows", self.len(), self.source_len())
            }
            TableState::Ready => format!("{} rows", self.len()),
            TableState::Empty => "Source is empty".into(),
            TableState::Failed(error) => format!("Load failed: {error}"),
            TableState::Refused(error) => format!("Load refused: {error}"),
            TableState::Unavailable => "Source unavailable".into(),
        }
    }

    #[must_use]
    pub fn row_time(&self, ordinal: u64) -> Option<u64> {
        let index = self.source_index(ordinal)?;
        match &self.rows {
            Rows::Generator(generator) => generator.transactions().get(index).map(|tx| tx.begin),
            Rows::Signals {
                histories, axis, ..
            } => axis
                .as_ref()
                .and_then(|axis| axis.get(index).copied())
                .or_else(|| {
                    histories
                        .first()
                        .filter(|h| axis.is_none() && index < h.len())
                        .map(|h| h.time(index))
                }),
            Rows::None => None,
        }
    }

    pub(crate) fn retime(&mut self, by: crate::trace::Rescale) {
        if let Some(RowIdentity::SignalTime(time)) = &mut self.selected {
            crate::trace::Retime::retime(time, by);
        }
    }

    fn ordinal_of(&self, identity: &RowIdentity) -> Option<u64> {
        let source_row = match (&self.rows, identity) {
            (Rows::Generator(generator), RowIdentity::Transaction(id)) => {
                generator.transaction_ordinal(*id).map(|x| x as u64)
            }
            (
                Rows::Signals {
                    histories, axis, ..
                },
                RowIdentity::SignalTime(time),
            ) => axis.as_ref().map_or_else(
                || {
                    histories.first().and_then(|h| {
                        let i = h.index_at(*time)?;
                        (h.time(i) == *time).then_some(i as u64)
                    })
                },
                |a| a.binary_search(time).ok().map(|x| x as u64),
            ),
            _ => None,
        }?;
        if let Some(order) = &self.query_order {
            return order.rank(source_row);
        }
        Some(source_row)
    }

    #[must_use]
    pub fn selected_ordinal(&self) -> Option<u64> {
        self.selected
            .as_ref()
            .and_then(|identity| self.ordinal_of(identity))
    }

    pub fn select(
        &mut self,
        doc: &mut Document,
        panel: crate::panels::PanelId,
        ordinal: u64,
        now: Instant,
    ) -> bool {
        let Some((identity, time)) = self.identity_and_time(ordinal) else {
            return false;
        };
        if let (RowIdentity::Transaction(id), Some(track)) =
            (&identity, self.source.generator_track())
        {
            doc.select(Some(crate::document::TxSelection {
                track,
                id: *id,
                origin: panel,
            }));
        }
        self.selected = Some(identity);
        self.viewport.reveal(
            ordinal,
            self.len(),
            self.layout.body.height(),
            self.row_px(),
        );
        self.nav.set_cursor(doc, Some(time));
        self.nav.reveal_cursor(doc, now);
        true
    }

    pub fn follow_selection(&mut self, doc: &Document) -> bool {
        let (Rows::Generator(_), Some(track)) = (&self.rows, self.source.generator_track()) else {
            return false;
        };
        let wanted = match doc.selection() {
            Some(selection) if selection.track == track => {
                Some(RowIdentity::Transaction(selection.id))
            }
            Some(_) => return false,
            None => None,
        };
        let changed = self.selected != wanted;
        self.selected = wanted;
        changed
    }

    fn row_px(&self) -> f32 {
        if self.layout.row_height > 0.0 {
            self.layout.row_height
        } else {
            ROW_HEIGHT
        }
    }

    fn identity_and_time(&self, ordinal: u64) -> Option<(RowIdentity, u64)> {
        let index = self.source_index(ordinal)?;
        match &self.rows {
            Rows::Generator(generator) => {
                let tx = generator.transactions().get(index)?;
                Some((RowIdentity::Transaction(tx.id), tx.begin))
            }
            Rows::Signals { .. } => {
                let time = self.row_time(ordinal)?;
                Some((RowIdentity::SignalTime(time), time))
            }
            Rows::None => None,
        }
    }

    pub fn command(
        &mut self,
        doc: &mut Document,
        panel: crate::panels::PanelId,
        command: TableCommand,
        now: Instant,
    ) -> bool {
        match command {
            TableCommand::ToggleFilters => {
                self.filters_visible = !self.filters_visible;
                true
            }
            TableCommand::SetFilter {
                source_index,
                filter,
            } => {
                if source_index >= self.filters.len() {
                    return false;
                }
                if filter
                    .as_ref()
                    .is_some_and(|f| f.query.len() > super::filter::MAX_QUERY_BYTES)
                {
                    self.query_error = Some(format!(
                        "Filter exceeds {} bytes",
                        super::filter::MAX_QUERY_BYTES
                    ));
                    return true;
                }
                let filter = filter.filter(super::filter::ColumnFilter::active);
                if self.filters[source_index] == filter && self.query_error.is_none() {
                    return false;
                }
                self.filters[source_index] = filter;
                self.query_changed();
                true
            }
            TableCommand::ClearFilters => {
                if self.active_filter_count() == 0 {
                    return false;
                }
                self.filters.fill(None);
                self.query_changed();
                true
            }
            TableCommand::SortColumn(column) => self.cycle_sort(column),
            TableCommand::NumberFormat(translator) => {
                if !format::number_translators(&self.translators)
                    .iter()
                    .any(|t| t.id() == translator)
                {
                    return false;
                }
                let changed = self.formats.update(|f| f.numbers = translator);
                if changed {
                    self.invalidate_window();
                }
                changed
            }
            TableCommand::ColumnFormat {
                source_index,
                translator,
            } => {
                let Some(menu) = self.column_formats(source_index) else {
                    return false;
                };
                if !menu.iter().any(|c| c.translator == translator) {
                    return false;
                }
                let changed = self
                    .formats
                    .update(|f| f.columns[source_index] = translator);
                if changed {
                    self.invalidate_window();
                }
                changed
            }
            TableCommand::Scroll(pixels) => {
                self.viewport
                    .scroll(pixels, self.len(), self.layout.body.height(), self.row_px());
                self.prepare_visible();
                true
            }
            TableCommand::Select(row) | TableCommand::GoTo(row) => {
                self.select(doc, panel, row, now)
            }
            TableCommand::First => self.select(doc, panel, 0, now),
            TableCommand::Last => self
                .len()
                .checked_sub(1)
                .is_some_and(|row| self.select(doc, panel, row, now)),
            TableCommand::Previous => self
                .selected_ordinal()
                .and_then(|r| r.checked_sub(1))
                .is_some_and(|r| self.select(doc, panel, r, now)),
            TableCommand::Next => self
                .selected_ordinal()
                .map_or(Some(0), |r| r.checked_add(1))
                .filter(|&r| r < self.len())
                .is_some_and(|r| self.select(doc, panel, r, now)),
            TableCommand::Page(direction) => {
                let page = RowViewport::full_rows(self.layout.body.height(), self.row_px());
                let current = self.selected_ordinal().unwrap_or(self.viewport.top);
                let target = if direction < 0 {
                    current.saturating_sub(page)
                } else {
                    current
                        .saturating_add(page)
                        .min(self.len().saturating_sub(1))
                };
                self.select(doc, panel, target, now)
            }
            TableCommand::ShowTransactionColumn(column) => {
                let column = column.canonical();
                if !self.columns.transaction_visible(column.clone()) {
                    self.edit_columns(|columns| columns.toggle_transaction(column.clone()));
                }
                if let ColumnSet::Transactions(columns) = self.columns.get() {
                    self.reveal_column = columns
                        .iter()
                        .find(|(_, c, _)| *c == &column)
                        .map(|(i, _, _)| i);
                }
                self.reveal_column.is_some()
            }
            TableCommand::ToggleTransactionColumn(column) => {
                let column = column.canonical();
                let showing = matches!(column, TransactionColumn::Attribute(_))
                    && !self.columns.transaction_visible(column.clone());
                let changed =
                    self.edit_columns(|columns| columns.toggle_transaction(column.clone()));
                if showing
                    && changed
                    && let ColumnSet::Transactions(columns) = self.columns.get()
                {
                    self.reveal_column = columns
                        .iter()
                        .find(|(_, c, _)| *c == &column)
                        .map(|(i, _, _)| i);
                }
                changed
            }
            TableCommand::MoveColumn {
                source_index,
                before,
            } => self.edit_columns(|columns| columns.move_transaction(source_index, before)),
            TableCommand::ToggleSignalColumn(index) => {
                self.edit_columns(|columns| columns.toggle_signal(index))
            }
            TableCommand::ResizeColumn {
                source_index,
                width,
            } => self.resize_column(source_index, width),
            TableCommand::ResetColumns => {
                let columns = self.edit_columns(|columns| {
                    columns.reset();
                    true
                });
                let widths = self.widths.update(|widths| widths.fill(None));
                columns || widths
            }
            TableCommand::ClearSelection => {
                let changed = self.selected.take().is_some();
                if changed && doc.selection().is_some_and(|s| s.origin == panel) {
                    doc.select(None);
                }
                changed
            }
            TableCommand::Cancel => {
                if self.querying() {
                    self.sort_requested = self.applied_sort();
                    self.filters = self
                        .query_order
                        .as_ref()
                        .map_or_else(|| vec![None; self.filters.len()], |o| o.filters.clone());
                    self.filters.resize(self.columns.len(), None);
                    self.query_revision = self.query_revision.wrapping_add(1);
                    if let Some((_, job)) = &self.query_job {
                        job.cancel();
                    }
                    self.query_error = None;
                    return true;
                }
                self.detach(doc);
                self.state = TableState::Failed("Cancelled".into());
                true
            }
            TableCommand::Retry => {
                if self.query_error.is_some() {
                    self.query_error = None;
                    true
                } else {
                    self.retry(doc)
                }
            }
        }
    }

    #[must_use]
    pub fn transaction_column(&self, index: usize) -> Option<&TransactionColumn> {
        match self.columns.get() {
            ColumnSet::Transactions(columns) => columns.column(index),
            ColumnSet::Signals { .. } => None,
        }
    }

    #[must_use]
    pub fn loaded_generator(&self) -> Option<&Arc<LoadedGenerator>> {
        match &self.rows {
            Rows::Generator(generator) => Some(generator),
            _ => None,
        }
    }

    #[must_use]
    pub fn number_format(&self) -> &'static str {
        self.translators
            .get(&self.formats.numbers)
            .expect("number translator")
            .badge()
    }

    #[must_use]
    pub fn number_formats(&self) -> Vec<FormatChoice> {
        format::number_translators(&self.translators)
            .iter()
            .map(|t| FormatChoice {
                translator: Some(t.id().into()),
                label: t.name().into(),
                preview: String::new(),
                checked: t.id() == self.formats.numbers,
            })
            .collect()
    }

    fn column_shape(&self, column: usize) -> Option<volna_trace::data::SignalShape> {
        match &self.rows {
            Rows::Signals { histories, .. } => column
                .checked_sub(1)
                .and_then(|i| histories.get(i))
                .map(|h| h.shape()),
            Rows::Generator(generator) => match self.transaction_column(column)? {
                TransactionColumn::Id => Some(volna_trace::data::SignalShape::Vector { width: 64 }),
                TransactionColumn::Attribute(key) => {
                    let entry = generator
                        .attribute_catalogue()
                        .iter()
                        .find(|a| &a.key == key)?;
                    if entry.value_type == AttributeType::Mixed {
                        return None;
                    }
                    let tx = generator.transactions().get(entry.example_row)?;
                    format::attribute_shape(&tx.attributes.iter().find(|a| &a.key == key)?.value)
                }
                _ => None,
            },
            Rows::None => None,
        }
    }

    fn column_translators(&self, column: usize) -> Vec<std::sync::Arc<dyn Translator>> {
        self.column_shape(column).map_or_else(Vec::new, |shape| {
            self.translators
                .applicable(shape)
                .into_iter()
                .filter(|t| {
                    matches!(self.source, TableSource::Signals(_))
                        || t.numeric_kind() != Some(crate::data::NumericKind::Float)
                })
                .collect()
        })
    }

    #[must_use]
    pub fn column_translator(&self, column: usize) -> Option<Arc<dyn Translator>> {
        (self.column_translators(column).len() > 1).then(|| {
            self.formats.translator(
                &self.translators,
                column,
                self.column_shape(column).expect("column shape"),
            )
        })
    }

    #[must_use]
    pub fn column_formats(&self, column: usize) -> Option<Vec<FormatChoice>> {
        let current = self.column_translator(column)?;
        let shape = self.column_shape(column)?;
        let inherited = self.formats.columns.get(column)?.is_none();
        let default = self.formats.default_translator(&self.translators, shape);
        let mut choices = vec![FormatChoice {
            translator: None,
            label: format!("Use table default · {}", default.name()),
            preview: String::new(),
            checked: inherited,
        }];
        for translator in self.column_translators(column) {
            let preview = match &self.rows {
                Rows::Generator(generator) => {
                    let example = match self.transaction_column(column) {
                        Some(TransactionColumn::Attribute(key)) => generator
                            .attribute_catalogue()
                            .iter()
                            .find(|a| &a.key == key)
                            .map(|a| a.example_row),
                        _ => self
                            .selected_ordinal()
                            .and_then(|i| self.source_index(i))
                            .or(Some(0)),
                    };
                    example
                        .and_then(|i| generator.transactions().get(i))
                        .map(|tx| self.transaction_cell(tx, column, 48, Some(translator.as_ref())))
                }
                Rows::Signals { histories, .. } => {
                    let history = &histories[column - 1];
                    let time = self
                        .selected_ordinal()
                        .and_then(|i| self.row_time(i))
                        .or_else(|| self.row_time(self.viewport.top));
                    time.and_then(|t| history.index_at(t)).map(|i| {
                        format::translate(translator.as_ref(), &history.value_view(Some(i)), 48)
                            .text
                    })
                }
                Rows::None => None,
            }
            .unwrap_or_default();
            choices.push(FormatChoice {
                checked: !inherited && translator.id() == current.id(),
                translator: Some(translator.id().into()),
                label: translator.name().into(),
                preview,
            });
        }
        Some(choices)
    }

    fn edit_columns(&mut self, f: impl FnOnce(&mut ColumnSet) -> bool) -> bool {
        let mut columns = self.columns.get().clone();
        if !f(&mut columns) || !self.columns.set(columns) {
            return false;
        }
        let count = self.columns.len();
        self.widths.update(|widths| widths.resize(count, None));
        self.formats.update(|f| f.columns.resize(count, None));
        if self.filters.len() != count {
            let removed = self
                .filters
                .get(count..)
                .is_some_and(|tail| tail.iter().any(Option::is_some));
            self.filters.resize(count, None);
            if removed {
                self.query_changed();
            }
        }
        if self
            .sort_requested
            .into_iter()
            .chain(self.applied_sort())
            .any(|s| s.column >= count)
        {
            self.reset_query();
        }
        self.invalidate_window();
        true
    }

    fn resize_column(&mut self, source_index: usize, width: f32) -> bool {
        if source_index >= self.widths.len() || !width.is_finite() {
            return false;
        }
        let minimum = ColumnHeader::minimum_width(self.column_translator(source_index).is_some());
        self.widths.update(|widths| {
            widths[source_index] = Some(width.clamp(minimum, MAX_COLUMN_WIDTH));
        })
    }

    #[must_use]
    pub fn resizing_column(&self) -> Option<usize> {
        match self.drag {
            Some(TableDrag::Column { source_index, .. }) => Some(source_index),
            _ => None,
        }
    }

    fn column_edge_at(&self, position: Point) -> Option<&super::layout::ColumnRect> {
        if !self.layout.header.contains(position)
            && !self.layout.filter_row.contains(position)
            && !self.layout.body.contains(position)
        {
            return None;
        }
        self.layout
            .columns
            .iter()
            .find(|column| column.resize.contains(position))
    }

    pub fn column_fit_width(
        &self,
        source_index: usize,
        theme: &Theme,
        measure: &mut dyn TextMeasure,
    ) -> Option<f32> {
        if source_index >= self.widths.len() {
            return None;
        }
        let has_format = self.column_translator(source_index).is_some();
        let mut width = measure.text_width(
            self.column_title(source_index),
            FontRole::Mono,
            theme.mono_size,
        ) + if has_format {
            32.0 + ColumnHeader::FORMAT_WIDTH
        } else {
            24.0
        } * theme.zoom;
        let end = self
            .viewport
            .top
            .saturating_add(RowViewport::visible_rows(
                self.layout.body.height() + self.viewport.subrow_px,
                self.layout.row_height,
            ))
            .min(self.len())
            .min(self.viewport.top.saturating_add(MAX_PREPARED_ROWS));
        for ordinal in self.viewport.top..end {
            if let Some(text) = self.cell_text(ordinal, source_index, FIT_PREVIEW_BYTES) {
                width = width.max(measure.text_width(&text, FontRole::Mono, theme.mono_size));
            }
        }
        Some(
            (width / theme.zoom + 16.0)
                .clamp(ColumnHeader::minimum_width(has_format), MAX_COLUMN_WIDTH),
        )
    }

    pub(crate) fn take_edits(
        &mut self,
        panel: crate::panels::PanelId,
        history: &mut crate::history::History,
    ) {
        if let Some(columns) = self.columns.take_before() {
            history.record(
                crate::history::Edit::Prop {
                    panel,
                    prop: crate::panels::content::Prop::Table(Prop::Columns(columns)),
                },
                Some("Change columns".into()),
            );
        }
        if let Some(formats) = self.formats.take_before() {
            history.record(
                crate::history::Edit::Prop {
                    panel,
                    prop: crate::panels::content::Prop::Table(Prop::Formats(formats)),
                },
                Some("Change table format".into()),
            );
        }
        if let Some(widths) = self.widths.take_before() {
            history.record(
                crate::history::Edit::Prop {
                    panel,
                    prop: crate::panels::content::Prop::Table(Prop::Widths(widths)),
                },
                Some("Resize table column".into()),
            );
        }
    }

    pub(crate) fn swap_columns(&mut self, columns: ColumnSet) -> ColumnSet {
        let old = self.columns.swap(columns);
        let previous = self.filters.clone();
        self.filters = (0..self.columns.len())
            .map(|i| {
                let column = self.transaction_column(i);
                let previous_index = match &old {
                    ColumnSet::Transactions(columns) => columns
                        .iter()
                        .find(|(_, c, _)| Some(*c) == column)
                        .map(|(j, _, _)| j),
                    ColumnSet::Signals { .. } => Some(i),
                };
                previous_index.and_then(|j| previous.get(j).cloned().flatten())
            })
            .collect();
        if !super::filter::same_filters(&previous, &self.filters) {
            self.query_changed();
        }
        if self
            .sort_requested
            .into_iter()
            .chain(self.applied_sort())
            .any(|sort| {
                let previous = match &old {
                    ColumnSet::Transactions(c) => c.column(sort.column),
                    ColumnSet::Signals { .. } => None,
                };
                previous != self.transaction_column(sort.column)
            })
        {
            self.reset_query();
        }
        self.invalidate_window();
        old
    }

    fn retry(&mut self, doc: &mut Document) -> bool {
        if self.panel_reservation.is_none() {
            match self.budget.reserve(PANEL_BYTES) {
                Ok(reservation) => {
                    self.panel_reservation = Some(reservation);
                    self.state = TableState::Loading;
                }
                Err(error) => {
                    self.state = TableState::Refused(format!(
                        "Table needs {PANEL_BYTES} bytes; admission failed: {error}"
                    ));
                    return true;
                }
            }
        }
        if !self.attached {
            let resident = HashMap::new();
            return self.attach(doc, &resident).is_ok();
        }
        if matches!(self.state, TableState::Refused(_)) {
            self.state = TableState::Loading;
            self.rows = Rows::None;
            self.axis_build = None;
            self.refresh(doc);
            return true;
        }
        match &self.source {
            TableSource::Generator(source) => {
                source.track().is_some_and(|track| doc.retry_track(track))
            }
            TableSource::Signals(sources) => {
                let mut requested = false;
                for signal in sources.iter().filter_map(|s| s.signal) {
                    if self.signal_results.get(&signal).is_some_and(Result::is_err) {
                        self.signal_results.remove(&signal);
                        requested |= doc.request_signal(signal);
                    }
                }
                if requested {
                    self.state = TableState::Loading;
                }
                requested
            }
        }
    }

    pub fn pointer(
        &mut self,
        doc: &mut Document,
        panel: crate::panels::PanelId,
        event: PointerEvent,
        now: Instant,
    ) -> bool {
        if let PointerEvent::Down {
            position,
            button: MouseButton::Left,
            ..
        } = event
            && let Some(column) = self.column_edge_at(position)
        {
            self.drag = Some(TableDrag::Column {
                source_index: column.source_index,
                start_x: position.x,
                width: column.rect.width() / (self.layout.row_height / ROW_HEIGHT),
            });
            return true;
        }
        match event {
            PointerEvent::Down {
                position,
                button: MouseButton::Left,
                ..
            } if self.layout.header.contains(position) => self
                .layout
                .columns
                .iter()
                .find(|column| column.header.sort_target.contains(position))
                .map(|column| column.source_index)
                .is_some_and(|column| self.cycle_sort(column)),
            PointerEvent::Down {
                position,
                button: MouseButton::Left,
                ..
            } if self.layout.vertical_bar.contains(position) => {
                let grab = if self.layout.vertical_thumb.contains(position) {
                    position.y - self.layout.vertical_thumb.top()
                } else {
                    self.layout.vertical_thumb.height() * 0.5
                };
                self.drag = Some(TableDrag::Vertical { grab });
                self.drag_vertical(position.y, grab);
                self.prepare_visible();
                true
            }
            PointerEvent::Down {
                position,
                button: MouseButton::Left,
                ..
            } if self.layout.horizontal_bar.contains(position) => {
                let grab = if self.layout.horizontal_thumb.contains(position) {
                    position.x - self.layout.horizontal_thumb.left()
                } else {
                    self.layout.horizontal_thumb.width() * 0.5
                };
                self.drag = Some(TableDrag::Horizontal { grab });
                self.drag_horizontal(position.x, grab);
                true
            }
            PointerEvent::Down {
                position,
                button: MouseButton::Left,
                ..
            } if self.layout.body.contains(position) || self.layout.gutter.contains(position) => {
                let y = position.y - self.layout.body.top();
                self.viewport
                    .row_at(y, self.len(), self.layout.row_height)
                    .is_some_and(|row| self.select(doc, panel, row, now))
            }
            PointerEvent::Wheel { dx, dy, .. } => {
                self.horizontal = (self.horizontal + dx).max(0.0);
                self.viewport.scroll(
                    dy,
                    self.len(),
                    self.layout.body.height(),
                    self.layout.row_height,
                );
                self.prepare_visible();
                true
            }
            PointerEvent::Move { position } => match self.drag {
                Some(TableDrag::Column {
                    source_index,
                    start_x,
                    width,
                }) => {
                    let zoom = self.layout.row_height / ROW_HEIGHT;
                    self.resize_column(source_index, width + (position.x - start_x) / zoom)
                }
                Some(TableDrag::Vertical { grab }) => {
                    self.drag_vertical(position.y, grab);
                    self.prepare_visible();
                    true
                }
                Some(TableDrag::Horizontal { grab }) => {
                    self.drag_horizontal(position.x, grab);
                    true
                }
                None => false,
            },
            PointerEvent::Up | PointerEvent::Leave if self.drag.take().is_some() => true,
            _ => false,
        }
    }

    #[must_use]
    pub fn dragging(&self) -> bool {
        self.drag.is_some()
    }

    pub(crate) fn cancel_drag(&mut self) {
        self.drag = None;
    }

    fn drag_vertical(&mut self, pointer_y: f32, grab: f32) {
        let track =
            (self.layout.vertical_bar.height() - self.layout.vertical_thumb.height()).max(0.0);
        let max_top = self.len().saturating_sub(RowViewport::full_rows(
            self.layout.body.height(),
            self.row_px(),
        ));
        let offset = (pointer_y - grab - self.layout.vertical_bar.top()).clamp(0.0, track);
        self.viewport.top = normalized_u64(offset, track, max_top);
        self.viewport.subrow_px = 0.0;
    }

    fn drag_horizontal(&mut self, pointer_x: f32, grab: f32) {
        let track =
            (self.layout.horizontal_bar.width() - self.layout.horizontal_thumb.width()).max(0.0);
        let max_scroll = (self.layout.content_width - self.layout.body.width()).max(0.0);
        let offset = (pointer_x - grab - self.layout.horizontal_bar.left()).clamp(0.0, track);
        self.horizontal = if track == 0.0 {
            0.0
        } else {
            offset / track * max_scroll
        };
    }

    pub fn layout(&mut self, bounds: Rect, theme: &Theme) -> &TableLayout {
        let zoom = theme.zoom;
        let row_height = ROW_HEIGHT * zoom;
        let header_height = HEADER_HEIGHT * zoom;
        let filter_height = if self.filters_visible {
            super::layout::FILTER_HEIGHT * zoom
        } else {
            0.0
        };
        let gutter_width = 64.0 * zoom;
        let bar = 12.0 * zoom;
        let status_height = row_height;
        let header = Rect::from_xywh(
            bounds.left() + gutter_width,
            bounds.top(),
            (bounds.width() - gutter_width - bar).max(0.0),
            header_height,
        );
        let filter_row = Rect::from_xywh(
            header.left(),
            header.bottom(),
            header.width(),
            filter_height,
        );
        let body = Rect::from_xywh(
            header.left(),
            filter_row.bottom(),
            header.width(),
            (bounds.height() - header_height - filter_height - status_height - bar).max(0.0),
        );
        let gutter = Rect::from_xywh(bounds.left(), body.top(), gutter_width, body.height());
        let vertical_bar = Rect::from_xywh(body.right(), body.top(), bar, body.height());
        let horizontal_bar = Rect::from_xywh(body.left(), body.bottom(), body.width(), bar);
        let status = Rect::from_xywh(
            bounds.left(),
            horizontal_bar.bottom(),
            bounds.width(),
            status_height,
        );
        let mut widths = self.visible_column_widths();
        for (index, width) in &mut widths {
            *width = width.max(ColumnHeader::minimum_width(
                self.column_translator(*index).is_some(),
            ));
        }
        let fixed_width = widths.iter().map(|(_, width)| width * zoom).sum::<f32>();
        if let Some((_, width)) = widths.iter_mut().find(|(index, _)| {
            matches!(self.source, TableSource::Generator(_))
                && self.transaction_column(*index) == Some(&TransactionColumn::Attributes)
                && self.widths[*index].is_none()
        }) {
            let other_width = fixed_width - *width * zoom;
            *width = width
                .max((body.width() - other_width) / zoom)
                .min(MAX_COLUMN_WIDTH);
        }
        let content_width = widths.iter().map(|(_, width)| *width * zoom).sum::<f32>();
        if let Some(target) = self.reveal_column.take() {
            let mut left = 0.0;
            for (index, width) in &widths {
                let right = left + width * zoom;
                if *index == target {
                    if left < self.horizontal {
                        self.horizontal = left;
                    } else if right > self.horizontal + body.width() {
                        self.horizontal = (right - body.width()).max(0.0);
                    }
                    break;
                }
                left = right;
            }
        }
        self.horizontal = self
            .horizontal
            .clamp(0.0, (content_width - body.width()).max(0.0));
        let previous_columns: Vec<_> = self
            .layout
            .columns
            .iter()
            .map(|column| (column.source_index, column.preview_bytes))
            .collect();
        let mut x = body.left() - self.horizontal;
        let mut columns = Vec::new();
        for (source_index, width) in widths {
            let width = width * zoom;
            let rect = Rect::from_xywh(x, header.top(), width, header_height);
            if rect.right() >= body.left() && rect.left() <= body.right() {
                columns.push(super::layout::ColumnRect {
                    source_index,
                    prepared_index: columns.len(),
                    rect,
                    resize: Rect::from_xywh(
                        rect.right() - 1.0 - theme.splitter_grab / 2.0,
                        header.top(),
                        theme.splitter_grab,
                        header_height + filter_height + body.height(),
                    ),
                    header: ColumnHeader::new(
                        rect,
                        self.column_translator(source_index).is_some(),
                        zoom,
                    ),
                    preview_bytes: ((width / (theme.mono_size * 0.5).max(1.0)).ceil() as usize * 4)
                        .max(PREVIEW_BYTES)
                        .next_power_of_two()
                        .min(ROW_PREVIEW_BYTES),
                });
            }
            x += width;
        }
        let preview_bytes = columns
            .iter()
            .map(|column| column.preview_bytes)
            .sum::<usize>();
        if preview_bytes > ROW_PREVIEW_BYTES {
            for column in &mut columns {
                column.preview_bytes = column.preview_bytes * ROW_PREVIEW_BYTES / preview_bytes;
            }
        }
        self.viewport.clamp(self.len(), body.height(), row_height);
        let visible = RowViewport::visible_rows(body.height(), row_height) as f32;
        let thumb_h = (vertical_bar.height() * (visible / self.len().max(1) as f32).min(1.0))
            .max(24.0 * zoom)
            .min(vertical_bar.height());
        let max_top = self
            .len()
            .saturating_sub(RowViewport::full_rows(body.height(), row_height));
        let fraction = if max_top == 0 {
            0.0
        } else {
            self.viewport.top as f32 / max_top as f32
        };
        let vertical_thumb = Rect::from_xywh(
            vertical_bar.left() + 2.0,
            vertical_bar.top() + (vertical_bar.height() - thumb_h) * fraction,
            (bar - 4.0).max(0.0),
            thumb_h,
        );
        let thumb_w = (horizontal_bar.width() * horizontal_bar.width() / content_width.max(1.0))
            .max(24.0 * zoom)
            .min(horizontal_bar.width());
        let hfrac = self.horizontal / (content_width - body.width()).max(1.0);
        let horizontal_thumb = Rect::from_xywh(
            horizontal_bar.left() + (horizontal_bar.width() - thumb_w) * hfrac,
            horizontal_bar.top() + 2.0,
            thumb_w,
            (bar - 4.0).max(0.0),
        );
        self.layout = TableLayout {
            filter_row,
            bounds,
            header,
            body,
            gutter,
            vertical_bar,
            vertical_thumb,
            horizontal_bar,
            horizontal_thumb,
            status,
            columns,
            row_height,
            horizontal: self.horizontal,
            content_width,
        };
        if previous_columns
            != self
                .layout
                .columns
                .iter()
                .map(|column| (column.source_index, column.preview_bytes))
                .collect::<Vec<_>>()
        {
            self.invalidate_window();
        }
        self.prepare_visible();
        &self.layout
    }

    fn visible_column_widths(&self) -> Vec<(usize, f32)> {
        match self.columns.get() {
            ColumnSet::Transactions(columns) => columns
                .iter()
                .filter(|(_, _, visible)| *visible)
                .map(|(i, c, _)| (i, self.widths[i].unwrap_or_else(|| c.width())))
                .collect(),
            ColumnSet::Signals { time, visible } => {
                std::iter::once((0, self.widths[0].unwrap_or(130.0)))
                    .filter(|_| *time)
                    .chain(
                        visible
                            .iter()
                            .enumerate()
                            .filter(|(_, show)| **show)
                            .map(|(i, _)| (i + 1, self.widths[i + 1].unwrap_or(180.0))),
                    )
                    .collect()
            }
        }
    }

    #[must_use]
    pub fn column_title(&self, source_index: usize) -> &str {
        match &self.source {
            TableSource::Generator(_) => self
                .transaction_column(source_index)
                .map_or("", |c| c.title()),
            TableSource::Signals(signals) => {
                if source_index == 0 {
                    "Time"
                } else {
                    signals
                        .get(source_index - 1)
                        .map_or("", |s| s.name.as_str())
                }
            }
        }
    }

    fn prepare_visible(&mut self) {
        if !matches!(self.state, TableState::Ready | TableState::Empty) {
            self.window = PreparedWindow::default();
            return;
        }
        let visible = RowViewport::visible_rows(self.layout.body.height(), self.layout.row_height)
            .saturating_add(4)
            .min(MAX_PREPARED_ROWS);
        let start = self.viewport.top.saturating_sub(2);
        let end = start.saturating_add(visible).min(self.len());
        if self.window.start == start
            && self.window.end == end
            && self.window.rows.len() as u64 == end.saturating_sub(start)
        {
            return;
        }
        let mut window = PreparedWindow {
            start,
            end,
            rows: Vec::with_capacity((end - start) as usize),
            bytes: 0,
        };
        let mut signal_hints = match &self.rows {
            Rows::Signals { histories, .. } => vec![None; histories.len()],
            _ => Vec::new(),
        };
        for ordinal in start..end {
            let Some(row) = self.prepare_row(ordinal, &mut signal_hints) else {
                continue;
            };
            window.bytes += row.cells.iter().map(|c| c.text.len()).sum::<usize>();
            if window.bytes > PANEL_BYTES as usize {
                break;
            }
            window.rows.push(row);
        }
        self.window = window;
    }

    fn prepare_row(&self, ordinal: u64, signal_hints: &mut [Option<usize>]) -> Option<PreparedRow> {
        let index = self.source_index(ordinal)?;
        match &self.rows {
            Rows::Generator(generator) => {
                let tx = generator.transactions().get(index)?;
                let cells = self
                    .layout
                    .columns
                    .iter()
                    .map(|column| PreparedCell {
                        text: self.transaction_cell(
                            tx,
                            column.source_index,
                            column.preview_bytes,
                            None,
                        ),
                        changed: false,
                        missing: match self.transaction_column(column.source_index) {
                            Some(TransactionColumn::Attribute(key)) => {
                                !tx.attributes.iter().any(|a| &a.key == key)
                            }
                            Some(TransactionColumn::Label) => {
                                !tx.attributes.iter().any(|a| a.key == "vtr.label")
                            }
                            _ => false,
                        },
                    })
                    .collect();
                Some(PreparedRow {
                    ordinal,
                    identity: RowIdentity::Transaction(tx.id),
                    time: tx.begin,
                    cells,
                })
            }
            Rows::Signals {
                histories, axis, ..
            } => {
                let row_time = axis
                    .as_ref()
                    .and_then(|a| a.get(index).copied())
                    .or_else(|| {
                        histories
                            .first()
                            .filter(|h| axis.is_none() && index < h.len())
                            .map(|h| h.time(index))
                    })?;
                let mut cells = Vec::new();
                for column in &self.layout.columns {
                    if column.source_index == 0 {
                        cells.push(PreparedCell {
                            text: row_time.to_string(),
                            changed: false,
                            missing: false,
                        });
                        continue;
                    }
                    let history = histories.get(column.source_index - 1)?;
                    let hint = signal_hints.get_mut(column.source_index - 1)?;
                    let sample = if self.sort_requested.is_some() {
                        history.index_at(row_time)
                    } else {
                        history.index_at_hint(row_time, hint.unwrap_or(0))
                    };
                    *hint = sample;
                    let missing = sample.is_none();
                    let changed = sample.is_some_and(|i| history.time(i) == row_time);
                    let text = self
                        .signal_value(
                            column.source_index,
                            history.as_ref(),
                            sample,
                            column.preview_bytes,
                        )
                        .text;
                    cells.push(PreparedCell {
                        text,
                        changed,
                        missing,
                    });
                }
                Some(PreparedRow {
                    ordinal,
                    identity: RowIdentity::SignalTime(row_time),
                    time: row_time,
                    cells,
                })
            }
            Rows::None => None,
        }
    }

    fn cell_text(&self, ordinal: u64, source_index: usize, limit: usize) -> Option<String> {
        match &self.rows {
            Rows::Generator(generator) => {
                let tx = generator.transactions().get(self.source_index(ordinal)?)?;
                Some(self.transaction_cell(tx, source_index, limit, None))
            }
            Rows::Signals { histories, .. } => {
                let time = self.row_time(ordinal)?;
                if source_index == 0 {
                    return Some(time.to_string());
                }
                let history = histories.get(source_index - 1)?;
                let sample = history.index_at(time);
                Some(
                    self.signal_value(source_index, history.as_ref(), sample, limit)
                        .text,
                )
            }
            Rows::None => None,
        }
    }

    fn signal_value(
        &self,
        column: usize,
        history: &dyn SignalHistory,
        sample: Option<usize>,
        limit: usize,
    ) -> Translated {
        let value = if sample.is_none() {
            ValueView::Unavailable
        } else {
            history.value_view(sample)
        };
        format::translate(
            self.formats
                .translator(&self.translators, column, history.shape())
                .as_ref(),
            &value,
            limit,
        )
    }

    #[must_use]
    pub fn prepared_row(&self, ordinal: u64) -> Option<&PreparedRow> {
        ordinal
            .checked_sub(self.window.start)
            .and_then(|offset| self.window.rows.get(offset as usize))
            .filter(|row| row.ordinal == ordinal)
    }

    pub fn accessible_rows(&self) -> impl Iterator<Item = AccessibleRow> + '_ {
        self.window.rows.iter().filter_map(|row| {
            let y =
                self.layout.body.top() + self.viewport.y_of(row.ordinal, self.layout.row_height);
            let bounds = Rect::from_xywh(
                self.layout.gutter.left(),
                y,
                self.layout.body.right() - self.layout.gutter.left(),
                self.layout.row_height,
            );
            (bounds.bottom() > self.layout.body.top() && bounds.top() < self.layout.body.bottom())
                .then(|| AccessibleRow {
                    ordinal: row.ordinal,
                    label: format!(
                        "Row {}. {}",
                        row.ordinal.saturating_add(1),
                        row.cells
                            .iter()
                            .map(|cell| cell.text.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                    selected: self.selected.as_ref() == Some(&row.identity),
                    bounds,
                })
        })
    }

    pub fn copy_tsv(&self) -> anyhow::Result<String> {
        let ordinal = self
            .selected_ordinal()
            .ok_or_else(|| anyhow::anyhow!("Select a row to copy."))?;
        let index = self
            .source_index(ordinal)
            .ok_or_else(|| anyhow::anyhow!("Selected row is unavailable"))?;
        let mut text = String::new();
        match (&self.rows, &self.source) {
            (Rows::Generator(generator), TableSource::Generator(source)) => {
                let tx = &generator.transactions()[index];
                append_exact(
                    &mut text,
                    "Generator\tID\tBegin\tEnd\tDuration\tStatus\n",
                    COPY_BYTES,
                )?;
                for (part_index, part) in source.path().iter().enumerate() {
                    if part_index > 0 {
                        append_exact(&mut text, ".", COPY_BYTES)?;
                    }
                    append_exact(&mut text, part, COPY_BYTES)?;
                }
                append_exact(
                    &mut text,
                    &format!(
                        "\t{}\t{}\t{}\t{}\t{}",
                        self.attribute_text(&AttributeValue::U64(tx.id.0), 4, None, COPY_BYTES),
                        tx.begin,
                        tx.end,
                        tx.end.saturating_sub(tx.begin),
                        tx.status.name()
                    ),
                    COPY_BYTES,
                )?;
            }
            (Rows::Signals { histories, .. }, TableSource::Signals(signals)) => {
                let time = self.row_time(ordinal).unwrap();
                append_exact(&mut text, "Time", COPY_BYTES)?;
                for source in signals {
                    append_exact(&mut text, "\t", COPY_BYTES)?;
                    append_exact(&mut text, &source.name, COPY_BYTES)?;
                }
                append_exact(&mut text, "\n", COPY_BYTES)?;
                append_exact(&mut text, &time.to_string(), COPY_BYTES)?;
                for (column, history) in histories.iter().enumerate() {
                    append_exact(&mut text, "\t", COPY_BYTES)?;
                    let sample = history.index_at(time);
                    let value = self.signal_value(
                        column + 1,
                        history.as_ref(),
                        sample,
                        COPY_BYTES.saturating_sub(text.len()),
                    );
                    anyhow::ensure!(!value.truncated, "Row exceeds the 64 KiB clipboard limit.");
                    append_exact(&mut text, &value.text, COPY_BYTES)?;
                }
            }
            _ => return Err(anyhow::anyhow!("Source is not ready.")),
        }
        Ok(text)
    }
}

impl Drop for TableModel {
    fn drop(&mut self) {
        if let Some((_, job)) = &self.query_job {
            job.cancel();
        }
        if let Some(order) = self.query_order.take() {
            super::query::retire(order);
        }
    }
}

impl TableModel {
    fn transaction_cell(
        &self,
        tx: &Transaction,
        source_index: usize,
        limit: usize,
        requested: Option<&dyn Translator>,
    ) -> String {
        let column = self
            .transaction_column(source_index)
            .expect("transaction column");
        match column {
            TransactionColumn::Attribute(key) => {
                tx.attributes.iter().find(|a| &a.key == key).map_or_else(
                    || "—".into(),
                    |a| self.attribute_text(&a.value, source_index, requested, limit),
                )
            }
            TransactionColumn::Label => tx
                .attributes
                .iter()
                .find(|a| a.key == "vtr.label")
                .map_or_else(
                    || "—".into(),
                    |a| self.attribute_text(&a.value, source_index, requested, limit),
                ),
            TransactionColumn::Begin => tx.begin.to_string(),
            TransactionColumn::Duration => tx.end.saturating_sub(tx.begin).to_string(),
            TransactionColumn::End => tx.end.to_string(),
            TransactionColumn::Id => self.attribute_text(
                &AttributeValue::U64(tx.id.0),
                source_index,
                requested,
                limit,
            ),
            TransactionColumn::Status => tx.status.name().into(),
            TransactionColumn::Attributes => {
                let mut text = String::new();
                let mut attributes = tx
                    .attributes
                    .iter()
                    .filter(|a| a.key != "vtr.label")
                    .peekable();
                if attributes.peek().is_none() {
                    return "—".into();
                }
                let budget = limit.saturating_sub("…".len());
                let mut cut = false;
                'attributes: while let Some(attribute) = attributes.next() {
                    let separator = if text.is_empty() { "" } else { " · " };
                    for part in [separator, attribute.key.as_str(), "="] {
                        let room = budget.saturating_sub(text.len());
                        push_limited(&mut text, part, budget);
                        if part.len() > room {
                            cut = true;
                            break 'attributes;
                        }
                    }
                    let remaining = budget.saturating_sub(text.len());
                    text.push_str(&self.attribute_text(
                        &attribute.value,
                        source_index,
                        requested,
                        remaining,
                    ));
                    if budget.saturating_sub(text.len()) < " · ".len() + 1 {
                        cut = attributes.peek().is_some() || remaining < "…".len();
                        break;
                    }
                }
                if cut && limit >= "…".len() && !text.ends_with('…') {
                    text.push('…');
                }
                text
            }
        }
    }

    fn attribute_text(
        &self,
        value: &AttributeValue,
        column: usize,
        requested: Option<&dyn Translator>,
        limit: usize,
    ) -> String {
        let translator = format::attribute_shape(value)
            .map(|shape| self.formats.translator(&self.translators, column, shape));
        format::attribute(value, requested.or(translator.as_deref()), limit)
    }
}

fn normalized_u64(offset: f32, track: f32, maximum: u64) -> u64 {
    if maximum == 0 || !offset.is_finite() || !track.is_finite() || track <= 0.0 {
        return 0;
    }
    const UNITS_PER_PIXEL: f64 = 1024.0;
    let denominator = ((f64::from(track) * UNITS_PER_PIXEL).round() as u64).max(1);
    let numerator =
        ((f64::from(offset.clamp(0.0, track)) * UNITS_PER_PIXEL).round() as u64).min(denominator);
    ((u128::from(maximum) * u128::from(numerator)) / u128::from(denominator)) as u64
}

pub(crate) enum Prop {
    Columns(ColumnSet),
    Widths(Vec<Option<f32>>),
    Formats(TableFormats),
}

impl Prop {
    pub(crate) fn kind(&self) -> u8 {
        match self {
            Self::Columns(_) => 0,
            Self::Widths(_) => 1,
            Self::Formats(_) => 2,
        }
    }
    pub(crate) fn bytes(&self) -> usize {
        match self {
            Self::Columns(columns) => columns.bytes(),
            Self::Widths(widths) => widths.len() * std::mem::size_of::<Option<f32>>(),
            Self::Formats(formats) => formats.bytes(),
        }
    }
}

impl TableModel {
    pub(crate) fn matches_prop(&self, prop: &Prop) -> bool {
        match prop {
            Prop::Columns(columns) => self.columns.get() == columns,
            Prop::Widths(widths) => self.widths.get() == widths,
            Prop::Formats(formats) => self.formats.get() == formats,
        }
    }

    pub(crate) fn swap_prop(&mut self, prop: Prop) -> Prop {
        match prop {
            Prop::Columns(columns) => Prop::Columns(self.swap_columns(columns)),
            Prop::Widths(widths) => Prop::Widths(self.widths.swap(widths)),
            Prop::Formats(formats) => {
                let old = self.formats.swap(formats);
                self.invalidate_window();
                Prop::Formats(old)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::point;
    use crate::pipeline::TrackSource;
    use crate::table::SignalSource;
    use crate::table::layout::MIN_COLUMN_WIDTH;
    use volna_trace::data::history::VecHistory;
    use volna_trace::data::transactions::{AttributeValue, TransactionAttribute, TxKind, TxStatus};
    use volna_trace::data::{SignalShape, WaveValue};

    fn history(times: &[u64], values: &[&str]) -> Arc<dyn SignalHistory> {
        Arc::new(VecHistory {
            shape: SignalShape::Bit,
            times: times.to_vec(),
            values: values
                .iter()
                .map(|value| WaveValue::Bits((*value).into()))
                .collect(),
            initial: WaveValue::Unavailable,
        })
    }

    fn signal_source(name: &str, id: u32) -> SignalSource {
        SignalSource {
            trace: crate::trace::TraceId::A,
            path: vec![name.into()],
            nth: None,
            var: Some(id as usize),
            signal: Some(Traced::new(crate::trace::TraceId::A, SignalRef(id))),
            name: name.into(),
        }
    }

    fn signal_model(budget: MemoryBudget) -> TableModel {
        let mut model = TableModel::new(
            TableSource::Signals(vec![signal_source("a", 1), signal_source("b", 2)]),
            Link::default(),
            budget,
            Translators::builtin(),
        );
        model
            .attach(&mut Document::new(), &Default::default())
            .unwrap();
        model
    }

    #[test]
    fn merged_axis_collapses_coincident_and_repeated_changes() {
        let budget = MemoryBudget::new(16 * 1024 * 1024);
        let mut model = signal_model(budget.clone());
        let a = history(&[5, 5, 10, 30], &["0", "1", "0", "1"]);
        let b = history(&[1, 10, 20], &["1", "0", "1"]);
        assert!(!model.build_signal_rows(vec![a, b]).unwrap());
        let Rows::Signals {
            axis: Some(axis), ..
        } = &model.rows
        else {
            panic!()
        };
        assert_eq!(&**axis, &[1, 5, 10, 20, 30]);
        assert_eq!(budget.used(), PANEL_BYTES + 5 * 8);
        drop(model);
        assert_eq!(budget.used(), 0);
    }

    #[test]
    fn single_signal_uses_history_as_its_axis_without_allocation() {
        let budget = MemoryBudget::new(8 * 1024 * 1024);
        let mut model = TableModel::new(
            TableSource::Signals(vec![signal_source("a", 1)]),
            Link::default(),
            budget.clone(),
            Translators::builtin(),
        );
        model
            .attach(&mut Document::new(), &Default::default())
            .unwrap();
        model
            .build_signal_rows(vec![history(&[2, 8], &["0", "1"])])
            .unwrap();
        assert_eq!(model.len(), 2);
        assert_eq!(model.row_time(1), Some(8));
        assert_eq!(budget.used(), PANEL_BYTES);
    }

    #[test]
    fn axis_refusal_is_atomic_and_reports_required_admission() {
        let budget = MemoryBudget::new(PANEL_BYTES + 16);
        let mut model = signal_model(budget.clone());
        let error = model
            .build_signal_rows(vec![
                history(&[1, 2], &["0", "1"]),
                history(&[3, 4], &["0", "1"]),
            ])
            .unwrap_err()
            .to_string();
        assert!(error.contains("32 bytes"));
        assert_eq!(budget.used(), PANEL_BYTES);
        assert_eq!(model.len(), 0);
    }

    #[test]
    fn signal_axis_work_is_sliced_and_cancel_releases_the_build() {
        let budget = MemoryBudget::new(16 * 1024 * 1024);
        let mut model = signal_model(budget.clone());
        let times: Vec<u64> = (0..20_000).collect();
        let values = vec!["0"; times.len()];
        assert_eq!(
            model
                .begin_signal_rows(vec![history(&times, &values), history(&times, &values)])
                .unwrap(),
            None
        );
        assert!(model.tick(Instant::now()));
        assert!(model.axis_build.is_some(), "one tick must remain bounded");
        assert!(budget.used() > PANEL_BYTES);

        model.attached = true;
        assert!(model.command(
            &mut Document::new(),
            crate::panels::PanelId(1),
            TableCommand::Cancel,
            Instant::now(),
        ));
        assert!(model.axis_build.is_none());
        assert_eq!(budget.used(), PANEL_BYTES);
    }

    #[test]
    fn signal_window_formats_only_horizontally_intersecting_columns() {
        let sources = (0..20)
            .map(|index| signal_source(&format!("s{index}"), index + 1))
            .collect();
        let histories = (0..20).map(|_| history(&[1, 2], &["0", "1"])).collect();
        let mut model = TableModel::new(
            TableSource::Signals(sources),
            Link::default(),
            MemoryBudget::new(16 * 1024 * 1024),
            Translators::builtin(),
        );
        model.build_signal_rows(histories).unwrap();
        model.state = TableState::Ready;
        model.layout(Rect::from_xywh(0.0, 0.0, 320.0, 240.0), &Theme::one_dark());
        let first: Vec<_> = model
            .layout
            .columns
            .iter()
            .map(|column| column.source_index)
            .collect();
        assert!(
            model
                .window
                .rows
                .iter()
                .all(|row| row.cells.len() == first.len())
        );
        assert!(first.len() < 21);

        model.horizontal = 2_000.0;
        model.layout(Rect::from_xywh(0.0, 0.0, 320.0, 240.0), &Theme::one_dark());
        let second: Vec<_> = model
            .layout
            .columns
            .iter()
            .map(|column| column.source_index)
            .collect();
        assert_ne!(first, second);
        assert!(
            model
                .window
                .rows
                .iter()
                .all(|row| row.cells.len() == second.len())
        );
    }

    #[test]
    fn signal_projection_borrows_before_enforcing_preview_and_copy_limits() {
        struct BorrowOnly {
            value: String,
        }
        impl SignalHistory for BorrowOnly {
            fn shape(&self) -> SignalShape {
                SignalShape::Text
            }
            fn len(&self) -> usize {
                1
            }
            fn time(&self, _: usize) -> u64 {
                7
            }
            fn value(&self, _: Option<usize>) -> WaveValue {
                panic!("table must not clone the complete signal value")
            }
            fn value_view(&self, _: Option<usize>) -> volna_trace::data::value_view::ValueView<'_> {
                volna_trace::data::value_view::ValueView::Text(self.value.as_str().into())
            }
            fn bit(&self, _: Option<usize>) -> volna_trace::data::Bit {
                volna_trace::data::Bit::Other
            }
        }

        let mut model = TableModel::new(
            TableSource::Signals(vec![signal_source("wide", 1)]),
            Link::default(),
            MemoryBudget::new(16 * 1024 * 1024),
            Translators::builtin(),
        );
        model
            .build_signal_rows(vec![Arc::new(BorrowOnly {
                value: "x".repeat(COPY_BYTES + 1),
            })])
            .unwrap();
        model.state = TableState::Ready;
        model.layout(Rect::from_xywh(0.0, 0.0, 500.0, 240.0), &Theme::one_dark());
        assert!(model.window.rows[0].cells[1].text.len() <= PREVIEW_BYTES);
        assert_eq!(
            model.column_fit_width(1, &Theme::one_dark(), &mut crate::scene::MonoMeasure),
            Some(MAX_COLUMN_WIDTH)
        );
        model.resize_column(1, 1200.0);
        model.layout(Rect::from_xywh(0.0, 0.0, 500.0, 240.0), &Theme::one_dark());
        assert!(model.window.rows[0].cells[1].text.len() > PREVIEW_BYTES);
        assert!(model.window.rows[0].cells[1].text.ends_with('…'));
        model.selected = Some(RowIdentity::SignalTime(7));
        assert!(model.copy_tsv().unwrap_err().to_string().contains("64 KiB"));
    }

    fn transaction(id: u64, begin: u64, label: &str) -> Transaction {
        Transaction {
            id: TransactionRef(id),
            generator: volna_trace::data::transactions::TrackRef(7),
            begin,
            end: begin + 8,
            status: TxStatus::Ok,
            kind: TxKind::Producer,
            parent: None,
            attributes: vec![
                TransactionAttribute {
                    key: "vtr.label".into(),
                    value: AttributeValue::Text(label.into()),
                },
                TransactionAttribute {
                    key: "addr".into(),
                    value: AttributeValue::U64(0x80),
                },
            ],
            events: vec![],
            stages: vec![],
        }
    }

    fn generator_model(records: Vec<Transaction>) -> TableModel {
        let generator = LoadedGenerator::new(
            volna_trace::data::transactions::TrackRef(7),
            records,
            HashMap::new(),
            vec![],
        )
        .unwrap();
        let mut model = TableModel::new(
            TableSource::Generator(TrackSource::Unresolved {
                trace: crate::trace::TraceId::A,
                path: vec!["request".into()],
            }),
            Link::default(),
            MemoryBudget::new(16 * 1024 * 1024),
            Translators::builtin(),
        );
        model.rows = Rows::Generator(Arc::new(generator));
        model.state = TableState::Ready;
        model
    }

    fn set_filter(
        model: &mut TableModel,
        column: usize,
        mode: super::super::filter::FilterMode,
        query: &str,
    ) {
        assert!(model.command(
            &mut Document::new(),
            crate::panels::PanelId(1),
            TableCommand::SetFilter {
                source_index: column,
                filter: Some(super::super::filter::ColumnFilter {
                    mode,
                    query: query.into(),
                    ..Default::default()
                })
            },
            Instant::now()
        ));
    }

    #[test]
    #[ignore = "10M resident transactions; run in release mode"]
    fn ten_million_transactions_filter_on_background_worker() {
        use super::super::filter::FilterMode;
        let rows = (0..10_000_000)
            .map(|i| Transaction {
                id: TransactionRef(i + 1),
                generator: volna_trace::data::transactions::TrackRef(7),
                begin: i,
                end: i + 1,
                status: TxStatus::Ok,
                kind: TxKind::Producer,
                parent: None,
                attributes: vec![],
                events: vec![],
                stages: vec![],
            })
            .collect();
        let mut model = generator_model(rows);
        set_filter(&mut model, 1, FilterMode::Expression, "> 5000000");
        set_filter(&mut model, 4, FilterMode::Expression, "<= 9000000");
        let start = std::time::Instant::now();
        finish_sort(&mut model);
        assert_eq!(model.len(), 3_999_999);
        assert_eq!(model.row_time(0), Some(5_000_001));
        assert_eq!(model.row_time(model.len() - 1), Some(8_999_999));
        assert_eq!(model.budget.used(), 1_406_256);
        eprintln!(
            "10M resident transactions, two numeric predicates: {:?}, {} index bytes",
            start.elapsed(),
            model.budget.used()
        );
    }

    #[test]
    fn filters_combine_raw_values_preserve_sort_and_selected_identity() {
        use super::super::filter::FilterMode;
        let mut records: Vec<_> = (1..=40)
            .map(|i| {
                transaction(
                    i,
                    i * 10,
                    if i % 2 == 0 {
                        "cache_load_miss"
                    } else {
                        "store"
                    },
                )
            })
            .collect();
        records[19].attributes[1].value = AttributeValue::U64(9_007_199_254_740_993);
        records[21].attributes[1].value = AttributeValue::U64(9_007_199_254_740_992);
        let mut model = generator_model(records);
        model.edit_columns(|c| c.toggle_transaction(TransactionColumn::Attribute("addr".into())));
        model.cycle_sort(1);
        model.cycle_sort(1);
        finish_sort(&mut model);
        model.selected = Some(RowIdentity::Transaction(TransactionRef(20)));
        set_filter(&mut model, 0, FilterMode::Wildcard, "*LOAD*");
        set_filter(&mut model, 1, FilterMode::Expression, "> 100 AND < 300");
        finish_sort(&mut model);
        assert_eq!(model.len(), 9);
        assert_eq!(model.row_time(0), Some(280));
        assert_eq!(model.selected_ordinal(), Some(4));
        assert!(model.applied_sort().unwrap().descending);
        set_filter(&mut model, 7, FilterMode::Expression, "= 9007199254740993");
        finish_sort(&mut model);
        assert_eq!(model.len(), 1);
        assert_eq!(model.row_time(0), Some(200));
        assert_eq!(model.selected_ordinal(), Some(0));
        let mut doc = Document::new();
        let panel = crate::panels::PanelId(1);
        model.command(&mut doc, panel, TableCommand::ToggleFilters, Instant::now());
        model.command(&mut doc, panel, TableCommand::ToggleFilters, Instant::now());
        model.command(
            &mut doc,
            panel,
            TableCommand::NumberFormat("udec".into()),
            Instant::now(),
        );
        assert_eq!(model.len(), 1);
        assert!(!model.filters_visible);
        model.command(&mut doc, panel, TableCommand::ClearFilters, Instant::now());
        finish_sort(&mut model);
        assert_eq!(model.len(), 40);
        assert_eq!(model.row_time(0), Some(400));
    }

    #[test]
    fn aggregate_missing_filters_do_not_treat_null_leaves_as_missing_columns() {
        use super::super::filter::{ColumnFilter, FilterMode, Missing};
        let mut records: Vec<_> = (1..=4).map(|i| transaction(i, i, "row")).collect();
        records[0].attributes.truncate(1);
        records[1].attributes[1].value = AttributeValue::List(vec![AttributeValue::Null]);
        records[2].attributes[1].value = AttributeValue::Null;
        records[3].attributes[1].value =
            AttributeValue::Map(vec![("nested".into(), AttributeValue::Null)]);
        let mut model = generator_model(records);
        let mut doc = Document::new();
        let panel = crate::panels::PanelId(1);
        for (missing, query) in [(Missing::Only, ""), (Missing::Include, "no match")] {
            model.command(
                &mut doc,
                panel,
                TableCommand::SetFilter {
                    source_index: 6,
                    filter: Some(ColumnFilter {
                        missing,
                        query: query.into(),
                        mode: FilterMode::Contains,
                        ..Default::default()
                    }),
                },
                Instant::now(),
            );
            finish_sort(&mut model);
            assert_eq!(model.len(), 1);
            assert_eq!(model.row_time(0), Some(1));
        }
        model.command(&mut doc, panel, TableCommand::ClearFilters, Instant::now());
        model.edit_columns(|c| c.toggle_transaction(TransactionColumn::Attribute("addr".into())));
        model.command(
            &mut doc,
            panel,
            TableCommand::SetFilter {
                source_index: 7,
                filter: Some(ColumnFilter {
                    missing: Missing::Only,
                    ..Default::default()
                }),
            },
            Instant::now(),
        );
        finish_sort(&mut model);
        assert_eq!(model.len(), 2);
        assert_eq!(model.row_time(0), Some(1));
        assert_eq!(model.row_time(1), Some(3));
    }

    #[test]
    fn filtered_sort_admission_includes_bitmap_and_retained_index() {
        use super::super::filter::FilterMode;
        let mut model = generator_model((1..=100).map(|i| transaction(i, i, "row")).collect());
        set_filter(&mut model, 4, FilterMode::Expression, ">= 99");
        finish_sort(&mut model);
        let bitmap = super::super::filter::index::required_bytes(100).unwrap();
        assert_eq!(model.budget.used(), bitmap);
        model.budget.set_limit(bitmap + 8);
        assert!(
            !model.can_sort_column(1),
            "a sparse sort still needs the source bitmap"
        );
        model.budget.set_limit(bitmap * 2 + 8);
        assert!(model.cycle_sort(1));
        finish_sort(&mut model);
        model.budget.set_limit(128);
        assert!(model.cycle_sort(1));
        finish_sort(&mut model);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while model.budget.used() != 8 {
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
        model.budget.set_limit(8);
        assert!(
            !model.can_sort_column(1),
            "returning filtered rows to source order needs a bitmap"
        );
        model.budget.set_limit(8 + bitmap);
        assert!(model.cycle_sort(1));
        finish_sort(&mut model);
        assert_eq!(model.applied_sort(), None);
        assert_eq!(model.len(), 2);
    }

    #[test]
    fn reentering_a_failed_filter_retries_and_rejected_drafts_do_not_poison_results() {
        use super::super::filter::{ColumnFilter, FilterMode, MAX_QUERY_BYTES};
        let mut model = generator_model((1..=100).map(|i| transaction(i, i, "row")).collect());
        let mut doc = Document::new();
        let panel = crate::panels::PanelId(1);
        model.budget.set_limit(0);
        set_filter(&mut model, 1, FilterMode::Expression, "> 50");
        model.tick(Instant::now());
        assert!(model.query_error.is_some());
        model.budget.set_limit(1024);
        set_filter(&mut model, 1, FilterMode::Expression, "> 50");
        finish_sort(&mut model);
        assert_eq!(model.len(), 50);
        model.command(
            &mut doc,
            panel,
            TableCommand::SetFilter {
                source_index: 1,
                filter: Some(ColumnFilter {
                    query: "x".repeat(MAX_QUERY_BYTES + 1),
                    ..Default::default()
                }),
            },
            Instant::now(),
        );
        assert!(model.query_error.is_some());
        set_filter(&mut model, 1, FilterMode::Expression, "> 50");
        finish_sort(&mut model);
        assert!(model.query_error.is_none());
        assert_eq!(model.len(), 50);
    }

    #[test]
    fn invalid_newer_and_refused_filters_keep_the_previous_result() {
        use super::super::filter::{ColumnFilter, FilterMode};
        let mut model = generator_model((1..=12_347).map(|i| transaction(i, i, "row")).collect());
        set_filter(&mut model, 1, FilterMode::Expression, "> 100");
        model.tick(Instant::now());
        set_filter(&mut model, 1, FilterMode::Expression, "> 12000");
        finish_sort(&mut model);
        assert_eq!(model.len(), 347);
        assert_eq!(model.row_time(0), Some(12001));
        set_filter(&mut model, 0, FilterMode::Regex, "[");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while model.querying() {
            model.tick(Instant::now());
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
        assert!(model.query_error.as_ref().unwrap().contains("Label"));
        assert_eq!(model.len(), 347);
        model.command(
            &mut Document::new(),
            crate::panels::PanelId(1),
            TableCommand::SetFilter {
                source_index: 0,
                filter: None,
            },
            Instant::now(),
        );
        finish_sort(&mut model);
        let retained = model.budget.used();
        model.budget.set_limit(retained);
        set_filter(&mut model, 1, FilterMode::Expression, "> 12200");
        model.tick(Instant::now());
        assert!(model.query_error.is_some());
        assert_eq!(model.len(), 347);
        assert_eq!(model.budget.used(), retained);
        model.budget.set_limit(1024 * 1024);
        model.command(
            &mut Document::new(),
            crate::panels::PanelId(1),
            TableCommand::Retry,
            Instant::now(),
        );
        finish_sort(&mut model);
        assert_eq!(model.len(), 147);
        set_filter(&mut model, 1, FilterMode::Expression, "< 10000");
        model.tick(Instant::now());
        model.command(
            &mut Document::new(),
            crate::panels::PanelId(1),
            TableCommand::Cancel,
            Instant::now(),
        );
        finish_sort(&mut model);
        assert_eq!(model.len(), 147);
        assert_eq!(
            model.filters[1],
            Some(ColumnFilter {
                mode: FilterMode::Expression,
                query: "> 12200".into(),
                ..Default::default()
            })
        );
        let budget = model.budget.clone();
        drop(model);
        while budget.used() != 0 {
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
    }

    #[test]
    fn missing_values_signal_axes_and_filter_row_geometry_are_consistent() {
        use super::super::filter::{ColumnFilter, FilterMode, Missing};
        let mut model = signal_model(MemoryBudget::new(1024 * 1024));
        model
            .build_signal_rows(vec![
                history(&[2, 5, 9], &["1", "0", "1"]),
                history(&[5, 8], &["1", "0"]),
            ])
            .unwrap();
        model.state = TableState::Ready;
        set_filter(&mut model, 0, FilterMode::Expression, "> 2");
        set_filter(&mut model, 1, FilterMode::Expression, "= 1");
        finish_sort(&mut model);
        assert_eq!(model.len(), 1);
        assert_eq!(model.row_time(0), Some(9));
        let mut doc = Document::new();
        let panel = crate::panels::PanelId(1);
        model.command(&mut doc, panel, TableCommand::ClearFilters, Instant::now());
        model.command(
            &mut doc,
            panel,
            TableCommand::SetFilter {
                source_index: 2,
                filter: Some(ColumnFilter {
                    missing: Missing::Only,
                    ..Default::default()
                }),
            },
            Instant::now(),
        );
        finish_sort(&mut model);
        assert_eq!(model.len(), 1);
        assert_eq!(model.row_time(0), Some(2));
        model.filters_visible = true;
        for mut theme in [Theme::one_dark(), Theme::volna(false)] {
            for zoom in [1.0, 1.5, 2.0] {
                theme.zoom = zoom;
                model.layout(Rect::from_xywh(0.0, 0.0, 360.0, 500.0), &theme);
                assert_eq!(model.layout.header.bottom(), model.layout.filter_row.top());
                assert_eq!(model.layout.filter_row.bottom(), model.layout.body.top());
                assert_eq!(
                    model.layout.filter_row.height(),
                    super::super::layout::FILTER_HEIGHT * zoom
                );
            }
        }
    }

    #[test]
    fn column_formats_override_defaults_without_changing_selection_or_sort() {
        let mut model = generator_model((1..=30).map(|i| transaction(i, i, "row")).collect());
        model.edit_columns(|c| c.toggle_transaction(TransactionColumn::Attribute("addr".into())));
        model.cycle_sort(7);
        finish_sort(&mut model);
        let mut doc = Document::new();
        let panel = crate::panels::PanelId(1);
        let now = Instant::now();
        model.select(&mut doc, panel, 20, now);
        let selected = model.selected.clone();
        let viewport = model.viewport;
        let sort = model.applied_sort();
        assert_eq!(model.cell_text(0, 7, 64).unwrap(), "0x80");
        assert!(
            model
                .column_formats(7)
                .unwrap()
                .iter()
                .any(|c| c.preview == "128")
        );
        assert!(model.command(
            &mut doc,
            panel,
            TableCommand::ColumnFormat {
                source_index: 7,
                translator: Some("udec".into())
            },
            now
        ));
        assert!(model.command(
            &mut doc,
            panel,
            TableCommand::NumberFormat("bin".into()),
            now
        ));
        assert_eq!(model.cell_text(0, 7, 64).unwrap(), "128");
        assert_eq!(model.cell_text(0, 4, 64).unwrap(), "0b1");
        assert_eq!(model.cell_text(0, 1, 64).unwrap(), "1");
        assert_eq!(model.selected, selected);
        assert_eq!(model.viewport, viewport);
        assert_eq!(model.applied_sort(), sort);
        assert!(!model.command(
            &mut doc,
            panel,
            TableCommand::ColumnFormat {
                source_index: 7,
                translator: Some("text".into())
            },
            now
        ));
        assert!(!model.command(
            &mut doc,
            panel,
            TableCommand::NumberFormat("float".into()),
            now
        ));
        model.edit_columns(|c| c.move_transaction(7, Some(1)));
        model.edit_columns(|c| c.toggle_transaction(TransactionColumn::Attribute("addr".into())));
        model.edit_columns(|c| c.toggle_transaction(TransactionColumn::Attribute("addr".into())));
        assert_eq!(model.clone_view().formats.get(), model.formats.get());
        assert_eq!(model.cell_text(0, 7, 64).unwrap(), "128");
        assert!(model.command(
            &mut doc,
            panel,
            TableCommand::ColumnFormat {
                source_index: 7,
                translator: None
            },
            now
        ));
        assert_eq!(model.cell_text(0, 7, 64).unwrap(), "0b10000000");
        model.layout(Rect::from_xywh(0.0, 0.0, 1400.0, 400.0), &Theme::one_dark());
        let slot = model
            .layout
            .columns
            .iter()
            .position(|c| c.source_index == 7)
            .unwrap();
        assert_eq!(model.window.rows[0].cells[slot].text, "0b10000000");
    }

    #[test]
    fn signal_formats_share_translators_for_display_previews_and_clipboard() {
        let mut model = signal_model(MemoryBudget::new(16 * 1024 * 1024));
        let history = |width, bits: &str| {
            Arc::new(VecHistory {
                shape: SignalShape::Vector { width },
                times: vec![7],
                values: vec![WaveValue::Bits(bits.into())],
                initial: WaveValue::Unavailable,
            }) as Arc<dyn SignalHistory>
        };
        model
            .build_signal_rows(vec![
                history(32, "00111111110000000000000000000000"),
                history(8, "11111111"),
            ])
            .unwrap();
        model.state = TableState::Ready;
        model.layout(Rect::from_xywh(0.0, 0.0, 800.0, 240.0), &Theme::one_dark());
        assert_eq!(model.window.rows[0].cells[1].text, "0x3fc00000");
        let choices = model.column_formats(1).unwrap();
        assert!(
            choices
                .iter()
                .any(|c| c.translator.as_deref() == Some("float") && c.preview == "1.5")
        );
        assert!(
            !model
                .column_formats(2)
                .unwrap()
                .iter()
                .any(|c| c.translator.as_deref() == Some("float"))
        );
        assert!(model.column_formats(0).is_none());
        let mut doc = Document::new();
        let panel = crate::panels::PanelId(1);
        for (column, translator) in [(1, "float"), (2, "sdec")] {
            assert!(model.command(
                &mut doc,
                panel,
                TableCommand::ColumnFormat {
                    source_index: column,
                    translator: Some(translator.into())
                },
                Instant::now()
            ));
        }
        model.layout(Rect::from_xywh(0.0, 0.0, 800.0, 240.0), &Theme::one_dark());
        assert_eq!(model.window.rows[0].cells[1].text, "1.5");
        assert_eq!(model.window.rows[0].cells[2].text, "-1");
        model.selected = Some(RowIdentity::SignalTime(7));
        assert_eq!(model.copy_tsv().unwrap(), "Time\ta\tb\n7\t1.5\t-1");
    }

    fn finish_sort(model: &mut TableModel) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while model.querying() {
            model.tick(Instant::now());
            assert!(std::time::Instant::now() < deadline, "sort did not finish");
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert!(model.query_error.is_none(), "{:?}", model.query_error);
        model.layout(Rect::from_xywh(0.0, 0.0, 1400.0, 400.0), &Theme::one_dark());
    }

    #[test]
    fn attribute_columns_keep_exact_values_missing_cells_and_identity_after_reorder() {
        let mut records = (1..=4)
            .map(|i| transaction(i, i, "row"))
            .collect::<Vec<_>>();
        records[0].attributes[1].value = AttributeValue::U64(u64::MAX);
        records[1].attributes[1].value = AttributeValue::U64(u64::MAX - 1);
        records[2].attributes.pop();
        records[3].attributes[1].value = AttributeValue::Null;
        let mut model = generator_model(records);
        let attribute = TransactionColumn::Attribute("addr".into());
        assert!(model.edit_columns(|c| c.toggle_transaction(attribute.clone())));
        let slot = 7;
        model.layout(Rect::from_xywh(0.0, 0.0, 1400.0, 400.0), &Theme::one_dark());
        let cell = model
            .layout
            .columns
            .iter()
            .position(|c| c.source_index == slot)
            .unwrap();
        assert_eq!(model.window.rows[0].cells[cell].text, "0xffffffffffffffff");
        assert!(model.window.rows[2].cells[cell].missing);
        assert_eq!(model.window.rows[2].cells[cell].text, "—");
        assert!(!model.window.rows[3].cells[cell].missing);
        assert_eq!(model.window.rows[3].cells[cell].text, "null");
        model.resize_column(slot, 240.0);
        model.edit_columns(|c| c.move_transaction(slot, Some(1)));
        model.edit_columns(|c| c.toggle_transaction(attribute.clone()));
        model.edit_columns(|c| c.toggle_transaction(attribute.clone()));
        assert_eq!(model.widths[slot], Some(240.0));
        let ColumnSet::Transactions(columns) = model.columns.get() else {
            panic!()
        };
        assert_eq!(columns.iter().nth(1).unwrap().0, slot);
        assert_eq!(
            columns.iter().filter(|(_, c, _)| *c == &attribute).count(),
            1
        );
        for expected in [[2, 1, 4, 3], [1, 2, 4, 3]] {
            model.cycle_sort(slot);
            finish_sort(&mut model);
            assert_eq!(
                (0..4)
                    .map(|i| match model.identity_and_time(i).unwrap().0 {
                        RowIdentity::Transaction(id) => id.0,
                        RowIdentity::SignalTime(_) => panic!(),
                    })
                    .collect::<Vec<_>>(),
                expected
            );
        }
        let before = model.columns.get().clone();
        model.edit_columns(|c| {
            c.reset();
            true
        });
        assert_eq!(model.sort_requested, None);
        model.columns.take_before();
        model.swap_columns(before);
        assert_eq!(model.column_title(slot), "addr");
        model.edit_columns(|c| {
            c.toggle_transaction(TransactionColumn::Attribute("vtr.label".into()))
        });
        assert_eq!(model.columns.len(), 8);
        assert!(!model.columns.transaction_visible(TransactionColumn::Label));
        model.command(
            &mut Document::new(),
            crate::panels::PanelId(1),
            TableCommand::ShowTransactionColumn(TransactionColumn::Attribute("vtr.label".into())),
            Instant::now(),
        );
        assert!(model.columns.transaction_visible(TransactionColumn::Label));
        assert_eq!(model.columns.len(), 8);
        assert_eq!(model.reveal_column, Some(0));
    }

    #[test]
    fn sort_cycles_exact_keys_stable_ties_and_preserves_selected_record() {
        let big = 1u64 << 54;
        let mut model = generator_model(vec![
            transaction(30, big + 2, "z"),
            transaction(10, big + 1, "a"),
            transaction(20, big + 1, "a"),
        ]);
        model.selected = Some(RowIdentity::Transaction(TransactionRef(30)));
        model.cycle_sort(1);
        finish_sort(&mut model);
        assert_eq!(
            (0..3)
                .map(|r| model.identity_and_time(r).unwrap().0)
                .collect::<Vec<_>>(),
            vec![
                RowIdentity::Transaction(TransactionRef(10)),
                RowIdentity::Transaction(TransactionRef(20)),
                RowIdentity::Transaction(TransactionRef(30))
            ]
        );
        assert_eq!(model.selected_ordinal(), Some(2));
        assert!(
            model
                .copy_tsv()
                .unwrap()
                .contains(&format!("\t0x1e\t{}", big + 2))
        );
        assert_eq!(model.window.rows[0].time, big + 1);
        model.cycle_sort(1);
        finish_sort(&mut model);
        assert_eq!(model.selected_ordinal(), Some(0));
        assert_eq!(
            model.identity_and_time(1).unwrap().0,
            RowIdentity::Transaction(TransactionRef(10))
        );
        assert_eq!(
            model.identity_and_time(2).unwrap().0,
            RowIdentity::Transaction(TransactionRef(20))
        );
        let mut doc = Document::new();
        model.command(
            &mut doc,
            crate::panels::PanelId(1),
            TableCommand::Last,
            Instant::now(),
        );
        assert_eq!(
            model.selected,
            Some(RowIdentity::Transaction(TransactionRef(20)))
        );
        model.cycle_sort(1);
        finish_sort(&mut model);
        assert_eq!(model.applied_sort(), None);
        assert_eq!(model.selected_ordinal(), Some(1));
        assert_eq!(model.row_time(0), Some(big + 1));
    }

    #[test]
    fn missing_values_stay_last_and_signal_rows_use_sorted_time_axis() {
        let mut open = transaction(1, 1, "open");
        open.status = vtr::TxStatus::Open;
        let mut model = generator_model(vec![open, transaction(2, 2, "done")]);
        for _ in 0..2 {
            model.cycle_sort(2);
            finish_sort(&mut model);
            assert_eq!(
                model.identity_and_time(1).unwrap().0,
                RowIdentity::Transaction(TransactionRef(1))
            );
        }
        let mut model = signal_model(MemoryBudget::new(16 * 1024 * 1024));
        model
            .build_signal_rows(vec![
                history(&[2, 4, 6], &["1", "0", "1"]),
                history(&[1, 5], &["0", "1"]),
            ])
            .unwrap();
        model.state = TableState::Ready;
        model.selected = Some(RowIdentity::SignalTime(4));
        model.cycle_sort(1);
        finish_sort(&mut model);
        assert_eq!(
            (0..5)
                .map(|r| model.row_time(r).unwrap())
                .collect::<Vec<_>>(),
            vec![4, 5, 2, 6, 1]
        );
        assert_eq!(model.selected_ordinal(), Some(0));
        assert_eq!(model.window.rows[0].cells[1].text, "0");
        assert!(model.window.rows[4].cells[1].missing);
        model.cycle_sort(1);
        finish_sort(&mut model);
        assert_eq!(
            (0..5)
                .map(|r| model.row_time(r).unwrap())
                .collect::<Vec<_>>(),
            vec![2, 6, 4, 5, 1]
        );
        assert_eq!(model.selected_ordinal(), Some(2));
    }

    #[test]
    fn sort_admission_accounts_for_retained_index_and_rechecks_before_allocation() {
        let mut model = generator_model((0..100).map(|i| transaction(i, i, "row")).collect());
        model.budget.set_limit(400);
        assert!(model.cycle_sort(1));
        let blocker = model.budget.reserve(400).unwrap();
        model.tick(Instant::now());
        assert!(
            model.query_error.is_some(),
            "admission must recheck after the click"
        );
        assert_eq!(model.applied_sort(), None);
        assert!(!model.can_sort_column(1));
        assert_eq!(model.budget.used(), 400);
        drop(blocker);
        model.command(
            &mut Document::new(),
            crate::panels::PanelId(1),
            TableCommand::Retry,
            Instant::now(),
        );
        finish_sort(&mut model);
        assert_eq!(
            model.budget.used(),
            400,
            "completed indices retain their reservation"
        );
        assert!(!model.can_sort_column(1));
        assert!(!model.cycle_sort(1));
        assert!(!model.can_sort_column(4));
        model.budget.set_limit(800);
        assert!(model.cycle_sort(1));
        finish_sort(&mut model);
        model.budget.set_limit(400);
        assert!(model.can_sort_column(1), "clearing requires no new index");
        assert!(model.cycle_sort(1));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while model.budget.used() != 0 {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert!(model.can_sort_column(4));
        model.budget.set_object_limit(399);
        assert!(!model.can_sort_column(4));
    }

    #[test]
    fn every_transaction_column_sorts_its_complete_typed_values() {
        let mut records = vec![
            transaction(1, 10, "z"),
            transaction(2, 20, "a"),
            transaction(3, 30, "m"),
        ];
        for (record, (duration, status, address)) in records.iter_mut().zip([
            (5, vtr::TxStatus::Open, 300),
            (100, vtr::TxStatus::Ok, 20),
            (10, vtr::TxStatus::Aborted, 100),
        ]) {
            record.end = record.begin + duration;
            record.status = status;
            record.attributes[1].value = AttributeValue::U64(address);
        }
        let mut model = generator_model(records);
        for (column, expected) in [
            [2, 3, 1],
            [1, 2, 3],
            [3, 2, 1],
            [3, 2, 1],
            [1, 2, 3],
            [3, 2, 1],
            [2, 3, 1],
        ]
        .into_iter()
        .enumerate()
        {
            model.cycle_sort(column);
            finish_sort(&mut model);
            let actual: Vec<_> = (0..3)
                .map(|rank| match model.identity_and_time(rank).unwrap().0 {
                    RowIdentity::Transaction(id) => id.0,
                    RowIdentity::SignalTime(_) => unreachable!(),
                })
                .collect();
            assert_eq!(actual, expected, "column {column}");
        }
    }

    #[test]
    fn newest_sort_wins_and_failed_admission_preserves_previous_order() {
        let mut model =
            generator_model((0..2000).map(|i| transaction(i, 2000 - i, "row")).collect());
        model.cycle_sort(1);
        model.tick(Instant::now());
        model.cycle_sort(4);
        model.cycle_sort(4);
        finish_sort(&mut model);
        assert_eq!(
            model.applied_sort(),
            Some(super::super::sort::Sort {
                column: 4,
                descending: true
            })
        );
        assert_eq!(
            model.identity_and_time(0).unwrap().0,
            RowIdentity::Transaction(TransactionRef(1999))
        );
        model.budget.set_limit(1);
        assert!(!model.can_sort_column(1));
        assert!(!model.cycle_sort(1));
        assert!(!model.querying());
        assert!(model.query_error.is_none());
        assert_eq!(
            model.identity_and_time(0).unwrap().0,
            RowIdentity::Transaction(TransactionRef(1999))
        );
        assert!(
            model.can_sort_column(4),
            "restoring source order needs no allocation"
        );
        model.budget.set_limit(16 * 1024 * 1024);
        assert!(model.can_sort_column(1));
        assert!(model.cycle_sort(1));
        finish_sort(&mut model);
        assert_eq!(model.applied_sort().unwrap().column, 1);
        model.cycle_sort(4);
        model.tick(Instant::now());
        model.command(
            &mut Document::new(),
            crate::panels::PanelId(1),
            TableCommand::Cancel,
            Instant::now(),
        );
        finish_sort(&mut model);
        assert_eq!(model.applied_sort().unwrap().column, 1);
    }

    #[test]
    fn body_column_edges_resize_after_scrolling_and_stay_inside_the_table() {
        let mut model = generator_model((1..=100).map(|id| transaction(id, id, "row")).collect());
        let theme = Theme::one_dark().zoomed(1.5);
        let bounds = Rect::from_xywh(20.0, 30.0, 800.0, 400.0);
        model.horizontal = 80.0;
        model.viewport.top = 20;
        model.layout(bounds, &theme);
        let column = model.layout.columns[0];
        let start = point(column.rect.right() - 1.0, model.layout.body.bottom() - 1.0);
        let mut doc = Document::new();
        let panel = crate::panels::PanelId(1);
        let now = Instant::now();
        assert!(model.pointer(
            &mut doc,
            panel,
            PointerEvent::Down {
                position: start,
                button: MouseButton::Left,
                modifiers: Default::default(),
            },
            now,
        ));
        assert_eq!(model.resizing_column(), Some(column.source_index));
        model.pointer(
            &mut doc,
            panel,
            PointerEvent::Move {
                position: point(start.x + 90.0, start.y + 100.0),
            },
            now,
        );
        model.pointer(&mut doc, panel, PointerEvent::Up, now);
        assert_eq!(
            model.widths[column.source_index],
            Some(column.rect.width() / theme.zoom + 60.0),
        );
        assert_eq!(model.selected, None);
        assert_eq!(model.viewport.top, 20);
        assert_eq!(model.horizontal, 80.0);
        assert!(!model.dragging());
        assert!(
            model
                .column_edge_at(point(start.x, model.layout.body.bottom()))
                .is_none()
        );
        assert!(
            model
                .column_edge_at(point(start.x, model.layout.header.top() - 1.0))
                .is_none()
        );

        model.horizontal = model.widths[column.source_index].unwrap() * theme.zoom;
        model.layout(bounds, &theme);
        let body = model.layout.body;
        let clipped_edge = point(body.left() - 2.0, body.top() + 5.0);
        assert!(model.layout.columns[0].resize.contains(clipped_edge));
        assert!(model.column_edge_at(clipped_edge).is_none());
        assert!(
            model
                .column_edge_at(point(body.right(), clipped_edge.y))
                .is_none()
        );
    }

    #[test]
    fn header_controls_remain_separate_at_minimum_width_after_scrolling_and_zooming() {
        use crate::scene::{MonoMeasure, Prim, Scene, TextCache};

        let key = "gpu.ενεργές_μονάδες".repeat(8);
        let mut record = transaction(1, 0, "row");
        record.attributes[1].key = key.clone();
        let mut model = generator_model(vec![record]);
        model.edit_columns(|c| c.toggle_transaction(TransactionColumn::Attribute(key.clone())));
        model.edit_columns(|c| c.move_transaction(7, Some(0)));
        model.resize_column(7, 1.0);
        for zoom in [0.75, 1.0, 1.5, 2.0] {
            for horizontal in [0.0, 20.0] {
                let theme = Theme::volna(true).zoomed(zoom);
                model.horizontal = horizontal;
                model.layout(Rect::from_xywh(0.0, 0.0, 360.0, 400.0), &theme);
                let column = model.layout.columns[0];
                let header = column.header;
                let format = header.format.unwrap();
                assert!(header.title.right() < header.sort_icon.left());
                assert!(header.sort_icon.right() < format.left());
                assert!(format.right() < column.resize.left());
                assert_eq!(
                    format.top() + format.height() * 0.5,
                    column.rect.top() + column.rect.height() * 0.5,
                );
                let mut scene = Scene::default();
                super::super::paint::paint(
                    &model,
                    &theme,
                    &mut TextCache::default(),
                    &mut MonoMeasure,
                    &mut scene,
                );
                let title = scene
                    .prims
                    .iter()
                    .find_map(|prim| match prim {
                        Prim::Text { origin, text, .. } if *origin == header.title.origin => {
                            Some(text)
                        }
                        _ => None,
                    })
                    .unwrap();
                assert!(title.ends_with('…'));
                assert!(
                    MonoMeasure.text_width(title, FontRole::Mono, theme.mono_size)
                        <= header.title.width()
                );
                let center = point(
                    format.left() + format.width() * 0.5,
                    format.top() + format.height() * 0.5,
                );
                assert!(!model.pointer(
                    &mut Document::new(),
                    crate::panels::PanelId(1),
                    PointerEvent::Down {
                        position: center,
                        button: MouseButton::Left,
                        modifiers: Default::default()
                    },
                    Instant::now(),
                ));
                assert!(model.sort_requested.is_none());
                assert!(!model.dragging());
            }
        }
    }

    #[test]
    fn header_arrows_follow_applied_sort_without_moving_the_format_control() {
        use crate::icons::IconName;
        use crate::scene::{MonoMeasure, Prim, Scene, TextCache};

        let mut model = generator_model(vec![transaction(2, 0, "row"), transaction(1, 1, "row")]);
        model.edit_columns(|c| c.toggle_transaction(TransactionColumn::Id));
        let theme = Theme::volna(true);
        model.layout(Rect::from_xywh(0.0, 0.0, 1400.0, 400.0), &theme);
        let header = model
            .layout
            .columns
            .iter()
            .find(|c| c.source_index == 4)
            .unwrap()
            .header;
        let arrow = |model: &TableModel| {
            let mut scene = Scene::default();
            super::super::paint::paint(
                model,
                &theme,
                &mut TextCache::default(),
                &mut MonoMeasure,
                &mut scene,
            );
            scene
                .prims
                .into_iter()
                .find_map(|prim| match prim {
                    Prim::Icon { name, rect, .. } if rect == header.sort_icon => Some(name),
                    _ => None,
                })
                .unwrap()
        };
        assert_eq!(arrow(&model), IconName::ArrowUpDown);
        for (expected, first_id) in [
            (IconName::ArrowUp, 1),
            (IconName::ArrowDown, 2),
            (IconName::ArrowUpDown, 2),
        ] {
            let previous = arrow(&model);
            assert!(model.cycle_sort(4));
            if model.querying() {
                assert_eq!(arrow(&model), previous);
            }
            finish_sort(&mut model);
            assert_eq!(arrow(&model), expected);
            assert_eq!(
                model.window.rows[0].identity,
                RowIdentity::Transaction(TransactionRef(first_id))
            );
            let current = model
                .layout
                .columns
                .iter()
                .find(|c| c.source_index == 4)
                .unwrap()
                .header;
            assert_eq!(current.format, header.format);
            assert_eq!(current.sort_icon, header.sort_icon);
        }
    }

    #[test]
    fn trailing_column_fills_space_until_dragged_and_reset_restores_auto_width() {
        let mut model = generator_model(vec![transaction(1, 0, "row")]);
        let theme = Theme::one_dark().zoomed(1.5);
        let bounds = Rect::from_xywh(20.0, 30.0, 1600.0, 240.0);
        model.layout(bounds, &theme);
        let attributes = *model.layout.columns.last().unwrap();
        assert!((attributes.rect.right() - model.layout.header.right()).abs() < 0.01);
        let start =
            crate::geometry::point(attributes.rect.right() - 2.0, attributes.rect.top() + 5.0);
        let mut doc = Document::new();
        let panel = crate::panels::PanelId(1);
        let now = Instant::now();
        assert!(model.pointer(
            &mut doc,
            panel,
            PointerEvent::Down {
                position: start,
                button: MouseButton::Left,
                modifiers: Default::default(),
            },
            now
        ));
        assert_eq!(model.resizing_column(), Some(attributes.source_index));
        model.pointer(
            &mut doc,
            panel,
            PointerEvent::Move {
                position: crate::geometry::point(start.x + 120.0, start.y + 300.0),
            },
            now,
        );
        model.pointer(&mut doc, panel, PointerEvent::Up, now);
        let width = attributes.rect.width() / theme.zoom + 80.0;
        assert!((model.widths[attributes.source_index].unwrap() - width).abs() < 0.01);
        assert_eq!(model.selected, None);
        assert!(!model.dragging());
        model.layout(bounds, &theme);
        assert!(model.layout.content_width > model.layout.header.width());
        assert_eq!(model.clone_view().widths.get(), model.widths.get());

        for _ in 0..2 {
            model.command(
                &mut doc,
                panel,
                TableCommand::ToggleTransactionColumn(TransactionColumn::Attributes),
                now,
            );
        }
        model.layout(Rect::from_xywh(0.0, 0.0, 2000.0, 240.0), &Theme::one_dark());
        assert!((model.layout.columns.last().unwrap().rect.width() - width).abs() < 0.01);
        model.command(&mut doc, panel, TableCommand::ResetColumns, now);
        model.layout(bounds, &theme);
        assert!(model.widths.iter().all(Option::is_none));
        assert!(
            (model.layout.columns.last().unwrap().rect.right() - model.layout.header.right()).abs()
                < 0.01
        );
    }

    #[test]
    fn widening_attributes_rebuilds_preview_and_keeps_unicode_and_rows_bounded() {
        let mut record = transaction(1, 0, "hidden label");
        record
            .attributes
            .extend((0..20).map(|index| TransactionAttribute {
                key: format!("field{index}"),
                value: AttributeValue::Text("α".repeat(20)),
            }));
        let records = (0..300)
            .map(|index| {
                let mut record = record.clone();
                record.id = TransactionRef(index + 1);
                record.begin = index;
                record.end = index + 8;
                record
            })
            .collect();
        let mut model = generator_model(records);
        let theme = Theme::one_dark();
        let bounds = Rect::from_xywh(0.0, 0.0, 1000.0, 10_000.0);
        model.layout(bounds, &theme);
        let first = &model.window.rows[0].cells.last().unwrap().text;
        assert!(!first.contains("field19="));
        assert!(model.resize_column(6, 4000.0));
        model.layout(bounds, &theme);
        let expanded = &model.window.rows[0].cells.last().unwrap().text;
        assert!(expanded.contains("field19="), "{expanded}");
        assert!(expanded.len() > PREVIEW_BYTES);
        assert!(!expanded.contains("vtr.label"));
        assert_eq!(model.window.rows.len(), MAX_PREPARED_ROWS as usize);
        assert!(model.window.bytes <= PANEL_BYTES as usize / 2);

        let mut record = transaction(1, 0, "row");
        record.attributes[1].key = "α".repeat(1000);
        let preview = model.transaction_cell(&record, 6, 32, None);
        assert!(preview.ends_with('…'));
        assert!(preview.len() <= 32);
    }

    #[test]
    fn fit_measures_header_and_visible_full_values_independently_of_preview_and_zoom() {
        let mut model = generator_model(vec![
            transaction(1, 0, "short"),
            transaction(2, 1, &"x".repeat(300)),
        ]);
        let bounds = Rect::from_xywh(0.0, 0.0, 1000.0, 100.0);
        let theme = Theme::one_dark();
        model.layout(bounds, &theme);
        let short = model
            .column_fit_width(0, &theme, &mut crate::scene::MonoMeasure)
            .unwrap();
        assert!(short < 210.0);
        model.viewport.subrow_px = 5.0;
        let long = model
            .column_fit_width(0, &theme, &mut crate::scene::MonoMeasure)
            .unwrap();
        assert!(long > 2100.0);
        let zoomed = theme.zoomed(2.0);
        model.layout(Rect::from_xywh(0.0, 0.0, 2000.0, 200.0), &zoomed);
        let zoomed_width = model
            .column_fit_width(0, &zoomed, &mut crate::scene::MonoMeasure)
            .unwrap();
        assert!((long - zoomed_width).abs() < 0.01);
        assert!(!model.resize_column(0, f32::NAN));
        assert!(!model.resize_column(usize::MAX, 100.0));
        model.resize_column(0, 0.0);
        assert_eq!(model.widths[0], Some(MIN_COLUMN_WIDTH));
        model.resize_column(0, f32::MAX);
        assert_eq!(model.widths[0], Some(MAX_COLUMN_WIDTH));
    }

    #[test]
    fn generator_window_is_bounded_and_selection_uses_identity() {
        let records = (1..=1000)
            .map(|id| transaction(id, id * 4, &format!("row {id}")))
            .collect();
        let generator = Arc::new(
            LoadedGenerator::new(
                volna_trace::data::transactions::TrackRef(7),
                records,
                HashMap::new(),
                vec![],
            )
            .unwrap(),
        );
        let mut model = TableModel::new(
            TableSource::Generator(TrackSource::Resolved {
                track: Traced::new(
                    crate::trace::TraceId::A,
                    volna_trace::data::transactions::TrackRef(7),
                ),
                path: vec!["soc".into(), "request".into()],
            }),
            Link::default(),
            MemoryBudget::new(16 * 1024 * 1024),
            Translators::builtin(),
        );
        model.rows = Rows::Generator(generator);
        model.state = TableState::Ready;
        model.layout.body = Rect::from_xywh(0.0, 0.0, 800.0, 10_000.0);
        model.layout.row_height = ROW_HEIGHT;
        model.prepare_visible();
        assert_eq!(model.window.rows.len(), MAX_PREPARED_ROWS as usize);
        model.selected = Some(RowIdentity::Transaction(TransactionRef(900)));
        assert_eq!(model.selected_ordinal(), Some(899));
        assert_eq!(
            model.copy_tsv().unwrap(),
            "Generator\tID\tBegin\tEnd\tDuration\tStatus\nsoc.request\t0x384\t3600\t3608\t8\tok"
        );
    }

    #[test]
    fn row_viewport_keeps_u64_positions_exact() {
        let mut viewport = RowViewport {
            top: u64::MAX - 1000,
            subrow_px: 0.0,
        };
        viewport.reveal(u64::MAX - 10, u64::MAX, 240.0, 24.0);
        assert_eq!(viewport.top, u64::MAX - 19);
        assert_eq!(viewport.row_at(216.0, u64::MAX, 24.0), Some(u64::MAX - 10));
    }

    #[test]
    fn row_viewport_scrolls_until_the_last_row_is_whole() {
        let mut viewport = RowViewport::default();
        viewport.scroll(f32::MAX / 2.0, 100, 100.0, 24.0);
        assert_eq!(viewport.top, 96);
        viewport.top = 0;
        viewport.reveal(4, 100, 100.0, 24.0);
        assert_eq!(viewport.top, 1, "a clipped row is not revealed");
    }

    #[test]
    fn normalized_scrollbar_math_preserves_u64_endpoints() {
        assert_eq!(normalized_u64(0.0, 997.0, u64::MAX), 0);
        assert_eq!(normalized_u64(997.0, 997.0, u64::MAX), u64::MAX);
        assert_eq!(
            normalized_u64(498.5, 997.0, u64::MAX),
            (u128::from(u64::MAX) / 2) as u64
        );
    }

    #[test]
    fn retry_reenters_panel_admission_after_capacity_is_released() {
        let budget = MemoryBudget::new(PANEL_BYTES);
        let blocker = budget.reserve(PANEL_BYTES).unwrap();
        let mut model = TableModel::new(
            TableSource::Signals(vec![signal_source("a", 1)]),
            Link::default(),
            budget.clone(),
            Translators::builtin(),
        );
        model
            .attach(&mut Document::new(), &Default::default())
            .unwrap();
        assert!(matches!(model.state, TableState::Refused(_)));
        drop(blocker);

        assert!(model.retry(&mut Document::new()));
        assert_eq!(budget.used(), PANEL_BYTES);
        assert!(matches!(model.state, TableState::Unavailable));
    }
}
