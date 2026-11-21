//! Hardware generators and cross-generator workload provenance for a synthetic SIMT GPU.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, ensure};
use vtr::{
    Attribute, ClockSource, ExecutionMode, NodeData, NodeId, ReadOptions, Reader, Record, Recorder,
    ScopeType, Source, SourceOptions, SpanKind, Status, Transaction, TransactionQuery, Value,
    Writer, WriterOptions,
};

const GRIDS: usize = 2;
const SMS: usize = 2;
const PARTITIONS: usize = 2;
const SLOTS: usize = 8;
const WARPS: usize = PARTITIONS * SLOTS;
const THREADS_PER_WARP: u64 = 32;
const REGISTERS: usize = 8;
const TILE_LEN: u64 = 600;
const TILES_PER_GRID: u64 = 2;
const INSTRUCTIONS: u64 = TILE_LEN * TILES_PER_GRID;
const BARRIER_OFFSET: u64 = 519;
const FULL_EXEC_MASK: u32 = u32::MAX;
const CLOCK_PATH: &str = "gpu.clock";

struct InstanceLabel(String);
impl vtr::Payload for InstanceLabel {
    fn bind(types: &mut vtr::Types) -> vtr::Result<vtr::TypeId> {
        let text = <String as vtr::Payload>::bind(types)?;
        types.structure(
            "InstanceLabel",
            vec![vtr::schema::Field::new("vtr.label", text)],
        )
    }
    fn encode(&self, out: &mut Vec<u8>) -> vtr::Result<()> {
        self.0.encode(out)
    }
    fn decode(reader: &mut vtr::PayloadReader<'_>) -> vtr::Result<Self> {
        Ok(Self(String::decode(reader)?))
    }
}

