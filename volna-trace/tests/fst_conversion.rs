//! Conversion fidelity and malformed FST inputs.

use std::path::PathBuf;
use std::sync::Arc;
use volna_trace::data::{SignalRef, WaveValue};
use volna_trace::session::OpenSpec;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../vtr/vtr-convert/tests/fixtures")
        .join(name)
}

fn converted(bytes: Vec<u8>) -> anyhow::Result<Arc<dyn volna_trace::Session>> {
    let dir = tempfile::tempdir()?;
    let input = dir.path().join("input.fst");
    let output = dir.path().join("output.vtr");
    std::fs::write(&input, bytes)?;
    let mut writer = vtr::Writer::create_with(
        &output,
        vtr::WriterOptions {
            dedup: false,
            background: false,
            ..Default::default()
        },
    )?;
    vtr_convert::fst::convert_fst(&input, &mut writer, &Default::default())?;
    writer.close()?;
    OpenSpec::Bytes {
        name: "output.vtr".into(),
        bytes: std::fs::read(output)?,
    }
    .open()
}

#[test]
fn declarations_aliases_and_every_sample_match_the_fst_decoder() {
    use fst_reader::{FstFilter, FstHierarchyEntry, FstReader, FstSignalValue};
    use std::collections::BTreeMap;

    for name in [
        "values.fst",
        "fst_types.fst",
        "zlib.fst",
        "landing_dram.fst",
    ] {
        let bytes = std::fs::read(fixture(name)).unwrap();
        let session = converted(bytes.clone()).unwrap();
        let mut reader = FstReader::open(std::io::Cursor::new(bytes)).unwrap();
        assert_eq!(
            session.info().timescale,
            reader.get_header().timescale_exponent
        );
        assert_eq!(session.info().time_range.1, reader.get_header().end_time);
        let mut declarations = Vec::new();
        reader
            .read_hierarchy(|entry| {
                if let FstHierarchyEntry::Var {
                    name,
                    tpe,
                    direction,
                    handle,
                    ..
                } = entry
                {
                    declarations.push((name, tpe, direction, handle.get_index()));
                }
            })
            .unwrap();
        assert_eq!(session.hierarchy().var_count(), declarations.len());
        let mut histories = BTreeMap::new();
        for (id, (name, tpe, direction, handle)) in declarations.iter().enumerate() {
            let var = session.hierarchy().var(id);
            assert_eq!(var.name, name);
            assert_eq!(var.var_type, vtr::VarType::from_code(*tpe as u16).name());
            let expected = match vtr::Direction::from_u8(*direction as u8) {
                vtr::Direction::Input => volna_trace::data::Direction::Input,
                vtr::Direction::Output => volna_trace::data::Direction::Output,
                vtr::Direction::InOut => volna_trace::data::Direction::InOut,
                _ => volna_trace::data::Direction::None,
            };
            assert_eq!(var.direction, expected);
            let history = session.load_signal(var.signal).unwrap();
            if let Some((signal, _, _)) = histories.get(handle) {
                assert_eq!(var.signal, *signal, "alias {name}");
            } else {
                histories.insert(*handle, (var.signal, history, 0usize));
            }
        }
        reader
            .read_signals(
                &FstFilter::all(),
                |time, handle, value| -> anyhow::Result<()> {
                    let (_, history, offset) = histories.get_mut(&handle.get_index()).unwrap();
                    assert_eq!(history.time(*offset), time, "{name}: {handle:?}");
                    let actual = history.value(Some(*offset));
                    match value {
                        FstSignalValue::Real(expected) => {
                            let WaveValue::Real(actual) = actual else {
                                panic!("expected real")
                            };
                            assert_eq!(actual.to_bits(), expected.to_bits());
                        }
                        FstSignalValue::String(bytes) => {
                            let expected = if history.shape()
                                == volna_trace::data::SignalShape::Text
                            {
                                WaveValue::Bytes(bytes.to_vec())
                            } else {
                                WaveValue::Bits(std::str::from_utf8(bytes)?.to_ascii_lowercase())
                            };
                            assert_eq!(actual, expected, "{name}: {handle:?} at {time}");
                        }
                    }
                    *offset += 1;
                    Ok(())
                },
            )
            .unwrap();
        for (_, history, count) in histories.into_values() {
            assert_eq!(history.len(), count, "{name}: sample count");
        }
    }
}

