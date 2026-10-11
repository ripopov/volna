# Volna viewer

`volna` is the GPUI desktop frontend for VTR and FST recordings. `volna-core`
owns viewer state, input interpretation, loading requests, viewport math, and
toolkit-neutral painting. `volna-trace` owns immutable recording data and the
complete-object remote protocol used by `volna-server`. Pipeline, table, and
transaction panels inspect runtime records from VTR. VDB attachment is not
implemented.

In Transaction details, **Trace parents** after **Related** loads the full parent
chain on demand and displays a clickable graph. Click a node to open its details;
cancel tracing or hide the graph when finished.

See the [`volna-core` guide](../volna-core/README.md) for the frontend contract,
headless tests, persistence model, and undo journal.

VTR transactions may be recorded with out-of-order timestamps, as in
temporally decoupled TLM simulations. Volna's loading layer sorts each
generator's transactions by `(begin, end, transaction ID)` before building
viewer indexes. The waveform panel positions intervals at their recorded
timestamps and assigns overlapping intervals to subrows; producer call
order does not determine their horizontal position. The raw VTR reader
itself retains file order.

## Build and run

Use Rust 1.97.1 or newer and the platform libraries required by GPUI. From the
repository root:

```sh
cargo build --locked -p volna
cargo run --locked -p volna -- volna/examples/picorv32.vtr
cargo run --locked -p volna -- --help
```

The launcher accepts VTR or FST paths, multiple traces, and saved
`.volna.json` workspaces. `--no-workspace` disables workspace reads and writes;
`--config-dir DIR` chooses the settings directory.

## GPU transaction example

[`examples/gpu_transaction_trace.rs`](examples/gpu_transaction_trace.rs) generates
and reads back the checked-in
[`examples/gpu_transaction_trace.vtr`](examples/gpu_transaction_trace.vtr).
Its static hardware streams model two SMs with four warp slots each; two grids
reuse those slots for 16 logical warps, each with exactly 5,000 instructions
(80,000 total). Select a `MemoryPacket` under
`gpu.memory_subsystem.illustrative_l3.requests` and follow its transaction parents
through L2, L1, an LSU request, instruction, warp, workgroup, grid, and bounded
command-stream execution. Hardware paths and logical TX ancestry are independent.

From the workspace root:

```sh
# Regenerate the delivered recording, close it, and verify it through Reader.
cargo run --locked -p volna --example gpu_transaction_trace
# Or write elsewhere without replacing the example asset.
cargo run --locked -p volna --example gpu_transaction_trace -- /tmp/gpu_transaction_trace.vtr
# Open the recording in the viewer.
cargo run --locked -p volna -- volna/examples/gpu_transaction_trace.vtr
# Round-trip test uses a temporary directory; only instruction counts are golden.
cargo test --locked -p volna --example gpu_transaction_trace
```

The trace includes pipelined arithmetic, coalesced memory requests, cache hits and
misses, DRAM and translation service, conflicted shared-memory accesses,
asynchronous matrix/DMA jobs, waits, and workgroup barriers. The generator prints
a readback summary and an actual packet-to-command-stream parent chain.
This is an educational deterministic schedule, not a GPU performance model:
one synthetic cycle equals one nanosecond, issue selection is strict round-robin,
and cache residency is scripted. In particular, the **illustrative L3** is an
intentional departure from modern NVIDIA GPUs, which normally expose L2 as the
last-level GPU cache. Per-SM matrix/DMA placement, resource counts, and latencies
are illustrative; the source documents the assumptions. No GPU runtime is needed.

## Architecture

The frontend translates GPUI input into `volna_core::Command` values and
passes them to `App`. `App` reports events and load requests; the frontend
executes those loads and returns results to `App`. A request carries a trace
identity and generation so a late response cannot update a replaced trace.
`volna-core` builds a display list and panel hit regions; GPUI paints that
list and owns window chrome. Local and remote sessions return the same
immutable trace objects. Remote transport carries raw trace data; settings,
presentation, and workspace edits stay in the viewer.

### Workspace persistence

Workspace files store panel structure, views, markers, and row choices. The
native frontend writes them atomically. Incompatible workspace versions are
discarded. Settings and recent items live separately in the config directory.

### Marker shortcuts

In waveform and pipeline panels, **Ctrl+1–6** places the numbered marker at
that panel's cursor, or moves it there if it exists. Moving preserves the
marker's name and measurement reference. Another marker at the same time
blocks placement. Each placement or move is one undoable step.

**1–9** jumps to the corresponding marker; **.** and **,** walk markers in
time order, and **`** returns from a jump. **M** adds the lowest free marker
number or names the marker at the cursor. **Ctrl+Alt+1–9** focuses panels;
**Cmd+1–9** also focuses panels on macOS.

### Undo and redo

Cockpit edits use the core journal: row, panel, marker, and workspace edits
form labelled steps. Navigation, selection, chrome state, and settings are
outside the journal. Frontends must hand edits to the owning model's journal
writer and expose the resulting undo/redo commands through the core.

## Checks

```sh
cargo fmt -p volna-core -p volna -- --check
cargo test --locked -p volna-core
cargo test --locked -p volna --lib --bins
cargo clippy --locked -p volna-core -p volna --all-targets -- -D warnings
```

The core tests are headless. Desktop GPUI tests require the platform SDK;
windowed operation also requires a display server.