macro_rules! vocabulary {
    ($($field:ident: $text:literal),* $(,)?) => {
        struct Names { $( $field: u32, )* }
        impl Names {
            fn new(w: &mut Writer) -> vtr::Result<Self> {
                Ok(Self { $( $field: w.intern($text)?, )* })
            }
            fn text(&self, id: u32) -> &'static str {
                $( if self.$field == id { return $text; } )*
                unreachable!("identifier is in the static vocabulary")
            }
        }
    };
}
vocabulary! {
    grid: "gpu.grid", kernel: "gpu.kernel",
    dimensions: "gpu.grid_dimensions", block: "gpu.block_coordinates",
    threads: "gpu.block_dimensions", warp: "gpu.logical_warp", epoch: "gpu.slot_epoch",
    instruction: "gpu.instruction", pc: "gpu.pc", sequence: "gpu.sequence", opcode: "gpu.opcode", class: "gpu.class",
    mask: "gpu.active_mask", exec_mask: "exec_mask", space: "gpu.address_space", operation: "gpu.operation",
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
    warp: Source<InstanceLabel>,
    instruction: Source<InstanceLabel>,
}
#[derive(Clone, Copy)]
struct Cache {
    packet: Source<InstanceLabel>,
    fill: Source<InstanceLabel>,
    writeback: Option<Source<InstanceLabel>>,
    endpoint: u32,
}
struct Sm {
    workgroup: Source<InstanceLabel>,
    slots: Vec<Slot>,
    alu: Vec<Source<InstanceLabel>>,
    request: Source<InstanceLabel>,
    tlb: Source<InstanceLabel>,
    l1: Cache,
    banks: Vec<Source<InstanceLabel>>,
    matrix: [Source<InstanceLabel>; 2],
    dma: [Source<InstanceLabel>; 3],
    dma_endpoint: u32,
    barrier: Source<InstanceLabel>,
}
struct Hardware {
    clock: ClockSource,
    queue: [Source<InstanceLabel>; 4],
    assignment: Source<InstanceLabel>,
    sms: Vec<Sm>,
    fabric: Source<InstanceLabel>,
    walker: Source<InstanceLabel>,
    l2: Cache,
    l3: Cache,
    controller: Source<InstanceLabel>,
    dram: Source<InstanceLabel>,
    dram_endpoint: u32,
}
fn scope(w: &mut Writer, parent: NodeId, name: &str) -> vtr::Result<NodeId> {
    w.scope_with(
        SourceOptions::new(name, Some(parent)),
        ScopeType::Generic,
        "",
    )
}
fn group<const N: usize>(
    w: &mut Writer,
    clock: ClockSource,
    parent: NodeId,
    name: &str,
    generators: [&str; N],
) -> vtr::Result<[Source<InstanceLabel>; N]> {
    let s = scope(w, parent, name)?;
    let mut sources = Vec::with_capacity(N);
    for name in generators {
        sources.push(generator(w, clock, s, name)?);
    }
    Ok(sources.try_into().unwrap_or_else(|_| unreachable!()))
}
fn generator(
    w: &mut Writer,
    clock: ClockSource,
    parent: NodeId,
    name: &str,
) -> vtr::Result<Source<InstanceLabel>> {
    let source =
        w.transactions::<InstanceLabel>(SourceOptions::new(name, Some(parent)), "TRANSACTION")?;
    w.associate_clock(source, clock)?;
    Ok(source)
}
fn cache(
    w: &mut Writer,
    clock: ClockSource,
    parent: NodeId,
    name: &str,
    endpoint: &str,
    last: bool,
) -> vtr::Result<Cache> {
    let c = scope(w, parent, name)?;
    let requests = scope(w, c, "requests")?;
    let packet = generator(w, clock, requests, "MemoryPacket")?;
    let fill = generator(w, clock, requests, "CacheFill")?;
    let writeback = if last {
        None
    } else {
        Some(generator(w, clock, requests, "Writeback")?)
    };
    Ok(Cache {
        packet,
        fill,
        writeback,
        endpoint: w.intern(endpoint)?,
    })
}
fn hardware(w: &mut Writer) -> vtr::Result<Hardware> {
    let gpu = w.scope_with(
        SourceOptions::new("gpu", None),
        ScopeType::Generic,
        "synthetic_simt",
    )?;
    let clock = w.clock(SourceOptions::new("clock", Some(gpu)))?;
    w.clock_run(clock, 0, 1)?;
    let cp = scope(w, gpu, "command_processor")?;
    let queue = group(
        w,
        clock,
        cp,
        "queue[0]",
        ["CommandStream", "Grid", "Wait", "Signal"],
    )?;
    let [assignment] = group(w, clock, cp, "work_distributor", ["WorkgroupAssignment"])?;
    let cluster = scope(w, gpu, "compute_cluster[0]")?;
    let mut sms = Vec::new();
    for sm in 0..SMS {
        let s = scope(w, cluster, &format!("sm[{sm}]"))?;
        let [workgroup] = group(w, clock, s, "workgroup_slot[0]", ["Workgroup"])?;
        let mut slots = Vec::new();
        let mut alu = Vec::new();
        for p in 0..PARTITIONS {
            let partition = scope(w, s, &format!("simt_partition[{p}]"))?;
            for slot in 0..SLOTS {
                let [warp, instruction] = group(
                    w,
                    clock,
                    partition,
                    &format!("warp_slot[{slot}]"),
                    ["Warp", "Instruction"],
                )?;
                slots.push(Slot { warp, instruction });
            }
            alu.push(
                group(
                    w,
                    clock,
                    partition,
                    "alu_pipeline[0]",
                    ["InstructionExecution"],
                )?[0],
            );
        }
        let lsu = scope(w, s, "load_store_unit")?;
        let [request] = group(w, clock, lsu, "request_port[0]", ["MemoryRequest"])?;
        let [tlb] = group(w, clock, lsu, "tlb", ["Translation"])?;
        let l1 = cache(
            w,
            clock,
            s,
            "l1_cache",
            &format!("gpu.compute_cluster[0].sm[{sm}].l1_cache"),
            false,
        )?;
        let shared = scope(w, s, "shared_memory")?;
        let banks = (0..2)
            .map(|b| group(w, clock, shared, &format!("bank[{b}]"), ["BankAccess"]).map(|x| x[0]))
            .collect::<vtr::Result<Vec<_>>>()?;
        let matrix = group(w, clock, s, "matrix_engine[0]", ["MatrixJob", "MatrixTile"])?;
        let dma = group(
            w,
            clock,
            s,
            "tensor_dma_engine[0]",
            ["TensorDmaJob", "TransferChunk", "MemoryRequest"],
        )?;
        let dma_endpoint = w.intern(&format!(
            "gpu.compute_cluster[0].sm[{sm}].tensor_dma_engine[0]"
        ))?;
        let [barrier] = group(w, clock, s, "barrier_unit", ["WorkgroupBarrier"])?;
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
    let [fabric] = group(w, clock, fabric_scope, "port[0]", ["MemoryPacket"])?;
    let memory = scope(w, gpu, "memory_subsystem")?;
    let [walker] = group(w, clock, memory, "page_walker[0]", ["PageWalk"])?;
    let l2 = cache(
        w,
        clock,
        memory,
        "l2_partition[0]",
        "gpu.memory_subsystem.l2_partition[0]",
        false,
    )?;
    let l3 = cache(
        w,
        clock,
        memory,
        "illustrative_l3",
        "gpu.memory_subsystem.illustrative_l3",
        true,
    )?;
    let mc = scope(w, memory, "memory_controller[0]")?;
    let [controller] = group(w, clock, mc, "requests", ["MemoryTransaction"])?;
    let [dram] = group(w, clock, mc, "dram_channel[0]", ["DramCommand"])?;
    let dram_endpoint = w.intern("gpu.memory_subsystem.memory_controller[0]")?;
    Ok(Hardware {
        clock,
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
    open_stages: BTreeMap<(u64, u32), u64>,
    instructions: BTreeMap<(u64, u32), vtr::ValueId>,
}
impl Emit {
    fn begin(
        &mut self,
        generator: Source<InstanceLabel>,
        parent: Option<Transaction<InstanceLabel>>,
        time: u64,
        label: &str,
    ) -> vtr::Result<Transaction<InstanceLabel>> {
        let tx = self.w.begin_at(
            generator,
            time,
            &InstanceLabel(label.into()),
            parent.map(Transaction::owner),
        )?;
        self.w.span_kind(tx.owner(), SpanKind::Internal)?;
        Ok(tx)
    }
    fn attr(&mut self, tx: Transaction<InstanceLabel>, key: u32, value: Value) -> vtr::Result<()> {
        self.w.attribute(tx.owner(), key, value)
    }
    fn stage(
        &mut self,
        tx: Transaction<InstanceLabel>,
        name: u32,
        begin: u64,
        end: u64,
    ) -> vtr::Result<()> {
        if end > begin {
            self.w
                .embedded_stage(tx.owner(), begin, name, self.n.lane, Some(end), vec![])?;
        }
        Ok(())
    }
    fn enter(
        &mut self,
        tx: Transaction<InstanceLabel>,
        name: u32,
        lane: u32,
        time: u64,
    ) -> vtr::Result<()> {
        if self.open_stages.contains_key(&(tx.id(), lane)) {
            self.leave(tx, lane, time)?;
        }
        let stage = self
            .w
            .embedded_stage(tx.owner(), time, name, lane, None, vec![])?;
        self.open_stages.insert((tx.id(), lane), stage);
        Ok(())
    }
    fn leave(&mut self, tx: Transaction<InstanceLabel>, lane: u32, time: u64) -> vtr::Result<()> {
        let stage = self
            .open_stages
            .remove(&(tx.id(), lane))
            .ok_or(vtr::Error::State("scripted lane has no active stage"))?;
        self.w.end_embedded_stage(tx.owner(), stage, time)
    }
    fn event(&mut self, tx: Transaction<InstanceLabel>, name: u32, time: u64) -> vtr::Result<()> {
        self.w
            .embedded_event(tx.owner(), time, name, vec![])
            .map(|_| ())
    }
    fn end(&mut self, tx: Transaction<InstanceLabel>, time: u64) -> vtr::Result<()> {
        self.event(tx, self.n.completed, time)?;
        self.w.end_at(tx, time, Status::Ok)
    }
    fn relate(
        &mut self,
        kind: u32,
        from: Transaction<InstanceLabel>,
        to: Transaction<InstanceLabel>,
    ) -> vtr::Result<()> {
        self.w.relate(kind, from.id(), to.id(), vec![])
    }
    fn service(
        &mut self,
        generator: Source<InstanceLabel>,
        parent: Transaction<InstanceLabel>,
        interval: (u64, u64),
        stage: u32,
        label: &str,
    ) -> vtr::Result<Transaction<InstanceLabel>> {
        let tx = self.begin(generator, Some(parent), interval.0, label)?;
        self.stage(tx, stage, interval.0, interval.1)?;
        self.end(tx, interval.1)?;
        Ok(tx)
    }
}

/// Non-overlapping busy-interval allocator for shared pipelined hardware units.
/// Because an instruction emits its full downstream memory/fabric subtree before
/// later instructions issue, future return hops can reserve slots ahead of `now`;
/// tracking disjoint intervals prevents later instructions from stalling behind
/// unrelated future return traffic.
#[derive(Default)]
struct ResourcePool {
    busy: Vec<(u64, u64)>,
}
impl ResourcePool {
    fn reserve(&mut self, earliest: u64, duration: u64, prune_before: u64) -> u64 {
        debug_assert!(duration > 0);
        if self.busy.len() > 64 {
            let keep = self.busy.partition_point(|&(_, end)| end <= prune_before);
            if keep > 0 {
                self.busy.drain(..keep);
            }
        }
        let mut idx = self.busy.partition_point(|&(_, end)| end <= earliest);
        let mut start = earliest;
        while idx < self.busy.len() && self.busy[idx].0 < start + duration {
            start = start.max(self.busy[idx].1);
            idx += 1;
        }
        let end = start + duration;
        let merge_prev = idx > 0 && self.busy[idx - 1].1 == start;
        let merge_next = idx < self.busy.len() && end == self.busy[idx].0;
        match (merge_prev, merge_next) {
            (true, true) => {
                self.busy[idx - 1].1 = self.busy[idx].1;
                self.busy.remove(idx);
            }
            (true, false) => self.busy[idx - 1].1 = end,
            (false, true) => self.busy[idx].0 = start,
            (false, false) => self.busy.insert(idx, (start, end)),
        }
        start
    }
}

#[derive(Default)]
struct Resources {
    now: u64,
    issue: [[u64; PARTITIONS]; SMS],
    alu: [[u64; PARTITIONS]; SMS],
    rr_cursor: [[usize; PARTITIONS]; SMS],
    lsu: [ResourcePool; SMS],
    bank: [[ResourcePool; 2]; SMS],
    matrix: [ResourcePool; SMS],
    dma: [u64; SMS],
    cache: [ResourcePool; SMS + 2],
    link: ResourcePool,
    dram: ResourcePool,
    walker: ResourcePool,
}

#[expect(
    clippy::too_many_arguments,
    reason = "explicit route and time at each scripted hop"
)]
fn hop(
    e: &mut Emit,
    h: &Hardware,
    r: &mut Resources,
    parent: Transaction<InstanceLabel>,
    start: u64,
    endpoints: (u32, u32),
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
    let link = r.link.reserve(start + 4, 4, r.now);
    e.stage(tx, e.n.credit, start + 4, link)?;
    e.stage(tx, e.n.link, link, link + 4)?;
    e.stage(tx, e.n.acceptance, link + 4, link + 5)?;
    e.end(tx, link + 5)?;
    Ok(link + 5)
}

fn dram(
    e: &mut Emit,
    h: &Hardware,
    r: &mut Resources,
    parent: Transaction<InstanceLabel>,
    arrival: u64,
) -> vtr::Result<u64> {
    let tx = e.begin(h.controller, Some(parent), arrival, "cold-line read")?;
    let start = r.dram.reserve(arrival, 22, r.now);
    e.stage(tx, e.n.queued, arrival, start)?;
    let mut time = start;
    for (name, duration, label) in [
        (e.n.activate, 5, "ACT"),
        (e.n.read, 12, "RD + burst"),
        (e.n.precharge, 5, "PRE"),
    ] {
        e.service(h.dram, tx, (time, time + duration), name, label)?;
        time += duration;
    }
    e.stage(tx, e.n.service, start, time)?;
    e.end(tx, time)?;
    Ok(time)
}

#[expect(
    clippy::too_many_arguments,
    reason = "explicit origin, route, and service parameters in the recursive cache path"
)]
fn cache_access(
    e: &mut Emit,
    h: &Hardware,
    r: &mut Resources,
    parent: Transaction<InstanceLabel>,
    sm: usize,
    level: usize,
    depth: usize,
    arrival: u64,
    address: u64,
    operation: u32,
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
    let start = r.cache[resource].reserve(arrival + 1, [2, 4, 6][level], r.now);
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
    if operation == e.n.atomic {
        e.stage(tx, e.n.execute, done, done + 8)?;
        done += 8;
    }
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
    Ok(done)
}

