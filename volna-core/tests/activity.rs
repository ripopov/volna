//! Signal and scope activity tests covering exact counts, sidecar builds, and stale results.

use vtr::Recorder;

#[cfg(unix)]
#[path = "../../volna-server/src/server.rs"]
mod server;

use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;

use volna_core::app::App;
use volna_core::data::ScopeActivity;
use volna_core::session::{LoadRequest, LoadResult};
use volna_core::sidebar::TreeNode;
use volna_core::trace::{TraceId, Traced};
use volna_core::wave::Viewport;
use volna_trace::data::SignalRef;
use volna_trace::session::{OpenSpec, Session};
use vtr::{Direction, ScopeType, SignalKind, VarType, Writer, WriterOptions};

fn write_trace(path: &Path, seed: u64) {
    let mut opts = WriterOptions::default();
    opts.encoding.waveform_records = 400;
    opts.metadata.timescale = -9;
    let mut w = Writer::create_with(path, opts).unwrap();
    let bit = SignalKind::Bits {
        width: 1,
        states: 2,
    };
    let byte = SignalKind::Bits {
        width: 8,
        states: 2,
    };
    let top = Some(
        w.scope_with(
            vtr::SourceOptions::new("top", None),
            ScopeType::Module,
            "top",
        )
        .unwrap(),
    );
    let clk = w
        .signal(
            vtr::SourceOptions::new("clk", top),
            bit,
            VarType::Wire,
            Direction::Input,
        )
        .unwrap();
    let mut units = Vec::new();
    for u in 0..3 {
        let unit = Some(
            w.scope_with(
                vtr::SourceOptions::new(format!("unit{u}"), top),
                ScopeType::Module,
                "unit",
            )
            .unwrap(),
        );
        w.alias(
            clk,
            vtr::SourceOptions::new("clk", unit),
            VarType::Wire,
            Direction::Input,
        )
        .unwrap();
        let a = w
            .signal(
                vtr::SourceOptions::new("a", unit),
                byte,
                VarType::Reg,
                Direction::Implicit,
            )
            .unwrap();
        let core = Some(
            w.scope_with(
                vtr::SourceOptions::new("core", unit),
                ScopeType::Module,
                "core",
            )
            .unwrap(),
        );
        let b = w
            .signal(
                vtr::SourceOptions::new("b", core),
                bit,
                VarType::Reg,
                Direction::Implicit,
            )
            .unwrap();
        let c = w
            .signal(
                vtr::SourceOptions::new("c", core),
                byte,
                VarType::Reg,
                Direction::Implicit,
            )
            .unwrap();
        units.push([a, b, c]);
    }
    let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    let mut rand = move |n: u64| {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x % n
    };
    for step in 0..3000u64 {
        let t = step * 10 + if step >= 1500 { 50_000 } else { 0 };
        w.advance_time(t).unwrap();
        w.emit_u64(clk, u64::from(((step & 1) as u8) != 0)).unwrap();
        for (u, sigs) in units.iter().enumerate() {
            if u == 2 && step >= 1500 {
                continue;
            }
            for &s in sigs {
                if rand(10 + 20 * u as u64) == 0 {
                    w.emit_u64(s, rand(200)).unwrap();
                }
            }
        }
    }
    w.close().unwrap();
}

fn index_beside(trace: &Path) {
    let r = vtr::Reader::open(trace).unwrap();
    let id = vtr::activity::Identity::of(&r).unwrap();
    vtr::activity::Sidecar::new(trace, &id, None)
        .write(|w| vtr::activity::build(&r, w, &vtr::activity::BuildOptions::default()))
        .unwrap();
}

fn settle(app: &mut App) {
    for _ in 0..20 {
        let requests = app.take_requests();
        if requests.is_empty() {
            return;
        }
        for r in requests {
            app.deliver(r.perform());
        }
    }
    panic!("requests keep coming");
}

fn open(path: &Path) -> (App, Arc<dyn Session>) {
    open_with_settings(path, "{}")
}

