//! Hardware streams and cross-stream workload provenance for a synthetic SIMT GPU.
//!
//! Regenerate: cargo run --locked -p volna --example gpu_transaction_trace
//! An optional argument replaces examples/gpu_transaction_trace.vtr in this crate.
//!
//! One cycle is one nanosecond, not a measured GPU clock. Two sequential grids
//! each run two workgroups of four 32-lane warps, reusing eight hardware slots.
//! Each warp runs 50 iterations of a 100-instruction tiled arithmetic loop.
//! Resource paths are static; logical ownership lives exclusively in TX parents.
//!
//! Scheduling assumptions: strict round-robin issue within each two-warp
//! partition, at most one issue/cycle; an unready turn can idle the scheduler
//! (no ready-warp selection). Fetch/decode take one cycle each. Four scoreboard
//! registers bound independent work. ADD/XOR take 2 cycles, FMA 6, branch 2,
//! plus issue and writeback. ALUs accept one operation/cycle (up to six in flight).
//! Each SM has one blocking LSU port, two wide shared-memory bank groups
//! (8 lanes per 3-cycle round), one matrix engine and one DMA engine. Cache
//! service is blocking per resource; the bidirectional fabric link serializes
//! 64-byte transfers at 16 bytes/cycle. Ingress/routing/arbitration/acceptance
//! cost 2/1/1/1 cycles, plus credit waits. Lookup latencies are 4/12/24 cycles,
//! return/fill 2/4, DRAM activate/read/precharge 12/40/12. Atomics add an 8-cycle
//! exclusive update; stores/atomics use 16 active lanes (64 bytes), loads use
//! all 32 lanes (two sectors). Matrix jobs execute four 40-cycle microtiles;
//! DMA jobs fetch two sectors through L2, bypassing L1, then deposit into shared.
//! Cache residency is scripted (prewarmed hits and distinct cold lines), not a
//! cache simulator. No merging, coherence, replacement, or virtual-memory model.
//!
//! The illustrative L3 is deliberately NOT representative of modern NVIDIA
//! GPUs, which normally expose L2 as the last-level GPU cache. Per-SM matrix/DMA
//! placement and all capacities/latencies are illustrative, not a SKU model.
//! Parents express provenance, not temporal containment: asynchronous jobs
//! outlive their launch instructions. Directed relations are consumer->producer
//! (depends_on), instruction->job (launches/waits_for), arrival->barrier
//! (arrives_at), warp->job (participates_in), and packet->fill (served_by).
//! Stage lanes are lifecycle tracks, never the 32 scalar SIMT lanes.
//! `gpu.resource` and numeric `gpu.destination_resource` values identify
//! hierarchy generator nodes; `gpu.job` identifies an asynchronous TX.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, ensure};
use vtr::{
    NodeId, ReadOptions, Reader, ScopeType, StrId, Timescale, Transaction, TxId, TxKind, TxQuery,
    TxStatus, Value, Writer, WriterOptions,
};

const GRIDS: usize = 2;
const SMS: usize = 2;
const PARTITIONS: usize = 2;
const SLOTS: usize = 2;
const WARPS: usize = PARTITIONS * SLOTS;
const INSTRUCTIONS: u64 = 5_000;

// A fixed vocabulary keeps interning out of the emission loop.
macro_rules! vocabulary {
    ($($field:ident: $text:literal),* $(,)?) => {
        struct Names { $( $field: StrId, )* }
        impl Names {
            fn new(w: &mut Writer) -> Self {
                Self { $( $field: w.intern($text), )* }
            }
        }
    };
}
vocabulary! {
    label: "vtr.label", grid: "gpu.grid", kernel: "gpu.kernel",
    dimensions: "gpu.grid_dimensions", block: "gpu.block_coordinates",
    threads: "gpu.block_dimensions", warp: "gpu.logical_warp", epoch: "gpu.slot_epoch",
    pc: "gpu.pc", sequence: "gpu.sequence", opcode: "gpu.opcode", class: "gpu.class",
    mask: "gpu.active_mask", space: "gpu.address_space", operation: "gpu.operation",
    address: "gpu.address", bytes: "gpu.bytes", outcome: "gpu.cache_outcome",
    shape: "gpu.tensor_shape", direction: "gpu.direction", source: "gpu.source_resource",
    destination: "gpu.destination_resource", route: "gpu.route", hop: "gpu.hop_index",
    round: "gpu.service_round", resource: "gpu.resource", job: "gpu.job", lane: "pipeline",
    residency: "residency", activity: "activity",
    fetch: "fetch", decode: "decode", ready: "operand_wait", scheduler: "scheduler_wait",
    issue: "issue", execute: "execute", memory_wait: "memory_wait", writeback: "writeback",
    queued: "queued", running: "running", resident: "resident", barrier_wait: "barrier_wait",
    ingress: "ingress_buffer", routing: "routing", arbitration: "switch_allocation",
    credit: "credit_wait", link: "link_traversal", acceptance: "downstream_acceptance",
    lookup: "cache_lookup", hit: "hit", miss: "miss", forwarding: "forwarding",
    returning: "return", fill: "fill", service: "service", translate: "translate",
    walk: "page_table_walk", activate: "activate", read: "read", precharge: "precharge",
    issued: "issued", request_created: "request_created", launched: "async_launched",
    completed: "completed", allocated: "allocated", dispatched: "dispatched",
    stalled: "stalled", resumed: "resumed", launches: "launches", waits_for: "waits_for",
    arrives_at: "arrives_at", participates: "participates_in", served_by: "served_by",
    depends: "depends_on", global: "global", shared: "shared", store: "write", atomic: "atomic",
    request: "request", response: "response", round_trip: "request_response",
    fabric_route: "single_switch", local_route: "local", dma_direction: "global_to_shared",
    fma: "FFMA", add: "IADD", xor: "XOR", branch: "BRA", load: "LDG",
    stg: "STG", atom: "ATOM.ADD", lds: "LDS", matrix: "MMA.ASYNC", dma: "TMA.LOAD",
    wait: "WAIT", barrier: "BAR.SYNC", arithmetic: "arithmetic", control: "control",
    memory: "memory", launch: "async_launch", synchronization: "synchronization",
}

