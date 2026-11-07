# Volna viewer

`volna` is the GPUI desktop frontend for VTR recordings. `volna-core`
owns viewer state, input interpretation, loading requests, viewport math, and
toolkit-neutral painting. `volna-trace` owns immutable recording data and the
complete-object remote protocol used by `volna-server`. Pipeline, table, and
record detail panels inspect runtime records from VTR. VDB attachment is not
implemented.

In Record details, **Trace parents** after **Related** loads the full parent
chain on demand and displays a clickable graph. Click a node to open its details;
cancel tracing or hide the graph when finished.

The fixed Record details header provides **Back** and **Forward** navigation through
visited records. Use **Alt+←/→** on Windows and Linux or **⌘[/]** on macOS;
shortcuts apply to the focused detail panel.

The **At cursor** list reserves a stable height above the record details, including
when no transactions are active. Both panes scroll independently. Drag the divider
to resize; double-click resets it. Tab to the divider and use **↑/↓** to resize,
**Shift** for larger steps, **Home/End** for the limits, or **Enter** to reset.
**Escape** cancels a drag. Workspace files remember each panel's divider height;
small panels temporarily clamp it to keep both panes usable.

Double-click a log or located-event generator to add it to waveforms, or use
**Add to Waves** in the member context menu. Logs use `log.message`; located
events use `vtr.label`. Sparse records have inline labels. Simultaneous records
share a counted label, and nearby timestamps merge into time-range groups as you
zoom out. Groups use trace-time boundaries and stable lanes, so panning translates
the badges and their timestamp anchors together. Severity counts retain rare
errors in a burst.

Selecting a group opens its records in the shared **Record details** panel.
Filter by severity or search the message, ID, timestamp, and argument previews;
pages retain individual record IDs and exact timestamps. **Zoom to group** fits
a time range. Records at the same timestamp remain grouped at every zoom.
A selected log shows its message, arguments, and runtime source; an event shows
its label and arguments; a transaction shows its lifetime and related activity.

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

Each trace name in the title bar has a source-path tooltip and an actions menu,
including when only one file is open. Open the menu with its dropdown arrow or
by right-clicking the name. The first item, **Copy path**, copies the source path
even after renaming the trace. Click the name to reveal the trace in the hierarchy;
double-click to rename it. Letter badges distinguish traces when several are open.

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

## Signal formatting

Waveform rows place Scope before Name, Value, and the waveform. Names stay aligned
when signals from different scopes share a name. Consecutive visible rows with
the same trace and scope share a faint band and one scope label. Tall bands show the scope name above its parent path; short
bands show the fitted full path. Labels stay visible while scrolling through a
band. Scope bands preserve row ordering and break at groups or unscoped rows.
Long paths retain their trailing portion, and hovering shows the full scope.
Drag the column dividers to resize Scope, Name, and Value. Column widths are saved
in the workspace. Choose the starting Scope state and width for new panels in
Settings → Waves; splits and tabs of an existing waveform panel inherit its state
instead. Collapse Scope with the chevron in its header or **Shift+S**;
it becomes a 20 px color rail. Drag its divider below 48 px to snap it closed,
or drag outward to expand it. The chevron and shortcut restore the last expanded
width. Scope bands use stable colors from the current theme, independently of
waveform colors. Colors are recognition aids and may repeat; hover a band for
the full scope path. Accessibility labels retain each signal's full path.
To inspect the bands, open `volna/examples/analog_showcase.vtr` and add
`analog.waves.sine`, `analog.waves.cosine`, `analog.power.cpu_mw`, and
`analog.power.gpu_mw`. Each pair shares one band. Collapse Scope with **Shift+S**
to inspect the same boundaries in the rail.

The compact state and expanded width are saved per panel; waveform panel schema
is v15, within workspace format v13. Linked timelines derive their width from a
saved reference, so collapse/expand cycles preserve the divider even when the
available space constrains it. Scope bands use hierarchy identities; distinct
scopes with identical paths remain separate. To compare repeated names, add
`soc.cpu0.pc` and `soc.cpu1.pc`
from `volna/examples/pipeline_showcase.vtr`.

Right-click a signal in Scope, Name, or Values to change **Value format**,
**Waveform**, **Vertical scale**, **Color**, or **Row height** directly from the
context menu. The heading identifies the signal or selected rows. Right-clicking
a selected signal keeps the selection; right-clicking an unselected signal
targets that signal alone. Left-dragging the format badge or analog status line
reorders rows.

**Waveform** offers Digital, Analog — steps (hold each sample), and
Analog — interpolated (straight lines between samples). **Vertical scale** offers
Whole trace, Visible time window, and Type limits where supported. Group rendering
has separate **Group display**, **Interpolation**, and **Group vertical scale**
controls. In mixed group/signal selections, **Signal waveform** and
**Signal vertical scale** affect only directly selected signals; selecting a group
does not implicitly change its contained signals through those controls.
**View → Expand All Groups / Collapse All Groups** operates on all groups.

