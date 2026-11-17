Volna is a GPUI-based viewer for VTR recordings, with synchronized waveform, pipeline, table, and record-detail panels,
derived signals, markers, and saved workspaces. The native app also imports FST and Kanata recordings.
Run `cargo run --locked -p volna -- volna/examples/picorv32.vtr` from the repository root with the pinned toolchain.

In a waveform panel, right-click a clock ruler and check **Use as grid** to align
the grid with that clock. Only one grid is drawn. Uncheck it or hide its ruler to
return to the time grid. The choice is saved per panel and supports undo; it does
not change the cursor or the clock used for cycle navigation.
