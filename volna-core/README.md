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
