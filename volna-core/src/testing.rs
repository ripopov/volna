//! Shared procedural traces plus viewer completion helpers.

use crate::App;
use std::sync::Arc;
pub use volna_trace::testing::{ProceduralTrace, hierarchy_session};

use crate::session::{LoadRequest, LoadResult};
use volna_trace::Session;
pub fn complete_open(app: &mut App, session: Arc<dyn Session>) {
    loop {
        let requests = app.take_requests();
        if requests.is_empty() {
            return;
        }
        for request in requests {
            app.deliver(match request {
                LoadRequest::Open {
                    trace, generation, ..
                } => LoadResult::Opened {
                    trace,
                    generation,
                    result: Ok(session.clone()),
                },
                request => request.perform(),
            });
        }
    }
}
pub fn hierarchy_document(hierarchy: impl Into<volna_trace::data::Hierarchy>) -> crate::Document {
    let mut doc = crate::Document::new();
    doc.set_session(hierarchy_session(hierarchy));
    doc
}

pub fn a<T>(item: T) -> crate::trace::Traced<T> {
    crate::trace::Traced::new(crate::trace::TraceId::A, item)
}

pub fn paths(
    path: &str,
) -> impl Fn(&crate::trace::TraceSlot) -> anyhow::Result<Option<String>> + '_ {
    move |slot| {
        Ok(Some(if slot.id.is_a() {
            path.to_owned()
        } else {
            slot.source.clone()
        }))
    }
}

pub fn a_all<T>(items: impl IntoIterator<Item = T>) -> Vec<crate::trace::Traced<T>> {
    items.into_iter().map(a).collect()
}
