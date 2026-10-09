# Volna viewer

`volna` is the GPUI desktop frontend for VTR and FST recordings. `volna-core`
owns viewer state, input interpretation, loading requests, viewport math, and
toolkit-neutral painting. `volna-trace` owns immutable recording data and the
complete-object remote protocol used by `volna-server`. Pipeline, table, and
transaction panels inspect runtime records from VTR. VDB attachment is not
implemented.

See the [`volna-core` guide](../volna-core/README.md) for the frontend contract,
headless tests, persistence model, and undo journal.

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
