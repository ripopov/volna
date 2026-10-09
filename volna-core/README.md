# Volna viewer core

`volna-core` owns the viewer's document state and interaction models without a
GUI toolkit. It accepts commands, reports events and data-load requests, and
builds display lists for wave, pipeline, table, and transaction panels. The
[`volna`](../volna/README.md) crate hosts those models in a GPUI desktop window.

## Build and test

From the repository root, with Rust 1.97.1 or newer:

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

The core consumes immutable VTR/FST sessions from
[`volna-trace`](../volna-trace/README.md). It keeps shared trace placement,
cursor, markers, panel state, and row selections in the document. Panel models
prepare layouts and toolkit-neutral `Scene` drawing commands; the frontend
paints the scene and hosts native window controls and dialogs. Local and remote
sessions use the same data contract.

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
wave panel workspace version 7.

For a three-line specimen, open `volna/examples/analog_showcase.vtr`, add the
`analog.phases` scope as a group, choose Overlaid lines and Linear, then
expand the group. The three phase signals share a scale and retain separate
values at the cursor. The `analog.queues` scope provides a Step specimen;
zooming out preserves each queue's short peaks independently.
