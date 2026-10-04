use std::path::{Path, PathBuf};
use vtr::{Reader, TxQuery, TxStatus, Value, Writer, WriterOptions};
fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}
fn writer(path: &Path) -> Writer {
    Writer::create_with(
        path,
        WriterOptions {
            dedup: false,
            background: false,
            ..Default::default()
        },
    )
    .unwrap()
}
fn attr<'a>(r: &Reader, tx: &'a vtr::Transaction, name: &str) -> &'a Value {
    &tx.attrs
        .iter()
        .find(|a| r.str(a.key) == name)
        .unwrap()
        .value
}
#[test]
fn kanata_cycles_threads_stages_labels_relations_and_open_instructions() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("trace.vtr");
    let mut w = writer(&path);
    vtr_cli::kanata::convert_kanata(fixture("pipeline.kanata"), &mut w).unwrap();
    w.close().unwrap();
    let r = Reader::open(path).unwrap();
    assert_eq!(r.meta().timescale, 0);
    assert!(r.meta().attrs.iter().any(
        |(k, v)| r.str(*k) == "time.unit" && matches!(v, Value::Str(s) if r.str(*s) == "cycle")
    ));
    let clock = &r.clocks()[0];
    assert_eq!(clock.path, "cpu.cycle");
    let timeline = r.clock(clock.id).unwrap();
    let stretch = &timeline.stretches()[0];
    assert_eq!((stretch.begin, stretch.end, stretch.period), (100, 106, 1));
    let streams: Vec<_> = r.streams().filter(|s| *s != clock.stream).collect();
    assert_eq!(streams.len(), 2);
    assert!(streams.iter().all(|s| r.stream_clock(*s) == Some(clock.id)));
    let all = r.transactions(&TxQuery::default()).unwrap();
    let find = |id| {
        all.iter()
            .find(|t| {
                t.attrs
                    .iter()
                    .any(|a| r.str(a.key) == "insn_id_in_sim" && a.value == Value::I64(id))
            })
            .unwrap()
    };
    let (a, b, c) = (find(10), find(11), find(12));
    assert_eq!((a.begin, a.end, a.status), (100, 103, TxStatus::Unset));
    assert_eq!((b.begin, b.end, b.status), (101, 103, TxStatus::Aborted));
    assert_eq!((c.begin, c.end, c.status), (103, 106, TxStatus::Open));
    assert!(matches!(attr(&r,a,"vtr.label"), Value::Str(s) if r.str(*s) == "load\nr1\nretired"));
    assert!(matches!(attr(&r,a,"detail"), Value::Str(s) if r.str(*s) == "address\n0x100"));
    assert_eq!((a.stages[0].begin, a.stages[0].end), (100, Some(101)));
    assert_eq!(r.str(a.stages[1].name), "X");
    assert!(a.stages[0]
        .attrs
        .iter()
        .any(|(k, v)| r.str(*k) == "vtr.label" && matches!(v,Value::Str(s) if r.str(*s)=="fetch")));
    let relations = r.relations_from(a.id).unwrap();
    assert_eq!(relations.len(), 1);
    assert_eq!(relations[0].to, b.id);
    assert_eq!(r.str(relations[0].kind), "wakeup");
}
#[test]
fn kanata_gzip_and_malformed_input() {
    use std::io::Write;
    let dir = tempfile::tempdir().unwrap();
    let gzip = dir.path().join("input");
    let mut encoder = flate2::write::GzEncoder::new(
        std::fs::File::create(&gzip).unwrap(),
        flate2::Compression::default(),
    );
    encoder
        .write_all(&std::fs::read(fixture("pipeline.kanata")).unwrap())
        .unwrap();
    encoder.finish().unwrap();
    let mut w = writer(&dir.path().join("gzip.vtr"));
    vtr_cli::kanata::convert_kanata(gzip, &mut w).unwrap();
    w.close().unwrap();
    for input in [
        "",
        "Kanata\t0003\n",
        "Kanata\t0004\nC=\t-1\n",
        "Kanata\t0004\nC=\t9\nC=\t8\n",
        "Kanata\t0004\nI\t0\n",
        "Kanata\t0004\nL\t8\t0\tmissing\n",
        "Kanata\t0004\nI\t0\t0\t0\nI\t0\t0\t0\n",
        "Kanata\t0004\nC=\t18446744073709551615\nC\t1\n",
        "Kanata\t0004\nunknown\n",
    ] {
        let source = dir.path().join("bad.kanata");
        std::fs::write(&source, input).unwrap();
        let mut w = writer(&dir.path().join("bad.vtr"));
        assert!(
            vtr_cli::kanata::convert_kanata(source, &mut w).is_err(),
            "{input}"
        );
    }
}
#[test]
fn vcd_timebase_aliases_values_events_and_export_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("trace.vtr");
    let mut w = writer(&path);
    vtr_cli::vcd::convert_vcd(fixture("waves.vcd"), &mut w).unwrap();
    w.close().unwrap();
    let r = Reader::open(&path).unwrap();
    assert_eq!(r.meta().timescale, -8);
    assert_eq!(r.time_range().unwrap().1, 10);
    let clk = r.find_signal("top.clk", '.').unwrap();
    assert_eq!(r.find_signal("top.alias", '.'), Some(clk));
    let bus = r.find_signal("top.bus [3:0]", '.').unwrap();
    assert_eq!(r.value_at(bus, 0).unwrap().to_ascii(), "10xz");
    assert_eq!(r.value_at(bus, 5).unwrap().to_ascii(), "z001");
    let real = r.find_signal("top.temperature", '.').unwrap();
    assert_eq!(r.value_at(real, 5).unwrap().to_ascii(), "-2.5");
    let event = r.find_signal("top.pulse", '.').unwrap();
    assert_eq!(
        r.changes(event, 0, 10)
            .unwrap()
            .iter()
            .map(|(t, _)| *t)
            .collect::<Vec<_>>(),
        [5, 5, 7]
    );
    let out = dir.path().join("export.vcd");
    vtr_cli::vcdout::write_vcd(&r, std::fs::File::create(&out).unwrap()).unwrap();
    let again = dir.path().join("again.vtr");
    let mut w = writer(&again);
    vtr_cli::vcd::convert_vcd(out, &mut w).unwrap();
    w.close().unwrap();
    let rr = Reader::open(again).unwrap();
    for name in ["top.clk", "top.bus [3:0]", "top.temperature", "top.pulse"] {
        let changes = |reader: &Reader| {
            reader
                .changes(reader.find_signal(name, '.').unwrap(), 0, 10)
                .unwrap()
                .iter()
                .map(|(t, v)| (*t, v.to_ascii()))
                .collect::<Vec<_>>()
        };
        assert_eq!(changes(&r), changes(&rr), "{name}");
    }
    assert_eq!(rr.time_range(), r.time_range());
}
#[test]
fn fst_conversion_matches_shared_histories_including_aliases_and_raw_values() {
    for name in ["values.fst", "values-wrapped.fst", "fst_types.fst"] {
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../volna-trace/tests/fixtures")
            .join(name);
        let session = volna_trace::OpenSpec::Path(source.clone()).open().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("trace.vtr");
        let mut w = writer(&out);
        vtr_cli::fst::convert_fst(&source, &mut w).unwrap();
        w.close().unwrap();
        let converted = volna_trace::OpenSpec::Path(out).open().unwrap();
        assert_eq!(session.info().timescale, converted.info().timescale);
        assert_eq!(session.info().time_range.1, converted.info().time_range.1);
        let (a, b) = (session.hierarchy(), converted.hierarchy());
        assert_eq!(a.var_count(), b.var_count());
        for id in 0..a.var_count() {
            assert_eq!(a.full_name(id), b.full_name(id));
            assert_eq!(a.var(id).shape, b.var(id).shape);
            assert_eq!(a.var(id).direction, b.var(id).direction);
            let x = session.load_signal(a.signal(id)).unwrap();
            let y = converted.load_signal(b.signal(id)).unwrap();
            assert_eq!(x.len(), y.len(), "{name}: {}", a.full_name(id));
            for i in 0..x.len() {
                assert_eq!(x.time(i), y.time(i));
                // Comparing bit patterns also covers NaN and signed zero reals.
                match (x.value(Some(i)), y.value(Some(i))) {
                    (
                        volna_trace::data::WaveValue::Real(a),
                        volna_trace::data::WaveValue::Real(b),
                    ) => assert_eq!(a.to_bits(), b.to_bits()),
                    (a, b) => assert_eq!(a, b),
                }
            }
            for other in 0..id {
                assert_eq!(
                    a.signal(id) == a.signal(other),
                    b.signal(id) == b.signal(other)
                );
            }
        }
    }
}
#[test]
fn export_rejects_unrepresentable_data_and_propagates_write_errors() {
    let dir = tempfile::tempdir().unwrap();
    for kind in [
        vtr::SignalKind::VarLen,
        vtr::SignalKind::bits(1, vtr::LogicStates::Nine),
    ] {
        let path = dir.path().join("trace.vtr");
        let mut w = writer(&path);
        w.add_var(
            None,
            "a",
            vtr::VarType::Wire,
            vtr::Direction::Implicit,
            kind,
        )
        .unwrap();
        w.close().unwrap();
        let r = Reader::open(path).unwrap();
        assert!(vtr_cli::vcdout::write_vcd(&r, Vec::new()).is_err());
    }
    let path = dir.path().join("trace.vtr");
    let mut w = writer(&path);
    vtr_cli::vcd::convert_vcd(fixture("waves.vcd"), &mut w).unwrap();
    w.close().unwrap();
    let r = Reader::open(path).unwrap();
    let mut tiny = [0u8; 8];
    assert!(vtr_cli::vcdout::write_vcd(&r, &mut tiny[..]).is_err());
}

