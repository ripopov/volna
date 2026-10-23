//! Parent graphs cross generator boundaries only after an explicit request.

use std::collections::HashMap;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use volna_core::{
    App, Command,
    panels::{PanelId, PanelsCommand},
    testing::{ProceduralTrace, a},
    transaction::{ParentTraceState, TransactionCommand},
};
use volna_trace::{
    Session,
    data::{
        Hierarchy, SignalHistory, SignalRef, TraceInfo,
        loaded_tracks::{LoadedGenerator, LoadedTrack, TransactionLocation},
        transactions::*,
    },
    session::Capabilities,
};

struct Ancestry {
    base: Arc<dyn Session>,
    tracks: Vec<Track>,
    generators: HashMap<TrackRef, Arc<LoadedGenerator>>,
    fail_parent: AtomicBool,
    budget: volna_trace::remote::memory::MemoryBudget,
}

impl Ancestry {
    fn new(depth: u32, same_track: bool, cycle: bool, missing_root: bool) -> Arc<Self> {
        let mut records = HashMap::<TrackRef, Vec<Transaction>>::new();
        let mut parents = HashMap::<TrackRef, HashMap<TransactionRef, TransactionLocation>>::new();
        for i in 1..=depth {
            let generator = TrackRef(if same_track { 1 } else { i });
            let parent = if i < depth {
                Some(i + 1)
            } else {
                cycle.then_some(1)
            };
            let location = parent.map(|p| TransactionLocation {
                transaction: TransactionRef(p.into()),
                generator: Some(TrackRef(if same_track { 1 } else { p })),
            });
            let txs = records.entry(generator).or_default();
            if missing_root && i == depth {
                continue;
            }
            let id = TransactionRef(i.into());
            txs.push(Transaction {
                id,
                generator,
                begin: 0,
                end: 10,
                status: Status::Ok,
                kind: SpanKind::Unspecified,
                parent: location.map(|p| p.transaction),
                attributes: vec![TransactionAttribute {
                    key: "vtr.label".into(),
                    value: AttributeValue::Text(format!("transaction {i}")),
                }],
                events: vec![],
                stages: vec![],
            });
            if let Some(location) = location {
                parents.entry(generator).or_default().insert(id, location);
            }
        }
        let mut tracks = vec![Track {
            id: TrackRef(0),
            path: vec!["ancestry".into()],
            kind: TrackKind::Generator {
                kind: "test".into(),
            },
            attributes: vec![],
        }];
        let generators = records
            .into_iter()
            .map(|(id, txs)| {
                tracks.push(Track {
                    id,
                    path: vec!["ancestry".into(), format!("generator {}", id.0)],
                    kind: TrackKind::Generator {
                        kind: "test".into(),
                    },
                    attributes: vec![],
                });
                (
                    id,
                    Arc::new(
                        LoadedGenerator::new(
                            id,
                            txs,
                            parents.remove(&id).unwrap_or_default(),
                            vec![],
                        )
                        .unwrap(),
                    ),
                )
            })
            .collect();
        Arc::new(Self {
            base: ProceduralTrace::session(100),
            tracks,
            generators,
            fail_parent: AtomicBool::new(false),
            budget: volna_trace::remote::memory::MemoryBudget::new(16 * 1024 * 1024),
        })
    }
}

impl Session for Ancestry {
    fn info(&self) -> &TraceInfo {
        self.base.info()
    }
    fn hierarchy(&self) -> &Hierarchy {
        self.base.hierarchy()
    }
    fn load_signal(&self, signal: SignalRef) -> anyhow::Result<Arc<dyn SignalHistory>> {
        self.base.load_signal(signal)
    }
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            waveforms: true,
            transactions: true,
            relations: false,
        }
    }
    fn tracks(&self) -> &[Track] {
        &self.tracks
    }
    fn memory_budget(&self) -> Option<volna_trace::remote::memory::MemoryBudget> {
        Some(self.budget.clone())
    }
    fn load_track(&self, track: TrackRef) -> anyhow::Result<LoadedTrack> {
        anyhow::ensure!(
            track != TrackRef(2) || !self.fail_parent.load(Ordering::Relaxed),
            "parent load failed"
        );
        Ok(LoadedTrack {
            track,
            generators: vec![self.generators[&track].clone()],
        })
    }
}

fn pump(app: &mut App) {
    loop {
        let requests = app.take_requests();
        if requests.is_empty() {
            break;
        }
        for request in requests {
            app.deliver(request.perform());
        }
    }
}

fn show(session: Arc<Ancestry>) -> (App, PanelId) {
    let mut app = App::new();
    app.set_session(session);
    let origin = app.panels.focused_id();
    app.handle(Command::SelectTransaction {
        panel: origin,
        track: a(TrackRef(1)),
        id: TransactionRef(1),
        cursor: None,
    });
    app.handle(Command::ShowTransaction { from: origin });
    let panel = app.panels.focused_id();
    pump(&mut app);
    (app, panel)
}

fn trace(app: &mut App, panel: PanelId) {
    app.handle(Command::Transaction(
        panel,
        TransactionCommand::TraceParents,
    ));
}