#[derive(Clone, Copy)]
struct Slot {
    warp: NodeId,
    instruction: NodeId,
}
#[derive(Clone, Copy)]
struct Cache {
    packet: NodeId,
    fill: NodeId,
    writeback: Option<NodeId>,
    endpoint: StrId,
}
struct Sm {
    workgroup: NodeId,
    slots: Vec<Slot>,
    alu: Vec<NodeId>,
    request: NodeId,
    tlb: NodeId,
    l1: Cache,
    banks: Vec<NodeId>,
    matrix: [NodeId; 2],
    dma: [NodeId; 3],
    dma_endpoint: StrId,
    barrier: NodeId,
}
struct Hardware {
    queue: [NodeId; 4],
    assignment: NodeId,
    sms: Vec<Sm>,
    fabric: NodeId,
    walker: NodeId,
    l2: Cache,
    l3: Cache,
    controller: NodeId,
    dram: NodeId,
    dram_endpoint: StrId,
}
fn scope(w: &mut Writer, parent: NodeId, name: &str) -> vtr::Result<NodeId> {
    w.add_scope(Some(parent), name, ScopeType::Generic, "")
}
fn stream<const N: usize>(
    w: &mut Writer,
    parent: NodeId,
    name: &str,
    generators: [&str; N],
) -> vtr::Result<[NodeId; N]> {
    let s = w.add_stream(Some(parent), name, "TRANSACTION")?;
    let mut ids = [NodeId(0); N];
    for (id, name) in ids.iter_mut().zip(generators) {
        *id = w.add_generator(s, name)?;
    }
    Ok(ids)
}
fn cache(
    w: &mut Writer,
    parent: NodeId,
    name: &str,
    endpoint: &str,
    last: bool,
) -> vtr::Result<Cache> {
    let c = scope(w, parent, name)?;
    let requests = w.add_stream(Some(c), "requests", "TRANSACTION")?;
    let packet = w.add_generator(requests, "MemoryPacket")?;
    let fill = w.add_generator(requests, "CacheFill")?;
    let writeback = if last {
        None
    } else {
        Some(w.add_generator(requests, "Writeback")?)
    };
    Ok(Cache {
        packet,
        fill,
        writeback,
        endpoint: w.intern(endpoint),
    })
}
fn hardware(w: &mut Writer) -> vtr::Result<Hardware> {
    let gpu = w.add_scope(None, "gpu", ScopeType::Generic, "synthetic_simt")?;
    let cp = scope(w, gpu, "command_processor")?;
    let queue = stream(
        w,
        cp,
        "queue[0]",
        ["CommandStream", "Grid", "Wait", "Signal"],
    )?;
    let [assignment] = stream(w, cp, "work_distributor", ["WorkgroupAssignment"])?;
    let cluster = scope(w, gpu, "compute_cluster[0]")?;
    let mut sms = Vec::new();
    for sm in 0..SMS {
        let s = scope(w, cluster, &format!("sm[{sm}]"))?;
        let [workgroup] = stream(w, s, "workgroup_slot[0]", ["Workgroup"])?;
        let mut slots = Vec::new();
        let mut alu = Vec::new();
        for p in 0..PARTITIONS {
            let partition = scope(w, s, &format!("simt_partition[{p}]"))?;
            for slot in 0..SLOTS {
                let [warp, instruction] = stream(
                    w,
                    partition,
                    &format!("warp_slot[{slot}]"),
                    ["Warp", "Instruction"],
                )?;
                slots.push(Slot { warp, instruction });
            }
            alu.push(stream(w, partition, "alu_pipeline[0]", ["InstructionExecution"])?[0]);
        }
        let lsu = scope(w, s, "load_store_unit")?;
        let [request] = stream(w, lsu, "request_port[0]", ["MemoryRequest"])?;
        let [tlb] = stream(w, lsu, "tlb", ["Translation"])?;
        let l1 = cache(
            w,
            s,
            "l1_cache",
            &format!("gpu.compute_cluster[0].sm[{sm}].l1_cache"),
            false,
        )?;
        let shared = scope(w, s, "shared_memory")?;
        let banks = (0..2)
            .map(|b| stream(w, shared, &format!("bank[{b}]"), ["BankAccess"]).map(|x| x[0]))
            .collect::<vtr::Result<Vec<_>>>()?;
        let matrix = stream(w, s, "matrix_engine[0]", ["MatrixJob", "MatrixTile"])?;
        let dma = stream(
            w,
            s,
            "tensor_dma_engine[0]",
            ["TensorDmaJob", "TransferChunk", "MemoryRequest"],
        )?;
        let dma_endpoint = w.intern(&format!(
            "gpu.compute_cluster[0].sm[{sm}].tensor_dma_engine[0]"
        ));
        let [barrier] = stream(w, s, "barrier_unit", ["WorkgroupBarrier"])?;
        sms.push(Sm {
            workgroup,
            slots,
            alu,
            request,
            tlb,
            l1,
            banks,
            matrix,
            dma,
            dma_endpoint,
            barrier,
        });
    }
    let fabric_scope = scope(w, gpu, "memory_fabric")?;
    let [fabric] = stream(w, fabric_scope, "port[0]", ["MemoryPacket"])?;
    let memory = scope(w, gpu, "memory_subsystem")?;
    let [walker] = stream(w, memory, "page_walker[0]", ["PageWalk"])?;
    let l2 = cache(
        w,
        memory,
        "l2_partition[0]",
        "gpu.memory_subsystem.l2_partition[0]",
        false,
    )?;
    let l3 = cache(
        w,
        memory,
        "illustrative_l3",
        "gpu.memory_subsystem.illustrative_l3",
        true,
    )?;
    let mc = scope(w, memory, "memory_controller[0]")?;
    let [controller] = stream(w, mc, "requests", ["MemoryTransaction"])?;
    let [dram] = stream(w, mc, "dram_channel[0]", ["DramCommand"])?;
    let dram_endpoint = w.intern("gpu.memory_subsystem.memory_controller[0]");
    Ok(Hardware {
        queue,
        assignment,
        sms,
        fabric,
        walker,
        l2,
        l3,
        controller,
        dram,
        dram_endpoint,
    })
}