**Copy value at cursor** copies the complete selected representation, even when the
cell is shortened. **Copy full signal path** copies the scope and signal name.
For several signals, each command copies one TSV-escaped cell per line in row
order. Single-signal copying produces the value or path directly. Value copying
is unavailable until a cursor value is loaded for every target. These commands
leave the signal-row clipboard intact; **Cut signals**, **Copy signals**, and
**Paste rows** manage rows between waveform panels. **Remove from view** removes
displayed rows without changing the recording; groups explicitly say
**Remove group and contents**.

Value format is omitted when only one representation is supported. Scalar bits
keep their bit representation, including when cycling formats with the keyboard.

Cursor values and digital waveform labels keep their selected format when it
fits. Overflowing decimal and real values use scientific notation, preserving
the sign and complete exponent; `≈` marks rounding. Hexadecimal and binary
overflows retain both ends with a `0x` or `0b` prefix, such as `0xab…34`.
Text retains its beginning. Numeric values show `…` when the available width
cannot hold a meaningful representation. These compact labels do not change
recorded values or clipboard contents.

For a native control specimen, open `volna/examples/analog_showcase.vtr` and add
`analog.waves.sine` and `analog.waves.cosine`. Select both rows and right-click
in any metadata column. Differing settings show **Mixed**; changing a setting
applies that setting to the selection. Waveform and scale submenus identify the
applicable subset when some selected signals cannot use the setting. Value-format
choices must support every target; an explanation appears when no common format
exists. Menus use the native theme, keyboard navigation, and window bounds at each
interface zoom.

## Derived signals and measurements

Select recorded numeric waveforms or applied waveform formulas and choose
**Derive**, or right-click one and choose **Derive signal…**. The docked
inspector opens a formula with the selected input bindings. The formula editor
uses an accent-colored frame and tinted surface in both themes. Moving average,
Low-pass, and Mean over intervals actions apply to the current expression,
retaining its arithmetic and sampling. When switching from a single interval
measurement, they use that measurement's input. Boolean expressions are converted
with `real(...)` when needed. Invalid expressions and combined scalar measurements
must be edited before applying these actions.
**Add recorded input…** searches all loaded traces, including signals that are not
displayed. Each compact binding row shows its alias, source, and numeric type. Click an
alias to insert it at the formula cursor. **Edit** expands the alias, numeric
interpretation, optional bit or slice, and **Replace…** and **Remove** actions.
Aliases are independent of row names. Unused bindings do not narrow the output domain
or truncate rows when adding results to a table. In windows narrower than 500 px,
the hierarchy temporarily yields its space to the formula inspector and returns
when the inspector closes.

A temporary **Draft** row previews the result on the waveform timeline, sharing
zoom, markers, cursor, and value readout. Initial settings compute automatically;
typing updates after a 300 ms pause. Presets and policy selections update
immediately; Ctrl+Enter (Command+Enter on macOS) requests an immediate preview.
Enter inserts a new line in the formula. Ctrl+Space opens completion for live
input aliases and functions with their argument signatures. Function hints appear
beneath the editor, and errors appear directly below the expression. The
collapsible guide provides searchable examples, **Insert** at the cursor, and
**Replace formula** using available value-input aliases. Replacements retain the
selected sampling unless the example specifies its own capture. **Copy** preserves the original
example text. Default sampling step appears only when a sampled function omits
`step`; measurement scope and missing-value controls appear for interval
measurements. Scope and missing-value menus show their choices directly. Renaming the output does not recompute it. The
last successful result remains dimmed and marked outdated while settings change.
**Add waveform** keeps the successful draft. Select it and choose **Derive** or
**Edit derived signal…** to edit; **Apply changes** commits the result. **Cancel**
restores the applied result. Reordering rows keeps the editor attached to its
source. Drafts are not saved and do not enter undo history; each Apply creates
one undo step. **Remove definition**, ordinary row removal, copy/paste, and
undo/redo also work.

### Formula language

Formulas support up to 16 recorded or applied waveform inputs. Examples:

```text
adc_in - reference
valid && ready
abs(measured - expected) > tolerance
moving_avg(adc_in, 4 us, step: 250 ns)
adc_in - reference @posedge clk
100 * interval_mean(real(valid))
sine(period: 20 us, amplitude: 2, step: 500 ns)
```

Supported syntax includes finite numeric constants, parentheses, unary `+`/`-`,
arithmetic `+ - * / %`, comparisons `< <= > >= == !=`, Boolean `&& || !`, and
`abs`, `min`, `max`, `real`, and `bool`. Temporal functions are `moving_avg`,
`lowpass`, and `integrate`; generators are `sine` and `clock`. Durations such as
`4 us` and `250 ns` must resolve exactly to integer timeline ticks. Named
arguments set steps, warm-up, unknown-data policies, phase, and initial values.
The saved **Default step** is used when a function omits `step`.