struct Request {
    generator: Source<InstanceLabel>,
    parent: Transaction<InstanceLabel>,
    sm: usize,
    start: u64,
    address: u64,
    depth: usize,
    operation: u32,
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
    let mut time = if dma {
        start
    } else {
        r.lsu[sm].reserve(start, 2, r.now)
    };
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
            let walk = r.walker.reserve(translated, 32, r.now);
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
    Ok(done)
}

#[derive(Clone, Copy)]
struct ResultAt {
    tx: Transaction<InstanceLabel>,
    ready: u64,
}
struct PendingBarrier {
    tx: Transaction<InstanceLabel>,
    arrival: u64,
}
struct Warp {
    tx: Transaction<InstanceLabel>,
    sequence: u64,
    fetch: u64,
    registers: [Option<ResultAt>; REGISTERS],
    matrix: Option<ResultAt>,
    dma: Option<ResultAt>,
    waiting_barrier: Option<PendingBarrier>,
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
    slot: usize,
    instruction: Transaction<InstanceLabel>,
    warp: Transaction<InstanceLabel>,
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
    let start = if dma {
        launch.max(r.dma[sm]) + 4
    } else {
        r.matrix[sm].reserve(launch + 4, 80, r.now)
    };
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
                    address: 0x8000_0000 + (sm * WARPS + slot) as u64 * 0x1000 + chunk * 64,
                    depth: if chunk == 0 { 2 } else { 1 },
                    operation: e.n.read,
                    dma: true,
                    page_walk: false,
                    active_mask: None,
                },
            )?;
            e.stage(tx, e.n.memory_wait, done, end)?;
            let bank = (slot + chunk as usize) % 2;
            let deposit = r.bank[sm][bank].reserve(end, 4, r.now);
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
            e.end(tx, done)?;
        }
        r.dma[sm] = done;
    } else {
        for tile in 0..4 {
            e.service(
                s.matrix[1],
                job,
                (done, done + 20),
                e.n.execute,
                &format!("microtile {tile}"),
            )?;
            done += 20;
        }
    }
    e.stage(job, e.n.running, start, done)?;
    e.end(job, done)?;
    Ok(ResultAt {
        tx: job,
        ready: done,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    fn is_alu(self) -> bool {
        matches!(self, Self::Fma | Self::Add | Self::Xor | Self::Branch)
    }
    fn alu_ii(self) -> u64 {
        if matches!(self, Self::Fma) { 2 } else { 1 }
    }
    fn names(self, n: &Names) -> (u32, u32) {
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

#[derive(Clone, Copy)]
struct Step {
    op: Op,
    dst: Option<usize>,
    srcs: [Option<usize>; 2],
    cache_depth: usize,
    sectors: u64,
    page_walk: bool,
    exec_mask: u32,
}

impl Step {
    fn alu(op: Op, dst: usize, src0: usize, src1: Option<usize>) -> Self {
        Self {
            op,
            dst: Some(dst % REGISTERS),
            srcs: [Some(src0 % REGISTERS), src1.map(|r| r % REGISTERS)],
            cache_depth: 0,
            sectors: 0,
            page_walk: false,
            exec_mask: FULL_EXEC_MASK,
        }
    }
    fn load(dst: usize, base: usize, depth: usize, sectors: u64, page_walk: bool) -> Self {
        Self {
            op: Op::Load,
            dst: Some(dst % REGISTERS),
            srcs: [Some(base % REGISTERS), None],
            cache_depth: depth,
            sectors,
            page_walk,
            exec_mask: FULL_EXEC_MASK,
        }
    }
    fn store(base: usize, val: usize, depth: usize, mask: u32) -> Self {
        Self {
            op: Op::Store,
            dst: None,
            srcs: [Some(base % REGISTERS), Some(val % REGISTERS)],
            cache_depth: depth,
            sectors: 1,
            page_walk: false,
            exec_mask: mask,
        }
    }
    fn with_mask(mut self, mask: u32) -> Self {
        self.exec_mask = mask;
        self
    }
    fn label(self, pc: u64, sequence: u64) -> String {
        let dst = self.dst.unwrap_or(0);
        let s0 = self.srcs[0].unwrap_or(0);
        let s1 = self.srcs[1].unwrap_or((s0 + 1) % REGISTERS);
        let off = ((sequence % 16) * 64) & 0x3ff;
        match self.op {
            Op::Fma => format!("{pc:#06x} FFMA r{dst}, r{s0}, r{s1}"),
            Op::Add => format!("{pc:#06x} IADD r{dst}, r{s0}, #{}", (sequence % 8 + 1) * 16),
            Op::Xor => format!("{pc:#06x} XOR r{dst}, r{s0}, r{s1}"),
            Op::Branch => {
                let target = if sequence % TILE_LEN == 328 {
                    pc + 17 * 4
                } else {
                    pc + 4
                };
                format!("{pc:#06x} BRA {target:#06x}")
            }
            Op::Load => format!("{pc:#06x} LDG r{dst}, [r{s0}+0x{off:x}]"),
            Op::Store => format!("{pc:#06x} STG [r{s0}+0x{off:x}], r{s1}"),
            Op::Atomic => format!("{pc:#06x} ATOM.ADD r{dst}, [r{s0}], r{s1}"),
            Op::Shared => format!("{pc:#06x} LDS r{dst}, [s0+0x{off:x}]"),
            Op::Matrix => format!("{pc:#06x} MMA.ASYNC m0, r{s0}, r{s1}"),
            Op::Dma => format!("{pc:#06x} TMA.LOAD [s0], [r{s0}]"),
            Op::WaitMatrix => format!("{pc:#06x} WAIT mma"),
            Op::WaitDma => format!("{pc:#06x} WAIT tma"),
            Op::Barrier => format!("{pc:#06x} BAR.SYNC #{}", sequence / TILE_LEN),
        }
    }
}

/// Plans each instruction across structured 600-instruction tiles so a merged
/// partition view exhibits distinct architectural phases:
///   0..28    Cold-tile memory miss storm (L3/DRAM + TLB walk)
///   28..220  Steady-state latency hiding (staggered L1/L2 loads + independent ALU)
///   220..300 Intra-warp RAW dependency chains (visible when soloing a warp)
///   300..380 Shared-memory bank conflicts (LDS) & SIMT divergence/reconvergence
///   380..470 Multi-warp async MMA.ASYNC & TMA.LOAD + WAIT overlap
///   470..520 Tail-latency skew leading into workgroup BAR.SYNC at 519
///   520..600 Post-barrier shared issue & VALU contention burst + epilogue
fn plan_step(sequence: u64, slot: usize, grid: usize) -> Step {
    let tile = sequence / TILE_LEN;
    let offset = sequence % TILE_LEN;
    let local = slot % SLOTS;

    if offset == BARRIER_OFFSET {
        return Step {
            op: Op::Barrier,
            dst: None,
            srcs: [None, None],
            cache_depth: 0,
            sectors: 0,
            page_walk: false,
            exec_mask: FULL_EXEC_MASK,
        };
    }

    match offset {
        // Phase 1: Prologue + Cold-tile miss storm across all 8 partition slots
        0..=11 => {
            let r = offset as usize % 4;
            match offset % 3 {
                0 => Step::alu(Op::Add, r, r, None),
                1 => Step::alu(Op::Xor, r + 4, r, Some((r + 1) % 4)),
                _ => Step::alu(Op::Fma, (r + 2) % 4, r, Some(r + 4)),
            }
        }
        12 => {
            let walk = local == ((tile as usize * 4 + grid) % SLOTS);
            Step::load(4, 0, 3, 1, walk)
        }
        13 => Step::alu(Op::Fma, 5, 4, Some(1)),
        14 => Step::alu(Op::Add, 2, 2, None),
        15 => Step::alu(Op::Fma, 6, 5, Some(2)),
        16 => {
            let depth = if (local + tile as usize).is_multiple_of(2) {
                2
            } else {
                1
            };
            Step::load(7, 0, depth, 2, false)
        }
        17 => Step::alu(Op::Fma, 3, 7, Some(6)),
        18..=27 => {
            let r = (offset as usize) % REGISTERS;
            match offset % 4 {
                0 => Step::alu(Op::Add, r, (r + 5) % REGISTERS, None),
                1 => Step::alu(Op::Xor, r, (r + 3) % REGISTERS, Some((r + 6) % REGISTERS)),
                _ => Step::alu(Op::Fma, r, (r + 4) % REGISTERS, Some((r + 7) % REGISTERS)),
            }
        }

        // Phase 2: Steady-state latency hiding (L1/L2 hits interleaved with independent ALU)
        28..=219 => {
            let phase = (offset - 28 + local as u64 * 2) % 16;
            let base = ((offset - 28) / 16) as usize % 4;
            let load_reg = base * 2;
            match phase {
                0 => {
                    let depth = usize::from((offset / 16 + local as u64).is_multiple_of(3));
                    Step::load(load_reg, 0, depth, 2, false)
                }
                1 => Step::alu(
                    Op::Fma,
                    (load_reg + 1) % 8,
                    (load_reg + 3) % 8,
                    Some((load_reg + 5) % 8),
                ),
                2 | 8 => Step::alu(Op::Add, (load_reg + 2) % 8, (load_reg + 4) % 8, None),
                3 | 10 => Step::alu(
                    Op::Fma,
                    (load_reg + 3) % 8,
                    (load_reg + 5) % 8,
                    Some((load_reg + 7) % 8),
                ),
                4 | 12 => Step::alu(
                    Op::Xor,
                    (load_reg + 4) % 8,
                    (load_reg + 6) % 8,
                    Some((load_reg + 1) % 8),
                ),
                5 | 13 => Step::alu(
                    Op::Fma,
                    (load_reg + 5) % 8,
                    (load_reg + 7) % 8,
                    Some((load_reg + 2) % 8),
                ),
                6 => Step::alu(
                    Op::Fma,
                    (load_reg + 6) % 8,
                    load_reg,
                    Some((load_reg + 1) % 8),
                ),
                7 => Step::alu(Op::Add, (load_reg + 7) % 8, (load_reg + 2) % 8, None),
                9 => Step::alu(
                    Op::Fma,
                    (load_reg + 1) % 8,
                    (load_reg + 6) % 8,
                    Some((load_reg + 3) % 8),
                ),
                11 if (offset / 16 + local as u64).is_multiple_of(2) => {
                    Step::store(0, (load_reg + 1) % 8, 0, 0x0000_ffff)
                }
                11 => Step::alu(
                    Op::Xor,
                    (load_reg + 2) % 8,
                    (load_reg + 4) % 8,
                    Some((load_reg + 7) % 8),
                ),
                14 => Step::alu(
                    Op::Fma,
                    (load_reg + 3) % 8,
                    (load_reg + 5) % 8,
                    Some((load_reg + 1) % 8),
                ),
                _ => Step::alu(
                    Op::Fma,
                    (load_reg + 4) % 8,
                    (load_reg + 6) % 8,
                    Some((load_reg + 2) % 8),
                ),
            }
        }

        // Phase 3: Intra-warp RAW dependency chains (tight register-carried chains)
        220..=299 => {
            let chain = (offset - 220) / 10;
            match (offset - 220) % 10 {
                0 => {
                    let depth = ((chain + local as u64) % 2) as usize;
                    Step::load(0, 7, depth, 2, false)
                }
                1 => Step::alu(Op::Fma, 1, 0, Some(6)),
                2 => Step::alu(Op::Fma, 2, 1, Some(6)),
                3 => Step::alu(Op::Add, 3, 2, None),
                4 => Step::alu(Op::Fma, 4, 3, Some(1)),
                5 => Step::alu(Op::Xor, 5, 4, Some(2)),
                6 => Step::alu(Op::Fma, 6, 5, Some(3)),
                7 => Step::alu(Op::Fma, 1, 6, Some(4)),
                8 => Step::alu(Op::Add, 6, 1, None),
                _ => Step::store(7, 6, 0, 0x0000_ffff),
            }
        }

        // Phase 4: Shared-memory bank conflicts (LDS) & SIMT branch divergence/reconvergence
        300..=327 => {
            let r = ((offset - 300) / 8) as usize % REGISTERS;
            match (offset - 300) % 8 {
                0 => Step {
                    op: Op::Shared,
                    dst: Some(r),
                    srcs: [Some((r + 5) % REGISTERS), None],
                    cache_depth: 0,
                    sectors: 0,
                    page_walk: false,
                    exec_mask: FULL_EXEC_MASK,
                },
                1 => Step::alu(Op::Fma, (r + 1) % 8, r, Some((r + 3) % 8)),
                2 | 6 => Step::alu(Op::Add, (r + 2) % 8, (r + 4) % 8, None),
                4 => Step::alu(Op::Xor, (r + 3) % 8, (r + 1) % 8, Some((r + 5) % 8)),
                _ => Step::alu(Op::Fma, (r + 4) % 8, (r + 2) % 8, Some((r + 6) % 8)),
            }
        }
        328 => Step {
            op: Op::Branch,
            dst: None,
            srcs: [Some(0), Some(1)],
            cache_depth: 0,
            sectors: 0,
            page_walk: false,
            exec_mask: FULL_EXEC_MASK,
        },
        329..=360 => {
            let mask = if offset <= 344 {
                0x0000_ffff
            } else {
                0xffff_0000
            };
            let rel = offset - 329;
            let r = rel as usize % REGISTERS;
            match rel % 16 {
                3 => Step {
                    op: Op::Shared,
                    dst: Some(r),
                    srcs: [Some((r + 6) % 8), None],
                    cache_depth: 0,
                    sectors: 0,
                    page_walk: false,
                    exec_mask: mask,
                },
                11 if offset <= 344 => Step {
                    op: Op::Atomic,
                    dst: Some(r),
                    srcs: [Some(0), Some((r + 7) % 8)],
                    cache_depth: 1,
                    sectors: 1,
                    page_walk: false,
                    exec_mask: mask,
                },
                11 => Step::store(0, (r + 7) % 8, 1, mask),
                1 | 5 | 9 | 13 => Step::alu(Op::Add, r, (r + 4) % 8, None).with_mask(mask),
                2 | 10 => Step::alu(Op::Xor, r, (r + 3) % 8, Some((r + 5) % 8)).with_mask(mask),
                _ => Step::alu(Op::Fma, r, (r + 6) % 8, Some((r + 7) % 8)).with_mask(mask),
            }
        }
        361 => Step {
            op: Op::Branch,
            dst: None,
            srcs: [Some(2), Some(3)],
            cache_depth: 0,
            sectors: 0,
            page_walk: false,
            exec_mask: FULL_EXEC_MASK,
        },
        362..=379 => {
            let r = offset as usize % REGISTERS;
            match offset % 9 {
                8 => Step::store(0, r, 0, 0x0000_ffff),
                1 | 4 => Step::alu(Op::Add, r, (r + 5) % 8, None),
                2 | 6 => Step::alu(Op::Xor, r, (r + 4) % 8, Some((r + 6) % 8)),
                _ => Step::alu(Op::Fma, r, (r + 3) % 8, Some((r + 7) % 8)),
            }
        }

        // Phase 5: Multi-warp async MMA.ASYNC & TMA.LOAD + WAIT overlap
        // Tile 0 uses slots {0,4} (MMA) and {1,5} (TMA); Tile 1 rotates to {2,6} and {3,7}
        // so all 8 partition slots launch async jobs across a grid while keeping engine queues crisp.
        380..=469 => {
            let active_group = (tile as usize % 2) * 2;
            let in_group = (local % 4) / 2 == active_group / 2;
            let pair = (local / 4) as u64;
            if in_group {
                if local.is_multiple_of(2) {
                    if offset == 384 + pair * 4 {
                        return Step {
                            op: Op::Matrix,
                            dst: None,
                            srcs: [Some(0), Some(1)],
                            cache_depth: 0,
                            sectors: 0,
                            page_walk: false,
                            exec_mask: FULL_EXEC_MASK,
                        };
                    }
                    if offset == 426 + pair * 4 {
                        return Step {
                            op: Op::WaitMatrix,
                            dst: Some(4),
                            srcs: [None, None],
                            cache_depth: 0,
                            sectors: 0,
                            page_walk: false,
                            exec_mask: FULL_EXEC_MASK,
                        };
                    }
                } else {
                    if offset == 388 + pair * 5 {
                        return Step {
                            op: Op::Dma,
                            dst: None,
                            srcs: [Some(0), None],
                            cache_depth: 0,
                            sectors: 0,
                            page_walk: false,
                            exec_mask: FULL_EXEC_MASK,
                        };
                    }
                    if offset == 434 + pair * 5 {
                        return Step {
                            op: Op::WaitDma,
                            dst: Some(5),
                            srcs: [None, None],
                            cache_depth: 0,
                            sectors: 0,
                            page_walk: false,
                            exec_mask: FULL_EXEC_MASK,
                        };
                    }
                }
            }
            let r = offset as usize % REGISTERS;
            match offset % 16 {
                0 => Step {
                    op: Op::Shared,
                    dst: Some(r),
                    srcs: [Some((r + 5) % 8), None],
                    cache_depth: 0,
                    sectors: 0,
                    page_walk: false,
                    exec_mask: FULL_EXEC_MASK,
                },
                1 => Step::alu(Op::Fma, (r + 1) % 8, r, Some((r + 3) % 8)),
                2 | 6 | 10 => Step::alu(Op::Add, r, (r + 4) % 8, None),
                5 | 13 => Step::alu(Op::Xor, r, (r + 3) % 8, Some((r + 6) % 8)),
                _ => Step::alu(Op::Fma, r, (r + 5) % 8, Some((r + 7) % 8)),
            }
        }

        // Phase 6: Tail-latency skew before BAR.SYNC (straggler hits deep DRAM + RAW chains)
        470..=518 => {
            let straggler = if tile == 0 { 7 } else { 3 };
            if local == straggler {
                match offset {
                    474 | 486 | 498 => Step::load(1, 0, 3, 1, offset == 486),
                    475 | 487 | 499 => Step::alu(Op::Fma, 2, 1, Some(3)),
                    476 | 488 | 500 => Step::alu(Op::Fma, 3, 2, Some(4)),
                    477 | 489 | 501 => Step::alu(Op::Fma, 4, 3, Some(5)),
                    _ => {
                        let r = offset as usize % REGISTERS;
                        Step::alu(Op::Fma, r, (r + 7) % 8, Some((r + 6) % 8))
                    }
                }
            } else {
                let r = offset as usize % REGISTERS;
                match offset % 6 {
                    0 if offset == 480 => Step::load(r, 0, 0, 2, false),
                    1 => Step::alu(Op::Add, r, (r + 4) % 8, None),
                    3 => Step::alu(Op::Xor, r, (r + 5) % 8, Some((r + 6) % 8)),
                    _ => Step::alu(Op::Fma, r, (r + 4) % 8, Some((r + 5) % 8)),
                }
            }
        }

        // Phase 7: Post-barrier shared issue & VALU contention burst + tile epilogue
        520..=579 => {
            let r = offset as usize % REGISTERS;
            match offset % 6 {
                2 => Step::alu(Op::Add, r, (r + 4) % 8, None),
                4 => Step::alu(Op::Xor, r, (r + 3) % 8, Some((r + 5) % 8)),
                _ => Step::alu(Op::Fma, r, (r + 4) % 8, Some((r + 6) % 8)),
            }
        }
        _ => {
            let r = offset as usize % REGISTERS;
            match offset % 10 {
                0 => Step::store(0, r, 1, 0x0000_ffff),
                6 => Step {
                    op: Op::Atomic,
                    dst: Some(r),
                    srcs: [Some(0), Some((r + 5) % 8)],
                    cache_depth: 1,
                    sectors: 1,
                    page_walk: false,
                    exec_mask: 0x0000_ffff,
                },
                1 | 5 => Step::alu(Op::Add, r, (r + 4) % 8, None),
                3 | 8 => Step::alu(Op::Xor, r, (r + 3) % 8, Some((r + 6) % 8)),
                _ => Step::alu(Op::Fma, r, (r + 5) % 8, Some((r + 6) % 8)),
            }
        }
    }
}

fn operand_ready(warp: &Warp, step: Step) -> u64 {
    let mut ready = warp.fetch + 2;
    if matches!(step.op, Op::Branch | Op::Barrier) {
        for producer in warp.registers.iter().flatten() {
            ready = ready.max(producer.ready);
        }
    } else {
        for reg in step.srcs.into_iter().flatten() {
            if let Some(producer) = warp.registers[reg] {
                ready = ready.max(producer.ready);
            }
        }
    }
    ready
}

fn eligible_issue(r: &Resources, warp: &Warp, sm: usize, slot: usize, step: Step) -> u64 {
    let partition = slot / SLOTS;
    let ready = operand_ready(warp, step);
    let base = ready.max(r.issue[sm][partition]);
    if step.op.is_alu() {
        base.max(r.alu[sm][partition])
    } else {
        base
    }
}

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
    step: Step,
) -> vtr::Result<(Transaction<InstanceLabel>, u64)> {
    let (opcode, class) = step.op.names(&e.n);
    let pc = 0x1000 + (sequence % TILE_LEN) * 4;
    let description = if let Some(id) = e.instructions.get(&(pc, opcode)) {
        *id
    } else {
        let id = e.w.intern_value(Value::Map(vec![
            vtr::Attribute {
                key: e.n.pc,
                value: Value::Pointer(pc),
            },
            vtr::Attribute {
                key: e.n.opcode,
                value: Value::Str(opcode),
            },
            vtr::Attribute {
                key: e.n.class,
                value: Value::Str(class),
            },
        ]))?;
        e.instructions.insert((pc, opcode), id);
        id
    };
    let label = step.label(pc, sequence);
    let tx = e.begin(
        h.sms[sm].slots[slot].instruction,
        Some(warp.tx),
        warp.fetch,
        &label,
    )?;
    for (key, value) in [
        (e.n.instruction, Value::Ref(description)),
        (e.n.sequence, Value::U64(sequence)),
        (e.n.exec_mask, Value::U64(u64::from(step.exec_mask))),
        (e.n.mask, Value::U64(u64::from(step.exec_mask))),
    ] {
        e.attr(tx, key, value)?;
    }

    let mut ready = warp.fetch + 2;
    let mut related = Vec::new();
    if matches!(step.op, Op::Branch | Op::Barrier) {
        for producer in warp.registers.iter().flatten() {
            ready = ready.max(producer.ready);
            if !related.contains(&producer.tx.id()) {
                related.push(producer.tx.id());
                e.relate(e.n.depends, tx, producer.tx)?;
            }
        }
    } else {
        for reg in step.srcs.into_iter().flatten() {
            if let Some(producer) = warp.registers[reg] {
                ready = ready.max(producer.ready);
                if !related.contains(&producer.tx.id()) {
                    related.push(producer.tx.id());
                    e.relate(e.n.depends, tx, producer.tx)?;
                }
            }
        }
    }

    let partition = slot / SLOTS;
    let issued = eligible_issue(r, warp, sm, slot, step);
    r.now = r.now.max(issued);
    e.enter(tx, e.n.fetch, e.n.lane, warp.fetch)?;
    e.enter(tx, e.n.decode, e.n.lane, warp.fetch + 1)?;
    e.leave(tx, e.n.lane, warp.fetch + 2)?;
    e.stage(tx, e.n.ready, warp.fetch + 2, ready)?;
    e.stage(tx, e.n.scheduler, ready, issued)?;
    e.stage(tx, e.n.issue, issued, issued + 1)?;
    let resource = match step.op {
        Op::Load | Op::Store | Op::Atomic => h.sms[sm].request,
        Op::Shared => h.sms[sm].banks[0],
        Op::Matrix => h.sms[sm].matrix[0],
        Op::Dma => h.sms[sm].dma[0],
        Op::Barrier => h.sms[sm].barrier,
        Op::WaitMatrix | Op::WaitDma => h.sms[sm].slots[slot].instruction,
        _ => h.sms[sm].alu[partition],
    };
    e.w.embedded_event(
        tx.owner(),
        issued,
        e.n.issued,
        vec![Attribute {
            key: e.n.resource,
            value: Value::U64(u64::from(resource.id())),
        }],
    )?;
    r.issue[sm][partition] = issued + 1;
    if step.op.is_alu() {
        r.alu[sm][partition] = issued + step.op.alu_ii();
    }
    r.rr_cursor[sm][partition] = (slot % SLOTS + 1) % SLOTS;
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
    step: Step,
) -> vtr::Result<()> {
    let (tx, issued) = issue(e, h, r, warp, sm, slot, sequence, step)?;
    let start = issued + 1;
    let done = match step.op {
        Op::Fma | Op::Add | Op::Xor | Op::Branch => {
            let latency = if matches!(step.op, Op::Fma) { 6 } else { 2 };
            e.service(
                h.sms[sm].alu[slot / SLOTS],
                tx,
                (start, start + latency),
                e.n.execute,
                e.n.text(step.op.names(&e.n).0).to_owned().as_str(),
            )?;
            e.stage(tx, e.n.execute, start, start + latency)?;
            start + latency
        }
        Op::Load | Op::Store | Op::Atomic => {
            let operation = match step.op {
                Op::Load => e.n.read,
                Op::Store => e.n.store,
                _ => e.n.atomic,
            };
            e.attr(tx, e.n.space, Value::Str(e.n.global))?;
            e.attr(tx, e.n.operation, Value::Str(operation))?;
            e.attr(tx, e.n.bytes, Value::U64(step.sectors * 64))?;
            let mut end = start;
            for sector in 0..step.sectors {
                let address = 0x1000_0000
                    + (grid * SMS * WARPS + sm * WARPS + slot) as u64 * 0x100_000
                    + sequence * 128
                    + sector * 64;
                e.w.embedded_event(
                    tx.owner(),
                    start,
                    e.n.request_created,
                    vec![
                        Attribute {
                            key: e.n.address,
                            value: Value::U64(address),
                        },
                        Attribute {
                            key: e.n.bytes,
                            value: Value::U64(64),
                        },
                    ],
                )?;
                let sector_depth = if sector == 0 {
                    step.cache_depth
                } else {
                    step.cache_depth.saturating_sub(1)
                };
                let sector_mask = if step.sectors == 2 {
                    0xffff << (sector * 16)
                } else {
                    u64::from(step.exec_mask)
                };
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
                        depth: sector_depth,
                        operation,
                        dma: false,
                        page_walk: step.page_walk && sector == 0,
                        active_mask: Some(sector_mask),
                    },
                )?);
            }
            e.stage(tx, e.n.memory_wait, start, end)?;
            end
        }
        Op::Shared => {
            e.attr(tx, e.n.space, Value::Str(e.n.shared))?;
            e.attr(tx, e.n.operation, Value::Str(e.n.read))?;
            let mut end = start;
            for bank in 0..2 {
                for round in 0..2 {
                    let begin = r.bank[sm][bank].reserve(start, 3, r.now);
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
                    end = end.max(begin + 3);
                }
            }
            e.stage(tx, e.n.memory_wait, start, end)?;
            end
        }
        Op::Matrix | Op::Dma => {
            let dma = matches!(step.op, Op::Dma);
            let job = async_job(e, h, r, sm, slot, tx, warp.tx, start, dma)?;
            if dma {
                warp.dma = Some(job);
            } else {
                warp.matrix = Some(job);
            }
            e.w.embedded_event(
                tx.owner(),
                start,
                e.n.launched,
                vec![Attribute {
                    key: e.n.job,
                    value: Value::U64(job.tx.id()),
                }],
            )?;
            e.stage(tx, e.n.execute, start, start + 2)?;
            start + 2
        }
        Op::WaitMatrix | Op::WaitDma => {
            let job = if matches!(step.op, Op::WaitDma) {
                warp.dma.take()
            } else {
                warp.matrix.take()
            }
            .expect("script launches before waiting");
            e.relate(e.n.waits_for, tx, job.tx)?;
            let end = (start + 1).max(job.ready);
            e.stage(tx, e.n.memory_wait, start, end)?;
            e.event(warp.tx, e.n.stalled, start)?;
            e.event(warp.tx, e.n.resumed, end)?;
            end
        }
        Op::Barrier => unreachable!("barriers are completed collectively"),
    };
    e.stage(tx, e.n.writeback, done, done + 1)?;
    e.end(tx, done + 1)?;
    if let Some(dst) = step.dst {
        warp.registers[dst] = Some(ResultAt {
            tx,
            ready: done + 1,
        });
    }
    warp.completion = warp.completion.max(done + 1);
    if matches!(step.op, Op::Branch | Op::WaitMatrix | Op::WaitDma) {
        warp.fetch = done + 1;
    }
    Ok(())
}