struct Emit {
    w: Writer,
    n: Names,
}
impl Emit {
    fn begin(
        &mut self,
        generator: NodeId,
        parent: Option<TxId>,
        time: u64,
        label: &str,
    ) -> vtr::Result<TxId> {
        let tx = self.w.begin_tx(generator, time)?;
        self.w.set_tx_kind(tx, TxKind::Internal)?;
        if let Some(parent) = parent {
            self.w.set_tx_parent(tx, parent)?;
        }
        self.attr(tx, self.n.label, Value::Text(label.into()))?;
        Ok(tx)
    }
    fn attr(&mut self, tx: TxId, key: StrId, value: Value) -> vtr::Result<()> {
        self.w.tx_attr(tx, key, &value)
    }
    fn stage(&mut self, tx: TxId, name: StrId, begin: u64, end: u64) -> vtr::Result<()> {
        if end > begin {
            self.w.tx_stage(tx, name, self.n.lane, begin, end, &[])?;
        }
        Ok(())
    }
    fn event(&mut self, tx: TxId, name: StrId, time: u64) -> vtr::Result<()> {
        self.w.tx_event(tx, time, name, &[])
    }
    fn end(&mut self, tx: TxId, time: u64) -> vtr::Result<()> {
        self.event(tx, self.n.completed, time)?;
        self.w.end_tx(tx, time, TxStatus::Ok)
    }
    fn relate(&mut self, kind: StrId, from: TxId, to: TxId) -> vtr::Result<()> {
        self.w.relate(kind, from, to, &[])
    }
    fn service(
        &mut self,
        generator: NodeId,
        parent: TxId,
        interval: (u64, u64),
        stage: StrId,
        label: &str,
    ) -> vtr::Result<TxId> {
        let tx = self.begin(generator, Some(parent), interval.0, label)?;
        self.stage(tx, stage, interval.0, interval.1)?;
        self.end(tx, interval.1)?;
        Ok(tx)
    }
}

#[derive(Default)]
struct Resources {
    issue: [[u64; PARTITIONS]; SMS],
    lsu: [u64; SMS],
    bank: [[u64; 2]; SMS],
    matrix: [u64; SMS],
    dma: [u64; SMS],
    // L1 per SM, then shared L2 and L3. Each permits one outstanding service.
    cache: [u64; SMS + 2],
    link: u64,
    dram: u64,
    walker: u64,
}

/// A real fabric transfer, separate from cache-local service packets. Buffering
/// can overlap earlier transfers; only link traversal reserves the shared link.
#[expect(
    clippy::too_many_arguments,
    reason = "explicit route and time at each scripted hop"
)]
fn hop(
    e: &mut Emit,
    h: &Hardware,
    r: &mut Resources,
    parent: TxId,
    start: u64,
    endpoints: (StrId, StrId),
    index: usize,
    response: bool,
) -> vtr::Result<u64> {
    let tx = e.begin(
        h.fabric,
        Some(parent),
        start,
        if response {
            "return flit"
        } else {
            "request flit"
        },
    )?;
    for (key, value) in [
        (e.n.source, Value::Str(endpoints.0)),
        (e.n.destination, Value::Str(endpoints.1)),
        (
            e.n.direction,
            Value::Str(if response { e.n.response } else { e.n.request }),
        ),
        (e.n.route, Value::Str(e.n.fabric_route)),
        (e.n.hop, Value::U64(index as u64)),
    ] {
        e.attr(tx, key, value)?;
    }
    e.stage(tx, e.n.ingress, start, start + 2)?;
    e.stage(tx, e.n.routing, start + 2, start + 3)?;
    e.stage(tx, e.n.arbitration, start + 3, start + 4)?;
    let link = (start + 4).max(r.link);
    e.stage(tx, e.n.credit, start + 4, link)?;
    e.stage(tx, e.n.link, link, link + 4)?;
    r.link = link + 4;
    e.stage(tx, e.n.acceptance, link + 4, link + 5)?;
    e.end(tx, link + 5)?;
    Ok(link + 5)
}

fn dram(
    e: &mut Emit,
    h: &Hardware,
    r: &mut Resources,
    parent: TxId,
    arrival: u64,
) -> vtr::Result<u64> {
    let tx = e.begin(h.controller, Some(parent), arrival, "cold-line read")?;
    let start = arrival.max(r.dram);
    e.stage(tx, e.n.queued, arrival, start)?;
    let mut time = start;
    for (name, duration, label) in [
        (e.n.activate, 12, "ACT"),
        (e.n.read, 40, "RD + burst"),
        (e.n.precharge, 12, "PRE"),
    ] {
        e.service(h.dram, tx, (time, time + duration), name, label)?;
        time += duration;
    }
    e.stage(tx, e.n.service, start, time)?;
    e.end(tx, time)?;
    r.dram = time;
    Ok(time)
}

