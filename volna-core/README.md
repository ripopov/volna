# Volna viewer core

`volna-core` owns the viewer's document state and interaction models without a
GUI toolkit. It accepts commands, reports events and data-load requests, and
builds display lists for wave, pipeline, table, and transaction panels. The
[`volna`](../volna/README.md) crate hosts those models in a GPUI desktop window.

## Build and test

From the repository root, with the pinned Rust toolchain:

```sh
cargo build --locked -p volna-core
cargo test --locked -p volna-core
cargo doc --locked -p volna-core --no-deps --open
```

The core tests run headlessly. The `testing` feature exposes procedural trace
fixtures and completion helpers for frontend integration tests.

## Frontend contract

`App` is the viewer state machine. A frontend translates input into `Command`
values, passes them to `App`, and drains its `Event` values. The frontend runs
queued `LoadRequest` values on its worker executor and returns `LoadResult`
values to `App`. Each request carries a trace identity and generation so a late
result cannot update a replaced trace.

The core consumes immutable VTR sessions from
[`volna-trace`](../volna-trace/README.md). It keeps shared trace placement,
cursor, markers, panel state, and row selections in the document. Panel models
prepare layouts and toolkit-neutral `Scene` drawing commands; the frontend
paints the scene and hosts native window controls and dialogs. Local and remote
sessions use the same data contract.

Hosts pass the window's physical pixels per logical pixel to each
`App::layout_panel` call. This factor is independent of interface zoom. A
`Prim::Presence` carries a bit mask with one texel per physical pixel in its
rectangle. The host converts it to an opaque ink image, paints it without
rescaling, and releases obsolete image resources. GPU handles and image formats
belong entirely to the host.

## Pipeline overview

Below two physical pixels per row, the pipeline paints opaque presence in the
canvas text color. Every stage contributes its natural time interval across the
whole source row, including overlay stages. Stageless transactions contribute
their lifetime. Half-open intervals preserve gaps; zero-duration stages occupy
their containing pixel. Open, aborted, error and unset records all contribute.
Aborted rows also leave ticks in the label column. Aggregate hover has no unique
record identity; zooming in restores stage colors, labels, lanes and selection.

Each immutable prepared generator owns a row hierarchy, shared by panels displaying that generator. Sixteen-row leaves and internal nodes retain closed geometry
bounds, lifetime counts and optional exact unions capped at 32 fragments. A
summary is used only when its entire row range fits one physical pixel inside
the clip. A nonempty geometry bound alone can establish presence only when both
axes fit inside one physical pixel. Otherwise queries descend to children and
source stages. Stage bounds are independent of transaction lifetimes, since
stages can extend outside them.

Nodes and unions occupy flat arrays without copied transactions. Each index
is limited to 64 MiB and charged to the existing admission pool; refused unions
or indexes use exact source refinement. Construction uses the load worker's
cooperative checkpoints. Placement scales the index with its immutable source;
owner replacement replaces both. Every frame rasterizes its current viewport
and physical phase. Output uses at most 4,096 occupied rectangle runs before
switching to `Prim::Presence`, bounding the display list independently of gap
count. Highly scattered stages can still require a source-sized query.

## Persistence and edits

Workspace files capture panel layout, views, markers, and row choices. The
frontend performs filesystem I/O and atomic writes; the core owns the
workspace model. Incompatible workspace versions are discarded. Settings and
recent items use separate stores.

The core journal groups row, panel, marker, and workspace edits into undoable
steps. Navigation, selection, window chrome, and settings are outside that
journal. Frontends route edits through the owning model's journal writer.

The crate root documents its module map and reexports `App`, `Command`,
`Event`, `LoadRequest`, `LoadResult`, `Scene`, and `Theme` for frontend hosts.

## Group plots

Select signal rows and press **G** to group them. The group's context menu
has three Draw choices: Activity, Stacked area (**Shift+A**), and Overlaid
lines (**Shift+O**). Numeric plots remain visible when the group is folded;
expanded member rows keep their own formats and heights. A default-height
group grows to three rows for its plot and returns to its former height on
choosing Activity, unless explicitly resized.

Both numeric group modes assign the same automatic colours in member order.
Explicit signal colours override them. Each member's waveform, name stripe,
and solid colour swatch match its line or area in the group plot.

Overlaid lines compare numeric members, including signals inside nested
groups, without summing them. Buses use their current numeric format, bits
use 0/1, and reals use finite values. Text, events, and transaction lanes are
excluded. Each member uses a solid line. Selecting a member or hovering near its line
brings it forward. The group menu chooses Step or Linear and a shared Whole
trace, Visible window, or Type limits range. Trace and window ranges include
zero; type limits use the union of integer limits, with trace bounds for
members without finite type limits.

Hovering the plot lists exact held values in each member's format. Linear
strokes interpolate geometry; probe values and cursor dots still read the
recorded sample. X, Z, missing samples, and non-finite reals interrupt only
their own line. Dense views retain each member's min/max envelope; long
histories reuse the analog summaries built on the load worker.

A member's context menu can hide or isolate it in the overlay. Show all
overlay lines restores the members. These choices keep the individual rows
available and exclude hidden lines from the shared range and group edge
navigation. Rendering, range, and visibility edits are undoable and saved in
wave panel workspace version 8.

For a three-line specimen, open `volna/examples/analog_showcase.vtr`, add the
`analog.phases` scope as a group, choose Overlaid lines and Linear, then
expand the group. The three phase signals share a scale and retain separate
values at the cursor. The `analog.queues` scope provides a Step specimen;
zooming out preserves each queue's short peaks independently.

## Bus expansion

The chevron beside a logic vector reveals bit rows while retaining the parent
waveform and value. Right expands a selected bus or enters its children; Left
returns to the parent or collapses it. Collapsing retains bit formats, colors,
heights and order. Recursive group folding leaves unopened buses collapsed.

The bus menu provides **Add bit or slice…** (for example `7`, `15:8` or `0:7`)
and **Reverse bit order**. A bit or slice can be added as an independent trace
that stays visible when its parent collapses. Numeric ranges in source names
are preserved when their width matches the recorded vector; otherwise vectors
use `[width-1:0]`, with the most significant bit first.

[`wave/bus.rs`](src/wave/bus.rs) owns these interactions and index labels.
[`data/projection.rs`](src/data/projection.rs) composes slices over shared raw
histories and builds transition indexes on the load worker, within the memory
budget. Only visible or selected projections request preparation; prepared
indexes are shared by equivalent views. Changes to other bits do not create
edges or dense activity in a bit row. Signal groups treat an expanded bus as
one member, preserving their aggregate values and edge navigation.

Expansion materializes child rows as an undoable edit. Subsequent folding is
presentation state. Workspace version 8 saves the source shape, projection,
expansion state and nested bit rows. The tree validates bus ownership during
edits and restore; an unrelated signal cannot be dropped into a bus.

For a specimen, open `volna/examples/landing.vtr`, add `soc.l2.mshr_used`, and
expand its three bits. The parent retains its numeric value above the wires.