#[test]
fn surfer_fst_types_loads() {
    let session = converted(std::fs::read(fixture("fst_types.fst")).unwrap()).unwrap();
    let signals: Vec<_> = session.hierarchy().vars().map(|v| v.signal).collect();
    assert_eq!(signals.len(), 76);
    for (signal, result) in session.load_signals(&signals) {
        let history = result.unwrap_or_else(|error| panic!("{signal:?}: {error:#}"));
        assert!(!history.is_empty(), "{signal:?}");
        let variable = session
            .hierarchy()
            .vars()
            .find(|v| v.signal == signal)
            .unwrap();
        if variable.var_type == "port" {
            assert!(matches!(history.value(Some(0)), WaveValue::Bytes(bytes) if bytes.len() == 8));
        }
    }
}

#[test]
fn equivalent_verilator_traces_agree_at_every_change() {
    for design in ["features", "operators", "pipeline"] {
        let fst = converted(std::fs::read(fixture(&format!("{design}.fst"))).unwrap()).unwrap();
        let vtr = OpenSpec::Path(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("tests/fixtures/{design}.vtr")),
        )
        .open()
        .unwrap();
        assert_eq!(fst.info().timescale, vtr.info().timescale);
        let signals: Vec<_> = fst.hierarchy().vars().map(|v| v.signal).collect();
        let histories = fst.load_signals(&signals);
        for (id, variable) in fst.hierarchy().vars().enumerate() {
            let name = fst.hierarchy().full_name(id);
            let other = vtr
                .hierarchy()
                .vars()
                .enumerate()
                .find(|(i, _)| vtr.hierarchy().full_name(*i) == name)
                .unwrap_or_else(|| panic!("missing {name}"));
            assert_eq!(variable.shape, other.1.shape);
            let a = histories[id].1.as_ref().unwrap();
            let b = vtr.load_signal(other.1.signal).unwrap();
            let times = (0..a.len())
                .map(|i| a.time(i))
                .chain((0..b.len()).map(|i| b.time(i)));
            for time in times {
                let left = a.value(a.index_at(time));
                let right = b.value(b.index_at(time));
                if matches!((&left, &right), (WaveValue::Real(a), WaveValue::Real(b)) if a.is_nan() && b.is_nan())
                {
                    continue;
                }
                assert_eq!(left, right, "{design}: {name} at {time}");
            }
        }
    }
}

#[test]
fn malformed_images_report_errors() {
    for bytes in [vec![], b"not a trace".to_vec(), vec![0; 9]] {
        assert!(converted(bytes).is_err());
    }
}

#[test]
fn truncated_sections_and_overflowing_lengths_return_errors() {
    let original = include_bytes!("../../vtr/vtr-convert/tests/fixtures/values.fst");
    for end in [9, 100, 329, original.len() - 1] {
        assert!(converted(original[..end].to_vec()).is_err(), "prefix {end}");
    }
    for size in [0u64, 7, u64::MAX] {
        let mut bytes = original.to_vec();
        let offset = 330;
        bytes[offset + 1..offset + 9].copy_from_slice(&size.to_be_bytes());
        assert!(converted(bytes).is_err());
    }
}

#[test]
fn gzip_wrapper_preserves_values_and_rejects_truncation() {
    let wrapped = include_bytes!("../../vtr/vtr-convert/tests/fixtures/values-wrapped.fst");
    assert_eq!(wrapped[0], 254);
    let open = |bytes: Vec<u8>| converted(bytes);
    let a = open(wrapped.to_vec()).unwrap();
    let b =
        open(include_bytes!("../../vtr/vtr-convert/tests/fixtures/values.fst").to_vec()).unwrap();
    assert_eq!(a.info().time_range, b.info().time_range);
    for id in 0..4 {
        let a = a.load_signal(a.hierarchy().var(id).signal).unwrap();
        let b = b.load_signal(b.hierarchy().var(id).signal).unwrap();
        assert_eq!(a.len(), b.len());
        for i in 0..a.len() {
            assert_eq!(a.time(i), b.time(i));
            assert_eq!(a.value(Some(i)), b.value(Some(i)));
        }
    }
    assert!(open(wrapped[..wrapped.len() - 1].to_vec()).is_err());
}