fn open_asking(path: &Path) -> (App, Arc<dyn Session>) {
    open_with_settings(path, r#"{"hierarchy.activityIndex":"ask"}"#)
}

fn open_with_settings(path: &Path, settings: &str) -> (App, Arc<dyn Session>) {
    let session = OpenSpec::Path(path.to_path_buf()).open().unwrap();
    let mut app = App::new();
    app.settings_loaded(settings);
    app.set_session(session.clone());
    settle(&mut app);
    (app, session)
}

fn brute(
    session: &Arc<dyn Session>,
    t_min: u64,
    t0: u64,
    t1: u64,
) -> (Vec<u32>, HashSet<SignalRef>) {
    let h = session.hierarchy();
    let signals: HashSet<SignalRef> = (0..h.var_count()).map(|v| h.var(v).signal).collect();
    let signals: Vec<SignalRef> = signals.into_iter().collect();
    let changing: HashSet<SignalRef> = session
        .load_signals(&signals)
        .into_iter()
        .filter(|(_, history)| {
            let history = history.as_ref().unwrap();
            (0..history.len())
                .map(|i| history.time(i))
                .any(|t| t > t_min && t >= t0 && t <= t1)
        })
        .map(|(s, _)| s)
        .collect();
    let scopes = (0..h.scope_count())
        .map(|scope| {
            let (mut below, mut stack) = (HashSet::new(), vec![scope]);
            while let Some(s) = stack.pop() {
                below.extend(
                    h.scope(s)
                        .vars
                        .iter()
                        .map(|v| h.var(v).signal)
                        .filter(|s| changing.contains(s)),
                );
                stack.extend(h.scope(s).children);
            }
            below.len() as u32
        })
        .collect();
    (scopes, changing)
}

fn node(scope: usize) -> TreeNode {
    TreeNode::scope(Traced::new(TraceId::A, scope))
}

fn view(app: &mut App, t0: u64, t1: u64) {
    app.doc.shared.viewport.set(Viewport {
        start: t0 as f64,
        end: t1 as f64,
    });
}

fn check_exact(app: &App, session: &Arc<dyn Session>, t0: u64, t1: u64) {
    let t_min = session.activity().unwrap().t_min();
    let (want, signals) = brute(session, t_min, t0, t1);
    for (scope, &n) in want.iter().enumerate() {
        let a = app.scope_activity(node(scope)).unwrap();
        assert_eq!(a.changing, n, "[{t0}, {t1}] scope {scope}");
    }
    for var in session.hierarchy().vars() {
        assert_eq!(
            app.signal_activity(Traced::new(TraceId::A, var.signal)),
            Some(signals.contains(&var.signal)),
            "[{t0}, {t1}] signal {:?}",
            var.signal
        );
    }
}

#[test]
fn meters_count_the_viewport_exactly() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("run.vtr");
    write_trace(&path, 1);
    index_beside(&path);
    let (mut app, session) = open(&path);
    assert!(session.activity().is_some());
    let top = node(session.hierarchy().roots().first().unwrap());
    for (t0, t1) in [
        (0, 80_000),
        (100, 1_000),
        (14_990, 15_010),
        (20_000, 21_000),
        (64_000, 65_005),
        (70_003, 70_004),
    ] {
        view(&mut app, t0, t1);
        settle(&mut app);
        check_exact(&app, &session, t0, t1);
    }
    view(&mut app, 70_000, 70_200);
    settle(&mut app);
    let h = session.hierarchy();
    let quiet: Vec<String> = (0..h.scope_count())
        .filter(|&s| app.scope_activity(node(s)).unwrap().quiet())
        .map(|s| h.scope_path(s).join("."))
        .collect();
    assert!(quiet.contains(&"top.unit2.core".to_string()), "{quiet:?}");
    assert!(!app.scope_activity(top).unwrap().quiet());
    assert_eq!(app.scope_activity(TreeNode::trace(TraceId::A)), None);
}

#[test]
fn a_narrow_view_is_exact_without_waveform_reads() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("run.vtr");
    write_trace(&path, 2);
    index_beside(&path);
    let (mut app, session) = open(&path);
    let (t0, t1) = (5_003, 5_004);
    view(&mut app, t0, t1);
    let requests = app.take_requests();
    assert!(matches!(
        requests.as_slice(),
        [LoadRequest::Activity { .. }]
    ));
    for request in requests {
        app.deliver(request.perform());
    }
    check_exact(&app, &session, t0, t1);
    assert!(app.take_requests().is_empty());
}

#[test]
fn fast_pans_coalesce_and_drop_old_classifications() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("run.vtr");
    write_trace(&path, 3);
    index_beside(&path);
    let (mut app, session) = open(&path);
    let top = node(session.hierarchy().roots().first().unwrap());
    view(&mut app, 100, 1_000);
    let classification = app.take_requests().pop().unwrap().perform();
    for t0 in [2_000, 3_000, 4_000] {
        view(&mut app, t0, t0 + 100);
        assert!(
            app.take_requests().is_empty(),
            "only one classification in flight"
        );
    }
    app.deliver(classification);
    assert_eq!(
        app.signal_activity(Traced::new(TraceId::A, SignalRef(0))),
        None
    );
    assert_eq!(
        app.scope_activity(top),
        None,
        "stale classifications stay hidden"
    );
    let requests = app.take_requests();
    assert!(matches!(
        requests.as_slice(),
        [LoadRequest::Activity {
            window: (4_000, 4_100),
            ..
        }]
    ));
    for r in requests {
        app.deliver(r.perform());
    }
    settle(&mut app);
    check_exact(&app, &session, 4_000, 4_100);
}

#[test]
fn replacing_a_trace_rejects_results_before_the_next_frame() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("run.vtr");
    write_trace(&path, 3);
    index_beside(&path);
    let (mut app, session) = open(&path);
    let top = node(session.hierarchy().roots().first().unwrap());
    view(&mut app, 100, 1_000);
    let classification = app.take_requests().pop().unwrap().perform();
    app.set_session(Arc::clone(&session));
    assert_eq!(app.scope_activity(top), None);
    assert_eq!(
        app.signal_activity(Traced::new(TraceId::A, SignalRef(0))),
        None
    );
    app.deliver(classification);
    assert_eq!(app.scope_activity(top), None);
    assert_eq!(
        app.signal_activity(Traced::new(TraceId::A, SignalRef(0))),
        None
    );
    settle(&mut app);
    let viewport = app.activity_viewport();
    check_exact(&app, &session, viewport.start as u64, viewport.end as u64);
}

