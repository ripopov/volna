//! A two-microsecond debugging lab with pipeline hazards, DMA, fault recovery, and typed value galleries.

use std::path::Path;
use vtr::schema::{Address, Bytes, StringId, TimeValue};
use vtr::{
    Attribute, Direction, ExecutionMode, NodeData, NodeId, Payload, PayloadReader, Reader,
    RecordKind, RecordQuery, Recorder, ScopeType, SignalId, SignalKind, SourceOptions, SpanKind,
    Status, TransactionQuery, Value, VarType, Writer, WriterOptions,
};

struct Instruction {
    label: String,
    pc: Address,
}
impl Payload for Instruction {
    fn bind(types: &mut vtr::Types) -> vtr::Result<vtr::TypeId> {
        let label_type = <String as Payload>::bind(types)?;
        let pc_type = <Address as Payload>::bind(types)?;
        types.structure(
            "Instruction",
            vec![
                vtr::schema::Field::new("vtr.label", label_type),
                vtr::schema::Field::new("pc", pc_type),
            ],
        )
    }
    fn encode(&self, out: &mut Vec<u8>) -> vtr::Result<()> {
        self.label.encode(out)?;
        self.pc.encode(out)?;
        Ok(())
    }
    fn decode(reader: &mut PayloadReader<'_>) -> vtr::Result<Self> {
        Ok(Self {
            label: <String as Payload>::decode(reader)?,
            pc: <Address as Payload>::decode(reader)?,
        })
    }
}

struct Label {
    label: String,
}
impl Payload for Label {
    fn bind(types: &mut vtr::Types) -> vtr::Result<vtr::TypeId> {
        let label_type = <String as Payload>::bind(types)?;
        types.structure(
            "Label",
            vec![vtr::schema::Field::new("vtr.label", label_type)],
        )
    }
    fn encode(&self, out: &mut Vec<u8>) -> vtr::Result<()> {
        self.label.encode(out)?;
        Ok(())
    }
    fn decode(reader: &mut PayloadReader<'_>) -> vtr::Result<Self> {
        Ok(Self {
            label: <String as Payload>::decode(reader)?,
        })
    }
}

struct CyclePhase {
    cycle: u64,
    phase: String,
    label: String,
}
impl Payload for CyclePhase {
    fn bind(types: &mut vtr::Types) -> vtr::Result<vtr::TypeId> {
        let cycle_type = <u64 as Payload>::bind(types)?;
        let phase_type = <String as Payload>::bind(types)?;
        let label_type = <String as Payload>::bind(types)?;
        types.structure(
            "CyclePhase",
            vec![
                vtr::schema::Field::new("cycle", cycle_type),
                vtr::schema::Field::new("phase", phase_type),
                vtr::schema::Field::new("vtr.label", label_type),
            ],
        )
    }
    fn encode(&self, out: &mut Vec<u8>) -> vtr::Result<()> {
        self.cycle.encode(out)?;
        self.phase.encode(out)?;
        self.label.encode(out)?;
        Ok(())
    }
    fn decode(reader: &mut PayloadReader<'_>) -> vtr::Result<Self> {
        Ok(Self {
            cycle: <u64 as Payload>::decode(reader)?,
            phase: <String as Payload>::decode(reader)?,
            label: <String as Payload>::decode(reader)?,
        })
    }
}

struct Arguments {
    boolean: bool,
    signed: i64,
    unsigned: u64,
    real: f64,
    interned: StringId,
    bytes: Bytes,
    time: TimeValue,
    pointer: Address,
    text: String,
    label: String,
}
impl Payload for Arguments {
    fn bind(types: &mut vtr::Types) -> vtr::Result<vtr::TypeId> {
        let boolean_type = <bool as Payload>::bind(types)?;
        let signed_type = <i64 as Payload>::bind(types)?;
        let unsigned_type = <u64 as Payload>::bind(types)?;
        let real_type = <f64 as Payload>::bind(types)?;
        let interned_type = <StringId as Payload>::bind(types)?;
        let bytes_type = <Bytes as Payload>::bind(types)?;
        let time_type = <TimeValue as Payload>::bind(types)?;
        let pointer_type = <Address as Payload>::bind(types)?;
        let text_type = <String as Payload>::bind(types)?;
        let label_type = <String as Payload>::bind(types)?;
        types.structure(
            "Arguments",
            vec![
                vtr::schema::Field::new("boolean", boolean_type),
                vtr::schema::Field::new("signed", signed_type),
                vtr::schema::Field::new("unsigned", unsigned_type),
                vtr::schema::Field::new("real", real_type),
                vtr::schema::Field::new("interned", interned_type),
                vtr::schema::Field::new("bytes", bytes_type),
                vtr::schema::Field::new("time", time_type),
                vtr::schema::Field::new("pointer", pointer_type),
                vtr::schema::Field::new("text", text_type),
                vtr::schema::Field::new("vtr.label", label_type),
            ],
        )
    }
    fn encode(&self, out: &mut Vec<u8>) -> vtr::Result<()> {
        self.boolean.encode(out)?;
        self.signed.encode(out)?;
        self.unsigned.encode(out)?;
        self.real.encode(out)?;
        self.interned.encode(out)?;
        self.bytes.encode(out)?;
        self.time.encode(out)?;
        self.pointer.encode(out)?;
        self.text.encode(out)?;
        self.label.encode(out)?;
        Ok(())
    }
    fn decode(reader: &mut PayloadReader<'_>) -> vtr::Result<Self> {
        Ok(Self {
            boolean: <bool as Payload>::decode(reader)?,
            signed: <i64 as Payload>::decode(reader)?,
            unsigned: <u64 as Payload>::decode(reader)?,
            real: <f64 as Payload>::decode(reader)?,
            interned: <StringId as Payload>::decode(reader)?,
            bytes: <Bytes as Payload>::decode(reader)?,
            time: <TimeValue as Payload>::decode(reader)?,
            pointer: <Address as Payload>::decode(reader)?,
            text: <String as Payload>::decode(reader)?,
            label: <String as Payload>::decode(reader)?,
        })
    }
}

fn attributes(w: &mut Writer) -> vtr::Result<Vec<Attribute>> {
    let ready = w.intern("READY")?;
    let inner = w.intern("samples")?;
    [
        ("optional", Value::Null),
        ("enabled", Value::Bool(true)),
        ("signed_i8", Value::I8(-8)),
        ("signed_i16", Value::I16(-16)),
        ("signed_i32", Value::I32(-32)),
        ("unsigned_u8", Value::U8(8)),
        ("unsigned_u16", Value::U16(16)),
        ("unsigned_u32", Value::U32(32)),
        ("float32", Value::F32(0.25)),
        ("signed", Value::I64(-42)),
        ("unsigned", Value::U64(1 << 60)),
        ("ratio", Value::F64(0.625)),
        ("state", Value::Str(ready)),
        ("packet", Value::Bytes(vec![0, 0x7f, 0x80, 0xff])),
        (
            "mask",
            Value::Bits {
                width: 8,
                data: vec![0xa5],
            },
        ),
        (
            "bus",
            Value::Logic {
                width: 4,
                data: vec![0xe4],
            },
        ),
        (
            "resolved",
            Value::Logic9 {
                width: 9,
                data: vec![0x10, 0x32, 0x54, 0x76, 8],
            },
        ),
        ("deadline", Value::Time(160)),
        (
            "opcode",
            Value::Enum {
                value: 2,
                name: ready,
            },
        ),
        ("handle", Value::Pointer(0x1000)),
        ("gain", Value::Fixed { raw: -13, scale: 3 }),
        (
            "voltage",
            Value::UFixed {
                raw: 3300,
                scale: 10,
            },
        ),
        (
            "vector",
            Value::List(vec![Value::I64(-1), Value::Bool(false)]),
        ),
        (
            "nested",
            Value::Map(vec![Attribute {
                key: inner,
                value: Value::List(vec![Value::F64(1.5), Value::Null]),
            }]),
        ),
        ("message", Value::Text("DMA → memory: café ✓".into())),
    ]
    .into_iter()
    .map(|(key, value)| {
        Ok(Attribute {
            key: w.intern(key)?,
            value,
        })
    })
    .collect()
}