fn complete_barrier(
    e: &mut Emit,
    h: &Hardware,
    warps: &mut [Warp],
    sm: usize,
    workgroup: Transaction<InstanceLabel>,
    generation: u64,
) -> vtr::Result<()> {
    let mut arrivals = Vec::with_capacity(warps.len());
    for warp in warps.iter_mut() {
        arrivals.push(
            warp.waiting_barrier
                .take()
                .expect("all warps must arrive before barrier release"),
        );
    }
    let begin = arrivals.iter().map(|b| b.arrival + 1).min().unwrap();
    let release = arrivals.iter().map(|b| b.arrival + 1).max().unwrap() + 4;
    let barrier = e.begin(
        h.sms[sm].barrier,
        Some(workgroup),
        begin,
        &format!("barrier generation {generation}"),
    )?;
    e.stage(barrier, e.n.barrier_wait, begin, release - 4)?;
    e.stage(barrier, e.n.service, release - 4, release)?;
    for (warp, pending) in warps.iter_mut().zip(arrivals) {
        e.relate(e.n.arrives_at, pending.tx, barrier)?;
        e.stage(pending.tx, e.n.barrier_wait, pending.arrival + 1, release)?;
        e.stage(pending.tx, e.n.writeback, release, release + 1)?;
        e.enter(warp.tx, e.n.barrier_wait, e.n.activity, pending.arrival + 1)?;
        e.enter(warp.tx, e.n.running, e.n.activity, release)?;
        e.end(pending.tx, release + 1)?;
        warp.fetch = release + 1;
        warp.completion = warp.completion.max(release + 1);
        warp.registers = [Some(ResultAt {
            tx: pending.tx,
            ready: release + 1,
        }); REGISTERS];
        warp.sequence += 1;
    }
    e.end(barrier, release)
}