#[test]
fn restoring_a_workspace_before_scope_sizes_arrive_keeps_activity_meters() {
    use volna_core::workspace::Workspace;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("run.vtr");
    write_trace(&path, 3);
    index_beside(&path);
    let session = OpenSpec::Path(path).open().unwrap();
    for initial in 0..3 {
        let mut app = App::new();
        app.set_session(session.clone());
        if initial == 2 {
            settle(&mut app);
        }
        let stale = (initial == 1).then(|| {
            app.take_requests()
                .into_iter()
                .find(|r| matches!(r, LoadRequest::Sizes { .. }))
                .expect("initial scope count")
                .perform()
        });
        let saved = Workspace::capture(&app, volna_core::testing::paths("run.vtr"), None).unwrap();
        saved
            .prepare(
                &app,
                "file:///tmp/run.vtr",
                "file:///tmp/run.vtr.volna.json",
            )
            .unwrap()
            .commit(&mut app)
            .unwrap();
        if let Some(stale) = stale {
            app.deliver(stale);
            assert!(app.doc.traces().get(TraceId::A).unwrap().sizes().is_none());
        }
        let requests = app.take_requests();
        assert_eq!(
            requests
                .iter()
                .filter(|r| matches!(r, LoadRequest::Sizes { .. }))
                .count(),
            usize::from(initial != 2),
            "only unfinished counts restart"
        );
        for request in requests {
            app.deliver(request.perform());
        }
        settle(&mut app);
        let window = app.activity_viewport();
        check_exact(
            &app,
            &session,
            window.start.max(0.0) as u64,
            window.end as u64,
        );
    }
}

#[test]
fn meters_follow_the_focused_panels_unlinked_viewport() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("run.vtr");
    write_trace(&path, 3);
    index_beside(&path);
    let (mut app, session) = open(&path);
    app.handle(volna_core::app::Command::AddVars(vec![Traced::new(
        TraceId::A,
        0,
    )]));
    view(&mut app, 0, 80_000);
    let nav = app.panels.focused_mut().kind.nav_mut().unwrap();
    nav.link.viewport = false;
    nav.local_viewport.set(Viewport {
        start: 70_000.0,
        end: 70_200.0,
    });
    settle(&mut app);
    check_exact(&app, &session, 70_000, 70_200);
    app.panels
        .focused_mut()
        .kind
        .nav_mut()
        .unwrap()
        .link
        .viewport = true;
    settle(&mut app);
    check_exact(&app, &session, 0, 80_000);
}

#[test]
fn meters_use_each_traces_timescale_and_round_inwards() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("run.vtr");
    write_trace(&path, 3);
    index_beside(&path);
    let (mut app, session) = open(&path);
    let fine = dir.path().join("fine.vtr");
    let mut writer_options = vtr::WriterOptions::default();
    writer_options.metadata.timescale = -12;
    let mut writer = Writer::create_with(&fine, writer_options).unwrap();
    writer.close().unwrap();
    let fine = OpenSpec::Path(fine).open().unwrap();
    app.doc.add_session(fine).unwrap();
    assert_eq!(
        app.doc
            .traces()
            .get(TraceId::A)
            .unwrap()
            .placement()
            .scale(),
        1000
    );
    view(&mut app, 70_000_100, 70_001_900);
    settle(&mut app);
    check_exact(&app, &session, 70_001, 70_001);
    assert_eq!(
        app.activity.counts(TraceId::A).unwrap().window,
        (70_001, 70_001)
    );
    view(&mut app, 70_000_100, 70_000_900);
    settle(&mut app);
    check_exact(&app, &session, 70_001, 70_000);
}

#[test]
fn failed_classifications_report_errors_and_hide_meters() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("run.vtr");
    write_trace(&path, 3);
    index_beside(&path);
    let (mut app, session) = open(&path);
    let top = node(session.hierarchy().roots().first().unwrap());
    app.take_events();
    view(&mut app, 5_003, 5_004);
    let mut read = app.take_requests().pop().unwrap();
    let LoadRequest::Activity { budget, .. } = &mut read else {
        panic!("expected classification")
    };
    *budget = volna_trace::remote::memory::MemoryBudget::new(0);
    app.deliver(read.perform());
    assert_eq!(
        app.signal_activity(Traced::new(TraceId::A, SignalRef(0))),
        None
    );
    assert!(app.take_events().iter().any(|e| matches!(e, volna_core::app::Event::Notice(message) if message.contains("memory budget exceeded"))));
    assert_eq!(
        app.scope_activity(top),
        None,
        "a failed classification must hide the meter"
    );
    assert!(app.take_requests().is_empty(), "a failure must not loop");
    view(&mut app, 100, 1_000);
    settle(&mut app);
    check_exact(&app, &session, 100, 1_000);
}

#[test]
fn an_index_of_another_trace_is_ignored() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("run.vtr");
    write_trace(&path, 4);
    index_beside(&path);
    write_trace(&path, 5);
    let (app, session) = open_asking(&path);
    assert!(session.activity().is_none());
    assert_eq!(
        app.signal_activity(Traced::new(TraceId::A, SignalRef(0))),
        None
    );
    let top = node(session.hierarchy().roots().first().unwrap());
    assert_eq!(
        app.scope_activity(top),
        None,
        "no meters, only the size column"
    );
    assert!(volna_core::sidebar::ScopeTreeModel::size(app.doc.traces(), top).is_some());
}