#[test]
fn vcd_short_vectors_strings_and_dump_intervals() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("values.vcd");
    std::fs::write(&input, "$var wire 4 ! bus $end\n$var string 1 \" text $end\n$enddefinitions $end\n#0\nb1 !\nshello \"\n#2\n$dumpoff\nbx !\n$end\n#4\n$dumpon\nbz !\n$end\n").unwrap();
    let path = dir.path().join("values.vtr");
    let mut w = writer(&path);
    vtr_cli::vcd::convert_vcd(input, &mut w).unwrap();
    w.close().unwrap();
    let r = Reader::open(path).unwrap();
    assert_eq!(r.meta().timescale, -9);
    let bus = r.find_signal("bus", '.').unwrap();
    for (time, want) in [(0, "0001"), (2, "xxxx"), (4, "zzzz")] {
        assert_eq!(r.value_at(bus, time).unwrap().to_ascii(), want);
    }
    let text = r.find_signal("text", '.').unwrap();
    assert!(
        matches!(r.value_at(text,0).unwrap(), vtr::OwnedSignalValue::VarLen(bytes) if bytes == b"hello")
    );
    assert_eq!(
        r.blackout()
            .iter()
            .map(|b| (b.time, b.active))
            .collect::<Vec<_>>(),
        [(2, false), (4, true)]
    );
}

#[test]
fn vcd_malformed_commands_and_aliases_are_errors() {
    let dir = tempfile::tempdir().unwrap();
    let header = "$var wire 1 ! a $end\n$enddefinitions $end\n";
    for input in [
        format!("{header}#0\n1&\n"),
        format!("{header}#10\n0!\n#9\n1!\n"),
        format!("{header}#0\nb"),
        format!("{header}#0\nk!\n"),
        format!("{header}#0\nb11 !\n"),
        format!("{header}$dumpvars\n0!\n"),
        "$var wire 1 ! a $end\n$var wire 2 ! alias $end\n$enddefinitions $end\n".into(),
        "$var wire 0 ! a $end\n$enddefinitions $end\n".into(),
        "$timescale 2 ns $end\n$enddefinitions $end\n".into(),
    ] {
        let path = dir.path().join("bad.vcd");
        std::fs::write(&path, &input).unwrap();
        let mut w = writer(&dir.path().join("out.vtr"));
        assert!(vtr_cli::vcd::convert_vcd(path, &mut w).is_err(), "{input}");
    }
}
