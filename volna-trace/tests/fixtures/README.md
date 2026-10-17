# Trace loader fixtures

These immutable VTR recordings exercise local loading, remote transport, and
viewer behavior. Tests read them directly; no simulator or submodule is needed.
Server process tests share this directory.

| Files | Coverage |
| --- | --- |
| `values.vtr` | Real values, arbitrary byte strings, nine-state logic, events, aliases, and a nonzero start time |
| `features.vtr`, `operators.vtr`, `pipeline.vtr` | Verilator waveforms paired with converter inputs |
| `counter.vtr`, `picorv32.vtr`, `landing.vtr` | Hierarchy views, page transfer, declaration identity, and alias counts |
| `feature_showcase.vtr`, `pipeline_showcase.vtr`, `landing_dram.vtr` | Scope sizes across waveform and transaction hierarchies |

## Provenance

The source recordings were imported from the
[prototype repository at revision 35d8aa8](https://github.com/ripopov/vtr/tree/35d8aa8fb1e328d95a0ceecd80fd0ed0bccc237c).
The `values` files and their generator came from `volna/volna-trace/tests/fixtures`;
the counter, PicoRV32, landing, and showcase recordings came from
`volna/volna/examples`. That directory's
[guide](https://github.com/ripopov/vtr/blob/35d8aa8fb1e328d95a0ceecd80fd0ed0bccc237c/volna/volna/examples/README.md)
describes their original generators.

`values.vtr` and `landing_dram.vtr` were generated with `vtr convert` from
inputs in [`vtr-convert`](../../../vtr/vtr-convert/tests/fixtures/README.md).
That guide records foreign-format fixture provenance and regeneration commands.

The VTR fixtures use format 2.0. Their hierarchy declarations use generator kinds
and ordinary grouping scopes; clocks are generators. Signal histories,
transaction IDs, timing, and log arguments are retained.

After replacing fixtures, run:

```sh
cargo test -p volna-trace -p volna-server -p volna-core --locked
```

New VTR test cases should normally create recordings in temporary directories
with `vtr::Writer`.