#[test]
fn a_row_labels_its_count_and_dims_only_nonempty_quiet_scopes() {
    let active = ScopeActivity {
        changing: 2483,
        total: 6779,
    };
    assert_eq!(active.label(), "2,483 / 6,779");
    assert_eq!(active.detail(), "2,483 of 6,779 signals change in the view");
    assert!((active.share() - 2483.0 / 6779.0).abs() < f32::EPSILON);
    assert!(!active.quiet());
    assert!(
        ScopeActivity {
            changing: 0,
            total: 5
        }
        .quiet()
    );
    let empty = ScopeActivity {
        changing: 0,
        total: 0,
    };
    assert!(!empty.quiet());
    assert_eq!(empty.share(), 0.0);
}

fn build_views(app: &App) -> Vec<volna_core::sidebar::activity::ActivityBuildView> {
    app.activity.builds(
        app.doc.traces(),
        app.settings.resolved().hierarchy.activity_index,
    )
}

#[test]
fn build_offer_obeys_policy_and_not_now() {
    use volna_core::app::Command;
    use volna_core::sidebar::activity::ActivityBuildState;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("run.vtr");
    write_trace(&path, 6);
    let (mut app, session) = open_asking(&path);
    assert!(matches!(
        build_views(&app)[0].state,
        ActivityBuildState::Offer {
            estimated_seconds: 1
        }
    ));
    assert!(session.activity().is_none());
    app.handle(Command::DismissActivity(TraceId::A));
    settle(&mut app);
    assert!(build_views(&app).is_empty());
    app.settings_loaded(r#"{"hierarchy.activityIndex": "always"}"#);
    let requests = app.take_requests();
    assert!(matches!(
        requests.as_slice(),
        [LoadRequest::BuildActivity { .. }]
    ));
    for r in requests {
        app.deliver(r.perform());
    }
    settle(&mut app);
    assert!(session.activity().is_some());
    assert!(build_views(&app).is_empty());
    let session = OpenSpec::Bytes {
        name: "bytes.vtr".into(),
        bytes: std::fs::read(&path).unwrap(),
    }
    .open()
    .unwrap();
    app.set_session(session);
    settle(&mut app);
    assert!(build_views(&app).is_empty());
}

#[test]
fn missing_indexes_build_automatically_once_and_install_exact_meters() {
    use volna_core::sidebar::activity::ActivityBuildState;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("automatic.vtr");
    write_trace(&path, 17);
    let session = OpenSpec::Path(path.clone()).open().unwrap();
    let mut app = App::new();
    app.set_session(session.clone());
    let requests = app.take_requests();
    assert_eq!(
        requests
            .iter()
            .filter(|r| matches!(r, LoadRequest::BuildActivity { .. }))
            .count(),
        1
    );
    assert!(matches!(
        build_views(&app)[0].state,
        ActivityBuildState::Building { .. }
    ));
    assert!(app.take_requests().is_empty(), "one build per trace");
    for request in requests {
        app.deliver(request.perform());
    }
    settle(&mut app);
    assert!(session.activity().is_some());
    assert!(build_views(&app).is_empty());
    view(&mut app, 100, 1_000);
    settle(&mut app);
    check_exact(&app, &session, 100, 1_000);
    assert!(OpenSpec::Path(path).open().unwrap().activity().is_some());
}

#[test]
fn disabling_builds_persists_and_reset_builds_with_the_sidebar_hidden() {
    use volna_core::app::{Command, SettingsCommand};
    use volna_core::settings::{Host, Value};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("run.vtr");
    write_trace(&path, 6);
    let mut settings = App::new();
    settings.configure_settings(Host::Native, None);
    settings.handle(Command::Settings(SettingsCommand::Set {
        id: "hierarchy.activityIndex".into(),
        value: Value::Text("never".into()),
    }));
    let (mut app, session) = open_with_settings(&path, settings.settings.text());
    settle(&mut app);
    assert!(build_views(&app).is_empty());
    assert!(session.activity().is_none());
    assert!(!path.with_extension("vtr.index").exists());
    app.sidebar_visible = false;
    app.handle(Command::Settings(SettingsCommand::Reset {
        id: "hierarchy.activityIndex".into(),
    }));
    settle(&mut app);
    assert!(session.activity().is_some());
    assert!(build_views(&app).is_empty());
}

#[test]
fn explicit_build_installs_meters_and_reports_block_progress() {
    use volna_core::app::Command;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("run.vtr");
    write_trace(&path, 6);
    let (mut app, session) = open_asking(&path);
    app.handle(Command::AddVars(vec![Traced::new(TraceId::A, 0)]));
    settle(&mut app);
    let undo = app.undo_label().map(str::to_owned);
    let resident = session.resident_bytes();
    app.handle(Command::BuildActivity(TraceId::A));
    let r = app.take_requests().pop().unwrap();
    let LoadRequest::BuildActivity { options, .. } = &r else {
        panic!("expected build")
    };
    let control = Arc::clone(options.control.as_ref().unwrap());
    assert_eq!(control.progress().completed, 0);
    app.handle(Command::BuildActivity(TraceId::A));
    assert!(app.take_requests().is_empty(), "one build per trace");
    app.deliver(r.perform());
    assert!(
        session.resident_bytes() > resident,
        "session owns the new immutable index"
    );
    let p = control.progress();
    assert!(p.total > 1 && p.completed == p.total, "{p:?}");
    assert!(
        !control.cancel(),
        "completed publication wins over late cancellation"
    );
    settle(&mut app);
    assert_eq!(
        app.undo_label(),
        undo.as_deref(),
        "building is not journaled"
    );
    view(&mut app, 100, 1_000);
    settle(&mut app);
    check_exact(&app, &session, 100, 1_000);
    let reopened = OpenSpec::Path(path).open().unwrap();
    assert!(
        reopened.activity().is_some(),
        "the next open uses the sidecar"
    );
}

#[test]
fn cancelled_and_closed_builds_leave_no_sidecar_or_temporary_file() {
    use volna_core::app::Command;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("run.vtr");
    write_trace(&path, 6);
    let (mut app, session) = open_asking(&path);
    app.handle(Command::BuildActivity(TraceId::A));
    app.handle(Command::CancelActivity(TraceId::A));
    assert!(app.take_requests().is_empty());
    for close in [false, true] {
        app.handle(Command::BuildActivity(TraceId::A));
        let r = app.take_requests().pop().unwrap();
        if close {
            app.handle(Command::CloseTrace);
        } else {
            app.handle(Command::CancelActivity(TraceId::A));
        }
        app.deliver(r.perform());
        settle(&mut app);
        assert!(session.activity().is_none());
        assert!(build_views(&app).is_empty());
        let files: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|p| p.unwrap().file_name())
            .collect();
        assert_eq!(files, [std::ffi::OsString::from("run.vtr")]);
    }
}

