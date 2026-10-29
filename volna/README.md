# Volna viewer

`volna` is the GPUI desktop frontend for VTR recordings. `volna-core`
owns viewer state, input interpretation, loading requests, viewport math, and
toolkit-neutral painting. `volna-trace` owns immutable recording data and the
complete-object remote protocol used by `volna-server`. Pipeline, table, and
transaction panels inspect runtime records from VTR. VDB attachment is not
implemented.

In Transaction details, **Trace parents** after **Related** loads the full parent
chain on demand and displays a clickable graph. Click a node to open its details;
cancel tracing or hide the graph when finished.

The fixed Transaction header provides **Back** and **Forward** navigation through
visited records. Use **Alt+←/→** on Windows and Linux or **⌘[/]** on macOS;
shortcuts apply to the focused detail panel.

See the [`volna-core` guide](../volna-core/README.md) for the frontend contract,
headless tests, persistence model, and undo journal.

The same viewer runs in the [VS Code extension](../volna-vscode/README.md).
Its WebAssembly build is shared with the documentation site through
`tools/build-web.mjs`; the extension bundles a native `volna-server` for the
workspace host.

VTR transactions may be recorded with out-of-order timestamps, as in
temporally decoupled TLM simulations. Volna's loading layer sorts each
generator's transactions by `(begin, end, transaction ID)` before building
viewer indexes. The waveform panel positions intervals at their recorded
timestamps and assigns overlapping intervals to subrows; producer call
order does not determine their horizontal position. The raw VTR reader
itself retains file order.

The native app opens FST and Kanata (`.kanata`, `.log`, and their `.gz` forms)
by running `vtr convert INPUT OUTPUT` in the background. VTR opens directly.
The original recording path is retained for names, recent files, workspace
association, and reopening. Each import gets a private temporary VTR and activity
indexes, removed when its last reader closes. Imports are converted again on
reopen; closing or replacing an unfinished import cancels its converter.

During conversion, the loading view shows phase progress, the original input
file size, and the current VTR size on disk. Preparation and finalization have
indeterminate bars. A Cancel conversion button stops the child process and
removes its temporary files. Additional recordings show the same information
above the open recording. FST reading measures recording time; Kanata reading
measures source bytes, so percentages describe the current phase rather than a
prediction of total conversion time.

The same status layout is used in the loading view and secondary import banners:

```text
Writing VTR                         42%
[========            ]
Input file                    256.0 MiB
Output VTR                     38.7 MiB
           Cancel conversion
```

The converter is resolved from `VOLNA_VTR`, then the executable built as Volna's
Cargo artifact dependency, then beside the Volna executable, then `PATH`.
Set `VOLNA_VTR` to an executable path, without arguments. Native distributions
include the matching converter. Building Volna builds both executables:

```sh
cargo build --locked -p volna
cargo run --locked -p volna -- recording.fst
```

The VS Code extension also imports FST and Kanata with its bundled converter.
The standalone browser imports FST, Kanata (`.kanata` and `.log`), and VCD,
including their `.gz` forms, through a separate converter WASM worker. The page
shows phase progress and input/output sizes. **Cancel import**, another recording,
or **Load sample** terminates the conversion worker and discards its memory.
The viewer receives a complete VTR image and keeps the original source filename
for workspace association. VTR opens directly without starting a converter.

Browser imports stay local and use RAM for the input, retained histories, and
converted VTR. The browser converter limits input, external gzip decompression,
and encoded output to 256 MiB each, with a separate 1 GiB WASM memory ceiling for
conversion. The viewer and file buffers need additional memory; available browser
RAM can impose a lower limit. No converted recording is written to disk. Native
and VS Code hosts use temporary files instead.

## Build and run

Use the pinned nightly toolchain in `rust-toolchain.toml` and the platform
libraries required by GPUI. From the repository root:

```sh
cargo build --locked -p volna
cargo run --locked -p volna -- volna/examples/picorv32.vtr
cargo run --locked -p volna -- --help
```

The launcher accepts VTR, FST, and Kanata paths, multiple traces, and saved
`.volna.json` workspaces. `--no-workspace` disables workspace reads and writes;
`--config-dir DIR` chooses the settings directory.

Build a native distribution directory for the current platform with both
executables and dependency license notices:

```sh
python volna/package_native.py --output atemp/volna-native
# Use --debug for a development distribution; --target-dir accepts a Cargo build directory.
```

Keep both executables in the same directory. The package uses the platform's
GPUI system libraries; it is not a macOS app bundle or a system installer.
The process-level import smoke test uses the converter built by Cargo:

```sh
cargo test -p volna --lib native_import::tests::real_converter
```

## Islands appearance

