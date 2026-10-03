# Trace loader fixtures

These small immutable recordings exercise the VTR/FST session API and remote
server. Tests read them directly; no simulator, FST writer, submodule, or other
checkout is needed. Server process tests share this directory.

| Files | Coverage |
| --- | --- |
| `values.fst`, `values-wrapped.fst` | Raw and gzip-wrapped FST: real values, arbitrary byte strings, nine-state logic, events, aliases, and a nonzero start time |
| `fst_types.fst` | FST declaration and value types, including ports |
| `features.{fst,vtr}`, `operators.{fst,vtr}`, `pipeline.{fst,vtr}` | Paired Verilator recordings compared at every value change |
| `counter.vtr`, `picorv32.vtr`, `landing.vtr` | Hierarchy views, page transfer, declaration identity, and alias counts |
| `feature_showcase.vtr`, `pipeline_showcase.vtr`, `landing_dram.fst` | Scope-size counts across waveform, transaction, and FST hierarchies |

## Provenance

The fixtures and `fst_values.c` were imported from the
[prototype repository at revision 35d8aa8](https://github.com/ripopov/vtr/tree/35d8aa8fb1e328d95a0ceecd80fd0ed0bccc237c).
The `values` files and their generator came from `volna/volna-trace/tests/fixtures`;
the counter, PicoRV32, landing, and showcase recordings came from
`volna/volna/examples`. That directory's
[guide](https://github.com/ripopov/vtr/blob/35d8aa8fb1e328d95a0ceecd80fd0ed0bccc237c/volna/volna/examples/README.md)
describes their original generators.

The FST type fixture and paired Verilator recordings came from the prototype's
Surfer fork, revision `cdcdfc85686b151953e1c6fa006ffe0591269754`, under
`examples/fst_types.fst` and `examples/verilator`. They are recorded regression
inputs; the simulator integration is not included in this workspace.

## Regenerate the value fixtures

`fst_values.c` uses GTKWave's FST writer API. To regenerate these two files,
provide an FST writer source directory containing `fstapi.c` and `fstapi.h`,
with the LZ4 and zlib development libraries available through `pkg-config`:

```sh
# Run from the workspace root; point FST_WRITER_DIR at the writer source.
cc volna-trace/tests/fixtures/fst_values.c "$FST_WRITER_DIR/fstapi.c" \
  -I"$FST_WRITER_DIR" $(pkg-config --cflags --libs liblz4 zlib) \
  -o /tmp/volna-fst-values
/tmp/volna-fst-values volna-trace/tests/fixtures/values.fst
/tmp/volna-fst-values volna-trace/tests/fixtures/values-wrapped.fst wrap
```

After replacing fixtures, run `cargo test -p volna-trace -p volna-server --locked`.
Format changes must update fixtures and assertions together. New VTR test cases
should normally create recordings in temporary directories with `vtr::Writer`.