/// `depth`: 0=L1 hit, 1=L2 hit, 2=illustrative L3 hit, 3=DRAM.
/// A cache miss retains the upstream packet as parent, while transport packets
/// describe the distinct request/response link work beneath that same packet.
#[expect(
    clippy::too_many_arguments,
    reason = "explicit origin, route, and service parameters in the recursive cache path"
)]
fn cache_access(
    e: &mut Emit,
    h: &Hardware,
    r: &mut Resources,
    parent: TxId,
    sm: usize,
    level: usize,
    depth: usize,
    arrival: u64,
    address: u64,
    operation: StrId,
) -> vtr::Result<u64> {
    let c = [h.sms[sm].l1, h.l2, h.l3][level];
    let resource = if level == 0 { sm } else { SMS + level - 1 };
    let tx = e.begin(
        c.packet,
        Some(parent),
        arrival,
        &format!("L{} {address:#x}", level + 1),
    )?;
    for (key, value) in [
        (e.n.address, Value::U64(address)),
        (e.n.bytes, Value::U64(64)),
        (e.n.operation, Value::Str(operation)),
        (e.n.direction, Value::Str(e.n.round_trip)),
        (e.n.route, Value::Str(e.n.local_route)),
        (e.n.source, Value::Str(c.endpoint)),
        (e.n.destination, Value::Str(c.endpoint)),
        (
            e.n.outcome,
            Value::Str(if depth == level { e.n.hit } else { e.n.miss }),
        ),
    ] {
        e.attr(tx, key, value)?;
    }
    let start = (arrival + 1).max(r.cache[resource]);
    e.stage(tx, e.n.ingress, arrival, arrival + 1)?;
    e.stage(tx, e.n.queued, arrival + 1, start)?;
    let lookup = start + [4, 12, 24][level];
    e.stage(tx, e.n.lookup, start, lookup)?;
    let mut done = lookup;
    if depth > level {
        let destination = if level == 2 {
            h.dram_endpoint
        } else {
            [h.l2, h.l3][level].endpoint
        };
        let arrived = hop(e, h, r, tx, lookup, (c.endpoint, destination), level, false)?;
        e.stage(tx, e.n.forwarding, lookup, arrived)?;
        let lower = if level == 2 {
            dram(e, h, r, tx, arrived)?
        } else {
            cache_access(
                e,
                h,
                r,
                tx,
                sm,
                level + 1,
                depth,
                arrived,
                address,
                e.n.read,
            )?
        };
        e.stage(tx, e.n.memory_wait, arrived, lower)?;
        done = hop(e, h, r, tx, lower, (destination, c.endpoint), level, true)?;
        e.stage(tx, e.n.returning, lower, done)?;
        let fill = e.service(
            c.fill,
            tx,
            (done, done + 4),
            e.n.fill,
            "install returned line",
        )?;
        e.relate(e.n.served_by, tx, fill)?;
        e.stage(tx, e.n.fill, done, done + 4)?;
        done += 4;
    } else {
        e.stage(tx, e.n.hit, done, done + 2)?;
        done += 2;
    }
    // Only the originating cache modifies data; lower requests fetch a line.
    // Atomics hold the blocking cache port through a read-modify-write step.
    if operation == e.n.atomic {
        e.stage(tx, e.n.execute, done, done + 8)?;
        done += 8;
    }
    // Commit modified bytes through a local write buffer. This is separate
    // resource work, not a second copy of the whole cache-service interval;
    // eviction/coherence traffic beyond the buffer is deliberately not modeled.
    if operation != e.n.read
        && let Some(generator) = c.writeback
    {
        e.service(
            generator,
            tx,
            (done, done + 3),
            e.n.writeback,
            "commit modified bytes",
        )?;
        e.stage(tx, e.n.writeback, done, done + 3)?;
        done += 3;
    }
    e.stage(tx, e.n.returning, done, done + 2)?;
    done += 2;
    e.end(tx, done)?;
    r.cache[resource] = done;
    Ok(done)
}

struct Request {
    generator: NodeId,
    parent: TxId,
    sm: usize,
    start: u64,
    address: u64,
    depth: usize,
    operation: StrId,
    dma: bool,
    page_walk: bool,
    active_mask: Option<u64>,
}
fn memory_request(e: &mut Emit, h: &Hardware, r: &mut Resources, req: Request) -> vtr::Result<u64> {
    let Request {
        generator,
        parent,
        sm,
        start,
        address,
        depth,
        operation,
        dma,
        page_walk,
        active_mask,
    } = req;
    let tx = e.begin(
        generator,
        Some(parent),
        start,
        &format!(
            "{} {address:#x}",
            if dma { "DMA read" } else { "lane sector" }
        ),
    )?;
    for (key, value) in [
        (e.n.address, Value::U64(address)),
        (e.n.bytes, Value::U64(64)),
        (e.n.space, Value::Str(e.n.global)),
        (e.n.operation, Value::Str(operation)),
    ] {
        e.attr(tx, key, value)?;
    }
    if let Some(mask) = active_mask {
        e.attr(tx, e.n.mask, Value::U64(mask))?;
    }
    let mut time = if dma { start } else { start.max(r.lsu[sm]) };
    e.stage(tx, e.n.queued, start, time)?;
    if !dma {
        let translation = e.begin(
            h.sms[sm].tlb,
            Some(tx),
            time,
            if page_walk { "TLB miss" } else { "TLB hit" },
        )?;
        e.stage(translation, e.n.translate, time, time + 2)?;
        let mut translated = time + 2;
        if page_walk {
            let walk = translated.max(r.walker);
            e.stage(translation, e.n.queued, translated, walk)?;
            e.service(
                h.walker,
                translation,
                (walk, walk + 32),
                e.n.walk,
                "cached page-table walk",
            )?;
            translated = walk + 32;
            e.stage(translation, e.n.walk, walk, translated)?;
            r.walker = translated;
        }
        e.end(translation, translated)?;
        e.stage(tx, e.n.translate, time, translated)?;
        time = translated;
    }
    let arrival = if dma {
        hop(
            e,
            h,
            r,
            tx,
            time,
            (h.sms[sm].dma_endpoint, h.l2.endpoint),
            0,
            false,
        )?
    } else {
        time
    };
    let mut done = cache_access(
        e,
        h,
        r,
        tx,
        sm,
        usize::from(dma),
        depth.max(usize::from(dma)),
        arrival,
        address,
        operation,
    )?;
    if dma {
        done = hop(
            e,
            h,
            r,
            tx,
            done,
            (h.l2.endpoint, h.sms[sm].dma_endpoint),
            0,
            true,
        )?;
    }
    e.stage(tx, e.n.memory_wait, time, done)?;
    e.end(tx, done)?;
    if !dma {
        r.lsu[sm] = done;
    }
    Ok(done)
}