#[test]
fn parents_load_on_demand_without_changing_selection_and_are_released_on_hide() {
    let (mut app, panel) = show(Ancestry::new(3, false, false, false));
    assert_eq!(
        app.panels.transaction(panel).unwrap().parents().state(),
        &ParentTraceState::Idle
    );
    assert!(app.doc.track(a(TrackRef(2))).is_none());
    assert!(app.take_requests().is_empty());
    let selection = app.doc.selection();
    trace(&mut app, panel);
    assert_eq!(
        app.panels.transaction(panel).unwrap().parents().state(),
        &ParentTraceState::Loading
    );
    assert!(app.doc.track(a(TrackRef(3))).is_none());
    pump(&mut app);
    let graph = app.panels.transaction(panel).unwrap().parents();
    assert_eq!(graph.state(), &ParentTraceState::Ready);
    assert_eq!(
        graph
            .nodes()
            .iter()
            .map(|n| (n.track.item.0, n.id.0, n.label.as_str()))
            .collect::<Vec<_>>(),
        [
            (1, 1, "transaction 1"),
            (2, 2, "transaction 2"),
            (3, 3, "transaction 3")
        ]
    );
    assert_eq!(app.doc.selection(), selection);
    trace(&mut app, panel);
    assert!(
        app.take_requests().is_empty(),
        "a repeat click must not queue work"
    );
    app.handle(Command::Transaction(
        panel,
        TransactionCommand::ClearParents,
    ));
    assert!(app.doc.track(a(TrackRef(2))).is_none());
    assert!(app.doc.track(a(TrackRef(3))).is_none());
    assert!(app.doc.resident_generator(a(TrackRef(1))).is_some());
}

#[test]
fn the_full_chain_ignores_the_details_row_limit_and_a_root_has_one_vertex() {
    let (mut app, panel) = show(Ancestry::new(64, true, false, false));
    app.panels
        .transaction_mut(panel)
        .unwrap()
        .set_detail_items(1);
    trace(&mut app, panel);
    let graph = app.panels.transaction(panel).unwrap().parents();
    assert_eq!(graph.state(), &ParentTraceState::Ready);
    assert_eq!(graph.nodes().len(), 64);
    assert_eq!(graph.nodes().last().unwrap().id, TransactionRef(64));
    assert!(
        app.take_requests().is_empty(),
        "resident ancestors need no loads"
    );
    app.handle(Command::Transaction(
        panel,
        TransactionCommand::Jump {
            track: a(TrackRef(1)),
            id: TransactionRef(64),
        },
    ));
    assert_eq!(
        app.panels.transaction(panel).unwrap().parents().state(),
        &ParentTraceState::Idle
    );
    trace(&mut app, panel);
    assert_eq!(
        app.panels
            .transaction(panel)
            .unwrap()
            .parents()
            .nodes()
            .len(),
        1
    );
}

#[test]
fn cancel_navigation_and_close_discard_late_ancestry_completions() {
    for action in 0..3 {
        let (mut app, panel) = show(Ancestry::new(3, false, false, false));
        trace(&mut app, panel);
        let requests = app.take_requests();
        assert_eq!(requests.len(), 1);
        let result = requests.into_iter().next().unwrap().perform();
        match action {
            0 => app.handle(Command::Transaction(
                panel,
                TransactionCommand::ClearParents,
            )),
            1 => app.handle(Command::Transaction(
                panel,
                TransactionCommand::Jump {
                    track: a(TrackRef(3)),
                    id: TransactionRef(3),
                },
            )),
            _ => app.handle(Command::Panels(PanelsCommand::Close(panel))),
        }
        app.deliver(result);
        assert!(app.doc.track(a(TrackRef(2))).is_none());
        if action != 2 {
            assert_eq!(
                app.panels.transaction(panel).unwrap().parents().state(),
                &ParentTraceState::Idle
            );
        }
        pump(&mut app);
    }
}

#[test]
fn failed_parent_loads_can_be_retried() {
    let session = Ancestry::new(3, false, false, false);
    session.fail_parent.store(true, Ordering::Relaxed);
    let (mut app, panel) = show(session.clone());
    trace(&mut app, panel);
    pump(&mut app);
    assert!(
        matches!(app.panels.transaction(panel).unwrap().parents().state(), ParentTraceState::Failed(error) if error.contains("parent load failed"))
    );
    assert!(app.doc.track(a(TrackRef(2))).is_none());
    session.fail_parent.store(false, Ordering::Relaxed);
    trace(&mut app, panel);
    pump(&mut app);
    assert_eq!(
        app.panels.transaction(panel).unwrap().parents().state(),
        &ParentTraceState::Ready
    );
}

#[test]
fn cycles_missing_parents_and_budget_refusals_do_not_report_a_complete_graph() {
    for (cycle, missing, message) in [(true, false, "cycle"), (false, true, "missing")] {
        let (mut app, panel) = show(Ancestry::new(3, false, cycle, missing));
        trace(&mut app, panel);
        pump(&mut app);
        assert!(
            matches!(app.panels.transaction(panel).unwrap().parents().state(), ParentTraceState::Failed(error) if error.contains(message))
        );
        assert!(app.doc.track(a(TrackRef(2))).is_none());
        assert!(app.doc.track(a(TrackRef(3))).is_none());
    }
    let (mut app, panel) = show(Ancestry::new(3, false, false, false));
    let budget = app
        .doc
        .session(volna_core::trace::TraceId::A)
        .unwrap()
        .memory_budget()
        .unwrap();
    budget.set_limit(budget.used());
    trace(&mut app, panel);
    assert!(
        matches!(app.panels.transaction(panel).unwrap().parents().state(), ParentTraceState::Failed(error) if error.contains("budget"))
    );
    assert!(app.take_requests().is_empty());
}
