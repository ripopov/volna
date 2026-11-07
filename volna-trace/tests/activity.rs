//! Activity loading across resource limits, explicit approval, and invalid sidecars.

use std::{path::Path, sync::Arc};
use volna_trace::{
    data::{ActivityLoadPolicy, ActivityMemoryRequired},
    remote::memory::MemoryBudget,
    session::OpenSpec,
};
use vtr::{
    Recorder,
    activity::{BuildControl, BuildOptions, Identity, Index, Sidecar},
};

fn recording(path: &Path) {
    let mut options = vtr::WriterOptions::default();
    options.encoding.waveform_records = 256;
    let mut writer = vtr::Writer::create_with(path, options).unwrap();
    let signal = writer
        .signal(
            vtr::SourceOptions::new("busy", None),
            vtr::SignalKind::Bits {
                width: 1,
                states: 2,
            },
            vtr::VarType::Wire,
            vtr::Direction::Implicit,
        )
        .unwrap();
    for step in 0..50_000 {
        writer.advance_time(step * 3).unwrap();
        writer.emit_u64(signal, step & 1).unwrap();
    }
    writer.close().unwrap();
}

fn options() -> BuildOptions {
    BuildOptions {
        memory: 8 << 20,
        control: Some(Arc::new(BuildControl::default())),
        ..Default::default()
    }
}

fn header_only_sidecar(valid: &[u8]) -> Vec<u8> {
    let mut bytes = valid[..112].to_vec();
    let mut directory = 1u64.to_le_bytes().to_vec();
    directory.extend_from_slice(&1u32.to_le_bytes());
    directory.extend_from_slice(&0u32.to_le_bytes());
    for number in [32u64, 56, 0, 0] {
        directory.extend_from_slice(&number.to_le_bytes());
    }
    bytes.extend_from_slice(&7u32.to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(&(directory.len() as u64).to_le_bytes());
    bytes.extend_from_slice(&crc32fast::hash(&directory).to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(&directory);
    bytes.extend_from_slice(&112u64.to_le_bytes());
    let length = bytes.len() as u64 + 16;
    bytes.extend_from_slice(&length.to_le_bytes());
    bytes.extend_from_slice(b"VTR_END\0");
    bytes
}

#[test]
fn automatic_loading_respects_object_limits_and_approval_is_scoped_to_the_index() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("busy.vtr");
    recording(&path);
    let session = OpenSpec::Path(path.clone()).open().unwrap();
    let budget = MemoryBudget::new(64 << 20);
    budget.set_object_limit(1 << 20);
    let error = session
        .build_activity(&options(), &budget, None, ActivityLoadPolicy::Automatic)
        .unwrap_err();
    let required = error.downcast_ref::<ActivityMemoryRequired>().unwrap().0;
    assert!(session.activity().is_none());
    assert_eq!(budget.used(), 0);
    session
        .build_activity(
            &options(),
            &budget,
            None,
            ActivityLoadPolicy::Approved {
                peak_bytes: required.peak,
            },
        )
        .unwrap();
    let index = session.activity().unwrap();
    assert!(index.memory_bytes() > budget.object_limit());
    assert_eq!(index.classify(15, 15), vec![vtr::SignalId(0)]);
    assert!(index.classify(16, 17).is_empty());
    assert_eq!(budget.object_limit(), 1 << 20);
    assert!(budget.reserve_object("another object", 2 << 20).is_err());
    drop(index);
    drop(session);
    assert_eq!(budget.used(), 0);
    let reopened = OpenSpec::Path(path).open().unwrap();
    assert!(reopened.activity_available());
    assert!(reopened.activity().is_none());
    let error = reopened
        .build_activity(&options(), &budget, None, ActivityLoadPolicy::Automatic)
        .unwrap_err();
    assert!(error.downcast_ref::<ActivityMemoryRequired>().is_some());
    assert!(reopened.activity().is_none());
    assert_eq!(budget.used(), 0);
}

#[test]
fn malformed_adjacent_indexes_fall_back_to_valid_cache_or_are_rebuilt() {
    let dir = tempfile::tempdir().unwrap();
    for use_cache in [true, false] {
        let path = dir.path().join(format!("run-{use_cache}.vtr"));
        recording(&path);
        let reader = vtr::Reader::open(&path).unwrap();
        let identity = Identity::of(&reader).unwrap();
        let mut valid = Vec::new();
        vtr::activity::build(&reader, &mut valid, &options()).unwrap();
        let mut malformed = header_only_sidecar(&valid);
        if use_cache {
            malformed[72..76].copy_from_slice(&u32::MAX.to_le_bytes());
            let crc = crc32fast::hash(&malformed[56..112]);
            malformed[48..52].copy_from_slice(&crc.to_le_bytes());
        }
        assert!(Index::estimate_memory(&malformed, &identity).is_ok());
        assert!(Index::decode(&malformed, &identity).is_err());
        let cache = dir.path().join(format!("cache-{use_cache}"));
        std::fs::create_dir_all(&cache).unwrap();
        let sidecar = Sidecar::new(&path, &identity, Some(&cache));
        std::fs::write(&sidecar.beside, &malformed).unwrap();
        if use_cache {
            std::fs::write(sidecar.cached.as_ref().unwrap(), &valid).unwrap();
        }
        let session = OpenSpec::Path(path).open().unwrap();
        if use_cache {
            assert_eq!(session.activity_image(Some(&cache)).unwrap(), valid);
        }
        session
            .build_activity(
                &options(),
                &MemoryBudget::new(64 << 20),
                Some(&cache),
                ActivityLoadPolicy::Automatic,
            )
            .unwrap();
        let index = session.activity().unwrap();
        assert_eq!(index.classify(15, 15), vec![vtr::SignalId(0)]);
        assert!(index.classify(16, 17).is_empty());
        if use_cache {
            assert_eq!(std::fs::read(&sidecar.beside).unwrap(), malformed);
        } else {
            Index::decode_for(&std::fs::read(&sidecar.beside).unwrap(), &reader).unwrap();
        }
    }
}