Single-bit inputs are Boolean. Wider vectors use the binding's unsigned, signed,
or IEEE floating-point interpretation; recorded reals remain real. Slices use
source HDL bit indices, such as `7:0`. `real(bit)` maps false/true to 0/1;
`bool(number)` tests for nonzero. There are no implicit Boolean/Real casts.
Unknowns propagate through arithmetic; division or remainder by zero and
non-finite results produce gaps. Boolean logic uses three values: false AND
unknown is false, true OR unknown is true, and unresolved cases remain unknown.

Values are held between events. Pointwise expressions use the sorted union of
input events, while temporal children keep independent grids anchored at their
resolved domain starts. Viewport and zoom changes never move those grids.
`x @ 1 ns` captures on a fixed grid, `x @sample_number` captures each
observed bus value change, `x @clk` captures published Boolean clock state changes, and `x @posedge clk` captures only known false-to-known true
transitions. Repeated captures remain available even when their values repeat.
Captures use timestamp-coalesced histories, so delta-cycle edges cannot be
recovered.

`interval_mean`, `interval_integral`, `interval_min`, `interval_max`, and
`interval_sum` produce measurements drawn as analog steps over the saved **Scope**.
The default **Each marker interval** splits the recording at its markers, including
the interval before the first marker and after the last. Applying saves the
interval boundaries; moving or deleting markers afterward does not alter the
saved result. With no interior markers, it covers the whole range. Unknown
results appear as gaps.

Other scopes are Whole range, explicit bounds, a marker snapshot, linked markers,
or a marker set. Marker sets produce one value per adjacent pair. Marker-based
scopes follow markers while previewing and save their bounds when applied.
Strict mode propagates unknown data, while skip mode uses known support and
reports coverage. **Add to waveform** plots the result. **Add to table** opens
the existing table panel with start times, values, and end ticks for measurements.
Waveform results use timestamp and value columns, including the final sample.
The table retains its values in the workspace. `interval_sum` requires a captured Real
waveform and counts samples rather than weighting them by time.

The expression is limited to 4,096 bytes, 256 expression nodes, and 32 nesting
levels. Output is limited to 1,000,000 events per expression node and jobs are
limited to 10,000,000 evaluation steps. HDL width-preserving arithmetic,
bitwise operations, user-defined functions, and scripts are not supported.

### Moving averages

`moving_avg(input, window, step: interval)` accepts a Real waveform expression,
a trailing window, and an output interval. Use the **Moving average** preset to
start with a bound input. Enter durations such as `4 us` or `250 ns`, or an integer number
of timeline ticks; durations must resolve to whole ticks. The mean weights each
held value by its duration in `[t − window, t)`. Warm-up can wait for a complete
window or use available history. Missing, NaN, and infinite inputs propagate a
gap by default; **skip unknown time** divides by the known duration instead.
A window without known duration is unavailable in both modes. The result is
always real-valued and retains the captured numerical interpretation and slice.
Bit inputs contribute 0 and 1; undefined logic follows the missing-data policy.
Text and event signals cannot be averaged. Output follows the explicit fixed
grid over the input trace's domain, with at most 1,000,000 samples.

### Reusable sampling

In Variables or a recorded waveform's context menu, choose **Use for sampling**
and then **Rising edge** for a bit or **Value changes** for a bit or bus. The first
choice for each trace becomes its default; additional choices remain available
without replacing it. The sampling submenu can make a choice the default or
remove it. Removing the default promotes that trace's next choice.

The formula editor's **Sampling** control always offers **@* · Any input change**,
which restores ordinary event-driven evaluation by removing explicit captures.
`@*` labels this existing mode in the control; formulas use their usual syntax
without a capture suffix. Partially captured expressions and mixed sampling
rules are shown as **Custom sampling**, since one rule does not describe all values. The control also lists registered choices and opens the
recorded-signal picker through **Choose another signal…**. The picker can also
set the chosen signal as the default for new formulas. Escape closes the picker
and returns focus to the expression. Failed selections preserve the draft and
keep the picker open. Unavailable saved choices can be removed from the menu. Choosing sampling binds
the signal and writes its capture into the expression, inside interval
measurements. New formulas use their input trace's default; formulas combining
traces require an explicit choice. Saved formulas keep their explicit captures
when defaults change. Value-change sampling observes bus transitions exactly,
including reset and wrap, without creating samples for skipped counter values.