#[test]
fn unsupported_time_and_dump_metadata_is_explicit_even_when_wrapped() {
    use std::io::Write;
    let original = include_bytes!("../../vtr/vtr-convert/tests/fixtures/values.fst");
    let mut shifted = original.to_vec();
    shifted[322..330].copy_from_slice(&(-10i64).to_be_bytes());
    let mut blackout = original.to_vec();
    blackout.push(2);
    blackout.extend_from_slice(&13u64.to_be_bytes());
    blackout.extend_from_slice(&[2, 0, 10, 1, 5]);
    for (bytes, expected) in [(shifted, "time offsets"), (blackout, "dump-off")] {
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(&bytes).unwrap();
        let compressed = encoder.finish().unwrap();
        let mut wrapped = vec![254];
        wrapped.extend_from_slice(&(compressed.len() as u64 + 16).to_be_bytes());
        wrapped.extend_from_slice(&(bytes.len() as u64).to_be_bytes());
        wrapped.extend_from_slice(&compressed);
        for image in [bytes, wrapped] {
            let error = converted(image).err().expect("unsupported metadata");
            assert!(error.to_string().contains(expected), "{error:#}");
        }
    }
}

#[test]
fn raw_bytes_reals_nine_states_and_time_boundaries() {
    let fst = converted(include_bytes!("../../vtr/vtr-convert/tests/fixtures/values.fst").to_vec())
        .unwrap();
    assert_eq!(fst.info().timescale, -12);
    assert_eq!(fst.info().time_range, (5, 20));
    let vars: Vec<_> = fst.hierarchy().vars().collect();
    let signals: Vec<_> = vars.iter().map(|v| v.signal).collect();
    let loaded = fst.load_signals(&signals);
    let bytes = loaded[0].1.as_ref().unwrap();
    assert_eq!(bytes.index_at(4), None);
    for result in &loaded[..4] {
        let history = result.1.as_ref().unwrap();
        assert_eq!(history.index_at(4), None);
    }
    assert_eq!(
        bytes.value(bytes.index_at(5)),
        WaveValue::Bytes(vec![b'a', 0, 255, b'\\', b'\n', 128])
    );
    assert_eq!(bytes.value(bytes.index_at(10)), WaveValue::Bytes(vec![]));
    assert_eq!(bytes.value(bytes.index_at(20)), WaveValue::Bytes(vec![]));
    let real = loaded[1].1.as_ref().unwrap();
    assert_eq!(real.value(real.index_at(5)), WaveValue::Real(1.25));
    assert_eq!(real.value(real.index_at(10)), WaveValue::Real(-2.5));
    let logic = loaded[2].1.as_ref().unwrap();
    assert_eq!(
        logic.value(logic.index_at(5)),
        WaveValue::Bits("01xzuwlh-".into())
    );
    assert_eq!(
        logic.value(logic.index_at(9)),
        WaveValue::Bits("01xzuwlh-".into())
    );
    assert_eq!(
        logic.value(logic.index_at(10)),
        WaveValue::Bits("111111111".into())
    );
    assert_eq!(vars[2].signal, vars[3].signal);
    assert!(fst.load_signal(SignalRef(u32::MAX)).is_err());
    assert!(fst.load_signals(&[]).is_empty());
    let event = loaded[4].1.as_ref().unwrap();
    assert_eq!(event.shape(), volna_trace::data::SignalShape::Event);
    assert_eq!(
        (0..event.len()).map(|i| event.time(i)).collect::<Vec<_>>(),
        vec![5, 10]
    );
    let file = tempfile::NamedTempFile::new().unwrap();
    let mut writer = vtr::Writer::create(file.path()).unwrap();
    let (_, signal) = writer
        .add_var(
            None,
            "bytes",
            vtr::VarType::String,
            vtr::Direction::Implicit,
            vtr::SignalKind::VarLen,
        )
        .unwrap();
    writer.set_time(5).unwrap();
    writer
        .emit_varlen(signal, &[b'a', 0, 255, b'\\', b'\n', 128])
        .unwrap();
    writer.set_time(10).unwrap();
    writer.emit_varlen(signal, &[]).unwrap();
    writer.close().unwrap();
    let vtr = OpenSpec::Path(file.path().into()).open().unwrap();
    let twin = vtr.load_signal(vtr.hierarchy().var(0).signal).unwrap();
    let signal = vtr.hierarchy().var(0).signal;
    let repeated = vtr.load_signals(&[signal, SignalRef(u32::MAX), signal]);
    assert!(repeated[1].1.is_err());
    assert!(Arc::ptr_eq(
        repeated[0].1.as_ref().unwrap(),
        repeated[2].1.as_ref().unwrap()
    ));
    for time in [5, 9, 10, 20] {
        assert_eq!(
            bytes.value(bytes.index_at(time)),
            twin.value(twin.index_at(time))
        );
    }
}
