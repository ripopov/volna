//! Prepared transaction projections must respect their byte and item admissions.
use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
    collections::HashMap,
    sync::Arc,
};
use volna_core::{
    Document,
    testing::{ProceduralTrace, a},
    transaction::view::{self, VIEW_BYTES, ViewPrefs},
};
use volna_trace::{
    Session,
    data::{
        Hierarchy, SignalHistory, SignalRef, TraceInfo,
        loaded_tracks::{LoadedGenerator, LoadedTrack},
        transactions::*,
    },
    session::Capabilities,
};

struct Allocator;
thread_local! { static LIVE: Cell<Option<(isize, usize)>> = const { Cell::new(None) }; }
fn account(delta: isize) {
    LIVE.with(|v| {
        if let Some((live, peak)) = v.get() {
            let live = live + delta;
            v.set(Some((live, peak.max(live.max(0) as usize))));
        }
    });
}
unsafe impl GlobalAlloc for Allocator {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(l) };
        if !p.is_null() {
            account(l.size() as isize);
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        account(-(l.size() as isize));
        unsafe { System.dealloc(p, l) };
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        let q = unsafe { System.realloc(p, l, n) };
        if !q.is_null() {
            account(n as isize - l.size() as isize);
        }
        q
    }
}
#[global_allocator]
static ALLOCATOR: Allocator = Allocator;
struct Fixture {
    base: Arc<dyn Session>,
    tracks: Vec<Track>,
    loaded: LoadedTrack,
}
impl Session for Fixture {
    fn info(&self) -> &TraceInfo {
        self.base.info()
    }
    fn hierarchy(&self) -> &Hierarchy {
        self.base.hierarchy()
    }
    fn load_signal(&self, s: SignalRef) -> anyhow::Result<Arc<dyn SignalHistory>> {
        self.base.load_signal(s)
    }
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            waveforms: true,
            transactions: true,
            relations: true,
        }
    }
    fn tracks(&self) -> &[Track] {
        &self.tracks
    }
    fn load_track(&self, _: TrackRef) -> anyhow::Result<LoadedTrack> {
        Ok(self.loaded.clone())
    }
}
fn record() -> Transaction {
    Transaction {
        id: TransactionRef(1),
        generator: TrackRef(1),
        begin: 0,
        end: 10,
        status: TxStatus::Ok,
        kind: TxKind::Unspecified,
        parent: None,
        attributes: vec![],
        events: vec![],
        stages: vec![],
    }
}
fn project(tx: Transaction, path: String, limit: usize) -> (view::TxView, usize) {
    project_relations(tx, path, limit, vec![])
}
fn project_relations(
    tx: Transaction,
    path: String,
    limit: usize,
    relations: Vec<volna_trace::data::loaded_tracks::LoadedRelation>,
) -> (view::TxView, usize) {
    let loaded = LoadedTrack {
        track: TrackRef(1),
        generators: vec![Arc::new(
            LoadedGenerator::new(TrackRef(1), vec![tx], HashMap::new(), relations).unwrap(),
        )],
    };
    let tracks = vec![
        Track {
            id: TrackRef(0),
            path: vec![path.clone()],
            kind: TrackKind::Stream {
                kind: "test".into(),
            },
            attributes: vec![],
        },
        Track {
            id: TrackRef(1),
            path: vec![path],
            kind: TrackKind::Generator {
                stream: TrackRef(0),
            },
            attributes: vec![],
        },
    ];
    let mut doc = Document::new();
    doc.set_session(Arc::new(Fixture {
        base: ProceduralTrace::session(100),
        tracks,
        loaded,
    }));
    doc.retain_track(a(TrackRef(1))).unwrap();
    for r in doc.take_requests() {
        doc.deliver(r.perform());
    }
    let prefs = ViewPrefs {
        detail_items: limit,
        ..ViewPrefs::default()
    };
    LIVE.with(|v| v.set(Some((0, 0))));
    let view = view::view(&doc, a(TrackRef(1)), TransactionRef(1), &prefs).unwrap();
    let peak = LIVE.with(|v| v.replace(None).unwrap().1);
    (view, peak)
}
#[test]
fn recorded_names_are_admitted_before_projection() {
    let mut tx = record();
    tx.stages.push(TransactionStage {
        name: "s".repeat(140_000),
        lane: "l".repeat(140_000),
        begin: 0,
        end: Some(10),
        attributes: vec![],
    });
    tx.events.push(TransactionEvent {
        name: "e".repeat(140_000),
        time: 1,
        attributes: vec![],
    });
    let (view, peak) = project(tx, "generator".into(), 20);
    assert!(peak <= VIEW_BYTES, "projection allocated {peak} bytes");
    assert!(view.truncated_bytes);
}
#[test]
fn attribute_keys_and_enums_are_admitted_before_projection() {
    for value in [
        AttributeValue::U64(42),
        AttributeValue::Enum {
            value: 42,
            name: "n".repeat(140_000),
        },
    ] {
        let mut tx = record();
        tx.attributes.push(TransactionAttribute {
            key: "k".repeat(140_000),
            value,
        });
        let (_, peak) = project(tx, "generator".into(), 20);
        assert!(peak <= VIEW_BYTES, "projection allocated {peak} bytes");
    }
}
#[test]
fn catalog_paths_are_admitted_before_projection() {
    let (view, peak) = project(record(), "p".repeat(140_000), 20);
    assert!(peak <= VIEW_BYTES, "projection allocated {peak} bytes");
    assert!(view.truncated_bytes);
}
#[test]
fn item_limit_also_bounds_lifeline_and_ticks() {
    let mut tx = record();
    for i in 0..10_000 {
        tx.stages.push(TransactionStage {
            name: "s".into(),
            lane: "0".into(),
            begin: 0,
            end: Some(10),
            attributes: vec![],
        });
        tx.events.push(TransactionEvent {
            name: "e".into(),
            time: i,
            attributes: vec![],
        });
    }
    let (view, peak) = project(tx, "generator".into(), 2);
    assert!(view.lifeline.iter().map(|l| l.cells.len()).sum::<usize>() <= 2);
    assert!(view.event_ticks.len() <= 2);
    assert_eq!(view.stages.total, 10_000);
    assert_eq!(view.events.total, 10_000);
    assert!(peak <= VIEW_BYTES, "projection allocated {peak} bytes");
}
#[test]
fn nested_collection_storage_is_admitted() {
    let mut tx = record();
    tx.stages = (0..1000)
        .map(|_| TransactionStage {
            name: "".into(),
            lane: "0".into(),
            begin: 0,
            end: Some(10),
            attributes: (0..1000)
                .map(|_| (String::new(), AttributeValue::Text(String::new())))
                .collect(),
        })
        .collect();
    let (_, peak) = project(tx, "generator".into(), 1000);
    assert!(peak <= VIEW_BYTES, "projection allocated {peak} bytes");
}

#[test]
fn relation_order_and_captions_use_bounded_storage() {
    use volna_trace::data::loaded_tracks::LoadedRelation;
    let mut tx = record();
    tx.attributes.push(TransactionAttribute {
        key: "vtr.label".into(),
        value: AttributeValue::Text("caption".repeat(20_000)),
    });
    let relations = (0..10_000)
        .map(|id| LoadedRelation {
            id,
            from_generator: TrackRef(1),
            to_generator: TrackRef(1),
            relation: Relation {
                kind: if id == 0 {
                    "a".repeat(140_000)
                } else {
                    "z".into()
                },
                from: tx.id,
                to: tx.id,
                attributes: vec![],
            },
        })
        .collect();
    let (view, peak) = project_relations(tx, "generator".into(), 2, relations);
    assert_eq!(view.related.total, 10_000);
    assert_eq!(view.related.rows.len(), 2);
    assert!(view.related.rows[0].group.starts_with('a'));
    assert!(peak <= VIEW_BYTES, "projection allocated {peak} bytes");
}
