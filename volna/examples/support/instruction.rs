//! Instruction payload shared by the simulated chip recordings.

use vtr::{Payload, PayloadReader};

pub(super) struct Instruction {
    pub(super) label: String,
    pub(super) insn_id: u64,
    pub(super) pc: u64,
}
impl Payload for Instruction {
    fn bind(types: &mut vtr::Types) -> vtr::Result<vtr::TypeId> {
        let label_type = <String as Payload>::bind(types)?;
        let insn_id_type = <u64 as Payload>::bind(types)?;
        let pc_type = <u64 as Payload>::bind(types)?;
        types.structure(
            "Instruction",
            vec![
                vtr::schema::Field::new("vtr.label", label_type),
                vtr::schema::Field::new("insn_id", insn_id_type),
                vtr::schema::Field::new("pc", pc_type),
            ],
        )
    }
    fn encode(&self, out: &mut Vec<u8>) -> vtr::Result<()> {
        self.label.encode(out)?;
        self.insn_id.encode(out)?;
        self.pc.encode(out)?;
        Ok(())
    }
    fn decode(reader: &mut PayloadReader<'_>) -> vtr::Result<Self> {
        Ok(Self {
            label: <String as Payload>::decode(reader)?,
            insn_id: <u64 as Payload>::decode(reader)?,
            pc: <u64 as Payload>::decode(reader)?,
        })
    }
}
