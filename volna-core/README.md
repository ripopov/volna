`volna-core` provides toolkit-independent viewer state, interactions, and drawing commands.
It manages waveform, pipeline, table, and record-detail panels, derived signals, workspaces, and undo/redo.
Frontends translate input into commands, run background loads, paint scenes, and handle windowing and filesystem I/O.
Tests run headlessly; run `cargo test --locked -p volna-core` from the repository root with the pinned Rust toolchain.