#[test]
fn a_failed_build_can_be_retried_explicitly() {
    use volna_core::app::{Command, Event};
    use volna_core::sidebar::activity::ActivityBuildState;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("run.vtr");
    write_trace(&path, 6);
    let (mut app, session) = open_asking(&path);
    app.handle(Command::BuildActivity(TraceId::A));
    let mut r = app.take_requests().pop().unwrap();
    let LoadRequest::BuildActivity { options, .. } = &mut r else {
        panic!("expected build")
    };
    options.memory = 1;
    app.deliver(r.perform());
    assert!(matches!(
        build_views(&app)[0].state,
        ActivityBuildState::Failed { .. }
    ));
    assert!(
        app.take_events()
            .iter()
            .any(|e| matches!(e, Event::Notice(message) if message.contains("memory limit")))
    );
    assert!(
        app.take_requests().is_empty(),
        "failure does not automatically loop"
    );
    app.handle(Command::BuildActivity(TraceId::A));
    settle(&mut app);
    assert!(session.activity().is_some());
    assert!(build_views(&app).is_empty());
}

#[cfg(unix)]
#[test]
fn viewer_builds_use_the_cache_for_a_read_only_directory() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let traces = dir.path().join("traces");
    let cache = dir.path().join("cache");
    std::fs::create_dir(&traces).unwrap();
    let path = traces.join("run.vtr");
    write_trace(&path, 6);
    let (mut app, session) = open_asking(&path);
    app.handle(volna_core::app::Command::BuildActivity(TraceId::A));
    let mut r = app.take_requests().pop().unwrap();
    let LoadRequest::BuildActivity { cache_dir, .. } = &mut r else {
        panic!("expected build")
    };
    *cache_dir = Some(cache.clone());
    std::fs::set_permissions(&traces, std::fs::Permissions::from_mode(0o555)).unwrap();
    assert!(
        std::fs::File::create(traces.join("probe")).is_err(),
        "this test requires an unprivileged user"
    );
    let result = r.perform();
    std::fs::set_permissions(&traces, std::fs::Permissions::from_mode(0o755)).unwrap();
    app.deliver(result);
    settle(&mut app);
    assert!(session.activity().is_some());
    let reader = vtr::Reader::open(&path).unwrap();
    let identity = vtr::activity::Identity::of(&reader).unwrap();
    let sidecar = vtr::activity::Sidecar::new(&path, &identity, Some(&cache));
    assert!(!sidecar.beside.exists());
    assert_eq!(sidecar.load(&identity).unwrap().0, sidecar.cached.unwrap());
}

#[test]
fn independent_sessions_share_a_single_sidecar_build() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("shared.vtr");
    write_trace(&path, 191);
    let sessions: Vec<_> = (0..2)
        .map(|_| OpenSpec::Path(path.clone()).open().unwrap())
        .collect();
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let controls: Vec<_> = (0..2)
        .map(|_| Arc::new(vtr::activity::BuildControl::default()))
        .collect();
    std::thread::scope(|scope| {
        for (session, control) in sessions.iter().zip(&controls) {
            let barrier = barrier.clone();
            scope.spawn(move || {
                barrier.wait();
                session
                    .build_activity(
                        &vtr::activity::BuildOptions {
                            threads: 2,
                            memory: 1 << 20,
                            control: Some(control.clone()),
                        },
                        &volna_trace::remote::memory::MemoryBudget::new(4 << 20),
                        None,
                    )
                    .unwrap();
            });
        }
    });
    assert_eq!(
        controls
            .iter()
            .filter(|c| c.progress().completed > 0)
            .count(),
        1
    );
    let bytes = sessions[0].activity_image(None).unwrap();
    assert_eq!(bytes, sessions[1].activity_image(None).unwrap());
    let index =
        vtr::activity::Index::decode(&bytes, &sessions[0].activity_identity().unwrap()).unwrap();
    assert_eq!(
        index.classify(300, 700),
        sessions[1].activity().unwrap().classify(300, 700)
    );
    assert!(
        !dir.path().read_dir().unwrap().any(|e| e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains(".tmp."))
    );
}

