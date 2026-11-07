//! Cancelled searches cannot publish into a newer query or editor.

use super::*;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use volna_core::computed::{RecordedMatch, RecordedSearch};

#[derive(Clone, PartialEq, Eq)]
struct SearchKey {
    query: String,
    generation: u64,
    traces: u64,
    sampling: bool,
}
struct SearchTask {
    cancelled: Arc<AtomicBool>,
    _task: gpui_kit::Task<()>,
}
impl Drop for SearchTask {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
}
#[derive(Default)]
pub(super) struct InputSearch {
    key: Option<SearchKey>,
    pub results: Option<Vec<RecordedMatch>>,
    task: Option<SearchTask>,
}
impl DerivedEditor {
    pub(super) fn close_picker(&mut self) {
        self.picker = None;
        self.input_search.task = None;
    }
    pub(super) fn open_picker(
        &mut self,
        target: InputPicker,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.picker = Some(target);
        self.search
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.sync_search(cx);
        self.search.read(cx).focus_handle(cx).focus(window, cx);
        cx.notify();
    }
    pub(super) fn sync_search(&mut self, cx: &mut Context<Self>) {
        if self.picker.is_none() {
            return;
        }
        let Some(owner) = self.owner.upgrade() else {
            return;
        };
        let ws = owner.read(cx);
        if ws
            .app
            .derived_editor
            .as_ref()
            .is_none_or(|e| e.id != self.id)
        {
            return;
        }
        let doc = &ws.app.doc;
        let key = SearchKey {
            query: self.search.read(cx).value().to_lowercase(),
            generation: doc.generation(),
            traces: doc.traces().revision(),
            sampling: self.picker == Some(InputPicker::Sampling),
        };
        if self.input_search.key.as_ref() == Some(&key)
            && (self.input_search.results.is_some() || self.input_search.task.is_some())
        {
            return;
        }
        self.input_search.task = None;
        let work = RecordedSearch::new(doc, &key.query);
        let work = if key.sampling {
            work.sampling_only()
        } else {
            work
        };
        self.input_search.key = Some(key.clone());
        self.input_search.results = None;
        let cancelled = Arc::new(AtomicBool::new(false));
        let worker_cancelled = cancelled.clone();
        let task = cx.spawn(async move |this, cx| {
            let results = cx
                .background_spawn(async move { work.run(&worker_cancelled) })
                .await;
            this.update(cx, |view, cx| {
                let current = view.owner.upgrade().is_some_and(|owner| {
                    let app = &owner.read(cx).app;
                    app.derived_editor
                        .as_ref()
                        .is_some_and(|editor| editor.id == view.id)
                        && app.doc.generation() == key.generation
                        && app.doc.traces().revision() == key.traces
                });
                if current && view.picker.is_some() && view.input_search.key.as_ref() == Some(&key)
                {
                    view.input_search.results = Some(results);
                    view.input_search.task = None;
                    cx.notify();
                }
            })
            .ok();
        });
        self.input_search.task = Some(SearchTask {
            cancelled,
            _task: task,
        });
        cx.notify();
    }
}