**Volna Dark** is the default theme. Select **Volna Islands Dark** in Settings →
Appearance to use rounded tab-group islands on continuous surrounding chrome.
**Volna Islands Light** pairs the same layout with Volna Light's data palette,
white islands, and a soft blue-to-sea-glass backdrop.
See [geometry and implementation](ISLANDS.md).

## Feature, pipeline, and landing examples

[`verilator_pipeline`](examples/verilator_pipeline/README.md) contains the bound
SystemVerilog tracer and verifier for `pipeline_demo.vtr`. It preserves the core's
waveforms while recording pipeline stages, child bus requests, relations, clock
timing, and diagnostics through the pinned Verilator integration.

[`feature_showcase.rs`](examples/feature_showcase.rs) records a two-microsecond
debugging lab with pipeline hazards, DMA traffic, an injected bus fault, and
recovery. It includes all attribute value types, declaration types, repeated
events, a recording gap, changing clocks, stacked waveforms, and typed logs.
Regeneration validates the complete recording and checks its simulation
invariants.

[`volna_theme.rs`](examples/volna_theme.rs) records 4,096 cycles of two cores,
including the C910 stage ladder, DMA descriptors, logs at five severities,
weak and unknown signal states, analog spikes, gated clocks, deep hierarchy,
flushes, and instructions still in flight at close.

[`pipeline_showcase.rs`](examples/pipeline_showcase.rs) simulates two pipelines
running the same instruction loop, including cache stalls, branch flushes,
dependencies, and child memory requests. [`landing.rs`](examples/landing.rs)
records a core, L2 cache, DRAM refills, an interrupt handler, declared clocks,
and typed logs. Both generators validate their completed recording with VTR's
reader and check the simulated populations and timing.

From the workspace root:

```sh
cargo run --locked -p volna --example feature_showcase -- volna/examples/feature_showcase.vtr
cargo run --locked -p volna --example volna_theme -- volna/examples/volna-theme.vtr
cargo run --locked -p volna --example pipeline_showcase -- volna/examples/pipeline_showcase.vtr
cargo run --locked -p volna --example landing -- volna/examples/landing.vtr --dram-vcd /tmp/landing_dram.vcd
vtr convert /tmp/landing_dram.vcd /tmp/landing_dram.vtr
```

The optional VCD contains the landing simulation's DRAM controller at picosecond
resolution. Its ACT commands and addresses align with the main recording's
refill transactions. The delivered `landing_dram.vtr` is converted from the
[matching FST fixture](../vtr/vtr-convert/tests/fixtures/).
After regenerating `landing.vtr`, retarget the transaction inspector in
`landing-page.volna.json` to the instruction labelled
`80000054: addi a4, a4, 4`; record IDs include embedded events and stages.

## GPU transaction example

[`examples/gpu_transaction_trace.rs`](examples/gpu_transaction_trace.rs) generates
and reads back the checked-in
[`examples/gpu_transaction_trace.vtr`](examples/gpu_transaction_trace.vtr).
Its static hardware generators model two SMs with four warp slots each; two grids
reuse those slots for 16 logical warps, each with exactly 5,000 instructions
(80,000 total). Select a `MemoryPacket` under
`gpu.memory_subsystem.illustrative_l3.requests` and follow its transaction parents
through L2, L1, an LSU request, instruction, warp, workgroup, grid, and bounded
command-stream execution. Hardware paths and logical TX ancestry are independent. Each generator binds
a typed `InstanceLabel` payload whose `vtr.label` field supplies the displayed
label. Optional attributes describe workload provenance; embedded stages and
events describe execution within each owner.

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

### Docking and transaction details

New waveform, pipeline, table, and Settings panels open in the center, even
when the sidebar has focus or the center is empty. Requests that reuse an
existing panel preserve its placement and reveal it if its sidebar is hidden.
Explicit split and new-tab actions stay in the chosen panel's region.