#[cfg(unix)]
#[test]
fn server_streams_a_cached_sidecar_and_keeps_it_after_disconnect() {
    use std::os::unix::net::UnixStream;
    use volna_trace::remote::ClientStep;
    use volna_trace::remote::{open::OpenTransfer, transport::*};
    use volna_trace::session::LoadResult;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("remote.vtr");
    write_trace(&path, 712);
    let session = OpenSpec::Path(path.clone()).open().unwrap();
    let (mut client, server) = UnixStream::pair().unwrap();
    client
        .set_read_timeout(Some(std::time::Duration::from_secs(10)))
        .unwrap();
    server
        .set_read_timeout(Some(std::time::Duration::from_secs(10)))
        .unwrap();
    let server_session = session.clone();
    let service = std::thread::spawn(move || {
        server::serve(
            server.try_clone().unwrap(),
            server,
            71,
            || Ok(server_session),
            || Ok(()),
        )
    });
    let mut opening = OpenTransfer::new(
        1,
        TraceId::A,
        1,
        4 << 20,
        volna_trace::remote::memory::MemoryBudget::new(8 << 20),
    )
    .unwrap();
    write_packet(&mut client, &opening.command()).unwrap();
    loop {
        let packet = read_packet(&mut client).unwrap().unwrap();
        let mut step = opening.accept(packet).unwrap();
        loop {
            match step {
                ClientStep::Yield => step = opening.step().unwrap(),
                ClientStep::Ack(ack) => {
                    write_packet(&mut client, &ack).unwrap();
                    break;
                }
                ClientStep::Complete { ack, result } => {
                    write_packet(&mut client, &ack).unwrap();
                    assert!(matches!(result, LoadResult::Opened { result: Ok(_), .. }));
                    break;
                }
            }
        }
        if opening.is_complete() {
            break;
        }
    }
    opening.finish().unwrap();
    write_packet(
        &mut client,
        &Packet {
            session: 71,
            request: 2,
            sequence: 0,
            body: Body::Command(Command::Activity { build: true }),
        },
    )
    .unwrap();
    let mut receiver = Receiver::new(71, 2, vec![ObjectId::Activity], 4 << 20).unwrap();
    let mut bytes = Vec::new();
    while !receiver.is_complete() {
        let packet = read_packet(&mut client).unwrap().unwrap();
        let ack = acknowledgement(&packet);
        if let Receive::Data(chunk) = receiver.accept(packet).unwrap() {
            bytes.extend(chunk);
        }
        write_packet(&mut client, &ack).unwrap();
    }
    receiver.finish().unwrap();
    let image: Vec<u8> = bincode::deserialize(&bytes).unwrap();
    let index =
        vtr::activity::Index::decode(&image, &session.activity_identity().unwrap()).unwrap();
    assert_eq!(
        index.classify(500, 1100),
        session.activity().unwrap().classify(500, 1100)
    );
    write_packet(
        &mut client,
        &Packet {
            session: 71,
            request: 3,
            sequence: 0,
            body: Body::Command(Command::Activity { build: false }),
        },
    )
    .unwrap();
    assert!(matches!(
        read_packet(&mut client).unwrap().unwrap().body,
        Body::Begin {
            object: ObjectId::Activity,
            ..
        }
    ));
    drop(client);
    assert!(service.join().unwrap().is_err());
    assert_eq!(session.activity_image(None).unwrap(), image);
    assert!(OpenSpec::Path(path).open().unwrap().activity().is_some());
}

#[cfg(unix)]
struct RemoteRig {
    wire: std::os::unix::net::UnixStream,
    client: volna_core::remote::client::RemoteClient,
    service: Option<std::thread::JoinHandle<anyhow::Result<()>>>,
    budget: volna_trace::remote::memory::MemoryBudget,
    commands: Vec<volna_trace::remote::transport::Command>,
}

