use volna_trace::{OpenSpec, data::transactions::TrackRef};
use vtr::{Payload, PayloadReader, Recorder, SourceOptions, Status, Writer};

struct Instruction {
    history: Vec<bool>,
    label: String,
}

impl Payload for Instruction {
    fn bind(types: &mut vtr::Types) -> vtr::Result<vtr::TypeId> {
        let history = Vec::<bool>::bind(types)?;
        let text = String::bind(types)?;
        types.structure(
            "InstructionWithHistory",
            vec![
                vtr::schema::Field::new("history", history),
                vtr::schema::Field::new("vtr.label", text),
            ],
        )
    }

    fn encode(&self, out: &mut Vec<u8>) -> vtr::Result<()> {
        self.history.encode(out)?;
        self.label.encode(out)
    }

    fn decode(reader: &mut PayloadReader<'_>) -> vtr::Result<Self> {
        Ok(Self {
            history: Vec::decode(reader)?,
            label: String::decode(reader)?,
        })
    }
}

#[test]
fn resource_labels_do_not_require_projecting_the_owners_history() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("history.vtr");
    let mut writer = Writer::create_with(
        &path,
        vtr::WriterOptions {
            mode: vtr::ExecutionMode::Inline,
            ..Default::default()
        },
    )
    .unwrap();
    let instructions = writer
        .transactions::<Instruction>(SourceOptions::new("instructions", None), "INSTRUCTION")
        .unwrap();
    let resources = writer
        .stages::<()>(SourceOptions::new("execute", None))
        .unwrap();
    let diagnostics = writer
        .transactions::<Vec<bool>>(SourceOptions::new("diagnostics", None), "INSTRUCTION")
        .unwrap();
    let label = writer.intern("vtr.label").unwrap();
    let payload = Instruction {
        history: vec![false; 2_000_000],
        label: "80000000: add a0,a1,a2".into(),
    };
    let first = writer.begin_at(instructions, 1, &payload, None).unwrap();
    let visit = writer
        .begin_stage_at(resources, first.owner(), 2, &())
        .unwrap();
    writer.end_stage_at(visit, 3).unwrap();
    writer.end_at(first, 4, Status::Ok).unwrap();
    let second = writer
        .begin_at(diagnostics, 5, &payload.history, None)
        .unwrap();
    writer
        .attribute(
            second.owner(),
            label,
            vtr::Value::Text("80000004: sub a0,a1,a2".into()),
        )
        .unwrap();
    let visit = writer
        .begin_stage_at(resources, second.owner(), 6, &())
        .unwrap();
    writer.end_stage_at(visit, 7).unwrap();
    writer.end_at(second, 8, Status::Ok).unwrap();
    writer.close().unwrap();

    let session = OpenSpec::Path(path).open().unwrap();
    let track = session.load_track(TrackRef(resources.id())).unwrap();
    let resources = &track.generators[0];
    let visits = resources.transactions();
    assert_eq!(visits.len(), 2);
    assert_eq!(
        resources.display_label(&visits[0]),
        Some("80000000: add a0,a1,a2")
    );
    assert_eq!(
        resources.display_label(&visits[1]),
        Some("80000004: sub a0,a1,a2")
    );
}