#[derive(Clone, Copy)]
struct ResultAt {
    tx: TxId,
    ready: u64,
}
struct Warp {
    tx: TxId,
    fetch: u64,
    registers: [Option<ResultAt>; 4],
    matrix: Option<ResultAt>,
    dma: Option<ResultAt>,
    completion: u64,
}
impl Warp {
    fn drained(&self) -> u64 {
        self.registers
            .iter()
            .flatten()
            .map(|x| x.ready)
            .max()
            .unwrap_or(self.fetch)
            .max(self.completion)
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "keep the launch instruction, logical warp, and engine identity explicit"
)]
fn async_job(
    e: &mut Emit,
    h: &Hardware,
    r: &mut Resources,
    sm: usize,
    instruction: TxId,
    warp: TxId,
    launch: u64,
    dma: bool,
) -> vtr::Result<ResultAt> {
    let s = &h.sms[sm];
    let job = e.begin(
        if dma { s.dma[0] } else { s.matrix[0] },
        Some(instruction),
        launch,
        if dma {
            "prefetch next tile"
        } else {
            "16x16 matrix tile group"
        },
    )?;
    e.attr(
        job,
        e.n.shape,
        Value::List(vec![Value::U64(16), Value::U64(16), Value::U64(16)]),
    )?;
    e.relate(e.n.launches, instruction, job)?;
    e.relate(e.n.participates, warp, job)?;
    let start = launch.max(if dma { r.dma[sm] } else { r.matrix[sm] }) + 4;
    e.stage(job, e.n.queued, launch, start)?;
    let mut done = start;
    if dma {
        e.attr(job, e.n.direction, Value::Str(e.n.dma_direction))?;
        for chunk in 0..2 {
            let tx = e.begin(s.dma[1], Some(job), done, &format!("chunk {chunk}"))?;
            let end = memory_request(
                e,
                h,
                r,
                Request {
                    generator: s.dma[2],
                    parent: tx,
                    sm,
                    start: done,
                    address: 0x8000_0000 + chunk * 64,
                    depth: 3,
                    operation: e.n.read,
                    dma: true,
                    page_walk: false,
                    active_mask: None,
                },
            )?;
            e.stage(tx, e.n.memory_wait, done, end)?;
            // Deposit into the same finite bank ports used by LDS instructions.
            let bank = chunk as usize;
            let deposit = end.max(r.bank[sm][bank]);
            e.stage(tx, e.n.queued, end, deposit)?;
            e.service(
                s.banks[bank],
                tx,
                (deposit, deposit + 4),
                e.n.service,
                "DMA shared deposit",
            )?;
            e.stage(tx, e.n.writeback, deposit, deposit + 4)?;
            done = deposit + 4;
            r.bank[sm][bank] = done;
            e.end(tx, done)?;
        }
        r.dma[sm] = done;
    } else {
        for tile in 0..4 {
            e.service(
                s.matrix[1],
                job,
                (done, done + 40),
                e.n.execute,
                &format!("microtile {tile}"),
            )?;
            done += 40;
        }
        r.matrix[sm] = done;
    }
    e.stage(job, e.n.running, start, done)?;
    e.end(job, done)?;
    Ok(ResultAt {
        tx: job,
        ready: done,
    })
}

#[derive(Clone, Copy)]
enum Op {
    Fma,
    Add,
    Xor,
    Branch,
    Load,
    Store,
    Atomic,
    Shared,
    Matrix,
    Dma,
    WaitMatrix,
    WaitDma,
    Barrier,
}
impl Op {
    fn at(sequence: u64, warp: usize) -> Self {
        if sequence == 2499 {
            return Self::Barrier;
        }
        if warp == 0 {
            match sequence {
                50 => return Self::Matrix,
                52 => return Self::Dma,
                58 => return Self::WaitDma,
                60 => return Self::WaitMatrix,
                _ => {}
            }
        }
        match sequence % 100 {
            0 => Self::Load,
            12 => Self::Shared,
            24 => Self::Store,
            36 => Self::Atomic,
            99 => Self::Branch,
            n if n % 8 == 1 => Self::Add,
            n if n % 8 == 2 => Self::Xor,
            _ => Self::Fma,
        }
    }
    fn names(self, n: &Names) -> (StrId, StrId) {
        match self {
            Self::Fma => (n.fma, n.arithmetic),
            Self::Add => (n.add, n.arithmetic),
            Self::Xor => (n.xor, n.arithmetic),
            Self::Branch => (n.branch, n.control),
            Self::Load => (n.load, n.memory),
            Self::Store => (n.stg, n.memory),
            Self::Atomic => (n.atom, n.memory),
            Self::Shared => (n.lds, n.memory),
            Self::Matrix => (n.matrix, n.launch),
            Self::Dma => (n.dma, n.launch),
            Self::WaitMatrix | Self::WaitDma => (n.wait, n.synchronization),
            Self::Barrier => (n.barrier, n.synchronization),
        }
    }
}

/// Returns the issued instruction and issue cycle; completion is class-specific.
#[expect(
    clippy::too_many_arguments,
    reason = "script coordinates are distinct from mutable warp scheduling state"
)]
fn issue(
    e: &mut Emit,
    h: &Hardware,
    r: &mut Resources,
    warp: &mut Warp,
    sm: usize,
    slot: usize,
    sequence: u64,
    op: Op,
) -> vtr::Result<(TxId, u64)> {
    let (opcode, class) = op.names(&e.n);
    let tx = e.begin(
        h.sms[sm].slots[slot].instruction,
        Some(warp.tx),
        warp.fetch,
        &format!("{} #{sequence}", e.w.string(opcode)),
    )?;
    for (key, value) in [
        (e.n.pc, Value::U64(0x1000 + (sequence % 100) * 4)),
        (e.n.sequence, Value::U64(sequence)),
        (e.n.opcode, Value::Str(opcode)),
        (e.n.class, Value::Str(class)),
        (
            e.n.mask,
            Value::U64(if matches!(op, Op::Store | Op::Atomic) {
                0xffff
            } else {
                0xffff_ffff
            }),
        ),
    ] {
        e.attr(tx, key, value)?;
    }
    // Each instruction reads its destination accumulator. Half the arithmetic
    // loop also reads the previous result, producing visible dependency stalls.
    let register = sequence as usize % 4;
    let mut operands = vec![warp.registers[register]];
    if sequence % 8 >= 4 {
        operands.push(warp.registers[(register + 3) % 4]);
    }
    if matches!(op, Op::Branch | Op::Barrier) {
        operands = warp.registers.to_vec();
    }
    let mut ready = warp.fetch + 2;
    for producer in operands.into_iter().flatten() {
        ready = ready.max(producer.ready);
        e.relate(e.n.depends, tx, producer.tx)?;
    }
    let issued = ready.max(r.issue[sm][slot / SLOTS]);
    e.w.tx_stage_begin(tx, e.n.fetch, e.n.lane, warp.fetch)?;
    e.w.tx_stage_begin(tx, e.n.decode, e.n.lane, warp.fetch + 1)?;
    e.w.tx_stage_end(tx, e.n.decode, e.n.lane, warp.fetch + 2)?;
    e.stage(tx, e.n.ready, warp.fetch + 2, ready)?;
    e.stage(tx, e.n.scheduler, ready, issued)?;
    e.stage(tx, e.n.issue, issued, issued + 1)?;
    let resource = match op {
        Op::Load | Op::Store | Op::Atomic => h.sms[sm].request,
        Op::Shared => h.sms[sm].banks[0],
        Op::Matrix => h.sms[sm].matrix[0],
        Op::Dma => h.sms[sm].dma[0],
        Op::Barrier => h.sms[sm].barrier,
        Op::WaitMatrix | Op::WaitDma => h.sms[sm].slots[slot].instruction,
        _ => h.sms[sm].alu[slot / SLOTS],
    };
    e.w.tx_event(
        tx,
        issued,
        e.n.issued,
        &[(e.n.resource, Value::U64(resource.0 as u64))],
    )?;
    r.issue[sm][slot / SLOTS] = issued + 1;
    warp.fetch = issued;
    Ok((tx, issued))
}