fn generate(path: &Path) -> vtr::Result<()> {
    let mut options = WriterOptions {
        mode: ExecutionMode::Inline,
        ..Default::default()
    };
    options.metadata.timescale = -9;
    options.metadata.writer = "Volna synthetic GPU transaction example".into();
    options.metadata.comment = "Educational, not cycle accurate: synthetic 1 GHz clock, 1 cycle = 1 ns. Illustrative L3 is not NVIDIA architecture; matrix/DMA placement and all timing are synthetic. See gpu_transaction_trace.rs.".into();
    let mut w = Writer::create_with(path, options)?;
    let h = hardware(&mut w)?;
    let n = Names::new(&mut w)?;
    let mut e = Emit {
        w,
        n,
        open_stages: BTreeMap::new(),
        instructions: BTreeMap::new(),
    };
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
                Value::List(vec![
                    Value::U64(WARPS as u64 * THREADS_PER_WARP),
                    Value::U64(1),
                    Value::U64(1),
                ]),
            )?;
            e.attr(wg, e.n.epoch, Value::U64(grid_index as u64))?;
            let assigned = dispatch + 4 + sm as u64 * 4;
            let assignment = e.begin(h.assignment, Some(wg), assigned, "allocate block slot")?;
            e.attr(
                assignment,
                e.n.destination,
                Value::U64(u64::from(h.sms[sm].workgroup.id())),
            )?;
            e.stage(assignment, e.n.service, assigned, assigned + 4)?;
            e.end(assignment, assigned + 4)?;
            let resident = assigned + 4;
            e.stage(wg, e.n.queued, dispatch, resident)?;
            e.enter(wg, e.n.resident, e.n.residency, resident)?;
            e.event(wg, e.n.allocated, resident)?;
            workgroups.push(wg);
            let mut group = Vec::new();
            for (slot, warp) in h.sms[sm].slots.iter().enumerate() {
                let logical = (grid_index * SMS + sm) * WARPS + slot;
                let tx = e.begin(warp.warp, Some(wg), dispatch, &format!("warp {logical}"))?;
                e.attr(tx, e.n.warp, Value::U64(logical as u64))?;
                e.attr(tx, e.n.epoch, Value::U64(grid_index as u64))?;
                e.event(tx, e.n.dispatched, dispatch)?;
                e.stage(tx, e.n.queued, dispatch, resident)?;
                e.enter(tx, e.n.resident, e.n.residency, resident)?;
                e.enter(tx, e.n.running, e.n.activity, resident)?;
                e.event(tx, e.n.allocated, resident)?;
                group.push(Warp {
                    tx,
                    sequence: 0,
                    fetch: resident + (slot % SLOTS) as u64,
                    registers: [None; REGISTERS],
                    matrix: None,
                    dma: None,
                    waiting_barrier: None,
                    completion: resident,
                });
            }
            warps.push(group);
        }

        struct Candidate {
            key: (u64, usize, usize, usize),
            sm: usize,
            slot: usize,
            step: Step,
        }

        // Event/cycle-ordered multi-warp scheduler across all SMs and partitions.
        loop {
            let mut released_any = false;
            for sm in 0..SMS {
                if warps[sm].iter().all(|w| w.waiting_barrier.is_some()) {
                    let generation = warps[sm][0].sequence / TILE_LEN;
                    complete_barrier(&mut e, &h, &mut warps[sm], sm, workgroups[sm], generation)?;
                    released_any = true;
                }
            }
            if released_any {
                continue;
            }

            let mut best: Option<Candidate> = None;
            for (sm, sm_warps) in warps.iter().enumerate() {
                for (slot, warp) in sm_warps.iter().enumerate() {
                    if warp.sequence >= INSTRUCTIONS || warp.waiting_barrier.is_some() {
                        continue;
                    }
                    let step = plan_step(warp.sequence, slot, grid_index);
                    let time = eligible_issue(&r, warp, sm, slot, step);
                    let partition = slot / SLOTS;
                    let local = slot % SLOTS;
                    let rr_rank = (local + SLOTS - r.rr_cursor[sm][partition]) % SLOTS;
                    let key = (time, sm, partition, rr_rank);
                    if best.as_ref().is_none_or(|b| key < b.key) {
                        best = Some(Candidate {
                            key,
                            sm,
                            slot,
                            step,
                        });
                    }
                }
            }

            let Some(Candidate { sm, slot, step, .. }) = best else {
                break;
            };
            let warp = &mut warps[sm][slot];
            let seq = warp.sequence;
            if step.op == Op::Barrier {
                let (tx, issued) = issue(&mut e, &h, &mut r, warp, sm, slot, seq, step)?;
                warp.waiting_barrier = Some(PendingBarrier {
                    tx,
                    arrival: issued,
                });
            } else {
                instruction(&mut e, &h, &mut r, warp, sm, slot, seq, grid_index, step)?;
                warp.sequence += 1;
            }
        }

        let mut end = dispatch;
        for (sm, group) in warps.iter().enumerate() {
            let group_end = group.iter().map(Warp::drained).max().unwrap() + 1;
            for warp in group {
                let done = warp.drained() + 1;
                e.leave(warp.tx, e.n.residency, done)?;
                e.leave(warp.tx, e.n.activity, done)?;
                e.end(warp.tx, done)?;
            }
            e.leave(workgroups[sm], e.n.residency, group_end)?;
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
    e.w.clock_stop(h.clock, dispatch)?;
    assert!(e.open_stages.is_empty());
    assert_eq!(e.w.stats()?.live_owners, 0);
    e.w.close()?;
    Ok(())
}

