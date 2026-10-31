//! A simulated chip with typed pipeline, cache, DRAM, clocks, logs, and an optional matching VCD.

use std::collections::HashMap;
use std::path::Path;
use vtr::schema::{Address, TimeValue};
use vtr::{
    Attribute, Direction, ExecutionMode, NodeId, Payload, PayloadReader, Reader, RecordKind,
    RecordQuery, Recorder, ScopeType, SignalId, SignalKind, SourceOptions, Status, Transaction,
    TransactionQuery, Value, VarType, Writer, WriterOptions,
};

struct Instruction {
    label: String,
    insn_id: u64,
    pc: u64,
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
struct MemoryRequest {
    label: String,
    address: u64,
    bytes: u64,
}
impl Payload for MemoryRequest {
    fn bind(types: &mut vtr::Types) -> vtr::Result<vtr::TypeId> {
        let label_type = <String as Payload>::bind(types)?;
        let address_type = <u64 as Payload>::bind(types)?;
        let bytes_type = <u64 as Payload>::bind(types)?;
        types.structure(
            "MemoryRequest",
            vec![
                vtr::schema::Field::new("vtr.label", label_type),
                vtr::schema::Field::new("address", address_type),
                vtr::schema::Field::new("bytes", bytes_type),
            ],
        )
    }
    fn encode(&self, out: &mut Vec<u8>) -> vtr::Result<()> {
        self.label.encode(out)?;
        self.address.encode(out)?;
        self.bytes.encode(out)?;
        Ok(())
    }
    fn decode(reader: &mut PayloadReader<'_>) -> vtr::Result<Self> {
        Ok(Self {
            label: <String as Payload>::decode(reader)?,
            address: <u64 as Payload>::decode(reader)?,
            bytes: <u64 as Payload>::decode(reader)?,
        })
    }
}
struct DramBurst {
    label: String,
    address: u64,
    bank: u64,
    row: u64,
}
impl Payload for DramBurst {
    fn bind(types: &mut vtr::Types) -> vtr::Result<vtr::TypeId> {
        let label_type = <String as Payload>::bind(types)?;
        let address_type = <u64 as Payload>::bind(types)?;
        let bank_type = <u64 as Payload>::bind(types)?;
        let row_type = <u64 as Payload>::bind(types)?;
        types.structure(
            "DramBurst",
            vec![
                vtr::schema::Field::new("vtr.label", label_type),
                vtr::schema::Field::new("address", address_type),
                vtr::schema::Field::new("bank", bank_type),
                vtr::schema::Field::new("row", row_type),
            ],
        )
    }
    fn encode(&self, out: &mut Vec<u8>) -> vtr::Result<()> {
        self.label.encode(out)?;
        self.address.encode(out)?;
        self.bank.encode(out)?;
        self.row.encode(out)?;
        Ok(())
    }
    fn decode(reader: &mut PayloadReader<'_>) -> vtr::Result<Self> {
        Ok(Self {
            label: <String as Payload>::decode(reader)?,
            address: <u64 as Payload>::decode(reader)?,
            bank: <u64 as Payload>::decode(reader)?,
            row: <u64 as Payload>::decode(reader)?,
        })
    }
}
struct Boot {
    cycle: u64,
    label: String,
}
impl Payload for Boot {
    fn bind(types: &mut vtr::Types) -> vtr::Result<vtr::TypeId> {
        let cycle_type = <u64 as Payload>::bind(types)?;
        let label_type = <String as Payload>::bind(types)?;
        types.structure(
            "Boot",
            vec![
                vtr::schema::Field::new("cycle", cycle_type),
                vtr::schema::Field::new("vtr.label", label_type),
            ],
        )
    }
    fn encode(&self, out: &mut Vec<u8>) -> vtr::Result<()> {
        self.cycle.encode(out)?;
        self.label.encode(out)?;
        Ok(())
    }
    fn decode(reader: &mut PayloadReader<'_>) -> vtr::Result<Self> {
        Ok(Self {
            cycle: <u64 as Payload>::decode(reader)?,
            label: <String as Payload>::decode(reader)?,
        })
    }
}
struct Interrupt {
    line: u64,
    pc: Address,
    label: String,
}
impl Payload for Interrupt {
    fn bind(types: &mut vtr::Types) -> vtr::Result<vtr::TypeId> {
        let line_type = <u64 as Payload>::bind(types)?;
        let pc_type = <Address as Payload>::bind(types)?;
        let label_type = <String as Payload>::bind(types)?;
        types.structure(
            "Interrupt",
            vec![
                vtr::schema::Field::new("line", line_type),
                vtr::schema::Field::new("pc", pc_type),
                vtr::schema::Field::new("vtr.label", label_type),
            ],
        )
    }
    fn encode(&self, out: &mut Vec<u8>) -> vtr::Result<()> {
        self.line.encode(out)?;
        self.pc.encode(out)?;
        self.label.encode(out)?;
        Ok(())
    }
    fn decode(reader: &mut PayloadReader<'_>) -> vtr::Result<Self> {
        Ok(Self {
            line: <u64 as Payload>::decode(reader)?,
            pc: <Address as Payload>::decode(reader)?,
            label: <String as Payload>::decode(reader)?,
        })
    }
}
struct Miss {
    address: Address,
    cycles: u64,
    label: String,
}
impl Payload for Miss {
    fn bind(types: &mut vtr::Types) -> vtr::Result<vtr::TypeId> {
        let address_type = <Address as Payload>::bind(types)?;
        let cycles_type = <u64 as Payload>::bind(types)?;
        let label_type = <String as Payload>::bind(types)?;
        types.structure(
            "Miss",
            vec![
                vtr::schema::Field::new("address", address_type),
                vtr::schema::Field::new("cycles", cycles_type),
                vtr::schema::Field::new("vtr.label", label_type),
            ],
        )
    }
    fn encode(&self, out: &mut Vec<u8>) -> vtr::Result<()> {
        self.address.encode(out)?;
        self.cycles.encode(out)?;
        self.label.encode(out)?;
        Ok(())
    }
    fn decode(reader: &mut PayloadReader<'_>) -> vtr::Result<Self> {
        Ok(Self {
            address: <Address as Payload>::decode(reader)?,
            cycles: <u64 as Payload>::decode(reader)?,
            label: <String as Payload>::decode(reader)?,
        })
    }
}
struct Ecc {
    address: Address,
    label: String,
}
impl Payload for Ecc {
    fn bind(types: &mut vtr::Types) -> vtr::Result<vtr::TypeId> {
        let address_type = <Address as Payload>::bind(types)?;
        let label_type = <String as Payload>::bind(types)?;
        types.structure(
            "Ecc",
            vec![
                vtr::schema::Field::new("address", address_type),
                vtr::schema::Field::new("vtr.label", label_type),
            ],
        )
    }
    fn encode(&self, out: &mut Vec<u8>) -> vtr::Result<()> {
        self.address.encode(out)?;
        self.label.encode(out)?;
        Ok(())
    }
    fn decode(reader: &mut PayloadReader<'_>) -> vtr::Result<Self> {
        Ok(Self {
            address: <Address as Payload>::decode(reader)?,
            label: <String as Payload>::decode(reader)?,
        })
    }
}
struct Idle {
    time: TimeValue,
    label: String,
}
impl Payload for Idle {
    fn bind(types: &mut vtr::Types) -> vtr::Result<vtr::TypeId> {
        let time_type = <TimeValue as Payload>::bind(types)?;
        let label_type = <String as Payload>::bind(types)?;
        types.structure(
            "Idle",
            vec![
                vtr::schema::Field::new("time", time_type),
                vtr::schema::Field::new("vtr.label", label_type),
            ],
        )
    }
    fn encode(&self, out: &mut Vec<u8>) -> vtr::Result<()> {
        self.time.encode(out)?;
        self.label.encode(out)?;
        Ok(())
    }
    fn decode(reader: &mut PayloadReader<'_>) -> vtr::Result<Self> {
        Ok(Self {
            time: <TimeValue as Payload>::decode(reader)?,
            label: <String as Payload>::decode(reader)?,
        })
    }
}
/// The page downloads this file: keep it small.
const MAX_BYTES: u64 = 150_000;
/// `soc.clk`: 2 ns, first rising edge at 1 ns. Times are nanoseconds.
const PERIOD: u64 = 2;
const FIRST_EDGE: u64 = 1;
/// Instructions fetched from the loop and the interrupt handler.
const INSTRUCTIONS: usize = 420;
/// The device interrupt arrives at this cycle.
const IRQ_CYCLE: u64 = 360;
/// Cycles of idle recording after the last instruction retires.
const IDLE_CYCLES: u64 = 40;
/// Cycles a read spends at L2 on a hit, and at DRAM on an L2 miss.
const L2_LATENCY: u64 = 4;
const DRAM_LATENCY: u64 = 14;

/// Rising edge that begins core cycle `cycle`.
fn edge(cycle: u64) -> u64 {
    FIRST_EDGE + PERIOD * cycle
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Alu,
    Mul,
    Load,
    Store,
    Branch,
    Return,
}

struct Op {
    text: &'static str,
    dst: Option<u8>,
    src: [Option<u8>; 2],
    kind: Kind,
}

const fn op(text: &'static str, dst: Option<u8>, src: [Option<u8>; 2], kind: Kind) -> Op {
    Op {
        text,
        dst,
        src,
        kind,
    }
}

/// A dot product over a buffer: loads that miss, a multiply, dependent adds,
/// a store and the loop branch.
const LOOP: &[Op] = &[
    op("lw   a1, 0(a2)", Some(11), [Some(12), None], Kind::Load),
    op("lw   a3, 0(a4)", Some(13), [Some(14), None], Kind::Load),
    op("mul  a5, a1, a3", Some(15), [Some(11), Some(13)], Kind::Mul),
    op("add  a0, a0, a5", Some(10), [Some(10), Some(15)], Kind::Alu),
    op("addi a2, a2, 4", Some(12), [Some(12), None], Kind::Alu),
    op("addi a4, a4, 4", Some(14), [Some(14), None], Kind::Alu),
    op("sw   a0, 0(s1)", None, [Some(10), Some(9)], Kind::Store),
    op("addi t0, t0, -1", Some(5), [Some(5), None], Kind::Alu),
    op("bnez t0, loop", None, [Some(5), None], Kind::Branch),
];

/// The interrupt handler: save, read the device's cause register (an
/// uncached read that goes to DRAM), acknowledge, restore, return.
const ISR: &[Op] = &[
    op(
        "csrrw sp, mscratch, sp",
        Some(2),
        [Some(2), None],
        Kind::Alu,
    ),
    op("sw   ra, 0(sp)", None, [Some(1), Some(2)], Kind::Store),
    op("lw   t1, 64(gp)", Some(6), [Some(3), None], Kind::Load),
    op("sw   t1, 68(gp)", None, [Some(6), Some(3)], Kind::Store),
    op("lw   ra, 0(sp)", Some(1), [Some(2), None], Kind::Load),
    op("mret", None, [None, None], Kind::Return),
];

const STAGES: [&str; 5] = ["F", "D", "X", "M", "W"];
const EXECUTE: usize = 2;
const MEMORY: usize = 3;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Level {
    L2,
    Dram,
}

/// A memory access at L2, in cycles.
struct Access {
    write: bool,
    begin: u64,
    end: u64,
    address: u64,
    level: Level,
}

struct Insn {
    index: usize,
    pc: u64,
    op: &'static Op,
    isr: bool,
    /// (begin, end) cycles per stage; flushed instructions have fewer.
    stages: Vec<(u64, u64)>,
    /// (begin, end, reason) of the waits between stages.
    stalls: Vec<(u64, u64, &'static str)>,
    end: u64,
    status: Status,
    deps: Vec<usize>,
    access: Option<Access>,
    l1_miss: bool,
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn chance(&mut self, one_in: u64) -> bool {
        self.next().is_multiple_of(one_in)
    }
}

struct Run {
    insns: Vec<Insn>,
    /// Cycle the interrupt was taken.
    irq: u64,
    /// Cycles at which a mispredicted branch flushed younger instructions.
    flushes: Vec<u64>,
}

/// In-order single-issue pipeline: operand waits, loads that miss stall the
/// memory stage, every fifth loop branch mispredicts, and one interrupt runs
/// the handler between two loop iterations.
fn simulate() -> Run {
    let mut rng = Rng(0x7a11_d1c3);
    let mut occupancy = vec![0u64; STAGES.len()];
    let mut writers: HashMap<u8, (usize, u64)> = HashMap::new();
    let mut insns: Vec<Insn> = Vec::new();
    let mut fetch = 6u64; // reset is released at cycle 4
    let mut loop_index = 0usize;
    let mut isr_index: Option<usize> = None;
    let mut irq = None;
    let mut branches = 0usize;
    let mut flush_at: Option<(u64, usize)> = None;
    let mut flushes = Vec::new();
    let mut iteration = 0u64;
    while insns.len() < INSTRUCTIONS {
        let index = insns.len();
        if irq.is_none() && flush_at.is_none() && isr_index.is_none() && fetch >= IRQ_CYCLE {
            irq = Some(fetch);
            isr_index = Some(0);
            fetch += 1;
        }
        let (op, pc, isr) = if let Some(i) = isr_index {
            (&ISR[i], 0x8000_0200 + 4 * i as u64, true)
        } else {
            let i = loop_index % LOOP.len();
            (&LOOP[i], 0x8000_0040 + 4 * i as u64, false)
        };
        let mut stages: Vec<(u64, u64)> = Vec::with_capacity(STAGES.len());
        let mut stalls = Vec::new();
        let mut deps = Vec::new();
        let mut access = None;
        let mut l1_miss = false;
        let mut begin = fetch;
        for k in 0..STAGES.len() {
            let mut start = begin.max(occupancy[k]);
            let mut reason = "pipeline busy";
            let mut duration = 1;
            if k == EXECUTE {
                for reg in op.src.iter().flatten() {
                    if let Some(&(producer, ready)) = writers.get(reg) {
                        deps.push(producer);
                        if ready > start {
                            start = ready;
                            reason = "operand";
                        }
                    }
                }
                if op.kind == Kind::Mul {
                    duration = 3;
                }
            }
            if k == MEMORY && matches!(op.kind, Kind::Load | Kind::Store) {
                let address = if isr {
                    0x4000_0040 + if op.kind == Kind::Store { 4 } else { 0 }
                } else {
                    0x1000_0000 + 0x40 * iteration + 0x800 * u64::from(loop_index % LOOP.len() == 1)
                };
                if op.kind == Kind::Load {
                    // The handler's device read is uncached; loop loads miss L1 one time in three.
                    l1_miss = (isr && op.text.contains("64(gp)")) || (!isr && rng.chance(3));
                    if l1_miss {
                        let level = if isr || rng.chance(2) {
                            Level::Dram
                        } else {
                            Level::L2
                        };
                        duration += if level == Level::Dram {
                            DRAM_LATENCY
                        } else {
                            L2_LATENCY
                        };
                        access = Some(Access {
                            write: false,
                            begin: start,
                            end: start + duration,
                            address,
                            level,
                        });
                    }
                } else {
                    // Write-through stores post to L2 after the memory stage.
                    access = Some(Access {
                        write: true,
                        begin: start + 1,
                        end: start + 4,
                        address,
                        level: Level::L2,
                    });
                }
            }
            if k > 0 && start > begin {
                // Waiting to enter stage k keeps the instruction in stage k-1.
                stages[k - 1].1 = start;
                occupancy[k - 1] = start;
                stalls.push((begin, start, reason));
            }
            let end = start + duration;
            stages.push((start, end));
            occupancy[k] = end;
            begin = end;
        }
        deps.sort_unstable();
        deps.dedup();
        if let Some(dst) = op.dst {
            let ready = if op.kind == Kind::Load {
                stages[MEMORY].1
            } else {
                stages[EXECUTE].1
            };
            writers.insert(dst, (index, ready));
        }
        let end = stages.last().unwrap().1;
        insns.push(Insn {
            index,
            pc,
            op,
            isr,
            stages,
            stalls,
            end,
            status: Status::Ok,
            deps,
            access,
            l1_miss,
        });
        fetch += 1;
        // A mispredicted branch resolves in execute and flushes the two
        // younger fall-through instructions fetched behind it.
        if let Some((resolve, first_victim)) = flush_at.filter(|(_, first)| index + 1 >= first + 2)
        {
            for victim in &mut insns[first_victim..] {
                victim.stages.retain(|(b, _)| *b < resolve);
                for stage in &mut victim.stages {
                    stage.1 = stage.1.min(resolve);
                }
                victim.stalls.retain(|(b, ..)| *b < resolve);
                for stall in &mut victim.stalls {
                    stall.1 = stall.1.min(resolve);
                }
                victim.end = resolve;
                victim.status = Status::Aborted;
                victim.access = None;
                victim.deps.clear();
            }
            flushes.push(resolve);
            fetch = resolve + 1;
            for slot in &mut occupancy {
                *slot = (*slot).min(fetch);
            }
            writers.retain(|_, (producer, _)| *producer < first_victim);
            flush_at = None;
            loop_index -= loop_index % LOOP.len(); // the branch was taken: loop head
            continue;
        }
        match isr_index {
            Some(i) if i + 1 == ISR.len() => isr_index = None,
            Some(i) => isr_index = Some(i + 1),
            None => {
                if op.kind == Kind::Branch {
                    branches += 1;
                    iteration += 1;
                    if branches.is_multiple_of(5) {
                        let resolve = insns[index].stages[EXECUTE].1;
                        flush_at = Some((resolve, index + 1));
                    }
                }
                loop_index += 1;
            }
        }
    }
    // The last fetches may still await a flush that never resolved: keep them.
    Run {
        insns,
        irq: irq.expect("the interrupt arrives before the program ends"),
        flushes,
    }
}

fn log_callsite<T: Payload + 'static>(
    w: &mut Writer,
    severity: u64,
    format: &str,
    line: u64,
) -> vtr::Result<vtr::PayloadType<T>> {
    vtr::logging::declare::<T>(
        w,
        vtr::logging::CallsiteOptions {
            severity,
            file: Some("tb/soc_tb.sv".into()),
            line: Some(line),
            function: Some("monitor".into()),
            ..vtr::logging::CallsiteOptions::new(format)
        },
    )
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

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let usage = "usage: landing OUTPUT.vtr [--dram-vcd DRAM.vcd]";
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (path, dram_vcd) = match args.as_slice() {
        [path] => (path, None),
        [path, flag, vcd] if flag == "--dram-vcd" => (path, Some(Path::new(vcd))),
        _ => return Err(usage.into()),
    };
    let path = Path::new(path);
    let run = simulate();
    let insns = &run.insns;
    let last_busy = insns.iter().map(|i| i.end).max().unwrap();
    let last_cycle = last_busy + IDLE_CYCLES;
    let dram_boost = edge(run.irq) + 1;
    let dram_gate = edge(last_busy + 8);

    let mut options = WriterOptions {
        mode: ExecutionMode::Inline,
        ..Default::default()
    };
    options.metadata.timescale = -9;
    options.metadata.file_type = 2;
    options.metadata.writer = "VTR landing demo".into();
    options.metadata.date = "2026-09-27".into();
    options.metadata.comment =
        "Volna landing demo: a dot-product loop with cache misses, DRAM refills and one interrupt."
            .into();
    let mut w = Writer::create_with(path, options)?;

    // -- hierarchy ------------------------------------------------------------------
    let soc = w.scope_with(
        SourceOptions::new("soc", None),
        ScopeType::Module,
        "soc_top",
    )?;
    let clk = bits(&mut w, soc, "clk", 1, 2, Direction::Input)?;
    let reset_n = bits(&mut w, soc, "reset_n", 1, 4, Direction::Input)?;
    let cpu = w.scope_with(
        SourceOptions::new("cpu0", Some(soc)),
        ScopeType::Core,
        "rv32_core",
    )?;
    let pc = bits(&mut w, cpu, "pc", 32, 2, Direction::Output)?;
    let phase = w.signal(
        SourceOptions::new("phase", Some(cpu)),
        SignalKind::VarLen,
        VarType::String,
        Direction::Implicit,
    )?;
    let stall = bits(&mut w, cpu, "stall", 1, 2, Direction::Output)?;
    let flush = bits(&mut w, cpu, "flush", 1, 2, Direction::Output)?;
    let irq = w.signal(
        SourceOptions::new("irq", Some(cpu)),
        SignalKind::Bits {
            width: 1,
            states: 2,
        },
        VarType::Event,
        Direction::Input,
    )?;
    let retired = bits(&mut w, cpu, "retired", 16, 2, Direction::Output)?;
    let pipeline = w.scope_with(
        SourceOptions::new("pipeline", Some(cpu)),
        ScopeType::Generic,
        "",
    )?;
    let instruction = w.transactions::<Instruction>(
        SourceOptions::new("instruction", Some(pipeline)),
        "PIPELINE",
    )?;

    let l2 = w.scope_with(
        SourceOptions::new("l2", Some(soc)),
        ScopeType::Module,
        "l2_cache",
    )?;
    let req_valid = bits(&mut w, l2, "req_valid", 1, 2, Direction::Input)?;
    let req_write = bits(&mut w, l2, "req_write", 1, 2, Direction::Input)?;
    let req_addr = bits(&mut w, l2, "req_addr", 32, 4, Direction::Input)?;
    let resp_valid = bits(&mut w, l2, "resp_valid", 1, 2, Direction::Output)?;
    let resp_data = bits(&mut w, l2, "resp_data", 32, 4, Direction::Output)?;
    let mshr = bits(&mut w, l2, "mshr_used", 3, 2, Direction::Output)?;
    let bus = w.scope_with(SourceOptions::new("bus", Some(l2)), ScopeType::Generic, "")?;
    let reads =
        w.transactions::<MemoryRequest>(SourceOptions::new("read", Some(bus)), "MEMORY_BUS")?;
    let writes =
        w.transactions::<MemoryRequest>(SourceOptions::new("write", Some(bus)), "MEMORY_BUS")?;

    let dram = w.scope_with(
        SourceOptions::new("dram", Some(soc)),
        ScopeType::Module,
        "lpddr_ctrl",
    )?;
    let dram_bus = w.scope_with(
        SourceOptions::new("bus", Some(dram)),
        ScopeType::Generic,
        "",
    )?;
    let bursts =
        w.transactions::<DramBurst>(SourceOptions::new("burst", Some(dram_bus)), "MEMORY_BUS")?;

    let perf = w.scope_with(
        SourceOptions::new("perf", Some(soc)),
        ScopeType::Module,
        "perf_monitor",
    )?;
    let ipc = real(&mut w, perf, "ipc")?;
    let bandwidth = real(&mut w, perf, "l2_bandwidth_gbps")?;
    let temperature = real(&mut w, perf, "temperature_c")?;

    let logs = w.logger(SourceOptions::new("log", Some(soc)))?;
    let events = w.events::<MemoryRequest>(SourceOptions::new("events", Some(soc)))?;
    let progress_sites = [
        log_callsite::<Boot>(&mut w, 2, "cycle {}: {}", 104)?,
        log_callsite::<Boot>(&mut w, 3, "cycle {}: {}", 108)?,
        log_callsite::<Boot>(&mut w, 4, "cycle {}: {}", 112)?,
    ];
    let boot_site = log_callsite::<Boot>(&mut w, 2, "reset released at cycle {}", 41)?;
    let irq_site = log_callsite::<Interrupt>(&mut w, 2, "irq {} taken at pc {}", 58)?;
    let miss_site = log_callsite::<Miss>(&mut w, 3, "l2 miss at {} refilled in {} cycles", 73)?;
    let ecc_site = log_callsite::<Ecc>(&mut w, 4, "dram ecc error at {} corrected", 88)?;
    let idle_site =
        log_callsite::<Idle>(&mut w, 2, "core idle, dram enters self-refresh at {}", 95)?;

    // Declared clocks: the dumped core clock, and a DRAM clock with no net
    // that boosts from 3 ns to 2 ns when the interrupt arrives and is gated
    // for self-refresh for 40 ns once the core goes idle.
    let core_clk = w.clock(SourceOptions::new("clk", Some(soc)))?;
    let dram_clk = w.clock(SourceOptions::new("dram_clk", Some(dram)))?;
    w.associate_clock(instruction, core_clk)?;
    w.associate_clock(reads, core_clk)?;
    w.associate_clock(writes, core_clk)?;
    w.associate_clock(bursts, dram_clk)?;
    w.clock_run(core_clk, FIRST_EDGE, PERIOD)?;
    w.clock_run(dram_clk, 0, 3)?;
    w.clock_stop(dram_clk, dram_boost - 1)?;
    w.clock_run(dram_clk, dram_boost, 2)?;
    w.clock_stop(dram_clk, dram_gate)?;
    w.clock_run(dram_clk, dram_gate + 40, 2)?;

    // -- waveforms, in time order --------------------------------------------------------
    let accesses: Vec<&Access> = insns.iter().filter_map(|i| i.access.as_ref()).collect();
    let isr_end = insns.iter().filter(|i| i.isr).map(|i| i.end).max().unwrap();
    let mut temp = 41.0f64;
    for t in 0..=edge(last_cycle) {
        w.advance_time(t)?;
        w.emit_u64(
            clk,
            u64::from(u8::from(
                t >= FIRST_EDGE && (t - FIRST_EDGE).is_multiple_of(PERIOD),
            )),
        )?;
        if t == 0 {
            w.emit_packed(reset_n, 4, &[2])?;
            w.emit_bytes(phase, b"reset")?;
            w.emit_u64(pc, 0x8000_0000)?;
            w.emit_packed(req_addr, 4, &[0xaa; 8])?;
            w.emit_packed(resp_data, 4, &[0xff; 8])?;
        }
        if t < FIRST_EDGE || !(t - FIRST_EDGE).is_multiple_of(PERIOD) {
            continue;
        }
        let c = (t - FIRST_EDGE) / PERIOD;
        w.emit_packed(reset_n, 4, &[u8::from(c >= 4)])?;
        let fetched = insns
            .iter()
            .find(|i| i.stages.first().is_some_and(|(b, _)| *b == c));
        if let Some(i) = fetched {
            w.emit_u64(pc, i.pc)?;
        }
        let phase_text: &[u8] = if c < 6 {
            b"reset"
        } else if c >= run.irq && c < isr_end {
            b"irq handler"
        } else if c >= last_busy {
            b"idle (wfi)"
        } else {
            b"dot product loop"
        };
        w.emit_bytes(phase, phase_text)?;
        w.emit_u64(
            stall,
            u64::from(u8::from(
                insns
                    .iter()
                    .any(|i| i.stalls.iter().any(|(b, e, _)| *b <= c && c < *e)),
            )),
        )?;
        w.emit_u64(flush, u64::from(u8::from(run.flushes.contains(&c))))?;
        if c == run.irq {
            w.emit_u64(irq, 1)?;
        }
        let done = insns
            .iter()
            .filter(|i| i.status == Status::Ok && i.end <= c)
            .count() as u64;
        w.emit_u64(retired, done)?;
        let request = accesses.iter().find(|a| a.begin == c);
        w.emit_u64(req_valid, u64::from(u8::from(request.is_some())))?;
        w.emit_u64(
            req_write,
            u64::from(u8::from(request.is_some_and(|a| a.write))),
        )?;
        match request {
            Some(a) => w.emit_u64(req_addr, a.address)?,
            None => w.emit_packed(req_addr, 4, &[0xaa; 8])?,
        }
        let response = accesses.iter().find(|a| !a.write && a.end == c + 1);
        w.emit_u64(resp_valid, u64::from(u8::from(response.is_some())))?;
        match response {
            Some(a) => w.emit_u64(
                resp_data,
                (a.address.wrapping_mul(0x9e37_79b9) >> 7) & 0xffff_ffff,
            )?,
            None => w.emit_packed(resp_data, 4, &[0xff; 8])?,
        }
        let outstanding = accesses
            .iter()
            .filter(|a| !a.write && a.begin <= c && c < a.end)
            .count() as u64;
        w.emit_u64(mshr, outstanding.min(7))?;
        if c.is_multiple_of(4) {
            // Over the last 32 cycles: retired instructions per cycle, and
            // 64-byte lines moved per nanosecond.
            let window = c.saturating_sub(32)..c;
            let recent = insns
                .iter()
                .filter(|i| i.status == Status::Ok && window.contains(&i.end))
                .count();
            let lines = accesses
                .iter()
                .filter(|a| window.contains(&a.begin))
                .count();
            let ipc_now = recent as f64 / 32.0;
            w.emit_real(ipc, ipc_now)?;
            w.emit_real(bandwidth, lines as f64 * 64.0 / (32.0 * PERIOD as f64))?;
            let target = 40.0 + 30.0 * ipc_now + 4.0 * lines as f64 / 8.0;
            temp += (target - temp) * 0.08;
            w.emit_real(temperature, (temp * 100.0).round() / 100.0)?;
        }
    }

    // -- transactions ------------------------------------------------------------------------
    let k_l1 = w.intern("l1")?;
    let k_detail = w.intern("detail")?;
    let k_reason = w.intern("reason")?;
    let k_level = w.intern("served_by")?;
    let k_ecc = w.intern("ecc")?;
    let k_wakeup = w.intern("wakeup")?;
    let k_causes = w.intern("causes")?;
    let k_refill = w.intern("refill")?;
    let lane0 = w.intern("0")?;
    let lane_stall = w.intern("stall")?;
    let stall_name = w.intern("stl")?;
    let stage_names: Vec<u32> = STAGES
        .iter()
        .map(|s| w.intern(s))
        .collect::<vtr::Result<_>>()?;
    let s_addr = w.intern("addr")?;
    let s_data = w.intern("data")?;
    let s_act = w.intern("act")?;
    let s_cas = w.intern("cas")?;
    let s_burst = w.intern("burst")?;
    let hit = w.intern("hit")?;
    let miss = w.intern("miss")?;
    let served_l2 = w.intern("L2")?;
    let served_dram = w.intern("DRAM")?;
    let corrected = w.intern("corrected")?;
    let reasons: HashMap<&str, u32> = ["operand", "pipeline busy"]
        .into_iter()
        .map(|r| Ok((r, w.intern(r)?)))
        .collect::<vtr::Result<_>>()?;

    w.log_at(
        logs,
        boot_site,
        edge(4),
        &Boot {
            cycle: 4,
            label: "reset released".into(),
        },
        None,
    )?;
    let mut ids: Vec<Transaction<Instruction>> = Vec::with_capacity(insns.len());
    let mut refills = 0u64;
    let mut dram_misses = Vec::new();
    let mut diagnostic_count = 0;
    let mut event_count = 0;
    for insn in insns {
        let begin = edge(insn.stages[0].0);
        let payload = Instruction {
            label: format!("{:08x}: {}", insn.pc, insn.op.text),
            insn_id: insn.index as u64,
            pc: insn.pc,
        };
        let tx = w.begin_at(instruction, begin, &payload, None)?;
        ids.push(tx);
        w.event_at(
            events,
            tx.owner(),
            begin,
            &MemoryRequest {
                label: format!("dispatch {}", insn.op.text),
                address: insn.pc,
                bytes: 4,
            },
        )?;
        event_count += 1;
        if insn.index % 8 == 0 || insn.index == 24 {
            let count = if insn.index == 24 { 300 } else { 1 };
            for occurrence in 0..count {
                let severity = if count == 1 {
                    usize::from(insn.index % 64 == 0)
                } else if occurrence % 50 == 0 {
                    2
                } else {
                    usize::from(occurrence % 12 == 0)
                };
                w.log_at(
                    logs,
                    progress_sites[severity],
                    begin,
                    &Boot {
                        cycle: insn.stages[0].0,
                        label: format!(
                            "{} (request {occurrence})",
                            ["response queued", "queue near capacity", "response timeout"]
                                [severity]
                        ),
                    },
                    Some(tx.owner()),
                )?;
                diagnostic_count += 1;
            }
        }
        if insn.index == 24 || insn.index == 96 {
            for occurrence in 0..128 {
                let time = if insn.index == 24 {
                    begin
                } else {
                    begin + occurrence % (edge(insn.end) - begin).max(1)
                };
                w.event_at(
                    events,
                    tx.owner(),
                    time,
                    &MemoryRequest {
                        label: format!("response fragment {occurrence}"),
                        address: insn.pc + occurrence * 4,
                        bytes: 4,
                    },
                )?;
                event_count += 1;
                if insn.index == 96 {
                    w.log_at(
                        logs,
                        progress_sites[occurrence as usize % 3],
                        time,
                        &Boot {
                            cycle: insn.stages[0].0,
                            label: format!("fragment {occurrence} received"),
                        },
                        Some(tx.owner()),
                    )?;
                    diagnostic_count += 1;
                }
            }
        }
        if insn.op.kind == Kind::Load && insn.status == Status::Ok {
            w.attribute(
                tx.owner(),
                k_l1,
                Value::Str(if insn.l1_miss { miss } else { hit }),
            )?;
        }
        for (k, (b, e)) in insn.stages.iter().enumerate() {
            w.embedded_stage(
                tx.owner(),
                edge(*b),
                stage_names[k],
                lane0,
                Some(edge(*e)),
                vec![],
            )?;
        }
        for (b, e, reason) in &insn.stalls {
            w.embedded_stage(
                tx.owner(),
                edge(*b),
                stall_name,
                lane_stall,
                Some(edge(*e)),
                vec![Attribute {
                    key: k_reason,
                    value: Value::Str(reasons[reason]),
                }],
            )?;
        }
        for &producer in &insn.deps {
            w.relate(k_wakeup, ids[producer].id(), tx.id(), vec![])?;
        }
        if insn.isr && insn.index == insns.iter().find(|i| i.isr).unwrap().index {
            w.log_at(
                logs,
                irq_site,
                edge(run.irq),
                &Interrupt {
                    line: 3,
                    pc: Address(insn.pc),
                    label: "device interrupt".into(),
                },
                Some(tx.owner()),
            )?;
        }
        if let Some(a) = &insn.access {
            let generator = if a.write { writes } else { reads };
            let payload = MemoryRequest {
                label: format!(
                    "{} {:#010x}",
                    if a.write { "write" } else { "read" },
                    a.address
                ),
                address: a.address,
                bytes: if a.write { 4 } else { 64 },
            };
            let request = w.begin_at(generator, edge(a.begin), &payload, Some(tx.owner()))?;
            w.attribute(
                request.owner(),
                k_level,
                Value::Str(if a.level == Level::Dram {
                    served_dram
                } else {
                    served_l2
                }),
            )?;
            w.embedded_stage(
                request.owner(),
                edge(a.begin),
                s_addr,
                lane0,
                Some(edge(a.begin + 1)),
                vec![],
            )?;
            w.embedded_stage(
                request.owner(),
                edge(a.end - 1),
                s_data,
                lane0,
                Some(edge(a.end)),
                vec![],
            )?;
            w.relate(k_causes, tx.id(), request.id(), vec![])?;
            if a.level == Level::Dram {
                // The refill: activate, column access, then the data burst.
                let (b, e) = (a.begin + 2, a.end - 2);
                let payload = DramBurst {
                    label: format!("refill {:#010x}", a.address),
                    address: a.address,
                    bank: (a.address >> 6) & 7,
                    row: a.address >> 13,
                };
                let burst = w.begin_at(bursts, edge(b), &payload, Some(request.owner()))?;
                w.embedded_stage(
                    burst.owner(),
                    edge(b),
                    s_act,
                    lane0,
                    Some(edge(b + 4)),
                    vec![],
                )?;
                w.embedded_stage(
                    burst.owner(),
                    edge(b + 4),
                    s_cas,
                    lane0,
                    Some(edge(e - 4)),
                    vec![],
                )?;
                w.embedded_stage(
                    burst.owner(),
                    edge(e - 4),
                    s_burst,
                    lane0,
                    Some(edge(e)),
                    vec![],
                )?;
                w.relate(k_refill, burst.id(), request.id(), vec![])?;
                refills += 1;
                let status = if refills == 9 {
                    w.attribute(burst.owner(), k_ecc, Value::Str(corrected))?;
                    w.log_at(
                        logs,
                        ecc_site,
                        edge(e - 1),
                        &Ecc {
                            address: Address(a.address),
                            label: "ecc error".into(),
                        },
                        Some(burst.owner()),
                    )?;
                    Status::Error
                } else {
                    Status::Ok
                };
                w.end_at(burst, edge(e), status)?;
                if dram_misses.len() < 6 {
                    w.log_at(
                        logs,
                        miss_site,
                        edge(a.end),
                        &Miss {
                            address: Address(a.address),
                            cycles: a.end - a.begin,
                            label: "l2 miss".into(),
                        },
                        Some(request.owner()),
                    )?;
                    dram_misses.push(a.address);
                }
            }
            w.end_at(request, edge(a.end), Status::Ok)?;
        }
        if insn.status == Status::Aborted {
            let detail = w.intern("squashed by a mispredicted branch")?;
            w.attribute(tx.owner(), k_detail, Value::Str(detail))?;
        }
        w.end_at(tx, edge(insn.end), insn.status)?;
    }
    w.log_at(
        logs,
        idle_site,
        dram_gate,
        &Idle {
            time: TimeValue(dram_gate),
            label: "self-refresh".into(),
        },
        None,
    )?;
    w.advance_time(edge(last_cycle))?;
    w.close()?;

    if let Some(vcd) = dram_vcd {
        write_dram_vcd(vcd, insns, last_cycle, dram_boost, dram_gate)?;
    }

    let size = std::fs::metadata(path)?.len();
    assert!(
        size < MAX_BYTES,
        "{} is {size} bytes, limit {MAX_BYTES}",
        path.display()
    );
    let r = Reader::open(path)?;
    let report = r.validate()?;
    assert!(report.missing.is_empty());
    let mut per_generator: HashMap<String, usize> = HashMap::new();
    let (mut flushed, mut errors) = (0, 0);
    let mut with_stalls = 0;
    let mut n_tx = 0;
    r.transactions(TransactionQuery::default(), |tx| {
        *per_generator
            .entry(
                r.string(r.node(NodeId(tx.generator().unwrap())).unwrap().name)
                    .unwrap()
                    .to_owned(),
            )
            .or_default() += 1;
        flushed += usize::from(tx.status() == Status::Aborted);
        errors += usize::from(tx.status() == Status::Error);
        with_stalls += usize::from(
            tx.stages()
                .iter()
                .any(|stage| r.string(stage.lane).unwrap() == "stall"),
        );
        n_tx += 1;
        true
    })?;
    let mut log_count = 0;
    r.records(
        RecordQuery {
            kind: Some(RecordKind::Log),
            ..Default::default()
        },
        |record| {
            vtr::logging::format_log_bytes(&r, &record).unwrap();
            log_count += 1;
            true
        },
    )?;
    let count = |name: &str| per_generator.get(name).copied().unwrap_or(0);
    assert_eq!(count("instruction"), INSTRUCTIONS);
    assert_eq!(count("burst") as u64, refills);
    assert!(count("read") > 20 && count("write") > 20 && refills >= 9);
    assert!(flushed > 0);
    assert_eq!(errors, 1);
    assert!(with_stalls > 0 && report.relations > 0);
    assert_eq!(log_count, 4 + dram_misses.len() + diagnostic_count);
    let mut located_events = 0;
    r.records(
        RecordQuery {
            kind: Some(RecordKind::Event),
            ..Default::default()
        },
        |_| {
            located_events += 1;
            true
        },
    )?;
    assert_eq!(located_events, event_count);
    for (source, clock) in [
        (instruction.id(), core_clk.id()),
        (reads.id(), core_clk.id()),
        (writes.id(), core_clk.id()),
        (bursts.id(), dram_clk.id()),
    ] {
        assert_eq!(r.clock_for(source)?, Some(clock));
    }
    let mut clock_count = 0;
    for clock in [core_clk.id(), dram_clk.id()] {
        r.transactions(
            TransactionQuery {
                generator: Some(clock),
                ..Default::default()
            },
            |record| {
                assert_eq!(record.kind(), RecordKind::Clock);
                clock_count += 1;
                true
            },
        )?;
    }
    assert_eq!(clock_count, 4);
    assert_eq!(r.waveform_time_range().unwrap().last, edge(last_cycle));
    let mut names: Vec<_> = per_generator.into_iter().collect();
    names.sort();
    println!(
        "{}: {size} bytes, 0..{} ns, irq at cycle {} ({} ns), {} flushes, {n_tx} owners, {} relations, {log_count} logs",
        path.display(),
        edge(last_cycle),
        run.irq,
        edge(run.irq),
        run.flushes.len(),
        report.relations
    );
    for (name, count) in names {
        println!("  {name}: {count}");
    }
    Ok(())
}

/// The LPDDR controller's dump of the run, in picoseconds: its clock (3 ns,
/// 2 ns from the interrupt, gated for self-refresh), `cke`, the command
/// state as text, and per refill the address, bank and row from ACT, the
/// data strobe during the burst and the corrected ECC error of the ninth.
fn write_dram_vcd(
    path: &Path,
    insns: &[Insn],
    last_cycle: u64,
    boost: u64,
    gate: u64,
) -> Result<(), Box<dyn std::error::Error>> {
    use std::fmt::Write as _;
    let ps = |ns: u64| ns * 1000;
    // (time in ps, variable id, VCD value)
    let mut ev: Vec<(u64, char, String)> = Vec::new();
    let end = ps(edge(last_cycle));
    let (mut t, mut level) = (0, 0u8);
    while t < end {
        let ns = t / 1000;
        let half = if ns < boost { 1500 } else { 1000 };
        let gated = ns >= gate && ns < gate + 40;
        if !gated {
            level ^= 1;
            ev.push((t, 'k', level.to_string()));
        } else if level == 1 {
            level = 0;
            ev.push((t, 'k', "0".into()));
        }
        t += half;
    }
    let bus = |v: Option<u64>| v.map_or_else(|| "bx".to_owned(), |v| format!("b{v:b}"));
    for (id, value) in [('e', "1"), ('s', "IDLE"), ('v', "0"), ('x', "0")] {
        ev.push((0, id, value.into()));
    }
    for id in ['a', 'b', 'r'] {
        ev.push((0, id, bus(None)));
    }
    ev.push((ps(gate), 'e', "0".into()));
    ev.push((ps(gate), 's', "SREF".into()));
    ev.push((ps(gate + 40), 'e', "1".into()));
    ev.push((ps(gate + 40), 's', "IDLE".into()));
    let refills = insns
        .iter()
        .filter_map(|i| i.access.as_ref())
        .filter(|a| a.level == Level::Dram);
    for (n, a) in refills.enumerate() {
        // The VTR's burst: ACT at `b`, CAS from `b + 4`, data from `e - 4`.
        let (b, e) = (a.begin + 2, a.end - 2);
        ev.push((ps(edge(b)), 's', "ACT".into()));
        ev.push((ps(edge(b)), 'a', bus(Some(a.address))));
        ev.push((ps(edge(b)), 'b', bus(Some((a.address >> 6) & 7))));
        ev.push((ps(edge(b)), 'r', bus(Some(a.address >> 13))));
        ev.push((ps(edge(b + 4)), 's', "RD".into()));
        ev.push((ps(edge(e - 4)), 's', "BURST".into()));
        ev.push((ps(edge(e - 4)), 'v', "1".into()));
        if n == 8 {
            ev.push((ps(edge(e - 1)), 'x', "1".into()));
            ev.push((ps(edge(e)), 'x', "0".into()));
        }
        ev.push((ps(edge(e)), 'v', "0".into()));
        ev.push((ps(edge(e)), 's', "IDLE".into()));
        for id in ['a', 'b', 'r'] {
            ev.push((ps(edge(e)), id, bus(None)));
        }
    }
    ev.sort_by_key(|e| e.0);
    let mut o = String::from(
        "$version VTR landing demo: LPDDR controller model $end\n$timescale 1ps $end\n\
         $scope module lpddr_tb $end\n$scope module ctrl $end\n",
    );
    let vars = [
        ('k', "wire", 1, "ck"),
        ('e', "wire", 1, "cke"),
        ('s', "string", 1, "state"),
        ('a', "wire", 32, "addr"),
        ('b', "wire", 3, "bank"),
        ('r', "wire", 19, "row"),
        ('v', "wire", 1, "dq_valid"),
        ('x', "wire", 1, "ecc_err"),
    ];
    for (id, kind, width, name) in vars {
        writeln!(o, "$var {kind} {width} {id} {name} $end")?;
    }
    o.push_str("$upscope $end\n$upscope $end\n$enddefinitions $end\n");
    let mut last = None;
    for (t, id, value) in ev {
        if last != Some(t) {
            writeln!(o, "#{t}")?;
            last = Some(t);
        }
        if id == 's' {
            writeln!(o, "s{value} {id}")?;
        } else if value.starts_with('b') {
            writeln!(o, "{value} {id}")?;
        } else {
            writeln!(o, "{value}{id}")?;
        }
    }
    writeln!(o, "#{end}")?;
    std::fs::write(path, o)?;
    Ok(())
}