#[expect(
    clippy::too_many_arguments,
    reason = "script coordinates are distinct from mutable warp scheduling state"
)]
fn instruction(
    e: &mut Emit,
    h: &Hardware,
    r: &mut Resources,
    warp: &mut Warp,
    sm: usize,
    slot: usize,
    sequence: u64,
    grid: usize,
) -> vtr::Result<()> {
    let op = Op::at(sequence, slot);
    let (tx, issued) = issue(e, h, r, warp, sm, slot, sequence, op)?;
    let start = issued + 1;
    let done = match op {
        Op::Fma | Op::Add | Op::Xor | Op::Branch => {
            let latency = if matches!(op, Op::Fma) { 6 } else { 2 };
            e.service(
                h.sms[sm].alu[slot / SLOTS],
                tx,
                (start, start + latency),
                e.n.execute,
                e.w.string(op.names(&e.n).0).to_owned().as_str(),
            )?;
            e.stage(tx, e.n.execute, start, start + latency)?;
            start + latency
        }
        Op::Load | Op::Store | Op::Atomic => {
            let operation = match op {
                Op::Load => e.n.read,
                Op::Store => e.n.store,
                _ => e.n.atomic,
            };
            e.attr(tx, e.n.space, Value::Str(e.n.global))?;
            e.attr(tx, e.n.operation, Value::Str(operation))?;
            let sectors = if matches!(op, Op::Load) { 2 } else { 1 };
            e.attr(tx, e.n.bytes, Value::U64(sectors * 64))?;
            let mut end = start;
            for sector in 0..sectors {
                // Distinct lines per logical origin/iteration; cache residency
                // for each line is a deliberate workload input, not simulated.
                let address = 0x1000_0000
                    + (grid * SMS * WARPS + sm * WARPS + slot) as u64 * 0x100_000
                    + sequence * 128
                    + sector * 64;
                e.w.tx_event(
                    tx,
                    start,
                    e.n.request_created,
                    &[
                        (e.n.address, Value::U64(address)),
                        (e.n.bytes, Value::U64(64)),
                    ],
                )?;
                end = end.max(memory_request(
                    e,
                    h,
                    r,
                    Request {
                        generator: h.sms[sm].request,
                        parent: tx,
                        sm,
                        start,
                        address,
                        depth: ((sequence / 100 + slot as u64 + sector) % 4) as usize,
                        operation,
                        dma: false,
                        page_walk: sequence == 0 && slot == 0 && sector == 0,
                        active_mask: Some(0xffff << (sector * 16)),
                    },
                )?);
            }
            e.stage(tx, e.n.memory_wait, start, end)?;
            end
        }
        Op::Shared => {
            e.attr(tx, e.n.space, Value::Str(e.n.shared))?;
            e.attr(tx, e.n.operation, Value::Str(e.n.read))?;
            // Two bank groups, two serialized rounds: an illustrative 2-way
            // conflict, not 32 independent VTR stage lanes.
            let mut end = start;
            for bank in 0..2 {
                for round in 0..2 {
                    let begin = start.max(r.bank[sm][bank]);
                    let access = e.begin(
                        h.sms[sm].banks[bank],
                        Some(tx),
                        begin,
                        "conflicted shared read",
                    )?;
                    e.attr(access, e.n.round, Value::U64(round))?;
                    e.attr(
                        access,
                        e.n.mask,
                        Value::U64(0xff << (bank * 16 + round as usize * 8)),
                    )?;
                    e.stage(access, e.n.service, begin, begin + 3)?;
                    e.end(access, begin + 3)?;
                    r.bank[sm][bank] = begin + 3;
                    end = end.max(begin + 3);
                }
            }
            e.stage(tx, e.n.memory_wait, start, end)?;
            end
        }
        Op::Matrix | Op::Dma => {
            let dma = matches!(op, Op::Dma);
            let job = async_job(e, h, r, sm, tx, warp.tx, start, dma)?;
            if dma {
                warp.dma = Some(job);
            } else {
                warp.matrix = Some(job);
            }
            e.w.tx_event(tx, start, e.n.launched, &[(e.n.job, Value::U64(job.tx))])?;
            e.stage(tx, e.n.execute, start, start + 2)?;
            start + 2
        }
        Op::WaitMatrix | Op::WaitDma => {
            let job = if matches!(op, Op::WaitDma) {
                warp.dma
            } else {
                warp.matrix
            }
            .expect("script launches before waiting");
            e.relate(e.n.waits_for, tx, job.tx)?;
            let end = (start + 1).max(job.ready);
            e.stage(tx, e.n.memory_wait, start, end)?;
            e.event(warp.tx, e.n.stalled, start)?;
            e.event(warp.tx, e.n.resumed, end)?;
            end
        }
        Op::Barrier => unreachable!("barriers are emitted collectively"),
    };
    e.stage(tx, e.n.writeback, done, done + 1)?;
    e.end(tx, done + 1)?;
    warp.registers[sequence as usize % 4] = Some(ResultAt {
        tx,
        ready: done + 1,
    });
    warp.completion = warp.completion.max(done + 1);
    // Branches and explicit waits stop the frontend, unlike scoreboarded loads.
    if matches!(op, Op::Branch | Op::WaitMatrix | Op::WaitDma) {
        warp.fetch = done + 1;
    }
    Ok(())
}