const STAGES: [&str; 5] = ["fetch", "decode", "execute", "memory", "writeback"];
const RESET_CYCLE: u64 = 4; // first rising edge after reset_n rises at 32 ns
const LOOP_ITERATIONS: u64 = 12;

/// The rising edge that begins core clock cycle `cycle`: `soc.clk` rises at 4 ns, every 8 ns.
fn cycle_time(cycle: u64) -> u64 {
    4 + 8 * cycle
}

#[derive(Clone, Copy, PartialEq)]
enum Op {
    Alu,
    Mul,
    Load,
    Store,
    Branch(u64),
    Halt,
}

struct Instr {
    asm: &'static str,
    op: Op,
    dst: Option<&'static str>,
    src: &'static [&'static str],
}

const fn ins(
    asm: &'static str,
    op: Op,
    dst: Option<&'static str>,
    src: &'static [&'static str],
) -> Instr {
    Instr { asm, op, dst, src }
}

const TEXT: u64 = 0x8000_0000;
const LOOP: u64 = TEXT + 0x18;
/// Boot, then a loop that sums and multiplies pairs of words, stores a
/// result on some iterations and rings the DMA doorbell (`8(s1)`) on each.
const PROGRAM: &[Instr] = &[
    ins("lui s1,0x10000", Op::Alu, Some("s1"), &[]),
    ins("sw zero,8(s1)", Op::Store, None, &["s1"]),
    ins("lui a0,0x80010", Op::Alu, Some("a0"), &[]),
    ins("addi s2,zero,12", Op::Alu, Some("s2"), &[]),
    ins("addi a1,zero,0", Op::Alu, Some("a1"), &[]),
    ins("addi a2,zero,0", Op::Alu, Some("a2"), &[]),
    ins("lw t1,0(a0)", Op::Load, Some("t1"), &["a0"]),
    ins("add a1,a1,t1", Op::Alu, Some("a1"), &["a1", "t1"]),
    ins("lw t2,4(a0)", Op::Load, Some("t2"), &["a0"]),
    ins("mul t3,t2,t1", Op::Mul, Some("t3"), &["t2", "t1"]),
    ins("addi a0,a0,8", Op::Alu, Some("a0"), &["a0"]),
    ins("xor a2,a2,t3", Op::Alu, Some("a2"), &["a2", "t3"]),
    ins("andi t4,a1,3", Op::Alu, Some("t4"), &["a1"]),
    ins(
        "beq t4,zero,0x8000003c",
        Op::Branch(TEXT + 0x3c),
        None,
        &["t4"],
    ),
    ins("sw a2,-8(a0)", Op::Store, None, &["a2", "a0"]),
    ins("sw a1,8(s1)", Op::Store, None, &["a1", "s1"]),
    ins("addi s2,s2,-1", Op::Alu, Some("s2"), &["s2"]),
    ins("bne s2,zero,0x80000018", Op::Branch(LOOP), None, &["s2"]),
    ins("fence", Op::Alu, None, &[]),
    ins("wfi", Op::Halt, None, &[]),
];