#[cfg(unix)]
impl RemoteRig {
    fn open(source: Arc<dyn Session>) -> (Self, Arc<dyn Session>) {
        use volna_core::remote::client::RemoteClient;
        let (wire, server) = std::os::unix::net::UnixStream::pair().unwrap();
        for stream in [&wire, &server] {
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(10)))
                .unwrap();
            stream
                .set_write_timeout(Some(std::time::Duration::from_secs(10)))
                .unwrap();
        }
        let service = std::thread::spawn(move || {
            server::serve(
                server.try_clone().unwrap(),
                server,
                89,
                || Ok(source),
                || Ok(()),
            )
        });
        let budget = volna_trace::remote::memory::MemoryBudget::new(16 << 20);
        let client = RemoteClient::new(TraceId::A, 1, 8 << 20, budget.clone()).unwrap();
        let mut rig = Self {
            wire,
            client,
            service: Some(service),
            budget,
            commands: vec![],
        };
        let mut opened = None;
        rig.pump(|result| {
            if let LoadResult::Opened { result, .. } = result {
                opened = Some(result.unwrap());
            }
        });
        (rig, opened.expect("remote Open completed"))
    }

    fn pump(&mut self, mut deliver: impl FnMut(LoadResult)) {
        use volna_core::remote::ClientStep;
        use volna_trace::remote::transport::*;
        while let Some(command) = self.client.take_command().unwrap() {
            let expected = match &command.body {
                Body::Command(Command::Signals(ids)) => ids.len(),
                _ => 1,
            };
            if let Body::Command(c) = &command.body {
                self.commands.push(c.clone());
            }
            write_packet(&mut self.wire, &command).unwrap();
            let mut completed = 0;
            loop {
                let packet = read_packet(&mut self.wire).unwrap().unwrap();
                let object_end = matches!(packet.body, Body::End | Body::Error { .. });
                let opening = command.session == 0;
                let mut step = self.client.accept(packet).unwrap();
                let mut opened = false;
                loop {
                    match step {
                        ClientStep::Yield => step = self.client.step().unwrap(),
                        ClientStep::Ack(ack) => {
                            write_packet(&mut self.wire, &ack).unwrap();
                            break;
                        }
                        ClientStep::Complete { ack, result } => {
                            opened = matches!(result, LoadResult::Opened { .. });
                            deliver(result);
                            write_packet(&mut self.wire, &ack).unwrap();
                            break;
                        }
                    }
                }
                if opening {
                    if opened {
                        break;
                    }
                } else {
                    if object_end {
                        completed += 1;
                    }
                    if completed == expected {
                        break;
                    }
                }
            }
        }
    }

    fn settle(&mut self, app: &mut App) {
        for _ in 0..30 {
            let requests = app.take_requests();
            if requests.is_empty() {
                return;
            }
            for request in requests {
                if request.remote_id().is_some() {
                    if let Err(result) = self.client.submit(request) {
                        app.deliver(result);
                    }
                } else {
                    app.deliver(request.perform());
                }
            }
            self.pump(|result| app.deliver(result));
        }
        panic!("remote activity did not settle");
    }
}

#[cfg(unix)]
impl Drop for RemoteRig {
    fn drop(&mut self) {
        drop(self.wire.shutdown(std::net::Shutdown::Both));
        if let Some(service) = self.service.take() {
            drop(service.join().unwrap());
        }
    }
}

#[cfg(unix)]
#[test]
fn remote_build_and_scope_counts_match_local_without_pan_index_requests() {
    use volna_trace::remote::transport::Command;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("remote-built.vtr");
    write_trace(&path, 731);
    let local = OpenSpec::Path(path).open().unwrap();
    let (mut rig, remote) = RemoteRig::open(local.clone());
    assert!(remote.activity().is_none());
    assert!(!remote.activity_available());
    assert_eq!(remote.activity_identity(), local.activity_identity());
    let mut app = App::default();
    app.set_session(remote.clone());
    let undo = app.undo_label().map(str::to_owned);
    rig.settle(&mut app);
    assert!(remote.activity().is_some());
    assert!(build_views(&app).is_empty());
    assert_eq!(app.undo_label(), undo.as_deref());
    assert_eq!(
        rig.commands
            .iter()
            .filter(|c| matches!(c, Command::Activity { build: true }))
            .count(),
        1
    );
    for (a, b) in [
        (0, 0),
        (0, 80000),
        (5003, 5004),
        (5000, 5001),
        (70000, 70030),
        (70001, 70001),
    ] {
        view(&mut app, a, b);
        rig.settle(&mut app);
        check_exact(&app, &local, a, b);
    }
    assert_eq!(
        rig.commands
            .iter()
            .filter(|c| matches!(c, Command::Activity { .. }))
            .count(),
        1
    );
    assert!(
        rig.commands
            .iter()
            .all(|c| !matches!(c, Command::Signals(_))),
        "exact sidecars answer activity without loading waveforms"
    );
    assert!(rig.budget.used() > remote.activity().unwrap().memory_bytes());
}

