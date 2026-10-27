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

These fixtures use the current VTR 3.4 wire format. Older recordings must be
regenerated. Record IDs include embedded stages and events; locate records by
their source, payload, and relationships when updating viewer expectations.

The `feature_showcase`, `pipeline_showcase`, and `landing` recordings are copies
of the outputs from the checked-in [Volna example generators](../../../volna/README.md#feature-pipeline-and-landing-examples).
Regenerate the examples, then copy their `.vtr` files into this directory.

`counter.vtr` and `picorv32.vtr` are converted from the checked-in VCDs in
[`source`](source). From the workspace root:

```sh
cargo run --locked --manifest-path vtr/Cargo.toml -p vtr-cli -- convert volna-trace/tests/fixtures/source/counter.vcd volna-trace/tests/fixtures/counter.vtr
cargo run --locked --manifest-path vtr/Cargo.toml -p vtr-cli -- convert volna-trace/tests/fixtures/source/picorv32.vcd volna-trace/tests/fixtures/picorv32.vtr
```

`features.vtr`, `operators.vtr`, `pipeline.vtr`, `values.vtr`, and
`landing_dram.vtr` are converted from inputs in
[`vtr-convert`](../../../vtr/vtr-convert/tests/fixtures/). The example directory also carries
copies of counter, PicoRV32, and landing DRAM recordings.

After replacing fixtures, run:

```sh
cargo test -p volna-trace -p volna-server -p volna-core --locked
```

New VTR test cases should normally create recordings in temporary directories
with `vtr::Writer`.
