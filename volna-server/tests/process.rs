use bincode::Options;
use serde::de::DeserializeOwned;
use std::io::Write;
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command as Process, Stdio};
use volna_trace::data::WaveValue;
use volna_trace::data::transactions::TrackRef;
use volna_trace::remote::history::PackedHistory;
use volna_trace::remote::objects::Metadata;
use volna_trace::remote::transport::*;
use volna_trace::remote::{ClientStep, signals::SignalTransfer};
use volna_trace::remote::{client::RemoteClient, memory::MemoryBudget};
use volna_trace::session::LoadResult;
use volna_trace::session::OpenSpec;
use volna_trace::testing::assert_hierarchy_eq;
use vtr::Recorder;

struct Server {
    child: std::sync::Arc<std::sync::Mutex<Child>>,
    deadline: std::sync::mpsc::Sender<()>,
    watchdog: Option<std::thread::JoinHandle<()>>,
    timed_out: std::sync::Arc<std::sync::atomic::AtomicBool>,
    input: ChildStdin,
    output: ChildStdout,
    session: u64,
    request: u64,
}
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.deadline.send(());
        if let Some(watchdog) = self.watchdog.take() {
            watchdog.join().unwrap();
        }
        let mut child = self.child.lock().unwrap();
        drop(child.kill());
        drop(child.wait());
        if !std::thread::panicking() {
            assert!(
                !self.timed_out.load(std::sync::atomic::Ordering::Acquire),
                "server test exceeded 60-second deadline"
            );
        }
    }
}
impl Server {
    fn drive(&mut self, client: &mut RemoteClient) -> LoadResult {
        let command = client.take_command().unwrap().expect("queued command");
        self.request = command.request;
        write_packet(&mut self.input, &command).unwrap();
        loop {
            let packet = read_packet(&mut self.output)
                .unwrap()
                .expect("client response");
            self.session = packet.session;
            let mut step = client.accept(packet).unwrap();
            loop {
                match step {
                    ClientStep::Yield => {
                        std::thread::yield_now();
                        step = client.step().unwrap();
                    }
                    ClientStep::Ack(ack) => {
                        write_packet(&mut self.input, &ack).unwrap();
                        break;
                    }
                    ClientStep::Complete { ack, result } => {
                        write_packet(&mut self.input, &ack).unwrap();
                        return result;
                    }
                }
            }
        }
    }
    fn start(path: &Path) -> Self {
        let mut child = Process::new(env!("CARGO_BIN_EXE_volna-server"))
            .arg(path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let input = child.stdin.take().unwrap();
        let output = child.stdout.take().unwrap();
        let child = std::sync::Arc::new(std::sync::Mutex::new(child));
        let timed_out = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (deadline, stop) = std::sync::mpsc::channel();
        let owned_child = child.clone();
        let expired = timed_out.clone();
        let watchdog = Some(std::thread::spawn(move || {
            if matches!(
                stop.recv_timeout(std::time::Duration::from_mins(1)),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout)
            ) {
                expired.store(true, std::sync::atomic::Ordering::Release);
                drop(owned_child.lock().unwrap().kill());
            }
        }));
        Self {
            input,
            output,
            child,
            deadline,
            watchdog,
            timed_out,
            session: 0,
            request: 0,
        }
    }
    fn command(&mut self, command: Command) {
        self.request += 1;
        write_packet(
            &mut self.input,
            &Packet {
                session: self.session,
                request: self.request,
                sequence: 0,
                body: Body::Command(command),
            },
        )
        .unwrap();
    }
    fn receive<T: DeserializeOwned>(&mut self, objects: Vec<ObjectId>) -> Vec<Result<T, String>> {
        let first = read_packet(&mut self.output).unwrap().expect("response");
        if self.session == 0 {
            self.session = first.session;
        }
        let mut receiver =
            Receiver::new(self.session, self.request, objects, 64 * 1024 * 1024).unwrap();
        let mut packet = first;
        let mut buffer = vec![];
        let mut result = vec![];
        loop {
            let ack = acknowledgement(&packet);
            match receiver.accept(packet).unwrap() {
                Receive::Begin { .. } => buffer.clear(),
                Receive::Data(bytes) => buffer.extend(bytes),
                Receive::Complete(_) => result.push(Ok(bincode::DefaultOptions::new()
                    .with_fixint_encoding()
                    .with_little_endian()
                    .reject_trailing_bytes()
                    .with_limit(buffer.len() as u64)
                    .deserialize(&buffer)
                    .unwrap())),
                Receive::Failed { message, .. } => result.push(Err(message)),
                Receive::ActivityMemoryRequired { .. } => {
                    result.push(Err("activity index needs approval".into()));
                }
            }
            write_packet(&mut self.input, &ack).unwrap();
            if receiver.is_complete() {
                break;
            }
            packet = read_packet(&mut self.output)
                .unwrap()
                .expect("next response");
        }
        receiver.finish().unwrap();
        result
    }
    fn open(&mut self, limit: u64) -> Result<Metadata, String> {
        use volna_trace::remote::{memory::MemoryBudget, open::OpenTransfer};
        self.request += 1;
        let mut transfer = OpenTransfer::new(
            self.request,
            0,
            19,
            limit,
            MemoryBudget::new(256 * 1024 * 1024),
        )
        .unwrap();
        write_packet(&mut self.input, &transfer.command()).unwrap();
        loop {
            let packet = read_packet(&mut self.output)
                .unwrap()
                .expect("Open response");
            let mut step = transfer.accept(packet).unwrap();
            loop {
                match step {
                    ClientStep::Ack(ack) => {
                        write_packet(&mut self.input, &ack).unwrap();
                        break;
                    }
                    ClientStep::Yield => {
                        std::thread::yield_now();
                        step = transfer.step().unwrap();
                    }
                    ClientStep::Complete { ack, result } => {
                        write_packet(&mut self.input, &ack).unwrap();
                        transfer.finish().unwrap();
                        let volna_trace::session::LoadResult::Opened {
                            generation, result, ..
                        } = result
                        else {
                            panic!("Open completion");
                        };
                        assert_eq!(generation, 19);
                        self.session = ack.session;
                        return result
                            .map(|session| {
                                assert_eq!(session.remote_id(), Some(self.session));
                                Metadata::from_session(session.as_ref())
                            })
                            .map_err(|error| format!("{error:#}"));
                    }
                }
            }
        }
    }
    fn histories(
        &mut self,
        signals: &[volna_trace::data::SignalRef],
    ) -> volna_trace::session::SignalLoads {
        let limit = 64 * 1024 * 1024;
        let mut receiver = SignalTransfer::new(
            self.session,
            self.request,
            0,
            73,
            signals,
            limit,
            volna_trace::remote::memory::MemoryBudget::new(256 * 1024 * 1024),
        )
        .unwrap();
        let mut results = Vec::new();
        while !receiver.is_complete() {
            let packet = read_packet(&mut self.output)
                .unwrap()
                .expect("history response");
            let mut step = receiver.accept(packet).unwrap();
            let ack = loop {
                match step {
                    ClientStep::Ack(ack) => break ack,
                    ClientStep::Complete { ack, result } => {
                        let volna_trace::session::LoadResult::Signals {
                            generation,
                            results: loaded,
                            ..
                        } = result
                        else {
                            panic!("expected signal completion");
                        };
                        assert_eq!(generation, 73);
                        results.extend(loaded);
                        break ack;
                    }
                    ClientStep::Yield => {
                        std::thread::yield_now();
                        step = receiver.step().unwrap();
                    }
                }
            };
            write_packet(&mut self.input, &ack).unwrap();
        }
        receiver.finish().unwrap();
        results
    }
    fn wait(&self) -> std::process::ExitStatus {
        loop {
            if let Some(status) = self.child.lock().unwrap().try_wait().unwrap() {
                return status;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
    fn close(&mut self) {
        self.command(Command::Close);
        assert!(self.wait().success());
    }
}

fn same_value(a: WaveValue, b: WaveValue) {
    match (a, b) {
        (WaveValue::Real(a), WaveValue::Real(b)) => assert_eq!(a.to_bits(), b.to_bits()),
        (a, b) => assert_eq!(a, b),
    }
}

#[test]
fn remote_open_and_tracks_preserve_compact_shared_graphs_within_object_limits() {
    use std::sync::Arc;
    use volna_trace::data::transactions::AttributeValue;
    use volna_trace::session::LoadRequest;
    use vtr::{Attribute, Value};

    let file = tempfile::NamedTempFile::new().unwrap();
    let mut writer = vtr::Writer::create(file.path()).unwrap();
    let key = writer.intern("description").unwrap();
    let mask = writer.intern("mask").unwrap();
    let data = writer
        .intern_value(Value::Bytes(vec![0x5a; 64 * 1024]))
        .unwrap();
    let mut chain = data;
    for _ in 0..80 {
        chain = writer
            .intern_value(Value::List(vec![Value::Ref(chain)]))
            .unwrap();
    }
    let mut graph = writer.intern_value(Value::U64(u64::MAX)).unwrap();
    for _ in 0..16 {
        graph = writer
            .intern_value(Value::List(vec![Value::Ref(graph); 2]))
            .unwrap();
    }
    let description = writer
        .intern_value(Value::List(vec![
            Value::Ref(data),
            Value::Ref(chain),
            Value::Ref(graph),
        ]))
        .unwrap();
    let attributes = vec![Attribute {
        key,
        value: Value::Ref(description),
    }];
    let mut options = vtr::SourceOptions::new("instructions", None);
    options.attributes = attributes.clone();
    let source = writer.transactions::<()>(options, "INSTRUCTION").unwrap();
    for time in 0..128 {
        let tx = writer.begin_at(source, time, &(), None).unwrap();
        writer
            .attribute(tx.owner(), key, Value::Ref(description))
            .unwrap();
        writer
            .attribute(tx.owner(), mask, Value::U64(1 << (time % 32)))
            .unwrap();
        writer
            .embedded_event(tx.owner(), time, key, attributes.clone())
            .unwrap();
        writer
            .embedded_stage(tx.owner(), time, key, 0, Some(time + 1), attributes.clone())
            .unwrap();
        writer.end_at(tx, time + 1, vtr::Status::Ok).unwrap();
        writer
            .relate(key, tx.id(), tx.id(), attributes.clone())
            .unwrap();
    }
    writer.close().unwrap();

    fn check(value: &AttributeValue) {
        let AttributeValue::List(parts) = value.resolved() else {
            panic!("description type");
        };
        let AttributeValue::Bytes(bytes) = parts[0].resolved() else {
            panic!("byte type");
        };
        assert_eq!(bytes.len(), 64 * 1024);
        assert!(bytes.iter().all(|byte| *byte == 0x5a));
        let mut chain = &parts[1];
        for _ in 0..80 {
            let AttributeValue::List(items) = chain.resolved() else {
                panic!("chain type");
            };
            assert_eq!(items.len(), 1);
            chain = &items[0];
        }
        assert_eq!(chain.resolved(), parts[0].resolved());
        let mut graph = &parts[2];
        for _ in 0..16 {
            let AttributeValue::List(items) = graph.resolved() else {
                panic!("graph type");
            };
            assert_eq!(items.len(), 2);
            let (AttributeValue::Shared(a), AttributeValue::Shared(b)) = (&items[0], &items[1])
            else {
                panic!("shared children");
            };
            assert!(a.ptr_eq(b));
            graph = &items[0];
        }
        assert_eq!(graph.resolved(), &AttributeValue::U64(u64::MAX));
    }

    let mut server = Server::start(file.path());
    let budget = MemoryBudget::new(4 * 1024 * 1024);
    let mut client = RemoteClient::new(0, 10, 1024 * 1024, budget.clone()).unwrap();
    let LoadResult::Opened { result, .. } = server.drive(&mut client) else {
        panic!("open");
    };
    let session = result.unwrap();
    check(&session.tracks()[0].attributes[0].1);
    assert!(
        client
            .submit(LoadRequest::Track {
                tag: 0,
                session: session.clone(),
                generation: 10,
                request_id: 1,
                track: TrackRef(source.id())
            })
            .is_ok()
    );
    let LoadResult::Track { result, .. } = server.drive(&mut client) else {
        panic!("track");
    };
    let loaded = result.unwrap();
    assert_eq!(loaded.generators[0].transactions().len(), 128);
    let first = &loaded.generators[0].transactions()[0].attributes[0].value;
    for row in loaded.generators[0].transactions() {
        check(&row.attributes[0].value);
        assert_eq!(
            row.attributes[1].value,
            AttributeValue::U64(1 << (row.begin % 32))
        );
        for value in [
            &row.attributes[0].value,
            &row.events[0].attributes[0].1,
            &row.stages[0].attributes[0].1,
        ] {
            let (AttributeValue::Shared(a), AttributeValue::Shared(b)) = (first, value) else {
                panic!("shared description");
            };
            assert!(a.ptr_eq(b));
        }
    }
    for relation in loaded.generators[0].relations() {
        let (AttributeValue::Shared(a), AttributeValue::Shared(b)) =
            (first, &relation.relation.attributes[0].1)
        else {
            panic!("shared relation");
        };
        assert!(a.ptr_eq(b));
    }
    let retained = Arc::clone(&loaded.generators[0]);
    drop(loaded);
    drop(session);
    drop(client);
    server.close();
    assert!(budget.used() >= 64 * 1024);
    assert!(budget.used() < 1024 * 1024);
    check(&retained.transactions()[0].attributes[0].value);
    drop(retained);
    assert_eq!(budget.used(), 0);
}

#[test]
fn foreign_recordings_return_an_actionable_open_error() {
    let dir = tempfile::tempdir().unwrap();
    for bytes in [
        include_bytes!("../../vtr/vtr-convert/tests/fixtures/values.fst").as_slice(),
        include_bytes!("../../vtr/vtr-convert/tests/fixtures/pipeline.kanata").as_slice(),
    ] {
        let path = dir.path().join("recording.vtr");
        std::fs::write(&path, bytes).unwrap();
        let mut server = Server::start(&path);
        let error = server.open(64 << 20).unwrap_err();
        assert!(error.contains("expected VTR"), "{error}");
        assert!(error.contains("vtr convert"), "{error}");
        assert!(server.wait().success());
    }
}

#[test]
fn vtr_process_histories_match_local_values_and_aliases() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../volna-trace/tests/fixtures");
    for file in ["picorv32.vtr", "features.vtr"] {
        let path = root.join(file);
        let local = OpenSpec::Path(path.clone()).open().unwrap();
        let mut server = Server::start(&path);
        let metadata = server.open(64 * 1024 * 1024).unwrap();
        metadata.validate().unwrap();
        assert_eq!(
            bincode::serialize(&metadata.info).unwrap(),
            bincode::serialize(local.info()).unwrap()
        );
        assert_eq!(metadata.capabilities, local.capabilities());
        assert_eq!(metadata.tracks, local.tracks());
        assert_hierarchy_eq(&metadata.hierarchy, local.hierarchy());
        let mut ids = vec![];
        for variable in local.hierarchy().vars().take(32) {
            if !ids.contains(&variable.signal.0) {
                ids.push(variable.signal.0);
            }
        }
        let mut request = ids.clone();
        request.push(ids[0]);
        server.command(Command::Signals(request));
        let histories = server.histories(
            &ids.iter()
                .copied()
                .map(volna_trace::data::SignalRef)
                .collect::<Vec<_>>(),
        );
        assert_eq!(
            histories.len(),
            ids.len(),
            "duplicate IDs only transfer once"
        );
        for (id, (returned, result)) in ids.into_iter().zip(histories) {
            assert_eq!(returned.0, id);
            let remote = result.unwrap();
            let local = local.load_signal(volna_trace::data::SignalRef(id)).unwrap();
            assert_eq!(remote.shape(), local.shape());
            assert_eq!(remote.len(), local.len());
            same_value(remote.value(None), local.value(None));
            for i in 0..local.len() {
                assert_eq!(remote.time(i), local.time(i));
                same_value(remote.value(Some(i)), local.value(Some(i)));
            }
        }
        server.close();
    }
}

#[test]
fn process_preserves_full_tracks_and_parallel_relations() {
    let file = tempfile::NamedTempFile::new().unwrap();
    let mut w = vtr::Writer::create(file.path()).unwrap();
    let stream = w
        .scope_with(
            vtr::SourceOptions::new("stream", None),
            vtr::ScopeType::Generic,
            "",
        )
        .unwrap();
    let a = w
        .transactions::<()>(vtr::SourceOptions::new("a", Some(stream)), "transaction")
        .unwrap();
    let b = w
        .transactions::<()>(vtr::SourceOptions::new("b", Some(stream)), "transaction")
        .unwrap();
    w.transactions::<()>(
        vtr::SourceOptions::new("empty", Some(stream)),
        "transaction",
    )
    .unwrap();
    let x = w.begin_at(a, 1, &(), None).unwrap();
    let y = w.begin_at(b, 2, &(), Some(x.owner())).unwrap();
    let key = w.intern("bytes").unwrap();
    w.attribute(y.owner(), key, vtr::Value::Bytes(vec![0, 255]))
        .unwrap();
    w.end_at(y, 2, vtr::Status::Aborted).unwrap();
    w.end_at(x, 100, vtr::Status::Ok).unwrap();
    let kind = w.intern("cause").unwrap();
    for _ in 0..2 {
        w.relate(
            kind,
            x.id(),
            y.id(),
            vec![vtr::Attribute {
                key,
                value: vtr::Value::F64(f64::NAN),
            }],
        )
        .unwrap();
    }
    w.close().unwrap();
    let mut server = Server::start(file.path());
    let budget = MemoryBudget::new(64 * 1024 * 1024);
    let mut client = RemoteClient::new(0, 10, 32 * 1024 * 1024, budget.clone()).unwrap();
    let LoadResult::Opened {
        tag: _,
        generation: 10,
        result,
    } = server.drive(&mut client)
    else {
        panic!("Open result");
    };
    let session = result.unwrap();
    assert!(
        client
            .submit(volna_trace::session::LoadRequest::Track {
                tag: 0,
                session: session.clone(),
                generation: 11,
                request_id: 71,
                track: TrackRef(a.id())
            })
            .is_ok()
    );
    let LoadResult::Track {
        tag: _,
        generation: 11,
        request_id: 71,
        track,
        result,
    } = server.drive(&mut client)
    else {
        panic!("track result");
    };
    assert_eq!(track, TrackRef(a.id()));
    let loaded = result.unwrap();
    let local = OpenSpec::Path(file.path().into())
        .open()
        .unwrap()
        .load_track(volna_trace::data::transactions::TrackRef(a.id()))
        .unwrap();
    assert_eq!(loaded.generators.len(), 1);
    for (a, b) in loaded.generators.iter().zip(&local.generators) {
        assert_eq!(a.transactions(), b.transactions());
        assert_eq!(
            bincode::serialize(a.relations()).unwrap(),
            bincode::serialize(b.relations()).unwrap()
        );
        for tx in a.transactions() {
            assert_eq!(a.parent(tx.id), b.parent(tx.id));
        }
    }
    assert!(
        client
            .submit(volna_trace::session::LoadRequest::Track {
                tag: 0,
                session: session.clone(),
                generation: 11,
                request_id: 72,
                track: TrackRef(u32::MAX)
            })
            .is_ok()
    );
    assert!(
        client
            .submit(volna_trace::session::LoadRequest::Track {
                tag: 0,
                session: session.clone(),
                generation: 11,
                request_id: 73,
                track: TrackRef(a.id())
            })
            .is_ok()
    );
    let LoadResult::Track {
        tag: _,
        request_id: 72,
        result,
        ..
    } = server.drive(&mut client)
    else {
        panic!("failed track");
    };
    assert!(result.is_err());
    let LoadResult::Track {
        tag: _,
        request_id: 73,
        result,
        ..
    } = server.drive(&mut client)
    else {
        panic!("next track");
    };
    assert_eq!(result.unwrap().generators[0].transactions().len(), 1);
    assert!(
        client
            .submit(volna_trace::session::LoadRequest::Track {
                tag: 0,
                session: session.clone(),
                generation: 11,
                request_id: 74,
                track: TrackRef(a.id())
            })
            .is_ok()
    );
    assert!(
        client
            .submit(volna_trace::session::LoadRequest::Track {
                tag: 0,
                session: session.clone(),
                generation: 11,
                request_id: 75,
                track: TrackRef(b.id())
            })
            .is_ok()
    );
    assert!(client.take_command().unwrap().is_some());
    let failed = client.disconnect("test disconnect");
    assert_eq!(failed.len(), 2);
    for (result, expected) in failed.into_iter().zip([74, 75]) {
        let LoadResult::Track {
            tag: _,
            generation: 11,
            request_id,
            result,
            ..
        } = result
        else {
            panic!("disconnect failure");
        };
        assert_eq!(request_id, expected);
        assert!(result.is_err());
    }
    drop(client);
    drop(session);
    assert_eq!(loaded.generators[0].transactions().len(), 1);
    assert!(budget.used() > 0);
    drop(loaded);
    assert_eq!(budget.used(), 0);
    server.close();
}

#[test]
fn metadata_limit_and_per_signal_failure_are_explicit() {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../volna-trace/tests/fixtures/picorv32.vtr");
    let mut server = Server::start(&path);
    assert!(server.open(1).unwrap_err().contains("limit"));
    assert!(read_packet(&mut server.output).unwrap().is_none());
    assert!(server.wait().success());
    let mut server = Server::start(&path);
    let meta = server.open(64 * 1024 * 1024).unwrap();
    let id = meta.hierarchy.var(0).signal.0;
    server.command(Command::Signals(vec![id, u32::MAX]));
    let replies =
        server.receive::<PackedHistory>(vec![ObjectId::Signal(id), ObjectId::Signal(u32::MAX)]);
    assert!(replies[0].is_ok());
    assert!(replies[1].is_err());
    server.close();
}

#[test]
fn changed_recording_invalidates_process() {
    let file = tempfile::NamedTempFile::new().unwrap();
    vtr::Writer::create(file.path()).unwrap().close().unwrap();
    let mut server = Server::start(file.path());
    server.open(64 * 1024 * 1024).unwrap();
    std::fs::OpenOptions::new()
        .append(true)
        .open(file.path())
        .unwrap()
        .write_all(&[0])
        .unwrap();
    server.command(Command::Track(0));
    assert!(read_packet(&mut server.output).unwrap().is_none());
    assert!(!server.wait().success());
}

#[test]
fn activity_sidecars_are_raw_cached_objects_shared_between_processes() {
    for (fixture, extension) in [("picorv32.vtr", "vtr"), ("values.vtr", "vtr")] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(format!("shared.{extension}"));
        std::fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join(format!("../volna-trace/tests/fixtures/{fixture}")),
            &path,
        )
        .unwrap();
        let mut first = Server::start(&path);
        let mut second = Server::start(&path);
        let a = first.open(64 << 20).unwrap();
        let b = second.open(64 << 20).unwrap();
        assert!(!a.activity.as_ref().unwrap().available);
        assert_eq!(a.activity, b.activity);
        assert!(!a.server.is_empty());
        first.command(Command::Activity {
            build: false,
            load_policy: volna_trace::data::ActivityLoadPolicy::Automatic,
        });
        assert!(first.receive::<Vec<u8>>(vec![ObjectId::Activity])[0].is_err());
        assert!(!path.with_extension(format!("{extension}.index")).exists());
        first.command(Command::Activity {
            build: true,
            load_policy: volna_trace::data::ActivityLoadPolicy::Automatic,
        });
        let image = first
            .receive::<Vec<u8>>(vec![ObjectId::Activity])
            .pop()
            .unwrap()
            .unwrap();
        let identity = a.activity.unwrap().identity().unwrap();
        let index = vtr::activity::Index::decode(&image, &identity).unwrap();
        assert_eq!(index.signal_count() as usize, a.info.signal_count);
        let index_path = path.with_extension(format!("{extension}.index"));
        let modified = std::fs::metadata(&index_path).unwrap().modified().unwrap();
        second.command(Command::Activity {
            build: true,
            load_policy: volna_trace::data::ActivityLoadPolicy::Automatic,
        });
        let other = second
            .receive::<Vec<u8>>(vec![ObjectId::Activity])
            .pop()
            .unwrap()
            .unwrap();
        assert_eq!(image, other);
        assert_eq!(
            modified,
            std::fs::metadata(&index_path).unwrap().modified().unwrap(),
            "second server reuses publication"
        );
        first.close();
        second.close();
    }
}

