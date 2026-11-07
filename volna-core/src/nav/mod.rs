//! Shared or local viewport, cursor, and transaction selection for timed panels.

pub mod tween;
use crate::history::Before;
pub use tween::{Lerp, Tween};

use web_time::Instant;

use crate::clock::ClockKey;
use crate::document::{Document, RecordRange, TxSelection};
use crate::trace::{Rescale, Retime};
use crate::wave::viewport::Viewport;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Link {
    pub viewport: bool,
    pub cursor: bool,
    pub transaction: bool,
}

impl Default for Link {
    fn default() -> Self {
        Self {
            viewport: true,
            cursor: true,
            transaction: true,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkDim {
    Viewport,
    Cursor,
    Transaction,
}

#[derive(Clone)]
pub struct NavState {
    pub link: Link,
    pub local_viewport: Tween<Viewport>,
    pub local_cursor: Option<u64>,
    local_selection: Option<TxSelection>,
    local_selection_range: Option<RecordRange>,
    revealed_selection: Option<TxSelection>,
    clocks: crate::clock::ClockView,
    // Row selection is transient; the navigation clock stays active after deselection.
    ruler_selection: Option<ClockKey>,
    clocks_before: crate::history::Before<ClockChoice>,
    back: Option<Spot>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spot {
    pub cursor: Option<u64>,
    pub viewport: Viewport,
}

const REVEAL_EDGE: f64 = 0.05;

pub(crate) type ClockChoice = (Option<Vec<ClockKey>>, Option<u64>);

impl Retime for NavState {
    fn retime(&mut self, by: Rescale) {
        self.local_viewport.retime(by);
        self.local_cursor.retime(by);
        self.local_selection_range.retime(by);
        self.clocks.origin.retime(by);
        if let Some(back) = &mut self.back {
            back.cursor.retime(by);
            back.viewport.retime(by);
        }
    }
}

impl Default for NavState {
    fn default() -> Self {
        Self::new()
    }
}

impl NavState {
    #[must_use]
    pub fn new() -> Self {
        Self {
            link: Link::default(),
            local_viewport: Tween::new(Viewport::fit((0, 1000))),
            local_cursor: None,
            local_selection: None,
            local_selection_range: None,
            revealed_selection: None,
            clocks: crate::clock::ClockView::default(),
            ruler_selection: None,
            clocks_before: Before::default(),
            back: None,
        }
    }

    #[must_use]
    pub fn clone_view(&self) -> Self {
        Self {
            link: self.link,
            local_viewport: Tween::new(self.local_viewport.value),
            local_cursor: self.local_cursor,
            local_selection: self.local_selection,
            local_selection_range: self.local_selection_range,
            revealed_selection: None,
            clocks: self.clocks.clone(),
            ruler_selection: None,
            clocks_before: Before::default(),
            back: None,
        }
    }

    pub fn reset(&mut self, limits: Option<(u64, u64)>) {
        self.local_cursor = None;
        self.local_selection = None;
        self.local_selection_range = None;
        self.revealed_selection = None;
        self.ruler_selection = None;
        self.back = None;
        let viewport = limits.map_or(self.local_viewport.value, Viewport::fit);
        self.local_viewport.set(viewport);
    }

    #[must_use]
    pub fn viewport(&self, doc: &Document) -> Viewport {
        self.viewport_state(doc).value
    }

    #[must_use]
    pub fn viewport_state<'a>(&'a self, doc: &'a Document) -> &'a Tween<Viewport> {
        if self.link.viewport {
            &doc.shared.viewport
        } else {
            &self.local_viewport
        }
    }

    pub fn viewport_state_mut<'a>(&'a mut self, doc: &'a mut Document) -> &'a mut Tween<Viewport> {
        if self.link.viewport {
            &mut doc.shared.viewport
        } else {
            &mut self.local_viewport
        }
    }

    #[must_use]
    pub fn cursor(&self, doc: &Document) -> Option<u64> {
        if self.link.cursor {
            doc.shared.cursor
        } else {
            self.local_cursor
        }
    }

    pub fn set_cursor(&mut self, doc: &mut Document, cursor: Option<u64>) -> bool {
        let current = if self.link.cursor {
            &mut doc.shared.cursor
        } else {
            &mut self.local_cursor
        };
        let changed = *current != cursor;
        *current = cursor;
        changed
    }

    #[must_use]
    pub fn selection(&self, doc: &Document) -> Option<TxSelection> {
        if self.link.transaction {
            doc.selection()
        } else {
            self.local_selection
        }
    }

    #[must_use]
    pub fn selection_range(&self, doc: &Document) -> Option<RecordRange> {
        if self.link.transaction {
            doc.selection_range()
        } else {
            self.local_selection_range
        }
    }

    pub fn select(&mut self, doc: &mut Document, selection: Option<TxSelection>) -> bool {
        self.select_range(doc, selection, None)
    }

    pub fn select_range(
        &mut self,
        doc: &mut Document,
        selection: Option<TxSelection>,
        range: Option<RecordRange>,
    ) -> bool {
        if self.link.transaction {
            doc.select_range(selection, range)
        } else {
            let range = selection.and(range);
            let changed = self.local_selection != selection || self.local_selection_range != range;
            self.local_selection = selection;
            self.local_selection_range = range;
            changed
        }
    }

    pub(crate) fn selection_to_reveal(&self, doc: &Document) -> Option<TxSelection> {
        self.selection(doc)
            .filter(|selected| Some(*selected) != self.revealed_selection)
    }

    pub(crate) fn did_reveal_selection(&mut self, selected: Option<TxSelection>) {
        self.revealed_selection = selected;
    }

    pub fn toggle_link(&mut self, doc: &Document, dim: LinkDim) {
        match dim {
            LinkDim::Transaction => {
                self.local_selection = doc.selection();
                self.local_selection_range = doc.selection_range();
                self.link.transaction = !self.link.transaction;
                self.revealed_selection = if self.link.transaction {
                    None
                } else {
                    self.local_selection
                };
            }
            LinkDim::Viewport => {
                self.local_viewport.set(doc.shared.viewport.value);
                self.link.viewport = !self.link.viewport;
            }
            LinkDim::Cursor => {
                self.local_cursor = doc.shared.cursor;
                self.link.cursor = !self.link.cursor;
            }
        }
    }

    pub fn tick(&mut self, now: Instant) -> bool {
        !self.link.viewport && self.local_viewport.tick(now)
    }

    #[must_use]
    pub fn is_animating(&self) -> bool {
        !self.link.viewport && self.local_viewport.is_animating()
    }

    pub fn animate_to(&mut self, doc: &mut Document, mut target: Viewport, now: Instant) {
        target.clamp(doc.limits());
        let mode = doc.navigation.animation;
        self.viewport_state_mut(doc).animate_to(target, now, mode);
    }

    pub fn jump_to(&mut self, doc: &mut Document, mut target: Viewport) {
        target.clamp(doc.limits());
        self.viewport_state_mut(doc).set(target);
    }

    pub fn zoom_at(&mut self, doc: &mut Document, x_px: f64, width_px: f64, factor: f64) {
        let mut target = self.viewport(doc);
        target.zoom_about(x_px, width_px, factor, doc.limits());
        self.viewport_state_mut(doc).set(target);
    }

    pub fn zoom_target_at(
        &mut self,
        doc: &mut Document,
        x_px: f64,
        width_px: f64,
        factor: f64,
        now: Instant,
    ) {
        let mut target = self.viewport_state(doc).target();
        target.zoom_about(x_px, width_px, factor, doc.limits());
        self.animate_to(doc, target, now);
    }

    pub fn zoom_center(&mut self, doc: &mut Document, width_px: f64, factor: f64, now: Instant) {
        let target = self.viewport_state(doc).target();
        let anchor_x = match self.cursor(doc) {
            Some(c) => {
                let x = target.x_of(c as f64, width_px);
                if (0.0..=width_px).contains(&x) {
                    x
                } else {
                    width_px / 2.0
                }
            }
            None => width_px / 2.0,
        };
        self.zoom_target_at(doc, anchor_x, width_px, factor, now);
    }

    pub fn zoom_fit(&mut self, doc: &mut Document, width_px: f64, margin_px: f64, now: Instant) {
        let target = Viewport::fit_px(doc.limits(), width_px, margin_px);
        self.animate_to(doc, target, now);
    }

    pub fn zoom_to_cursor(&mut self, doc: &mut Document, now: Instant) {
        let Some(cursor) = self.cursor(doc) else {
            return;
        };
        let half_width = self.viewport_state(doc).target().width() / 4.0;
        let target = Viewport {
            start: cursor as f64 - half_width,
            end: cursor as f64 + half_width,
        };
        self.animate_to(doc, target, now);
    }

    pub fn go_to_start(&mut self, doc: &mut Document, now: Instant) {
        let mut target = self.viewport_state(doc).target();
        target.go_to_start(doc.limits());
        self.animate_to(doc, target, now);
    }

    pub fn go_to_end(&mut self, doc: &mut Document, now: Instant) {
        let mut target = self.viewport_state(doc).target();
        target.go_to_end(doc.limits());
        self.animate_to(doc, target, now);
    }

    pub fn go_to_cursor(&mut self, doc: &mut Document, now: Instant) {
        let Some(c) = self.cursor(doc) else { return };
        let mut target = self.viewport_state(doc).target();
        target.center_on(c as f64, doc.limits());
        self.animate_to(doc, target, now);
    }

    pub fn pan_fraction(&mut self, doc: &mut Document, frac: f64, now: Instant) {
        let mut target = self.viewport_state(doc).target();
        let w = target.width();
        target.start += w * frac;
        target.end += w * frac;
        self.animate_to(doc, target, now);
    }

    pub fn pan_px(&mut self, doc: &mut Document, dx_px: f64, width_px: f64) {
        let mut target = self.viewport(doc);
        target.pan_px(dx_px, width_px, doc.limits());
        self.viewport_state_mut(doc).set(target);
    }

    #[must_use]
    pub fn selected_clock(
        &self,
        doc: &Document,
    ) -> Option<std::sync::Arc<crate::clock::ClockTimeline>> {
        self.clocks.selected(&doc.clocks)?.timeline().cloned()
    }

    pub fn step_cycle(&mut self, doc: &mut Document, forward: bool, now: Instant) -> bool {
        let Some(timeline) = self.selected_clock(doc) else {
            return false;
        };
        let vp = self.viewport(doc);
        let to = match (self.cursor(doc), forward) {
            (Some(c), true) => timeline.next_edge(c),
            (Some(c), false) => timeline.prev_edge(c),
            (None, true) => timeline.next_edge(vp.start.max(0.0) as u64),
            (None, false) => timeline.prev_edge(vp.end.max(0.0) as u64 + 1),
        };
        let Some(to) = to else { return false };
        self.set_cursor(doc, Some(to));
        self.reveal_cursor(doc, now);
        true
    }

    pub fn go_to_cycle(
        &mut self,
        doc: &mut Document,
        shown: i64,
        now: Instant,
    ) -> Result<(), String> {
        let clock = self
            .clocks
            .selected(&doc.clocks)
            .ok_or("no clock to count cycles in")?;
        let name = clock.name.clone();
        let timeline = clock
            .timeline()
            .ok_or("the clock is still loading")?
            .clone();
        let time = self
            .clocks
            .absolute_cycle(&timeline, shown)
            .and_then(|c| timeline.edge(c))
            .ok_or_else(|| format!("{name} has no cycle {shown}"))?;
        self.set_cursor(doc, Some(time));
        self.go_to_cursor(doc, now);
        Ok(())
    }

    pub fn toggle_cycle_origin(&mut self, doc: &Document) {
        let cursor = self.cursor(doc);
        self.note_clocks();
        self.clocks.origin = if self.clocks.origin == cursor {
            None
        } else {
            cursor
        };
    }

    pub fn select_clock(&mut self, key: ClockKey) {
        self.clocks.selected = Some(key);
    }

    pub fn select_ruler(&mut self, key: ClockKey) {
        self.select_clock(key.clone());
        self.ruler_selection = Some(key);
    }

    #[must_use]
    pub fn ruler_selection(&self) -> Option<&ClockKey> {
        self.ruler_selection.as_ref()
    }

    pub fn clear_ruler_selection(&mut self) {
        self.ruler_selection = None;
    }

    #[must_use]
    pub fn clocks(&self) -> &crate::clock::ClockView {
        &self.clocks
    }

    pub fn show_ruler(&mut self, clocks: &crate::clock::Clocks, key: &ClockKey) {
        self.note_clocks();
        self.clocks.show_ruler(clocks, key);
    }

    pub fn hide_ruler(&mut self, clocks: &crate::clock::Clocks, key: &ClockKey) {
        self.note_clocks();
        self.clocks.hide_ruler(clocks, key);
        if self.ruler_selection.as_ref() == Some(key) {
            self.clear_ruler_selection();
        }
    }

    pub fn toggle_ruler(&mut self, clocks: &crate::clock::Clocks, key: &ClockKey) {
        self.note_clocks();
        self.clocks.toggle_ruler(clocks, key);
        if self.ruler_selection.as_ref() == Some(key) {
            self.clear_ruler_selection();
        }
    }

    pub(crate) fn forget_trace(&mut self, trace: crate::trace::TraceId) {
        if self
            .ruler_selection
            .as_ref()
            .is_some_and(|key| key.trace == trace)
        {
            self.clear_ruler_selection();
        }
        if self.local_selection.is_some_and(|s| s.track.trace == trace) {
            self.local_selection = None;
            self.local_selection_range = None;
        }
        if self
            .revealed_selection
            .is_some_and(|s| s.track.trace == trace)
        {
            self.revealed_selection = None;
        }
        let mut clocks = self.clocks.clone();
        if clocks.forget_trace(trace) {
            self.note_clocks();
        }
        self.clocks = clocks;
    }

    fn note_clocks(&mut self) {
        let clocks = &self.clocks;
        self.clocks_before
            .note(|| (clocks.rulers.clone(), clocks.origin));
    }

    pub(crate) fn restore_clocks(&mut self, view: crate::clock::ClockView) {
        self.clear_ruler_selection();
        self.clocks = view;
        self.clocks_before.take();
    }

    pub(crate) fn swap_clocks(&mut self, (rulers, origin): ClockChoice) -> ClockChoice {
        debug_assert!(!self.clocks_before.is_held(), "edits were not collected");
        let old = (
            std::mem::replace(&mut self.clocks.rulers, rulers),
            std::mem::replace(&mut self.clocks.origin, origin),
        );
        if let Some(selected) = &self.clocks.selected
            && self
                .clocks
                .rulers
                .as_ref()
                .is_some_and(|r| !r.contains(selected))
        {
            self.clocks.selected = None;
        }
        if self.ruler_selection.as_ref().is_some_and(|key| {
            self.clocks
                .rulers
                .as_ref()
                .is_some_and(|rulers| !rulers.contains(key))
        }) {
            self.clear_ruler_selection();
        }
        old
    }

    pub(crate) fn take_edits(
        &mut self,
        panel: crate::panels::PanelId,
        clocks: &crate::clock::Clocks,
        history: &mut crate::history::History,
    ) {
        let Some((rulers, origin)) = self.clocks_before.take() else {
            return;
        };
        if (&rulers, &origin) == (&self.clocks.rulers, &self.clocks.origin) {
            return;
        }
        let label = clocks_label(clocks, rulers.as_deref(), origin, &self.clocks);
        history.record(
            crate::history::Edit::Prop {
                panel,
                prop: crate::panels::content::Prop::Clocks { rulers, origin },
            },
            Some(label),
        );
    }

    #[must_use]
    pub fn spot(&self, doc: &Document) -> Spot {
        Spot {
            cursor: self.cursor(doc),
            viewport: self.viewport_state(doc).target(),
        }
    }

    pub fn jump_cursor(&mut self, doc: &mut Document, t: u64, now: Instant) -> bool {
        if self.cursor(doc) == Some(t) {
            self.reveal(doc, t, now);
            return false;
        }
        self.back = Some(self.spot(doc));
        self.set_cursor(doc, Some(t));
        self.reveal(doc, t, now);
        true
    }

    pub fn back(&mut self, doc: &mut Document, now: Instant) -> bool {
        let Some(spot) = self.back.take() else {
            return false;
        };
        self.back = Some(self.spot(doc));
        self.set_cursor(doc, spot.cursor);
        self.animate_to(doc, spot.viewport, now);
        true
    }

    pub fn reveal(&mut self, doc: &mut Document, t: u64, now: Instant) -> bool {
        let mut target = self.viewport_state(doc).target();
        let edge = target.width() * REVEAL_EDGE;
        let t = t as f64;
        if (target.start + edge..=target.end - edge).contains(&t) {
            return false;
        }
        target.center_on(t, doc.limits());
        self.animate_to(doc, target, now);
        true
    }

    pub(crate) fn reveal_record(&mut self, doc: &mut Document, begin: u64, end: u64, now: Instant) {
        let target = self.viewport_state(doc).target();
        if begin as f64 <= target.end && end as f64 >= target.start {
            return;
        }
        self.reveal(doc, begin, now);
    }

    pub fn reveal_cursor(&mut self, doc: &mut Document, now: Instant) {
        let Some(c) = self.cursor(doc) else { return };
        let c = c as f64;
        let mut target = self.viewport_state(doc).target();
        if c < target.start || c > target.end {
            target.center_on(c, doc.limits());
            self.animate_to(doc, target, now);
        }
    }
}

fn clocks_label(
    clocks: &crate::clock::Clocks,
    rulers: Option<&[ClockKey]>,
    origin: Option<u64>,
    now: &crate::clock::ClockView,
) -> String {
    let before = rulers.unwrap_or(&clocks.defaults);
    let after = now.ruler_keys(clocks);
    let name = |key: &ClockKey| key.item.rsplit('.').next().unwrap_or(&key.item).to_owned();
    if let Some(shown) = after.iter().find(|p| !before.contains(p)) {
        return format!("Show ruler {}", name(shown));
    }
    if let Some(hidden) = before.iter().find(|p| !after.contains(p)) {
        return format!("Hide ruler {}", name(hidden));
    }
    match (origin, now.origin) {
        (_, Some(_)) if origin != now.origin => "Set cycle origin".into(),
        (Some(_), None) => "Clear cycle origin".into(),
        _ => "Clock rulers".into(),
    }
}
