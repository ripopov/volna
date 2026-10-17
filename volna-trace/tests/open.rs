use volna_trace::OpenSpec;

#[test]
fn foreign_recordings_are_rejected_from_paths_and_bytes_regardless_of_name() {
    let dir = tempfile::tempdir().unwrap();
    for bytes in [
        include_bytes!("../../vtr/vtr-convert/tests/fixtures/values.fst").as_slice(),
        include_bytes!("../../vtr/vtr-convert/tests/fixtures/values-wrapped.fst").as_slice(),
        include_bytes!("../../vtr/vtr-convert/tests/fixtures/pipeline.kanata").as_slice(),
        b"not a recording",
        b"",
    ] {
        for name in ["recording.vtr", "recording.fst", "recording.kanata"] {
            let path = dir.path().join(name);
            std::fs::write(&path, bytes).unwrap();
            for spec in [
                OpenSpec::Path(path),
                OpenSpec::Bytes {
                    name: name.into(),
                    bytes: bytes.to_vec(),
                },
            ] {
                let error = spec.open().err().expect("foreign recording rejected");
                assert!(error.to_string().contains("expected VTR"), "{error:#}");
                assert!(error.to_string().contains("vtr convert"), "{error:#}");
            }
        }
    }
}

#[test]
fn vtr_header_determines_loading_independently_of_extension() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("recording.fst");
    let image = include_bytes!("fixtures/values.vtr");
    std::fs::write(&path, image).unwrap();
    for spec in [
        OpenSpec::Path(path),
        OpenSpec::Bytes {
            name: "recording.kanata".into(),
            bytes: image.to_vec(),
        },
    ] {
        let session = spec.open().unwrap();
        assert_eq!(session.format(), Some("VTR"));
        let signal = session.hierarchy().var(2).signal;
        let history = session.load_signal(signal).unwrap();
        assert_eq!(
            history.value(history.index_at(5)),
            volna_trace::data::WaveValue::Bits("01xzuwlh-".into())
        );
    }
}