fn barrier(
    e: &mut Emit,
    h: &Hardware,
    r: &mut Resources,
    warps: &mut [Warp],
    sm: usize,
    workgroup: TxId,
    sequence: u64,
) -> vtr::Result<()> {
    let mut arrivals = Vec::new();
    for (slot, warp) in warps.iter_mut().enumerate() {
        arrivals.push(issue(e, h, r, warp, sm, slot, sequence, Op::Barrier)?);
    }
    let begin = arrivals.iter().map(|x| x.1 + 1).min().unwrap();
    let release = arrivals.iter().map(|x| x.1 + 1).max().unwrap() + 4;
    let barrier = e.begin(
        h.sms[sm].barrier,
        Some(workgroup),
        begin,
        "barrier generation 0",
    )?;
    e.stage(barrier, e.n.barrier_wait, begin, release - 4)?;
    e.stage(barrier, e.n.service, release - 4, release)?;
    for (warp, (tx, arrival)) in warps.iter_mut().zip(arrivals) {
        e.relate(e.n.arrives_at, tx, barrier)?;
        e.stage(tx, e.n.barrier_wait, arrival + 1, release)?;
        e.stage(tx, e.n.writeback, release, release + 1)?;
        e.w.tx_stage_begin(warp.tx, e.n.barrier_wait, e.n.activity, arrival + 1)?;
        e.w.tx_stage_begin(warp.tx, e.n.running, e.n.activity, release)?;
        e.end(tx, release + 1)?;
        warp.fetch = release + 1;
        warp.completion = release + 1;
        warp.registers = [Some(ResultAt {
            tx,
            ready: release + 1,
        }); 4];
    }
    e.end(barrier, release)
}

fn generate(path: &Path) -> vtr::Result<()> {
    let mut w = Writer::builder().timescale(Timescale::Nanoseconds)
        .writer_name("Volna synthetic GPU transaction example")
        .comment("Educational, not cycle accurate: 1 cycle = 1 ns. Illustrative L3 is not NVIDIA architecture; matrix/DMA placement and all timing are synthetic. See gpu_transaction_trace.rs.")
        .options(WriterOptions { background: false, ..WriterOptions::default() }).create(path)?;
    let h = hardware(&mut w)?;
    let n = Names::new(&mut w);
    let mut e = Emit { w, n };
    let mut r = Resources::default();
    let command = e.begin(h.queue[0], None, 0, "tiled compute submission")?;
    let mut dispatch = 8;
    for grid_index in 0..GRIDS {
        let grid = e.begin(
            h.queue[1],
            Some(command),
            dispatch,
            &format!("tiled_kernel grid {grid_index}"),
        )?;
        e.attr(grid, e.n.grid, Value::U64(grid_index as u64))?;
        e.attr(grid, e.n.kernel, Value::Text("tiled_arithmetic".into()))?;
        e.attr(
            grid,
            e.n.dimensions,
            Value::List(vec![Value::U64(SMS as u64), Value::U64(1), Value::U64(1)]),
        )?;
        let mut workgroups = Vec::new();
        let mut warps = Vec::new();
        for sm in 0..SMS {
            let wg = e.begin(
                h.sms[sm].workgroup,
                Some(grid),
                dispatch,
                &format!("block {sm}, grid {grid_index}"),
            )?;
            e.attr(
                wg,
                e.n.block,
                Value::List(vec![Value::U64(sm as u64), Value::U64(0), Value::U64(0)]),
            )?;
            e.attr(
                wg,
                e.n.threads,
                Value::List(vec![Value::U64(128), Value::U64(1), Value::U64(1)]),
            )?;
            e.attr(wg, e.n.epoch, Value::U64(grid_index as u64))?;
            let assigned = dispatch + 4 + sm as u64 * 4;
            let assignment = e.begin(h.assignment, Some(wg), assigned, "allocate block slot")?;
            e.attr(
                assignment,
                e.n.destination,
                Value::U64(h.sms[sm].workgroup.0 as u64),
            )?;
            e.stage(assignment, e.n.service, assigned, assigned + 4)?;
            e.end(assignment, assigned + 4)?;
            let resident = assigned + 4;
            e.stage(wg, e.n.queued, dispatch, resident)?;
            e.w.tx_stage_begin(wg, e.n.resident, e.n.residency, resident)?;
            e.event(wg, e.n.allocated, resident)?;
            workgroups.push(wg);
            let mut group = Vec::new();
            for slot in 0..WARPS {
                let logical = (grid_index * SMS + sm) * WARPS + slot;
                let tx = e.begin(
                    h.sms[sm].slots[slot].warp,
                    Some(wg),
                    dispatch,
                    &format!("warp {logical}"),
                )?;
                e.attr(tx, e.n.warp, Value::U64(logical as u64))?;
                e.attr(tx, e.n.epoch, Value::U64(grid_index as u64))?;
                e.event(tx, e.n.dispatched, dispatch)?;
                e.stage(tx, e.n.queued, dispatch, resident)?;
                e.w.tx_stage_begin(tx, e.n.resident, e.n.residency, resident)?;
                e.w.tx_stage_begin(tx, e.n.running, e.n.activity, resident)?;
                e.event(tx, e.n.allocated, resident)?;
                group.push(Warp {
                    tx,
                    fetch: resident + slot as u64,
                    registers: [None; 4],
                    matrix: None,
                    dma: None,
                    completion: resident,
                });
            }
            warps.push(group);
        }
        // Emission order is deliberately not timestamp order: independent SMs
        // and outstanding jobs advance separately, as permitted by the VTR API.
        for sequence in 0..INSTRUCTIONS {
            for sm in 0..SMS {
                if sequence == 2499 {
                    barrier(
                        &mut e,
                        &h,
                        &mut r,
                        &mut warps[sm],
                        sm,
                        workgroups[sm],
                        sequence,
                    )?;
                } else {
                    for (slot, warp) in warps[sm].iter_mut().enumerate() {
                        instruction(&mut e, &h, &mut r, warp, sm, slot, sequence, grid_index)?;
                    }
                }
            }
        }
        let mut end = dispatch;
        for (sm, group) in warps.iter().enumerate() {
            let group_end = group.iter().map(Warp::drained).max().unwrap() + 1;
            for warp in group {
                let done = warp.drained() + 1;
                e.w.tx_stage_end(warp.tx, e.n.resident, e.n.residency, done)?;
                e.w.tx_stage_end(warp.tx, e.n.running, e.n.activity, done)?;
                e.end(warp.tx, done)?;
            }
            e.w.tx_stage_end(workgroups[sm], e.n.resident, e.n.residency, group_end)?;
            e.end(workgroups[sm], group_end)?;
            end = end.max(group_end);
        }
        let signal = e.service(
            h.queue[3],
            grid,
            (end, end + 1),
            e.n.service,
            "grid completion fence",
        )?;
        e.end(grid, end + 1)?;
        let wait = e.begin(
            h.queue[2],
            Some(command),
            end.saturating_sub(4),
            "await grid fence",
        )?;
        e.relate(e.n.waits_for, wait, signal)?;
        e.stage(wait, e.n.memory_wait, end - 4, end + 1)?;
        e.end(wait, end + 1)?;
        dispatch = end + 64;
    }
    e.end(command, dispatch)?;
    // Explicit completion is independent of waveform time (never advanced here).
    assert_eq!(e.w.open_tx_count(), 0);
    e.w.close()
}