For a native sampling specimen, open `volna/examples/feature_showcase.vtr`, find
`soc.clk` in Variables, and register its rising edge. Derive `soc.temperature_c`
and select **Mean over intervals**; the formula retains `@posedge clk`. Register
`soc.data` with **Value changes**, then switch sampling from the editor. Check the
menus and picker in both themes, including a narrow inspector.

### Execution and saved definitions

Formulas use the native/background Web Worker executor and perform
memory admission before scanning input values or allocating output. **Stop**,
cancellation, and calculation changes cancel abandoned work. **Resume** and
**Retry** restart stopped or failed work. Late completions cannot replace a
newer preview. Invalid drafts and failed previews preserve the applied row.

Workspace version 20 and wave panel version 20 store formula expressions,
persistent formula IDs and revisions, ordered aliases, stable recorded paths and
duplicate-path ordinals, recursive applied-formula inputs, source shapes,
numeric interpretations, slices, default steps, saved ranges, marker scopes,
and unknown-data policies. Results are recomputed on reopening. Recorded domains
follow the current recordings; generator and literal-only domains use the saved
range. Missing or ambiguous bindings remain repairable. Renaming or removing an
applied source row does not invalidate dependent definitions, and dependency
cycles are rejected before Apply. **Open recorded inputs in table** opens all
resolved recorded inputs.

For a native control specimen, open `volna/examples/analog_showcase.vtr`, add
`analog.waves.sine`, derive a formula, add `analog.waves.cosine` with the picker,
and enter `input - cosine` (`sine` is a reserved generator name). Switch themes
while the inspector is open. For a
moving average, derive `analog.waves.noise` with a `4 us` window and `250 ns`
output interval, then change the window and cancel to inspect the original row.

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
typed logs, and located events. The landing trace includes sparse records, dense
sequences, and hundreds of records at the same timestamp. The
[`landing-page.volna.json`](examples/landing-page.volna.json) workspace includes
both point tracks and the shared detail panel.
[`landing-points.volna.json`](examples/landing-points.volna.json) opens a focused
log/event demonstration with the 300-message burst selected. Both generators
validate their completed recording with VTR's reader and check the simulated
populations and timing.

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

Record details keeps attribute names and values in separate, single-line columns.
Long names truncate from the left to keep their suffix visible; values truncate
from the right. Hover to read the complete text, or use **Copy** for the record
as TSV. Timing tiles and record labels also truncate within the panel.
The right sidebar stops shrinking at 280 logical pixels (scaled by interface
zoom). Drag its divider past half that width and release to hide it; reopening
restores its width from before the drag.

Double-clicking a hierarchy member or pressing Enter adds it to Waves. Signals,
clocks, transactions, logs, and events share the focused Waves panel; when another
panel has focus, an existing Waves panel is reused before creating one. Adding
an existing member selects and reveals its row, expanding enclosing groups as
needed. **Add to Waves** uses the same behavior. **Open in Pipeline** and
**Open in table** remain explicit choices in the member context menu.

New waveform, pipeline, table, and Settings panels open in the center, even
when the sidebar has focus or the center is empty. Requests that reuse an
existing panel preserve its placement and reveal it if its sidebar is hidden.
Explicit split and new-tab actions stay in the chosen panel's region.

Opening a pipeline automatically reveals an unpinned transaction inspector,
creating one in the right sidebar when needed. Keyboard focus stays in the
pipeline. New pipeline panels start at the first transaction with a small leading
margin and a readable cycle width matching the default row height. Tracks without
a usable clock estimate the scale from the first transaction's stage durations
or lifetime. Reusing a panel or restoring a workspace preserves its view;
navigation during loading takes precedence over the automatic opening view.
The right-hand instruction navigator shows sampled record lifetimes across the
whole stream. Click or drag it to reveal a row at its beginning without changing
zoom, cursor, or selection. Its accent-colored stream and tinted viewport follow
the active theme; flushed samples use the error color and right-edge status
ticks. A contrasting outline with a separating halo marks the visible rows.
The compact open/close grip has a larger hit area. Drag its left edge
to resize; click the edge or the **Navigator** toolbar button to hide or restore
it. The panel menu also offers **Instruction navigator**, **Widen instruction
navigator**, and **Narrow instruction navigator** for keyboard operation. Width
and visibility are saved per panel; narrow windows limit the strip to a quarter
of the panel while retaining the saved width.

Ordinary vertical wheel scrolling follows the stream in time, preserving the
execution phase at the viewport center as the top record changes. Horizontal
and Shift+wheel gestures pan time; dragging the canvas pans freely. Manual row
navigation suspends **Follow activity**. Time movement follows the panel's shared
viewport setting. Stacked linked timelines reserve a common right gutter so
their time axes stay aligned as the navigator is resized or hidden.

With no selected record the inspector shows its empty state; selecting
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
Workspace version 9 stores the center tree, optional right dock (including its
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
rules and row visibility are saved with the table; workspace table schema is v7.

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