#[cfg(unix)]
#[test]
fn remote_cached_index_loads_once_even_when_build_policy_is_never() {
    use volna_trace::remote::transport::Command;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("remote-cached.vtr");
    write_trace(&path, 738);
    index_beside(&path);
    let local = OpenSpec::Path(path).open().unwrap();
    let (mut rig, remote) = RemoteRig::open(local.clone());
    assert!(remote.activity_available());
    let mut app = App::default();
    app.settings_loaded(r#"{"hierarchy.activityIndex":"never"}"#);
    app.set_session(remote.clone());
    rig.settle(&mut app);
    assert!(remote.activity().is_some());
    assert!(rig.commands.contains(&Command::Activity { build: false }));
    assert!(!rig.commands.contains(&Command::Activity { build: true }));
    view(&mut app, 70000, 80000);
    rig.settle(&mut app);
    check_exact(&app, &local, 70000, 80000);
}

#[cfg(unix)]
#[test]
fn remote_exact_index_counts_many_signals_without_waveform_reads() {
    use volna_trace::remote::transport::{Command, MAX_BATCH};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("batch.vtr");
    let mut writer = Writer::create(&path).unwrap();
    let top = Some(
        writer
            .scope_with(
                vtr::SourceOptions::new("top", None),
                ScopeType::Module,
                "top",
            )
            .unwrap(),
    );
    let signals: Vec<_> = (0..MAX_BATCH * 2 + 7)
        .map(|i| {
            writer
                .signal(
                    vtr::SourceOptions::new(format!("s{i}"), top),
                    SignalKind::Bits {
                        width: 32,
                        states: 2,
                    },
                    VarType::Reg,
                    Direction::Implicit,
                )
                .unwrap()
        })
        .collect();
    for step in 0..1000 {
        writer.advance_time(step * 10).unwrap();
        for (i, signal) in signals.iter().enumerate() {
            writer.emit_u64(*signal, step + i as u64).unwrap();
        }
    }
    writer.close().unwrap();
    index_beside(&path);
    let local = OpenSpec::Path(path).open().unwrap();
    assert_eq!(local.info().signal_count as usize, MAX_BATCH * 2 + 7);
    let (mut rig, remote) = RemoteRig::open(local.clone());
    let mut app = App::default();
    app.set_session(remote);
    rig.settle(&mut app);
    for (t0, t1) in [(15, 15), (20, 20)] {
        view(&mut app, t0, t1);
        let before = rig.commands.len();
        rig.settle(&mut app);
        check_exact(&app, &local, t0, t1);
        assert_eq!(
            rig.commands[before..]
                .iter()
                .filter(|c| matches!(c, Command::Signals(_)))
                .count(),
            0
        );
    }
}

#[cfg(unix)]
#[test]
fn a_client_disconnect_during_an_authorized_server_build_keeps_the_cache() {
    struct GatedBuild {
        inner: Arc<dyn Session>,
        entered: std::sync::mpsc::Sender<()>,
        release: std::sync::Mutex<std::sync::mpsc::Receiver<()>>,
    }
    impl Session for GatedBuild {
        fn info(&self) -> &volna_trace::data::TraceInfo {
            self.inner.info()
        }
        fn hierarchy(&self) -> &volna_trace::data::Hierarchy {
            self.inner.hierarchy()
        }
        fn load_signal(
            &self,
            s: SignalRef,
        ) -> anyhow::Result<Arc<dyn volna_trace::data::SignalHistory>> {
            self.inner.load_signal(s)
        }
        fn activity(&self) -> Option<Arc<volna_trace::data::ActivityIndex>> {
            self.inner.activity()
        }
        fn activity_identity(&self) -> Option<vtr::activity::Identity> {
            self.inner.activity_identity()
        }
        fn activity_build_info(&self) -> Option<volna_trace::data::ActivityBuildInfo> {
            self.inner.activity_build_info()
        }
        fn activity_image(&self, cache: Option<&Path>) -> anyhow::Result<Vec<u8>> {
            self.inner.activity_image(cache)
        }
        fn build_activity(
            &self,
            options: &vtr::activity::BuildOptions,
            budget: &volna_trace::remote::memory::MemoryBudget,
            cache: Option<&Path>,
        ) -> anyhow::Result<()> {
            self.entered.send(())?;
            self.release
                .lock()
                .unwrap()
                .recv_timeout(std::time::Duration::from_secs(10))?;
            self.inner.build_activity(options, budget, cache)
        }
    }
    use volna_trace::remote::transport::write_packet;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("disconnected.vtr");
    write_trace(&path, 993);
    let source = OpenSpec::Path(path.clone()).open().unwrap();
    let (entered, started) = std::sync::mpsc::channel();
    let (release, gate) = std::sync::mpsc::channel();
    let (mut rig, remote) = RemoteRig::open(Arc::new(GatedBuild {
        inner: source.clone(),
        entered,
        release: std::sync::Mutex::new(gate),
    }));
    rig.client
        .submit(LoadRequest::BuildActivity {
            trace: TraceId::A,
            generation: 1,
            session: remote.clone(),
            options: vtr::activity::BuildOptions {
                control: Some(Arc::new(vtr::activity::BuildControl::default())),
                ..Default::default()
            },
            budget: rig.budget.clone(),
            cache_dir: None,
        })
        .unwrap_or_else(|_| panic!("build rejected"));
    let command = rig.client.take_command().unwrap().unwrap();
    write_packet(&mut rig.wire, &command).unwrap();
    started
        .recv_timeout(std::time::Duration::from_secs(10))
        .unwrap();
    assert!(source.activity().is_none());
    rig.wire.shutdown(std::net::Shutdown::Both).unwrap();
    let results = rig.client.disconnect("lost connection during build");
    assert!(matches!(
        results.as_slice(),
        [LoadResult::ActivityBuilt { result: Err(_), .. }]
    ));
    release.send(()).unwrap();
    assert!(rig.service.take().unwrap().join().unwrap().is_err());
    assert!(
        remote.activity().is_none(),
        "a disconnected client installs no response"
    );
    assert!(source.activity().is_some());
    assert!(
        OpenSpec::Path(path).open().unwrap().activity().is_some(),
        "another client reuses the completed cache"
    );
}