fn generator_name(rd: &Reader, tx: &Transaction) -> String {
    rd.str(rd.hierarchy().name(tx.generator)).to_owned()
}
fn label(rd: &Reader, tx: &Transaction) -> String {
    tx.attrs
        .iter()
        .find(|a| rd.str(a.key) == "vtr.label")
        .map(|a| match &a.value {
            Value::Text(s) => s.clone(),
            Value::Str(s) => rd.str(*s).into(),
            _ => String::new(),
        })
        .unwrap_or_default()
}

/// Deliberately small readback oracle: instruction counts are the only goldens.
/// Shape/timing changes should be reviewed, not frozen by this API example.
fn readback(path: &Path) -> anyhow::Result<()> {
    let rd = Reader::open_with(
        path,
        ReadOptions {
            verify_crc: true,
            ..ReadOptions::default()
        },
    )?;
    let mut counts = BTreeMap::<TxId, u64>::new();
    let mut warps = BTreeMap::new();
    let mut packet = None;
    rd.visit_transactions(&TxQuery::default(), |tx| {
        match rd.str(rd.hierarchy().name(tx.generator)) {
            "Instruction" => {
                *counts.entry(tx.parent.unwrap_or(0)).or_default() += 1;
            }
            "Warp" => {
                warps.insert(tx.id, tx.generator);
            }
            "MemoryPacket" if rd.full_path(tx.generator, ".").contains("illustrative_l3") => {
                packet.get_or_insert(tx.id);
            }
            _ => {}
        }
        true
    })?;
    ensure!(
        warps.len() == GRIDS * SMS * WARPS,
        "unexpected logical warp population"
    );
    for id in warps.keys() {
        ensure!(
            counts.get(id) == Some(&INSTRUCTIONS),
            "warp {id} instruction count differs from {INSTRUCTIONS}"
        );
    }
    let total: u64 = counts.values().sum();
    ensure!(
        total == GRIDS as u64 * SMS as u64 * WARPS as u64 * INSTRUCTIONS,
        "unexpected total instruction count"
    );

    // A stream query includes Warp and Instruction, while a generator query
    // selects just one type. Stop early: do not retain the entire trace.
    let (&warp_id, &warp_gen) = warps.first_key_value().context("no warp")?;
    let slot = rd
        .hierarchy()
        .parent(warp_gen)
        .context("warp generator has no stream")?;
    let mut instruction = None;
    rd.visit_transactions(
        &TxQuery {
            stream: Some(slot),
            ..TxQuery::default()
        },
        |tx| {
            if generator_name(&rd, tx) == "Instruction" {
                instruction = Some(tx.clone());
                false
            } else {
                true
            }
        },
    )?;
    let instruction = instruction.context("slot query found no instruction")?;
    ensure!(
        instruction.parent == Some(warp_id),
        "instruction must belong to its slot's warp"
    );
    for generator in [warp_gen, instruction.generator] {
        let mut found = false;
        rd.visit_transactions(
            &TxQuery {
                generator: Some(generator),
                ..TxQuery::default()
            },
            |_| {
                found = true;
                false
            },
        )?;
        ensure!(found, "generator query was empty");
    }
    println!(
        "{}: {} logical warps x {INSTRUCTIONS} = {total} instructions; {} transactions; {} bytes",
        path.display(),
        warps.len(),
        rd.tx_counts().0,
        std::fs::metadata(path)?.len()
    );
    println!("Illustrative L3 packet -> command-stream ancestry (actual TX parents):");
    let mut id = packet.context("no illustrative L3 packet")?;
    let mut names = Vec::new();
    for _ in 0..20 {
        let tx = rd
            .transaction(id)?
            .context("missing ancestry transaction")?;
        let name = generator_name(&rd, &tx);
        let stream = rd
            .hierarchy()
            .parent(tx.generator)
            .context("generator has no stream")?;
        println!(
            "  TX {} | {} | {} | {}",
            tx.id,
            name,
            rd.full_path(stream, "."),
            label(&rd, &tx)
        );
        names.push(name);
        match tx.parent {
            Some(parent) => id = parent,
            None => break,
        }
    }
    ensure!(
        names.ends_with(
            &["Instruction", "Warp", "Workgroup", "Grid", "CommandStream"].map(String::from)
        ),
        "packet lost its logical ancestry"
    );
    Ok(())
}

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args_os().skip(1);
    let path = args.next().map(PathBuf::from).unwrap_or_else(|| {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/gpu_transaction_trace.vtr")
    });
    ensure!(
        args.next().is_none(),
        "usage: gpu_transaction_trace [OUTPUT.vtr]"
    );
    generate(&path)?;
    readback(&path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_counts_and_ancestry() -> anyhow::Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("gpu.vtr");
        generate(&path)?;
        readback(&path)
    }
}