Opening a pipeline automatically reveals an unpinned transaction inspector,
creating one in the right sidebar when needed. Keyboard focus stays in the
pipeline. With no selected record the inspector shows its empty state; selecting
a record fills it immediately. An existing inspector retains its placement,
including in the center. If all inspectors are pinned, a separate unpinned
inspector is opened.
Pinning keeps a record in place so the next request for details opens another
details tab. The panel menu offers **Move to center** and
**Move to right sidebar**. Waveform, pipeline, and table panels offer the same
placement actions. Tabs can also be dragged between the center and right dock.
**Hide sidebar** preserves every tab and its state, and the title-bar toggle
remains available to reopen it. **Close tab** (or the tab's × button) removes an
individual panel. Central panels retain **Close panel**. Requesting details again
reveals an existing unpinned inspector in its current location.

Right-click a tab name to open its panel actions, including placement, splitting,
new waveform tabs, closing, and renaming. Actions target that tab even when another
tab is active. Timed panels also offer viewport and cursor links; transaction
panels offer record pinning. The same actions appear in the panel's overflow menu.

The GPUI Kit `DockArea` renders both regions, including their tabs, splits, and
resize handles. The core owns panel identities and validates the complete
layout before accepting widget changes. Moving a panel preserves its view and
record state and is undoable. In the native app, panel menus also offer **Move
to new window** and **Return to main window**. Auxiliary windows share the
document, navigation links, settings, and undo history, and support tabs and
splits. A single-panel auxiliary window shows its title and panel controls in the
window header; adding a tab or split restores the dock tab bar. Closing an auxiliary
window returns its panels to the main window.
Workspace version 8 stores the center tree, optional right dock (including its
width and visibility), and auxiliary pane trees. The native app restores those
windows; the web app opens their panels in the main window.

### Workspace persistence

Workspace files store panel structure, views, markers, and row choices. The
native frontend writes them atomically. Incompatible workspace versions are
discarded. Settings and recent items live separately in the config directory.

Table columns have 1px draggable dividers extending through the header and body,
including the edge after Attributes.
Dragging resizes one column and moves the columns that follow it. Double-click
fits the header and visible values; a focused divider supports Left, Right, and
Enter. Attributes initially fills the remaining width, and its bounded preview
grows with the column. Table panel schema version 4 stores column keys, visibility,
order, and widths, including hidden columns. **Columns → Reset to defaults** restores column
visibility, order, and automatic widths.

Double-clicking a log generator opens a table with **Time** (Begin),
**Severity** (`log.severity`), and **Message** (`log.message`). Message fills the
remaining width by default. Logs use the same column editing, filtering, sorting,
and workspace persistence as other tables; resetting columns restores the log
preset. Activating an already open log generator focuses its table.

**Columns → Add attribute columns…** opens a searchable checklist of transaction
attributes. Keys and observed types are collected during track loading; searching
uses that catalogue without scanning transactions. Checkboxes update the table
immediately and keep the picker open. `vtr.label` controls the existing Label
column. New attribute columns appear before the Attributes preview. Headers can
be dragged to reorder columns, and their context menu offers **Hide column**.
Hidden columns retain their widths and positions. Attribute cells distinguish
missing values (`—`) from explicit `null`. The Transaction panel’s attribute
context menu offers **Show as table column**, reusing a table for the same
generator or opening one.

### Table sorting

Click a column header to cycle ascending, descending, and source order. Tab to a
header and use Enter or Space for the same action; divider shortcuts still resize
columns. An accent arrow shows the **applied** direction; paired arrows mark
unsorted columns. The 40px header aligns titles, arrows, and format chips. Long
titles truncate, while format controls retain their width. A pin and accent
border distinguish column overrides from inherited formats. While sorting, the status
bar labels the previous order and offers Cancel. Selection follows the record,
including after sorting, Go to, and copying. Sort choices are local to the open
view and are not saved in workspace files.

Sorting on native and web compares complete typed values, never cell previews: integers and
timestamps stay exact, text uses case-sensitive byte order, and ties retain
source order in both directions. Missing labels, unavailable signal samples, and
open transaction end/duration values sort last. Attributes compare key/value
sequences in stored order, excluding `vtr.label`. Different attribute types sort
by type; real values use IEEE total order and logic values use width then bits.

Sorting stays entirely in RAM. A cancellable worker sorts an admitted index in
place, using 32-bit source ordinals for up to 2³² rows and 64-bit ordinals above
that. The 100M-row index needs 400 MB. Its shared-budget reservation remains held
for the index's entire lifetime; the previous index also remains charged while
its replacement is built. Allocation failures preserve the previous order.

When there is insufficient budget for a new index, the header's sort icons turn
gray and mouse/keyboard sorting is refused. Returning a descending column to
source order without filters remains available because it frees memory. With
filters active, returning to source order requires admission for a new bitmap.
Sorting becomes available again when budget permits; after an allocation error,
use Retry. No
sort files or memory mappings are used.
Access to a sorted row is O(1); locating a selected source row is O(log N). Source
rows must also fit in RAM. Native and web use the same engine on GPUI's background executor.
The browser uses shared-memory Web Workers; it never runs a sort on the UI thread.
WASM32 has a 4 GiB address-space ceiling shared by source data, indices, and runtime,
so its maximum dataset size is lower than native even on machines with more RAM.

The web build requires `rust-src` (`rustup component add rust-src`).
`tools/build-web.mjs` rebuilds the standard library with atomics and emits a
shared-memory viewer module. The docs build also emits the independent,
single-threaded `vtr-convert-wasm` module, loaded only for browser imports.
Serve it over HTTPS (localhost also works), with
`Cross-Origin-Opener-Policy: same-origin` and
`Cross-Origin-Embedder-Policy: require-corp` on the top-level page and its frames.
The browser must support `SharedArrayBuffer` and `Atomics.waitAsync`; startup
reports missing support instead of silently running background work on the UI thread.
The docs development server and browser tests set these headers; `docs/public/_headers`
is included for static hosts that support that configuration. On GitHub Pages,
a same-origin service worker supplies the headers and reloads the page once; it
does not cache recordings or assets. Content Security Policy must allow `worker-src blob:`;
the current GPUI worker launcher also needs `unsafe-eval` to discover its module URL.

After building the docs, `npm run test:sorting` in `docs/` checks a 1,000,000-row
browser table, cancellation, selection, memory refusal, keyboard sorting, and
animation frames during sorting. It also tests static-host isolation and missing
browser capability errors.

`npm run test:imports` verifies the browser converter, compressed fixtures, loaded
pipeline and waveform data, workspace identity, responsive frames, keyboard
cancellation, replacement, error recovery, and progress controls in both themes
at desktop and 360px widths. It requires an existing docs build.

The reproducible scale check sorts and verifies 100M procedural keys in RAM.
It measures the sorter, independently of trace loading and frame latency:

```sh
cargo test --release -p volna-core --lib hundred_million_rows -- --ignored --nocapture
```

### Column filtering

In a Table panel, **Filter** after **Columns** shows an entry row below the
headers. Enter applies a column's draft; Escape restores its applied query.
Collapsing the row preserves the rules, and the button shows the active count.
Clear filters removes every rule, including rules on hidden columns. Filter
rules and row visibility are saved with the table; workspace table schema is v6.

The mode button in each editor offers contains, exact, wildcard (`*`, `?`, and
backslash escaping), fuzzy subsequence, regular expression, and numeric expression
matching. Text defaults to case insensitive; Match case is an explicit override.
Fuzzy matches query characters in order, so `clm` matches `cache_load_miss`.
Regex uses Rust's bounded, non-backtracking engine. Missing values can be excluded,
included, or selected alone. Rules on different columns combine with AND.
The Attributes preview column searches attribute keys and complete leaf values.

Numeric expressions support `=`, `!=`, `<`, `<=`, `>`, `>=`, inclusive `100..500`
ranges, AND/OR, and `&&`/`||` (AND has precedence). Decimal, hexadecimal, negative,
and scientific literals are accepted. Unitless time comparisons use raw trace
ticks; explicit `fs`, `ps`, `ns`, `us`/`µs`, `ms`, and `s` suffixes use the trace's
timescale. Named time units (such as cycles) accept raw ticks and reject SI
suffixes. Integer and fixed-point comparisons stay exact, independently of the
chosen number translator. Real values use floating-point comparisons; NaN and
infinity do not match numeric expressions. Logic values compare numerically when
they contain only binary digits and fit in 64 bits. Open transaction end/duration,
null attributes, and unavailable signal samples are missing values.

Query compilation, scanning, sorting matches, index allocation, and index disposal
run on the same background executor as sorting, including Web Workers in the
browser. Input is applied explicitly rather than scanning on every keystroke.
Newer requests cancel older work; only a result for the current request is
published. The previous result stays usable while filtering, on invalid syntax,
and on admission failure. Cancel restores its rules and ordering; Retry repeats a
failed query. Queries are limited to 4096 bytes per column.

Unsorted matches use a rank/select bitmap with one bit per source row and one
64-bit prefix per 512 rows: 1,406,256 bytes for 10M rows. A scan is O(N), with
short-circuiting column predicates and borrowed text; it never formats cells.
Only visible rows are prepared for rendering. Sorting a filtered table allocates
an index for the M matches and uses O(M log M) work; source ordinal width determines
whether entries use 32 or 64 bits. The shared memory budget admits both the old
and replacement result, including temporary bitmap/index overlap. Source data
must still fit in the native or browser memory limits described above.

The scale checks exercise a bitmap scan, a background wildcard/numeric query over
procedural histories, and filtering 10M resident transactions. They exclude trace
loading and require release mode:

```sh
cargo test --release -p volna-core --lib ten_million -- --ignored --nocapture
```

### Marker shortcuts

Across the workspace, **Ctrl+1–6** places the numbered marker at the cursor,
or moves it there if it exists. Waveform, pipeline, and table panels use their
own effective cursor; transaction details and other non-timed panels use the
shared cursor. **1–9** jumps to the corresponding marker with the same cursor
scope. Text fields and dialogs keep their normal number input. Moving preserves
the marker's name and measurement reference. Another marker at the same time
blocks placement. Each placement or move is one undoable step.

In waveform and pipeline panels, **.** and **,** walk markers in time order,
and **`** returns from a jump. **M** adds the lowest free marker
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