#[test]
fn deep_shared_values_close_after_local_remote_and_concurrent_owners_release() {
    const CHILD_DIRECTORY: &str = "VOLNA_TEST_SHARED_TEARDOWN_DIRECTORY";
    const LEVELS: usize = 100_000;
    let Some(directory) = std::env::var_os(CHILD_DIRECTORY) else {
        let directory = tempfile::tempdir().unwrap();
        let mut child = Process::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "deep_shared_values_close_after_local_remote_and_concurrent_owners_release",
                "--nocapture",
            ])
            .env(CHILD_DIRECTORY, directory.path())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        while child.try_wait().unwrap().is_none() {
            if std::time::Instant::now() >= deadline {
                child.kill().unwrap();
                let output = child.wait_with_output().unwrap();
                panic!(
                    "shared teardown exceeded its deadline: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "shared teardown child failed: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    };
    use std::sync::{Arc, Barrier};
    use volna_trace::{
        data::transactions::AttributeValue,
        session::{LoadRequest, LocalSession, Session},
    };
    use vtr::{Attribute, SourceOptions, Status, Value, Writer};

    fn check(mut value: &AttributeValue, levels: usize) {
        for _ in 0..levels {
            let AttributeValue::List(values) = value.resolved() else {
                panic!("description structure changed");
            };
            assert_eq!(values.len(), 1);
            value = &values[0];
        }
        assert!(matches!(value.resolved(), AttributeValue::U64(1)));
    }

    let path = std::path::PathBuf::from(directory).join("deep.vtr");
    let mut writer = Writer::create(&path).unwrap();
    let key = writer.intern("description").unwrap();
    let mut value = writer.intern_value(Value::U64(1)).unwrap();
    for _ in 0..LEVELS {
        value = writer
            .intern_value(Value::List(vec![Value::Ref(value)]))
            .unwrap();
    }
    let mut options = SourceOptions::new("instructions", None);
    options.attributes = vec![Attribute {
        key,
        value: Value::Ref(value),
    }];
    let source = writer.transactions::<()>(options, "INSTRUCTION").unwrap();
    let tx = writer.begin_at(source, 0, &(), None).unwrap();
    writer
        .attribute(tx.owner(), key, Value::Ref(value))
        .unwrap();
    writer.end_at(tx, 1, Status::Ok).unwrap();
    writer.close().unwrap();

    let session = LocalSession::open(&path).unwrap();
    check(&session.tracks()[0].attributes[0].1, LEVELS);
    drop(session);
    println!("closed local recording");

    let session = LocalSession::open(&path).unwrap();
    let loaded = session.load_track(TrackRef(source.id())).unwrap();
    let retained = loaded.generators[0].transactions()[0].attributes[0]
        .value
        .clone();
    drop(loaded);
    drop(session);
    check(&retained, LEVELS);
    drop(retained);
    println!("released retained description");

    let mut server = Server::start(&path);
    let budget = MemoryBudget::new(128 << 20);
    let mut client = RemoteClient::new(0, 10, 4 << 20, budget.clone()).unwrap();
    let LoadResult::Opened { result, .. } = server.drive(&mut client) else {
        panic!("open");
    };
    let session = result.unwrap();
    check(&session.tracks()[0].attributes[0].1, LEVELS);
    assert!(
        client
            .submit(LoadRequest::Track {
                tag: 0,
                session: session.clone(),
                generation: 10,
                request_id: 1,
                track: TrackRef(source.id())
            })
            .is_ok()
    );
    let LoadResult::Track { result, .. } = server.drive(&mut client) else {
        panic!("track");
    };
    let loaded = result.unwrap();
    let retained = Arc::clone(&loaded.generators[0]);
    drop(loaded);
    drop(session);
    drop(client);
    server.close();
    assert!(budget.used() > 0);
    check(&retained.transactions()[0].attributes[0].value, LEVELS);
    drop(retained);
    assert_eq!(budget.used(), 0);
    println!("closed remote recording and released its retained generator");

    let mut value = AttributeValue::U64(1);
    for level in 0..LEVELS {
        value = match level % 3 {
            0 => AttributeValue::shared(AttributeValue::Map(vec![
                ("previous".into(), value),
                ("sibling".into(), AttributeValue::Text("kept".into())),
            ])),
            1 => AttributeValue::shared(AttributeValue::List(vec![
                AttributeValue::Bool(true),
                value,
            ])),
            _ => AttributeValue::shared(value),
        };
    }
    let barrier = Arc::new(Barrier::new(9));
    let threads: Vec<_> = (0..8)
        .map(|_| {
            let value = value.clone();
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                drop(value);
            })
        })
        .collect();
    drop(value);
    barrier.wait();
    for thread in threads {
        thread.join().unwrap();
    }
    println!("released concurrent map, list, and alias owners");
}