/// A deterministic hash for "random" outcomes.
fn mix(mut x: u64) -> u64 {
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

struct Stall {
    name: &'static str,
    label: String,
    begin: u64,
    end: u64,
}

/// One fetched instruction; times are core clock cycles.
struct Insn {
    pc: u64,
    asm: &'static str,
    op: Op,
    wrong_path: bool,
    /// Position on the committed path (wrong-path instructions share their predecessor's).
    order: u64,
    enter: [Option<u64>; 5],
    end: u64,
    squashed: bool,
    busy: u64,
    /// Source register and producing instruction.
    deps: Vec<(&'static str, usize)>,
    miss: bool,
    /// The cycle the injected fault caught this access in memory.
    fault: Option<u64>,
    redirect: Option<u64>,
    waiting: Option<(u64, &'static str, usize)>,
    stalls: Vec<Stall>,
}

impl Insn {
    fn label(&self) -> String {
        let path = if self.wrong_path { " (wrong path)" } else { "" };
        format!("0x{:08x} {}{path}", self.pc, self.asm)
    }
}

fn simulate() -> Vec<Insn> {
    let mut run: Vec<Insn> = Vec::new();
    let mut slot: [Option<usize>; 5] = [None; 5];
    let mut last_writer: std::collections::HashMap<&str, usize> = Default::default();
    let (mut fetch_pc, mut wrong_path, mut halted) = (TEXT, false, false);
    let (mut order, mut iteration, mut fault_pending) = (0u64, 0u64, true);
    let mut cycle = RESET_CYCLE;
    let recovery = (896u64 - 4).div_ceil(8);
    let is_access = |insn: &Insn| matches!(insn.op, Op::Load | Op::Store) && !insn.wrong_path;
    loop {
        // The fault catches the access in memory, or the next one to get there.
        if fault_pending
            && cycle_time(cycle) >= 768
            && let Some(k) = slot[3].filter(|&k| is_access(&run[k]))
        {
            fault_pending = false;
            run[k].fault = Some(cycle);
            run[k].busy = recovery;
        }
        if let Some(k) = slot[4].filter(|&k| run[k].busy <= cycle) {
            run[k].end = cycle;
            slot[4] = None;
        }
        // Memory to writeback; a long memory access is a stall.
        if let Some(k) = slot[3].filter(|&k| run[k].busy <= cycle && slot[4].is_none()) {
            let entered = run[k].enter[3].unwrap();
            let fault = run[k].fault.map_or(cycle, |at| at.max(entered + 1));
            if fault > entered + 1 {
                let label = format!("data cache miss at 0x{:08x}", run[k].pc);
                run[k].stalls.push(Stall {
                    name: "dcache miss",
                    label,
                    begin: entered + 1,
                    end: fault,
                });
            }
            if fault < cycle {
                let label = "bus fault: the access is retried until recovery".to_owned();
                run[k].stalls.push(Stall {
                    name: "bus fault",
                    label,
                    begin: fault,
                    end: cycle,
                });
            }
            run[k].enter[4] = Some(cycle);
            run[k].busy = cycle + 1;
            slot[3] = None;
            slot[4] = Some(k);
        }
        // Execute to memory; a mispredicted branch squashes what follows it.
        if let Some(k) = slot[2].filter(|&k| run[k].busy <= cycle && slot[3].is_none()) {
            let mut busy = cycle + 1;
            if is_access(&run[k]) {
                if fault_pending && cycle_time(cycle) >= 768 {
                    fault_pending = false;
                    run[k].fault = Some(cycle);
                    busy = recovery;
                } else if run[k].miss {
                    busy += 4;
                }
            }
            run[k].enter[3] = Some(cycle);
            run[k].busy = busy;
            slot[2] = None;
            slot[3] = Some(k);
            if let Some(redirect) = run[k].redirect {
                for younger in [slot[1].take(), slot[0].take()].into_iter().flatten() {
                    run[younger].end = cycle;
                    run[younger].squashed = true;
                }
                fetch_pc = redirect;
                wrong_path = false;
            }
        }
        // Decode to execute, once every operand can be forwarded.
        if let Some(k) = slot[1].filter(|&k| run[k].busy <= cycle) {
            let blocked = run[k].deps.iter().copied().find(|&(_, p)| {
                let producer = &run[p];
                let available = match producer.op {
                    Op::Load => producer.enter[4],
                    _ => producer.enter[3],
                };
                available.is_none_or(|at| at > cycle)
            });
            match (blocked, run[k].waiting) {
                (Some((reg, p)), None) => run[k].waiting = Some((cycle, reg, p)),
                (None, Some((begin, reg, p))) => {
                    let label = format!("waits for {reg} from 0x{:08x} {}", run[p].pc, run[p].asm);
                    run[k].stalls.push(Stall {
                        name: "operand",
                        label,
                        begin,
                        end: cycle,
                    });
                    run[k].waiting = None;
                }
                _ => {}
            }
            if blocked.is_none() && slot[2].is_none() {
                run[k].enter[2] = Some(cycle);
                run[k].busy = cycle + if run[k].op == Op::Mul { 3 } else { 1 };
                slot[1] = None;
                slot[2] = Some(k);
            }
        }
        if let Some(k) = slot[0].filter(|&k| run[k].busy <= cycle && slot[1].is_none()) {
            run[k].enter[1] = Some(cycle);
            run[k].busy = cycle + 1;
            slot[0] = None;
            slot[1] = Some(k);
        }
        if slot[0].is_none() && !halted {
            let instr = &PROGRAM[((fetch_pc - TEXT) / 4) as usize];
            let k = run.len();
            let deps = instr
                .src
                .iter()
                .filter_map(|reg| last_writer.get(reg).map(|&p| (*reg, p)))
                .collect();
            let mut insn = Insn {
                pc: fetch_pc,
                asm: instr.asm,
                op: instr.op,
                wrong_path,
                order,
                enter: [Some(cycle), None, None, None, None],
                end: 0,
                squashed: false,
                busy: cycle + 1,
                deps,
                miss: instr.op == Op::Load && mix(order + 4).is_multiple_of(4),
                fault: None,
                redirect: None,
                waiting: None,
                stalls: Vec::new(),
            };
            let mut next = fetch_pc + 4;
            if let Op::Branch(target) = instr.op {
                let predicted = target < fetch_pc;
                let taken = if wrong_path {
                    predicted
                } else if target == LOOP {
                    iteration += 1;
                    iteration < LOOP_ITERATIONS
                } else {
                    mix(1000 + iteration).is_multiple_of(3)
                };
                if !wrong_path && taken != predicted {
                    insn.redirect = Some(if taken { target } else { fetch_pc + 4 });
                    wrong_path = true;
                }
                if predicted {
                    next = target;
                }
            }
            if !wrong_path || insn.redirect.is_some() {
                if let Some(dst) = instr.dst {
                    last_writer.insert(dst, k);
                }
                order += 1;
                halted = instr.op == Op::Halt;
            }
            fetch_pc = next;
            slot[0] = Some(k);
            run.push(insn);
        }
        if halted && slot.iter().all(Option::is_none) {
            return run;
        }
        cycle += 1;
        assert!(
            cycle_time(cycle) < 2040,
            "the program runs past the capture"
        );
    }
}

/// Which stages hold an instruction in each core cycle: stage `s` from the
/// cycle an instruction enters it until it enters the next or leaves.
fn stage_occupancy(run: &[Insn]) -> Vec<[bool; 5]> {
    let cycles = run.iter().map(|insn| insn.end).max().unwrap_or(0) as usize + 1;
    let mut busy = vec![[false; 5]; cycles];
    for insn in run {
        for s in 0..STAGES.len() {
            let Some(enter) = insn.enter[s] else { break };
            let leave = insn.enter.get(s + 1).copied().flatten().unwrap_or(insn.end);
            for cycle in enter..leave {
                busy[cycle as usize][s] = true;
            }
        }
    }
    busy
}

/// A deterministic value in `[-1, 1)` for noise.
fn noise(seed: u64) -> f64 {
    (mix(seed) % 2048) as f64 / 1024.0 - 1.0
}

/// Rounded to 1/64, as a monitor's fixed-point readout would be.
fn quantized(v: f64) -> f64 {
    (v * 64.0).round() / 64.0
}

/// Rising edges of the declared DMA clock (see `write`), which runs at
/// 12 ns, stops for the fault window, runs at 6 ns, then at 16 ns.
fn dma_edges() -> Vec<u64> {
    let mut edges = Vec::new();
    for (begin, stop, period) in [(8, 768, 12), (904, 1400, 6), (1406, 2048, 16)] {
        edges.extend((begin..stop).step_by(period));
    }
    edges
}

/// Whether a DMA transfer (one every 128 ns from 64 ns) is accessing memory.
fn dma_active(t: u64) -> bool {
    (80..1536).contains(&t) && (t - 80) % 128 < 40
}

fn bits(
    w: &mut Writer,
    parent: NodeId,
    name: &str,
    width: u32,
    states: u8,
    dir: Direction,
) -> vtr::Result<SignalId> {
    w.signal(
        SourceOptions::new(name, Some(parent)),
        SignalKind::Bits { width, states },
        VarType::Logic,
        dir,
    )
}

fn real(w: &mut Writer, parent: NodeId, name: &str) -> vtr::Result<SignalId> {
    w.signal(
        SourceOptions::new(name, Some(parent)),
        SignalKind::Real,
        VarType::Real,
        Direction::Implicit,
    )
}

fn log_callsite<T: Payload + 'static>(
    w: &mut Writer,
    severity: u64,
    format: &str,
    line: u64,
    function: &str,
) -> vtr::Result<vtr::Callsite<T>> {
    w.callsite::<T>(vtr::CallsiteOptions {
        severity,
        file: Some("debug_lab.cpp".into()),
        line: Some(line),
        function: Some(function.into()),
        ..vtr::CallsiteOptions::new(format)
    })
}

fn write(path: &Path) -> vtr::Result<()> {
    let mut options = WriterOptions {
        mode: ExecutionMode::Inline,
        ..Default::default()
    };
    options.metadata.timescale = -9;
    options.metadata.file_type = 2;
    options.metadata.time_zero = -16;
    options.metadata.writer = "VTR feature showcase".into();
    options.metadata.date = "2026-10-23".into();
    options.metadata.comment =
        "Synthetic 2 us debugging lab: boot, DMA traffic, injected bus fault, recovery.".into();
    let mut w = Writer::create_with(path, options)?;
    let seed = w.intern("seed")?;
    let mut metadata = w.metadata()?;
    metadata.attributes.push(Attribute {
        key: seed,
        value: Value::U64(2026),
    });
    w.set_metadata(metadata)?;
    let attrs = attributes(&mut w)?;
    let soc = w.scope_with(
        SourceOptions {
            name: "soc".into(),
            parent: None,
            attributes: attrs.clone(),
        },
        ScopeType::Module,
        "debug_lab",
    )?;
    let clk = bits(&mut w, soc, "clk", 1, 2, Direction::Input)?;
    let reset = bits(&mut w, soc, "reset_n", 1, 4, Direction::Input)?;
    let valid = bits(&mut w, soc, "valid", 1, 2, Direction::Output)?;
    let ready = bits(&mut w, soc, "ready", 1, 4, Direction::Input)?;
    let address = bits(&mut w, soc, "address", 32, 2, Direction::Output)?;
    let data = bits(&mut w, soc, "data", 32, 4, Direction::InOut)?;
    let wide = bits(&mut w, soc, "payload_128", 128, 2, Direction::Output)?;
    let resolved = bits(&mut w, soc, "resolved_bus", 9, 9, Direction::InOut)?;
    let temperature = real(&mut w, soc, "temperature_c")?;
    let sine_fast = real(&mut w, soc, "sine_fast")?;
    let sine_slow = real(&mut w, soc, "sine_slow")?;
    let message = w.signal(
        SourceOptions::new("phase", Some(soc)),
        SignalKind::VarLen,
        VarType::String,
        Direction::Implicit,
    )?;
    let packet = w.signal(
        SourceOptions::new("packet_bytes", Some(soc)),
        SignalKind::VarLen,
        VarType::Bytes,
        Direction::Implicit,
    )?;
    let event = w.signal(
        SourceOptions::new("interrupt", Some(soc)),
        SignalKind::Bits {
            width: 1,
            states: 2,
        },
        VarType::Event,
        Direction::Output,
    )?;
    let parameter = w.signal(
        SourceOptions::new("BUS_WIDTH", Some(soc)),
        SignalKind::Bits {
            width: 32,
            states: 2,
        },
        VarType::Parameter,
        Direction::Implicit,
    )?;
    let entries = [
        ("BOOT", "00"),
        ("RUN", "01"),
        ("FAULT", "10"),
        ("RECOVER", "11"),
    ]
    .into_iter()
    .map(|(name, bits)| Ok((w.intern(name)?, w.intern(bits)?)))
    .collect::<vtr::Result<Vec<_>>>()?;
    let table = w.enum_table(SourceOptions::new("phase_t", Some(soc)), entries)?;
    let enum_key = w.intern("enum_table")?;
    let mut state_options = SourceOptions::new("state", Some(soc));
    state_options.attributes.push(Attribute {
        key: enum_key,
        value: Value::U64(u64::from(table.0)),
    });
    let state = w.signal(
        state_options,
        SignalKind::Bits {
            width: 2,
            states: 2,
        },
        VarType::Enum,
        Direction::Output,
    )?;
    w.alias(
        clk,
        SourceOptions::new("clock_alias", Some(soc)),
        VarType::Wire,
        Direction::Input,
    )?;
    let cpu = w.scope_with(
        SourceOptions::new("cpu", Some(soc)),
        ScopeType::Core,
        "tiny_cpu",
    )?;
    let pc = bits(&mut w, cpu, "pc", 32, 2, Direction::Output)?;
    let pipeline = w.scope("thread0", Some(cpu))?;
    let instructions = w.transactions::<Instruction>(
        SourceOptions::new("instructions", Some(pipeline)),
        "PIPELINE",
    )?;
    let speculative = w.transactions::<Instruction>(
        SourceOptions::new("speculative", Some(pipeline)),
        "PIPELINE",
    )?;
    let dma = w.scope_with(
        SourceOptions::new("dma", Some(soc)),
        ScopeType::ScModule,
        "dma_engine",
    )?;
    let bus = w.scope("memory_bus", Some(dma))?;
    let reads = w.transactions::<Label>(SourceOptions::new("read", Some(bus)), "MEMORY_BUS")?;
    let writes = w.transactions::<Label>(SourceOptions::new("write", Some(bus)), "MEMORY_BUS")?;
    let idle = w.transactions::<Label>(SourceOptions::new("idle", Some(bus)), "MEMORY_BUS")?;
    w.scope("standby_bus", Some(dma))?;
    let logs = w.logger(SourceOptions::new("log", Some(soc)))?;
    let core_clk = w.clock(SourceOptions::new("clk", Some(soc)))?;
    let dma_clk = w.clock(SourceOptions::new("dma_clk", Some(dma)))?;
    w.associate_clock(instructions, core_clk)?;
    w.associate_clock(speculative, core_clk)?;
    for source in [reads, writes, idle] {
        w.associate_clock(source, dma_clk)?;
    }
    w.clock_run(core_clk, 4, 8)?;
    w.clock_run(dma_clk, 8, 12)?;
    w.clock_stop(dma_clk, 768)?;
    w.clock_run(dma_clk, 904, 6)?;
    w.clock_stop(dma_clk, 1400)?;
    w.clock_run(dma_clk, 1406, 16)?;
    let mut sites = Vec::new();
    for (i, (severity, name)) in [
        (0, "TRACE"),
        (1, "DEBUG"),
        (2, "INFO"),
        (3, "WARN"),
        (4, "ERROR"),
        (5, "FATAL"),
        (9, "SEVERITY_9"),
    ]
    .into_iter()
    .enumerate()
    {
        sites.push(log_callsite::<CyclePhase>(
            &mut w,
            severity,
            &format!("{name}: cycle={{}} phase={{}}"),
            40 + i as u64,
            "tick",
        )?);
    }
    let typed_site = log_callsite::<Arguments>(
        &mut w,
        1,
        "typed args: {} {} {} {} {} {} {} {} {}",
        80,
        "inspect_packet",
    )?;

    let type_gallery = w.scope_with(
        SourceOptions::new("type_gallery", Some(soc)),
        ScopeType::Package,
        "",
    )?;
    let scopes = w.scope_with(
        SourceOptions::new("scopes", Some(type_gallery)),
        ScopeType::Generic,
        "",
    )?;
    let mut gallery = Vec::new();
    for code in (0..=22).chain(64..=68).chain([200]) {
        let kind = ScopeType::from_code(code);
        let scope = w.scope_with(SourceOptions::new(kind.name(), Some(scopes)), kind, "")?;
        let signal = bits(&mut w, scope, "active", 1, 2, Direction::Implicit)?;
        gallery.push((
            signal,
            SignalKind::Bits {
                width: 1,
                states: 2,
            },
            VarType::Logic,
        ));
    }
    let declarations = w.scope_with(
        SourceOptions::new("declarations", Some(type_gallery)),
        ScopeType::Generic,
        "",
    )?;
    for code in (0..=29).chain([64, 65, 200]) {
        let typ = VarType::from_code(code);
        let kind = match typ {
            VarType::Real | VarType::RealParameter | VarType::RealTime | VarType::ShortReal => {
                SignalKind::Real
            }
            VarType::String | VarType::Bytes | VarType::Port => SignalKind::VarLen,
            VarType::Event => SignalKind::Bits {
                width: 1,
                states: 2,
            },
            VarType::Enum => SignalKind::Bits {
                width: 2,
                states: 2,
            },
            _ => SignalKind::Bits {
                width: 16,
                states: 4,
            },
        };
        let mut signal_options = SourceOptions::new(typ.name(), Some(declarations));
        if typ == VarType::Enum {
            signal_options.attributes.push(Attribute {
                key: enum_key,
                value: Value::U64(u64::from(table.0)),
            });
        }
        let signal = w.signal(
            signal_options,
            kind,
            typ,
            Direction::from_u8((code % 6) as u8),
        )?;
        gallery.push((signal, kind, typ));
    }
    let literal_names = w.scope_with(
        SourceOptions::new("literal.names", Some(type_gallery)),
        ScopeType::Struct,
        "",
    )?;
    w.alias(
        data,
        SourceOptions::new("escaped.signal[3]", Some(literal_names)),
        VarType::Wire,
        Direction::Linkage,
    )?;
    let occupancy = w.scope_with(
        SourceOptions::new("occupancy", Some(cpu)),
        ScopeType::Generic,
        "",
    )?;
    let stage_busy = STAGES
        .iter()
        .map(|stage| bits(&mut w, occupancy, stage, 1, 2, Direction::Output))
        .collect::<vtr::Result<Vec<_>>>()?;
    let power = w.scope_with(
        SourceOptions::new("power", Some(soc)),
        ScopeType::Module,
        "power_monitor",
    )?;
    let power_mode = w.signal(
        SourceOptions::new("mode", Some(power)),
        SignalKind::VarLen,
        VarType::String,
        Direction::Implicit,
    )?;
    let io_mw = real(&mut w, power, "io_mw")?;
    let sram_mw = real(&mut w, power, "sram_mw")?;
    let dma_mw = real(&mut w, power, "dma_mw")?;
    let core_mw = real(&mut w, power, "core_mw")?;
    let leakage_mw = real(&mut w, power, "leakage_mw")?;
    let queue = w.scope_with(
        SourceOptions::new("queue", Some(dma)),
        ScopeType::Module,
        "dma_queues",
    )?;
    let channels =
        ["ch0", "ch1", "ch2", "ch3"].map(|name| bits(&mut w, queue, name, 4, 4, Direction::Output));
    let [ch0, ch1, ch2, ch3] = channels;
    let channels = [ch0?, ch1?, ch2?, ch3?];
    let pmic = w.scope_with(
        SourceOptions::new("pmic", Some(soc)),
        ScopeType::Module,
        "pmic",
    )?;
    let solar_ma = real(&mut w, pmic, "solar_ma")?;
    let usb_ma = real(&mut w, pmic, "usb_ma")?;
    let radio_ma = real(&mut w, pmic, "radio_ma")?;
    let core_ma_draw = real(&mut w, pmic, "core_ma")?;
    let service_key = w.intern("service.name")?;
    let mut resource_options = SourceOptions::new("firmware", None);
    resource_options.attributes.push(Attribute {
        key: service_key,
        value: Value::Text("dma-demo".into()),
    });
    let resource = w.scope_with(resource_options, ScopeType::Resource, "")?;
    let otel = w.scope("driver", Some(resource))?;
    let spans = w.transactions::<Label>(
        SourceOptions::new("submit_transfer", Some(otel)),
        "otel.scope",
    )?;
    // The simulation advances through the recording gap so resuming capture
    // reflects current state rather than replaying missed samples.
    let cpu_run = simulate();
    let busy = stage_occupancy(&cpu_run);
    let halt = cycle_time(busy.len() as u64);
    let stages_at = |t: u64| -> [bool; 5] {
        t.checked_sub(4)
            .and_then(|t| busy.get((t / 8) as usize).copied())
            .unwrap_or_default()
    };
    let in_flight = |t: u64| stages_at(t).iter().filter(|&&b| b).count() as f64;
    let edges = dma_edges();
    let mut next_edge = 0;
    let (mut depth, mut serve) = ([0u64; 4], 0);
    let mut reading = None;
    for t in 0..=2048u64 {
        w.advance_time(t)?;
        if t == 960 {
            w.dump(false)?;
        }
        if t == 992 {
            w.dump(true)?;
        }
        // The queues advance on every DMA clock edge, recorded or not: a
        // channel takes a descriptor now and then, more often while a
        // transfer runs, and one is served per edge, round robin.
        let edge = edges.get(next_edge) == Some(&t);
        if edge {
            next_edge += 1;
            for (ch, d) in depth.iter_mut().enumerate() {
                let rate = [6, 3, 8, 2][ch] + if dma_active(t) { 55 } else { 0 };
                if mix(t * 8 + ch as u64) % 100 < rate {
                    *d = (*d + 1).min(15);
                }
            }
            if let Some(k) = (0..4).map(|k| (serve + k) % 4).find(|&k| depth[k] > 0) {
                depth[k] -= 1;
                serve = (k + 1) % 4;
            }
        }
        if (960..992).contains(&t) {
            continue;
        }
        let fault_window = (768..896).contains(&t);
        let stages = stages_at(t);
        let n = in_flight(t);
        // An inrush spike when reset releases, one-nanosecond glitches now
        // and then, and a gated clock once the program halts.
        let core = if t < 32 {
            4.0
        } else if t < 40 {
            64.0 - 6.0 * (t - 32) as f64
        } else if t >= halt {
            6.0 + 0.3 * noise(t)
        } else {
            16.0 + 7.0 * n
                + 2.5 * noise(t)
                + if mix(t ^ 0x5eed).is_multiple_of(173) {
                    25.0
                } else {
                    0.0
                }
        };
        let dma_power = if fault_window {
            0.5
        } else if dma_active(t) {
            21.0 + 3.0 * noise(t + 1)
        } else {
            2.0 + 0.3 * noise(t + 1)
        };
        let sram = 5.0
            + if stages[3] { 9.0 } else { 0.0 }
            + if dma_active(t) && !fault_window {
                6.0
            } else {
                0.0
            }
            + 0.6 * noise(t + 2);
        let io = 1.5 + if t % 128 < 12 { 4.0 } else { 0.0 } + 0.2 * noise(t + 3);
        let temperature_now = 35.0 + (((t / 8) % 64) as f64 - 32.0).abs() / 8.0;
        let leakage = 3.0 + 0.1 * (temperature_now - 35.0);
        for (id, mw) in [
            (io_mw, io),
            (sram_mw, sram),
            (dma_mw, dma_power),
            (core_mw, core),
            (leakage_mw, leakage),
        ] {
            w.emit_real(id, quantized(mw))?;
        }
        let mode = if t < 32 {
            "reset"
        } else if fault_window {
            "fault"
        } else if t >= halt {
            "halted"
        } else if (896..1024).contains(&t) {
            "recover"
        } else {
            "run"
        };
        w.emit_bytes(power_mode, mode.as_bytes())?;
        if t == 0 || edge {
            for (id, d) in channels.iter().zip(depth) {
                w.emit_u64(*id, d)?;
            }
        }
        if t == 768 {
            w.emit_packed(channels[2], 4, &[0xaa])?;
        }
        if t % 8 == 4 || t == 0 {
            for (id, on) in stage_busy.iter().zip(stages) {
                w.emit_u64(*id, u64::from(on))?;
            }
        }
        if t.is_multiple_of(8) {
            let angle = std::f64::consts::TAU * t as f64 / 2048.0;
            let solar = 32.0 + 12.0 * angle.sin() + 1.5 * noise(t + 4);
            let usb = match t {
                0..1300 => 0.0,
                1300..1400 => 1.4 * (t - 1300) as f64,
                _ => 140.0 + 3.0 * noise(t + 5),
            };
            let radio = -1.5
                - if t % 512 >= 200 && t % 512 < 264 {
                    58.0
                } else {
                    0.0
                };
            let draw = if t >= halt { -2.0 } else { -6.0 - 5.0 * n };
            for (id, ma) in [
                (solar_ma, solar),
                (usb_ma, usb),
                (radio_ma, radio),
                (core_ma_draw, draw),
            ] {
                w.emit_real(id, quantized(ma))?;
            }
        }
        if !t.is_multiple_of(4) {
            continue;
        }
        let cycle = t / 8;
        let phase = if t < 64 {
            0
        } else if t < 768 {
            1
        } else if t < 896 {
            2
        } else {
            3
        };
        w.emit_u64(clk, (t / 4) & 1)?;
        w.emit_u64(reset, u64::from(t >= 32))?;
        w.emit_u64(valid, u64::from(t >= 64 && !cycle.is_multiple_of(4)))?;
        if phase == 2 {
            w.emit_packed(ready, 4, &[2])?;
        } else {
            w.emit_u64(ready, u64::from(!cycle.is_multiple_of(7)))?;
        }
        w.emit_u64(address, 0x1000 + 4 * (cycle % 64))?;
        if phase == 2 {
            w.emit_packed(data, 4, &[0x55, 0x55, 0x00, 0x00, 0xff, 0xff, 0xaa, 0xaa])?;
        } else {
            w.emit_u64(data, cycle.wrapping_mul(0x9e37_79b9) & 0xffff_ffff)?;
        }
        w.emit_words(
            wide,
            128,
            &[cycle as u32, 0xdead_beef, !(cycle as u32), 0x0123_4567],
        )?;
        w.emit_packed(
            resolved,
            9,
            if cycle.is_multiple_of(2) {
                &[0x78, 0x56, 0x34, 0x12, 0]
            } else {
                &[0x10, 0x32, 0x54, 0x76, 8]
            },
        )?;
        w.emit_real(temperature, 35.0 + ((cycle % 64) as f64 - 32.0).abs() / 8.0)?;
        // Sampled every 4 ns: amplitude 0.5 with a 128 ns period, and
        // amplitude 20 with a 1,024 ns period.
        let angle = std::f64::consts::TAU * t as f64;
        w.emit_real(sine_fast, 0.5 * (angle / 128.0).sin())?;
        w.emit_real(sine_slow, 20.0 * (angle / 1024.0).sin())?;
        w.emit_bytes(
            message,
            [
                b"BOOT".as_slice(),
                b"RUN",
                b"FAULT: retry",
                b"RECOVER -> RUN",
            ][phase],
        )?;
        w.emit_bytes(packet, &[0, cycle as u8, 0x80, 0xff])?;
        w.emit_u64(parameter, 32)?;
        w.emit_u64(state, phase as u64)?;
        w.emit_u64(pc, 0x8000_0000 + 4 * cycle)?;
        if t.is_multiple_of(128) {
            w.emit_u64(event, 1)?;
            w.emit_u64(event, 1)?;
        }
        for (id, kind, typ) in &gallery {
            let sample = match typ {
                VarType::Parameter | VarType::RealParameter | VarType::Supply1 => 1,
                VarType::Supply0 => 0,
                VarType::Enum => cycle % 4,
                _ => cycle,
            };
            match kind {
                SignalKind::Bits { width, .. } => {
                    w.emit_u64(*id, if *width == 1 { sample & 1 } else { sample })?;
                }
                SignalKind::Real => w.emit_real(*id, 1.25 + sample as f64 / 16.0)?,
                SignalKind::VarLen => {
                    w.emit_bytes(
                        *id,
                        if cycle.is_multiple_of(2) {
                            b"hello"
                        } else {
                            b"world"
                        },
                    )?;
                }
            }
        }
        if t == 1024 {
            w.flush()?;
            let hotplug_sensor = w.scope_with(
                SourceOptions::new("hotplug_sensor", None),
                ScopeType::ScModule,
                "late_sensor",
            )?;
            reading = Some(bits(
                &mut w,
                hotplug_sensor,
                "reading",
                8,
                4,
                Direction::Output,
            )?);
            w.alias(
                data,
                SourceOptions::new("sample", Some(hotplug_sensor)),
                VarType::Wire,
                Direction::Output,
            )?;
        }
        if let Some(reading) = reading {
            w.emit_u64(reading, cycle & 0xff)?;
        }
    }

    let main_lane = w.intern("main")?;
    let stall_lane = w.intern("stall")?;
    let stage_names = STAGES
        .iter()
        .map(|name| w.intern(name))
        .collect::<vtr::Result<Vec<_>>>()?;
    let execute = stage_names[2];
    let memory = w.intern("memory")?;
    let dependency = w.intern("dependency")?;
    let request = w.intern("request")?;
    let retire = w.intern("retire")?;
    let mispredict = w.intern("mispredict")?;
    let label_key = w.intern("vtr.label")?;
    let fault = w.intern("fault_injected")?;
    let mut insn_tx = Vec::with_capacity(cpu_run.len());
    for insn in &cpu_run {
        let tx = w.begin_at(
            if insn.wrong_path {
                speculative
            } else {
                instructions
            },
            cycle_time(insn.enter[0].unwrap()),
            &Instruction {
                pc: Address(insn.pc),
                label: insn.label(),
            },
            None,
        )?;
        insn_tx.push(tx);
        let end = cycle_time(insn.end);
        for (s, name) in stage_names.iter().enumerate() {
            let Some(enter) = insn.enter[s] else { break };
            let leave = insn
                .enter
                .get(s + 1)
                .copied()
                .flatten()
                .map_or(end, cycle_time);
            let mut stage_attrs = vec![Attribute {
                key: label_key,
                value: Value::Text(format!("{} 0x{:08x}", STAGES[s], insn.pc)),
            }];
            if s == 3 && matches!(insn.op, Op::Load | Op::Store) {
                stage_attrs.push(Attribute {
                    key: fault,
                    value: Value::Bool(insn.fault.is_some()),
                });
            }
            w.embedded_stage(
                tx.owner(),
                cycle_time(enter),
                *name,
                main_lane,
                Some(leave),
                stage_attrs,
            )?;
        }
        for stall in &insn.stalls {
            let name = w.intern(stall.name)?;
            w.embedded_stage(
                tx.owner(),
                cycle_time(stall.begin),
                name,
                stall_lane,
                Some(cycle_time(stall.end)),
                vec![Attribute {
                    key: label_key,
                    value: Value::Text(stall.label.clone()),
                }],
            )?;
        }
        if insn.redirect.is_some() {
            w.embedded_event(
                tx.owner(),
                cycle_time(insn.enter[3].unwrap()),
                mispredict,
                vec![Attribute {
                    key: label_key,
                    value: Value::Text(format!("mispredicted 0x{:08x}; fetch redirected", insn.pc)),
                }],
            )?;
        }
        if !insn.squashed {
            w.embedded_event(
                tx.owner(),
                end,
                retire,
                vec![Attribute {
                    key: label_key,
                    value: Value::Text(format!("retire 0x{:08x}", insn.pc)),
                }],
            )?;
        }
        w.end_at(
            tx,
            end,
            if insn.squashed {
                Status::Aborted
            } else {
                Status::Ok
            },
        )?;
    }
    for (k, insn) in cpu_run.iter().enumerate() {
        for &(reg, producer) in &insn.deps {
            if insn.squashed || insn.order - cpu_run[producer].order > 4 {
                continue;
            }
            w.relate(
                dependency,
                insn_tx[producer].id(),
                insn_tx[k].id(),
                vec![Attribute {
                    key: label_key,
                    value: Value::Text(format!(
                        "{reg}: 0x{:08x} to 0x{:08x}",
                        cpu_run[producer].pc, insn.pc
                    )),
                }],
            )?;
        }
    }
    let doorbells = cpu_run
        .iter()
        .zip(&insn_tx)
        .filter(|(insn, _)| !insn.squashed && insn.asm.ends_with(",8(s1)"))
        .map(|(insn, tx)| (cycle_time(insn.enter[3].unwrap()), *tx))
        .collect::<Vec<_>>();
    for i in (0..48u64).step_by(4) {
        let begin = 64 + i * 32;
        let rung = doorbells
            .iter()
            .take_while(|(t, _)| *t <= begin + 16)
            .count();
        let doorbell = doorbells[rung.max(1) - 1].1;
        let span = w.begin_at(
            spans,
            begin,
            &Label {
                label: format!("submission {i}"),
            },
            None,
        )?;
        w.span_kind(span.owner(), SpanKind::Client)?;
        let transfer = w.begin_at(
            if i.is_multiple_of(8) { reads } else { writes },
            begin + 16,
            &Label {
                label: format!("DMA transfer {i}"),
            },
            Some(span.owner()),
        )?;
        w.span_kind(transfer.owner(), SpanKind::Consumer)?;
        for attr in &attrs {
            w.attribute(transfer.owner(), attr.key, attr.value.clone())?;
        }
        let mut stage_attrs = attrs[..2].to_vec();
        stage_attrs.push(Attribute {
            key: label_key,
            value: Value::Text(format!("DMA memory access {i}")),
        });
        w.embedded_stage(
            transfer.owner(),
            begin + 16,
            execute,
            memory,
            Some(begin + 56),
            stage_attrs,
        )?;
        let mut event_attrs = attrs[2..4].to_vec();
        event_attrs.push(Attribute {
            key: label_key,
            value: Value::Text(format!("DMA request {i}")),
        });
        w.embedded_event(transfer.owner(), begin + 40, request, event_attrs)?;
        let mut relation_attrs = attrs[4..6].to_vec();
        relation_attrs.push(Attribute {
            key: label_key,
            value: Value::Text(format!("doorbell store starts DMA transfer {i}")),
        });
        w.relate(request, doorbell.id(), transfer.id(), relation_attrs)?;
        w.log_at(
            logs,
            sites[2],
            begin + 20,
            &CyclePhase {
                cycle: i,
                phase: "DMA issued".into(),
                label: format!("DMA issue log {i}"),
            },
            Some(transfer.owner()),
        )?;
        w.end_at(
            transfer,
            begin + 64,
            if i == 24 { Status::Error } else { Status::Ok },
        )?;
        w.end_at(span, begin + 72, Status::Ok)?;
    }
    for (i, site) in sites.into_iter().enumerate() {
        w.log_at(
            logs,
            site,
            768 + i as u64 * 16,
            &CyclePhase {
                cycle: 96 + i as u64 * 2,
                phase: "injected fault / recovery".into(),
                label: format!("injected fault log {i}"),
            },
            None,
        )?;
    }
    let interned = w.intern("READY")?;
    w.log_at(
        logs,
        typed_site,
        900,
        &Arguments {
            boolean: true,
            signed: -7,
            unsigned: 42,
            real: 3.25,
            interned: StringId(interned),
            bytes: Bytes(vec![0, 255]),
            time: TimeValue(900),
            pointer: Address(0x2000),
            text: "café ✓".into(),
            label: "typed argument sample".into(),
        },
        None,
    )?;
    for (i, kind) in [
        SpanKind::Internal,
        SpanKind::Server,
        SpanKind::Client,
        SpanKind::Producer,
        SpanKind::Consumer,
    ]
    .into_iter()
    .enumerate()
    {
        let tx = w.begin_at(
            spans,
            1800 + i as u64 * 16,
            &Label {
                label: format!("span instance {i}"),
            },
            None,
        )?;
        w.span_kind(tx.owner(), kind)?;
        w.end_at(
            tx,
            1808 + i as u64 * 16,
            if i == 0 { Status::Unset } else { Status::Ok },
        )?;
    }
    let open = w.begin_at(
        writes,
        2000,
        &Label {
            label: "unfinished write 0".into(),
        },
        None,
    )?;
    w.embedded_stage(
        open.owner(),
        2000,
        execute,
        memory,
        None,
        vec![Attribute {
            key: label_key,
            value: Value::Text("unfinished memory access 0".into()),
        }],
    )?;
    w.close()?;
    verify(path, core_clk.id(), dma_clk.id())
}

fn verify(path: &Path, core_clock: u32, dma_clock: u32) -> vtr::Result<()> {
    let size = std::fs::metadata(path)?.len();
    assert!(size < 500_000, "demo exceeds 500 KB: {size}");
    let r = Reader::open(path)?;
    assert_eq!(r.find_generator("soc.clk"), Some(core_clock));
    assert_eq!(r.find_generator("soc.dma.dma_clk"), Some(dma_clock));
    let report = r.validate()?;
    assert!(report.missing.is_empty());
    assert_eq!(r.completion(), vtr::Completion::Complete);
    let range = r.waveform_time_range().unwrap();
    assert_eq!((range.first, range.last), (0, 2048));
    assert_eq!(r.blackouts().len(), 2);
    assert!(r.block_count() >= 2);
    let tags = r
        .node(r.find("soc").unwrap())?
        .attributes
        .iter()
        .map(|a| a.value.tag())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(tags, (0..=24).collect());
    let mut changes = 0;
    for id in 0..r.signal_count() {
        changes += r.load_signal(SignalId(id))?.len();
    }
    let mut transactions = Vec::new();
    r.transactions(TransactionQuery::default(), |tx| {
        transactions.push(tx);
        true
    })?;
    let labelled = |attrs: &[Attribute]| {
        attrs.iter().any(|attr| {
            r.string(attr.key).unwrap() == "vtr.label"
                && matches!(attr.value, Value::Str(_) | Value::Text(_))
        })
    };
    for tx in transactions
        .iter()
        .filter(|tx| tx.kind() != RecordKind::Clock)
    {
        assert!(
            matches!(tx.payload().field("vtr.label")?.scalar()?, Value::Text(text) if !text.is_empty())
        );
        assert!(
            tx.events().iter().all(|event| labelled(&event.attributes)),
            "owner {} has an unlabeled event",
            tx.id()
        );
        assert!(
            tx.stages().iter().all(|stage| labelled(&stage.attributes)),
            "owner {} has an unlabeled stage",
            tx.id()
        );
    }
    let statuses = transactions
        .iter()
        .map(|t| t.status() as u8)
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(statuses, (0..=4).collect());
    let kinds = transactions
        .iter()
        .map(|t| t.span_kind() as u8)
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(kinds, (0..=5).collect());
    let unfinished = transactions
        .iter()
        .find(|tx| tx.status() == Status::Open && tx.kind() == RecordKind::Transaction)
        .unwrap();
    assert_eq!(unfinished.end(), 2048);
    assert_eq!(unfinished.stages()[0].end, None);
    assert_eq!(r.lookup(unfinished.stages()[0].id)?.unwrap().end(), 2048);
    let occurrences = r.load_signal(r.find_signal("soc.interrupt").unwrap())?;
    assert!(
        occurrences
            .times()
            .windows(2)
            .any(|pair| pair[0] == pair[1])
    );
    let mut relations = 0;
    r.scan_relations(|relation| {
        assert!(labelled(&relation.attributes));
        relations += 1;
        true
    })?;
    let mut logs = 0;
    r.records(RecordQuery { kind: Some(RecordKind::Log), ..Default::default() }, |record| {
        r.format_log_bytes(&record).unwrap();
        assert!(matches!(record.payload().field("vtr.label").unwrap().scalar().unwrap(), Value::Text(text) if !text.is_empty()));
        logs += 1; true
    })?;
    assert_eq!(logs, 20);
    let clock_edges = |clock| -> vtr::Result<Vec<u64>> {
        let mut edges = Vec::new();
        r.transactions(
            TransactionQuery {
                generator: Some(clock),
                ..Default::default()
            },
            |stretch| {
                let period = stretch
                    .attributes()
                    .iter()
                    .find(|a| r.string(a.key).unwrap() == "vtr.period")
                    .map(|a| match a.value {
                        Value::Time(period) => period,
                        _ => panic!("clock period is not time"),
                    })
                    .unwrap();
                edges.extend((stretch.begin()..=stretch.end()).step_by(period as usize));
                true
            },
        )?;
        edges.sort_unstable();
        Ok(edges)
    };
    let clk = r.load_signal(r.find_signal("soc.clk").unwrap())?;
    let rising = (1..clk.len())
        .filter(|&i| clk.get(i).as_u64() == Some(1))
        .map(|i| clk.times()[i])
        .collect::<Vec<_>>();
    let edges = clock_edges(core_clock)?
        .into_iter()
        .filter(|t| !(960..992).contains(t))
        .collect::<Vec<_>>();
    assert_eq!(edges, rising, "declared core clock reproduces dumped clk");
    assert_eq!(clock_edges(dma_clock)?, dma_edges());
    assert_eq!(r.cycle_at(dma_clock, 800)?, r.cycle_at(dma_clock, 768)?);
    for (path, clock) in [
        ("soc.cpu.thread0.instructions", core_clock),
        ("soc.cpu.thread0.speculative", core_clock),
        ("soc.dma.memory_bus.read", dma_clock),
        ("soc.dma.memory_bus.write", dma_clock),
    ] {
        assert_eq!(r.clock_for(r.find_generator(path).unwrap())?, Some(clock));
    }
    let instructions = r.find_generator("soc.cpu.thread0.instructions").unwrap();
    let speculative = r.find_generator("soc.cpu.thread0.speculative").unwrap();
    let insns = transactions
        .iter()
        .filter(|tx| tx.generator() == Some(instructions))
        .collect::<Vec<_>>();
    let lanes = insns
        .iter()
        .flat_map(|tx| tx.stages())
        .map(|stage| r.string(stage.lane).unwrap())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(lanes, ["main", "stall"].into());
    assert_eq!(
        (0..2048)
            .map(|t| insns
                .iter()
                .filter(|i| i.begin() <= t && t < i.end())
                .count())
            .max(),
        Some(5)
    );
    assert!(
        transactions
            .iter()
            .filter(|t| t.generator() == Some(speculative))
            .all(|t| t.status() == Status::Aborted)
    );
    let load = |path: &str| r.load_signal(r.find_signal(path).unwrap());
    let stage_bits = STAGES
        .iter()
        .map(|stage| load(&format!("soc.cpu.occupancy.{stage}")))
        .collect::<vtr::Result<Vec<_>>>()?;
    let fetched = transactions
        .iter()
        .filter(|tx| tx.generator() == Some(instructions) || tx.generator() == Some(speculative))
        .collect::<Vec<_>>();
    for t in (8..2048).step_by(8).filter(|t| !(952..1000).contains(t)) {
        let stacked = stage_bits
            .iter()
            .map(|s| s.value_at(t).as_u64().unwrap())
            .sum::<u64>();
        let flying = fetched
            .iter()
            .filter(|tx| tx.begin() <= t && t < tx.end())
            .count() as u64;
        assert_eq!(stacked, flying, "stage occupancy at {t} ns");
    }
    for rail in ["io_mw", "sram_mw", "dma_mw", "core_mw"] {
        assert!(load(&format!("soc.power.{rail}"))?.len() > 1500);
    }
    let ch2 = load("soc.dma.queue.ch2")?;
    assert!(ch2.value_at(800).as_u64().is_none());
    assert!(ch2.value_at(1000).as_u64().is_some());
    let pmic = ["solar_ma", "usb_ma", "radio_ma", "core_ma"]
        .into_iter()
        .map(|name| load(&format!("soc.pmic.{name}")))
        .collect::<vtr::Result<Vec<_>>>()?;
    let net = (0..2048)
        .step_by(8)
        .map(|t| {
            pmic.iter()
                .map(|s| match s.value_at(t) {
                    vtr::signal::SignalValue::Real(v) => v,
                    _ => panic!("missing current at {t}"),
                })
                .sum::<f64>()
        })
        .collect::<Vec<_>>();
    assert!(net.iter().any(|&n| n < 0.0) && net.iter().any(|&n| n > 0.0));
    let vars = r
        .declarations()
        .nodes
        .iter()
        .filter(|node| matches!(node.data, NodeData::Signal { .. }))
        .count();
    println!(
        "{}: {size} bytes; {vars} variables / {} signals; {changes} changes; {} owners; {logs} logs; {relations} relations",
        path.display(),
        r.signal_count(),
        transactions.len()
    );
    Ok(())
}

fn main() -> vtr::Result<()> {
    let path = std::env::args_os()
        .nth(1)
        .expect("usage: feature_showcase OUTPUT.vtr");
    write(Path::new(&path))
}
