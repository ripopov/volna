use volna_trace::remote::{
    memory::MemoryBudget,
    objects::TrackPayload,
    tracks::{TrackDecoder, TrackStep},
};
use volna_trace::{OpenSpec, data::transactions::TrackRef};

#[test]
fn remote_resource_loading_can_be_cancelled_after_transfer() {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/pipeline_showcase.vtr");
    let reader = vtr::Reader::open(&path).unwrap();
    let session = OpenSpec::Path(path).open().unwrap();
    let source = reader
        .find_generator("soc.cpu1.frontend.fetch_queue")
        .unwrap();
    let hardware = session.load_track(TrackRef(source)).unwrap();
    let bytes = bincode::serialize(&TrackPayload::from_loaded(&hardware)).unwrap();
    let budget = MemoryBudget::new(8 << 20);
    let mut decoder =
        TrackDecoder::new(bytes.len() as u64, bytes.len() as u64, &budget, session).unwrap();
    let mut chunks = bytes.chunks(4096);
    loop {
        match decoder.step().unwrap() {
            TrackStep::NeedInput => decoder.feed(chunks.next().unwrap().to_vec()).unwrap(),
            TrackStep::Yield if chunks.len() == 0 => break,
            TrackStep::Yield => {}
            TrackStep::Decoded(_) => panic!("resource loading must yield before completing"),
        }
    }
    assert!(budget.used() > 0);
    drop(decoder);
    assert_eq!(budget.used(), 0);
}

#[test]
fn instruction_visits_are_visible_on_shared_hardware_resource_tracks() {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/pipeline_showcase.vtr");
    let reader = vtr::Reader::open(&path).unwrap();
    let session = OpenSpec::Path(path).open().unwrap();

    for (core, resource) in [("cpu0", "fetch_buffer"), ("cpu1", "fetch_queue")] {
        let instructions = reader
            .find_generator(&format!("soc.{core}.pipeline.instruction"))
            .unwrap();
        let fetch = reader
            .find_generator(&format!("soc.{core}.frontend.{resource}"))
            .unwrap();
        let pipeline = session.load_track(TrackRef(instructions)).unwrap();
        let instruction = &pipeline.generators[0].transactions()[0];
        let visit = instruction
            .stages
            .iter()
            .find(|stage| stage.name == resource)
            .unwrap();
        assert_eq!(visit.generator, Some(TrackRef(fetch)));
        assert_eq!((visit.begin, visit.end), (1, Some(2)));

        let hardware = session.load_track(TrackRef(fetch)).unwrap();
        let bytes = bincode::serialize(&TrackPayload::from_loaded(&hardware)).unwrap();
        let budget = MemoryBudget::new(64 << 20);
        let mut decoder = TrackDecoder::new(
            bytes.len() as u64,
            bytes.len() as u64,
            &budget,
            session.clone(),
        )
        .unwrap();
        let mut chunks = bytes.chunks(4096);
        let remote = loop {
            match decoder.step().unwrap() {
                TrackStep::NeedInput => decoder.feed(chunks.next().unwrap().to_vec()).unwrap(),
                TrackStep::Yield => {}
                TrackStep::Decoded(track) => break track,
            }
        };
        for occupants in [&hardware.generators[0], &remote.generators[0]] {
            let occupant = occupants.transaction(visit.id).unwrap();
            assert_eq!((occupant.begin, occupant.end), (1, 2));
            assert_eq!(
                occupants.display_label(occupant),
                Some("80000000: lw   a1, 0(a2)")
            );
            assert!(
                !occupant
                    .attributes
                    .iter()
                    .any(|attr| attr.key == "vtr.label")
            );
            let owner = occupants.parent(occupant.id).unwrap();
            assert_eq!(owner.transaction, instruction.id);
            assert_eq!(owner.generator, Some(TrackRef(instructions)));
            assert!(occupants.transactions().iter().any(|other| {
                other.id != occupant.id
                    && occupants.parent(other.id).unwrap().transaction != instruction.id
            }));
        }
    }
}