fn generator_name<'a>(reader: &'a Reader, record: &Record) -> vtr::Result<&'a str> {
    let source = NodeId(record.generator().expect("owner source"));
    reader.string(reader.node(source)?.name)
}

fn readback(path: &Path) -> anyhow::Result<()> {
    let rd = Reader::open_with(
        path,
        ReadOptions {
            verify_checksums: true,
            ..Default::default()
        },
    )?;
    let validation = rd.validate()?;
    ensure!(
        validation.missing.is_empty(),
        "recording has unresolved references"
    );
    let clock = rd
        .find_generator(CLOCK_PATH)
        .context("GPU clock is missing")?;
    let mut instruction_generators = Vec::new();
    for (id, node) in rd.declarations().nodes.iter().enumerate() {
        if let NodeData::Generator { kind, .. } = node.data
            && rd.string(kind)? == "TRANSACTION"
        {
            ensure!(
                rd.clock_for(id as u32)? == Some(clock),
                "{} does not use the GPU clock",
                rd.full_path(NodeId(id as u32))?
            );
            if rd.string(node.name)? == "Instruction" {
                instruction_generators.push(id as u32);
            }
        }
    }
    ensure!(
        instruction_generators.len() == SMS * WARPS,
        "expected {} Instruction generators (8 slots/partition), found {}",
        SMS * WARPS,
        instruction_generators.len()
    );

    let mut stretches = Vec::new();
    rd.transactions(
        TransactionQuery {
            generator: Some(clock),
            ..Default::default()
        },
        |record| {
            stretches.push(record);
            true
        },
    )?;
    ensure!(
        stretches.len() == 1 && stretches[0].begin() == 0,
        "GPU clock must run continuously from time zero"
    );
    ensure!(
        stretches[0].attributes().iter().any(
            |attr| rd.string(attr.key).unwrap() == "vtr.period" && attr.value == Value::Time(1)
        ),
        "GPU clock must have a one-nanosecond period"
    );

    let mut counts = BTreeMap::<u64, u64>::new();
    let mut warps = BTreeMap::new();
    let mut packet = None;
    let mut last_tx_end = 0;
    let mut invalid_exec_mask = None;
    let mut invalid_block_dims = None;
    let mut divergent_instructions = 0u64;
    let mut owners = 0u64;

    let mut scheduler_wait_count = 0u64;
    let mut operand_wait_count = 0u64;
    let mut memory_wait_count = 0u64;
    let mut barrier_wait_durations = Vec::new();
    let mut page_walk_count = 0u64;
    let mut bank_access_count = 0u64;
    let mut matrix_launch_slots = BTreeSet::new();
    let mut dma_launch_slots = BTreeSet::new();
    let mut tx_to_gen = BTreeMap::<u64, u32>::new();
    let mut grid_intervals = Vec::<(u64, u64)>::new();
    let mut p0_mem_events = Vec::<(u64, i32, u32)>::new();
    let mut p0_issue_events = Vec::<(u64, u32)>::new();

    let p0_prefix = "gpu.compute_cluster[0].sm[0].simt_partition[0].";
    let expected_threads = Value::List(vec![
        Value::U64(WARPS as u64 * THREADS_PER_WARP),
        Value::U64(1),
        Value::U64(1),
    ]);

    rd.transactions(TransactionQuery::default(), |tx| {
        last_tx_end = last_tx_end.max(tx.end());
        owners += 1;
        let source = tx.generator().unwrap();
        let name = generator_name(&rd, &tx).unwrap();
        let full_path = rd.full_path(NodeId(source)).unwrap();
        match name {
            "Grid" => {
                grid_intervals.push((tx.begin(), tx.end()));
            }
            "Workgroup" => {
                let dims = tx
                    .attributes()
                    .iter()
                    .find(|a| rd.string(a.key).unwrap() == "gpu.block_dimensions")
                    .map(|a| &a.value);
                if dims != Some(&expected_threads) {
                    invalid_block_dims.get_or_insert(tx.id());
                }
            }
            "Instruction" => {
                tx_to_gen.insert(tx.id(), source);
                *counts.entry(tx.parent().unwrap_or(0)).or_default() += 1;
                match tx
                    .attributes()
                    .iter()
                    .find(|a| rd.string(a.key).unwrap() == "exec_mask")
                    .map(|a| &a.value)
                {
                    Some(Value::U64(mask)) if *mask > 0 && *mask <= u64::from(FULL_EXEC_MASK) => {
                        divergent_instructions += u64::from(*mask != u64::from(FULL_EXEC_MASK));
                    }
                    _ => {
                        invalid_exec_mask.get_or_insert(tx.id());
                    }
                }
                let in_p0 = full_path.starts_with(p0_prefix);
                for st in tx.stages() {
                    let sname = rd.string(st.name).unwrap();
                    let dur = st.end.unwrap_or(tx.end()).saturating_sub(st.begin);
                    match sname {
                        "scheduler_wait" if dur > 0 => scheduler_wait_count += 1,
                        "operand_wait" if dur > 0 => operand_wait_count += 1,
                        "memory_wait" if dur > 0 => {
                            memory_wait_count += 1;
                            if in_p0 {
                                p0_mem_events.push((st.begin, 1, source));
                                p0_mem_events.push((st.end.unwrap_or(tx.end()), -1, source));
                            }
                        }
                        "barrier_wait" if dur > 0 => barrier_wait_durations.push(dur),
                        "issue" if in_p0 => p0_issue_events.push((st.begin, source)),
                        _ => {}
                    }
                }
            }
            "Warp" => {
                warps.insert(tx.id(), source);
            }
            "PageWalk" => {
                page_walk_count += 1;
            }
            "BankAccess" => {
                bank_access_count += 1;
            }
            "MatrixJob" => {
                if let Some(parent_id) = tx.parent()
                    && let Some(&slot_gen) = tx_to_gen.get(&parent_id)
                {
                    matrix_launch_slots.insert(slot_gen);
                }
            }
            "TensorDmaJob" => {
                if let Some(parent_id) = tx.parent()
                    && let Some(&slot_gen) = tx_to_gen.get(&parent_id)
                {
                    dma_launch_slots.insert(slot_gen);
                }
            }
            "MemoryPacket" if full_path.contains("illustrative_l3") => {
                packet.get_or_insert(tx.id());
            }
            _ => {}
        }
        true
    })?;

    ensure!(
        invalid_block_dims.is_none(),
        "workgroup {invalid_block_dims:?} has unexpected gpu.block_dimensions"
    );
    ensure!(
        invalid_exec_mask.is_none(),
        "instruction {invalid_exec_mask:?} lacks a nonzero SIMT32 exec_mask"
    );
    ensure!(
        divergent_instructions > 0,
        "trace contains no lane divergence"
    );
    ensure!(
        scheduler_wait_count > 0,
        "trace contains no scheduler_wait contention"
    );
    ensure!(
        operand_wait_count > 0,
        "trace contains no operand_wait stalls"
    );
    ensure!(
        memory_wait_count > 0,
        "trace contains no memory_wait stalls"
    );
    ensure!(page_walk_count > 0, "trace contains no TLB page walks");
    ensure!(
        bank_access_count > 0,
        "trace contains no shared-memory bank accesses"
    );
    ensure!(
        matrix_launch_slots.len() >= SLOTS,
        "expected MatrixJob launches across multiple warp slots, found {}",
        matrix_launch_slots.len()
    );
    ensure!(
        dma_launch_slots.len() >= SLOTS,
        "expected TensorDmaJob launches across multiple warp slots, found {}",
        dma_launch_slots.len()
    );

    let min_bar = barrier_wait_durations.iter().copied().min().unwrap_or(0);
    let max_bar = barrier_wait_durations.iter().copied().max().unwrap_or(0);
    ensure!(
        !barrier_wait_durations.is_empty() && max_bar > min_bar + 50,
        "expected workgroup barrier tail-latency skew (min={min_bar}, max={max_bar})"
    );

    // Sweep concurrent memory_wait intervals across sm[0].simt_partition[0] slots.
    p0_mem_events.sort_unstable_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    let mut active_by_slot = BTreeMap::<u32, i32>::new();
    let mut peak_mem_slots = 0usize;
    let mut peak_mem = 0i32;
    let mut cur_mem = 0i32;
    for (_, delta, slot_gen) in p0_mem_events {
        cur_mem += delta;
        let entry = active_by_slot.entry(slot_gen).or_default();
        *entry += delta;
        if *entry == 0 {
            active_by_slot.remove(&slot_gen);
        }
        peak_mem = peak_mem.max(cur_mem);
        peak_mem_slots = peak_mem_slots.max(active_by_slot.len());
    }
    ensure!(
        peak_mem_slots == SLOTS,
        "expected all {SLOTS} distinct partition slots concurrently in memory_wait during miss storm, peak distinct slots was {peak_mem_slots}"
    );

    p0_issue_events.sort_unstable_by_key(|&(t, _)| t);
    let issue_switches = p0_issue_events
        .windows(2)
        .filter(|w| w[0].1 != w[1].1)
        .count();
    ensure!(
        issue_switches > (INSTRUCTIONS as usize) * 2,
        "expected dense interleaved issue across partition slots, found only {issue_switches} switches"
    );
    let first_grid = grid_intervals.first().copied().context("no Grid record")?;
    let max_issue_bubble = p0_issue_events
        .windows(2)
        .filter(|w| w[0].0 >= first_grid.0 && w[1].0 <= first_grid.1)
        .map(|w| w[1].0.saturating_sub(w[0].0))
        .max()
        .unwrap_or(0);
    ensure!(
        max_issue_bubble >= 50,
        "expected an intra-grid issue-unit bubble during miss storm, max gap was {max_issue_bubble}"
    );

    let mut depends_relations = 0u64;
    rd.scan_relations(|rel| {
        if rd.string(rel.kind).unwrap() == "depends_on" {
            depends_relations += 1;
        }
        true
    })?;
    ensure!(
        depends_relations > 0,
        "trace contains no depends_on relations"
    );

    ensure!(
        stretches[0].end() >= last_tx_end,
        "GPU clock stops before the last transaction"
    );
    ensure!(
        rd.cycle_at(clock, last_tx_end)? == Some(last_tx_end),
        "GPU cycle count differs from its one-nanosecond ticks"
    );
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

    let (&warp_id, &warp_gen) = warps.first_key_value().context("no warp")?;
    let slot = rd
        .node(NodeId(warp_gen))?
        .parent
        .context("warp generator has no group")?;
    let instruction_source = rd
        .children(Some(slot))
        .iter()
        .copied()
        .find(|id| {
            let node = rd.node(*id).unwrap();
            matches!(node.data, NodeData::Generator { .. })
                && rd.string(node.name).unwrap() == "Instruction"
        })
        .context("slot has no instruction source")?;
    let mut instruction = None;
    rd.transactions(
        TransactionQuery {
            generator: Some(instruction_source.0),
            ..Default::default()
        },
        |tx| {
            instruction = Some(tx);
            false
        },
    )?;
    let instruction = instruction.context("slot query found no instruction")?;
    ensure!(
        instruction.parent() == Some(warp_id),
        "instruction must belong to its slot's warp"
    );
    for generator in [warp_gen, instruction.generator().unwrap()] {
        let mut found = false;
        rd.transactions(
            TransactionQuery {
                generator: Some(generator),
                ..Default::default()
            },
            |_| {
                found = true;
                false
            },
        )?;
        ensure!(found, "generator query was empty");
    }
    println!(
        "{}: {} logical warps x {INSTRUCTIONS} = {total} instructions; {owners} owners; {} bytes; peak_p0_mem_wait={peak_mem}; barrier_wait=[{min_bar}..{max_bar}]; sched_wait={scheduler_wait_count}; op_wait={operand_wait_count}; depends={depends_relations}",
        path.display(),
        warps.len(),
        std::fs::metadata(path)?.len()
    );
    println!("Illustrative L3 packet -> command-group ancestry (record parents):");
    let mut id = packet.context("no illustrative L3 packet")?;
    let mut names = Vec::new();
    for _ in 0..20 {
        let tx = rd.lookup(id)?.context("missing ancestry transaction")?;
        let name = generator_name(&rd, &tx)?;
        let group = rd
            .node(NodeId(tx.generator().unwrap()))?
            .parent
            .context("generator has no group")?;
        let label = tx.payload().field("vtr.label")?.decode::<String>()?;
        println!(
            "  TX {} | {} | {} | {}",
            tx.id(),
            name,
            rd.full_path(group)?,
            label
        );
        names.push(name.to_owned());
        match tx.parent() {
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
    let path = args.next().map_or_else(
        || PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/gpu_transaction_trace.vtr"),
        PathBuf::from,
    );
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

    /// Scenario: A user inspects SIMT branch divergence in a warp's instruction stream.
    /// Requirement: Each tile splits into complementary 16-lane active masks after BRA
    /// and reconverges to a full 32-lane mask, and workgroup barriers execute full-mask.
    #[test]
    fn divergent_paths_reconverge() {
        for tile in 0..TILES_PER_GRID {
            let base = tile * TILE_LEN;
            assert_eq!(plan_step(base + 328, 0, 0).op, Op::Branch);
            assert_eq!(plan_step(base + 328, 0, 0).exec_mask, FULL_EXEC_MASK);
            for offset in 0..16 {
                let first = plan_step(base + 329 + offset, 0, 0).exec_mask;
                let second = plan_step(base + 345 + offset, 0, 0).exec_mask;
                assert_eq!(first & second, 0);
                assert_eq!(first | second, FULL_EXEC_MASK);
                assert_eq!(first.count_ones(), 16);
            }
            assert_eq!(plan_step(base + 361, 0, 0).exec_mask, FULL_EXEC_MASK);
            assert_eq!(
                plan_step(base + BARRIER_OFFSET, 0, 0).exec_mask,
                FULL_EXEC_MASK
            );
        }
    }

    /// Scenario: A developer generates the synthetic GPU trace and opens it in Volna.
    /// Requirement: The emitted VTR archive passes checksum and reference validation,
    /// has 8 warp slots per SIMT partition, preserves CommandStream ancestry, and exhibits
    /// multi-warp contention, miss-storm overlap, RAW stalls, barrier skew, and async overlap.
    #[test]
    fn round_trip_counts_and_ancestry() -> anyhow::Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("gpu.vtr");
        generate(&path)?;
        readback(&path)
    }
}
