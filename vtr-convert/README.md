# VTR conversion

`vtr-convert` imports FST, Kanata, and VCD into VTR without depending on Volna's
viewer or trace loader. The [`vtr` CLI](../vtr-cli/README.md) supplies argument
parsing, native files, and atomic destination publication. Browser WASM hosting
is not implemented yet.

Call `fst::convert_fst`, `kanata::convert_kanata`, or `vcd::convert_vcd` with an
input path and a fresh `vtr::Writer`, then close the writer. Discard output after
any conversion error. For FST, disable writer deduplication to retain recorded
samples, including repeated events. Foreign-format decoding dependencies belong
in this crate; the viewer and `volna-server` read VTR only.

See the [CLI guide](../vtr-cli/README.md#conversion-semantics-and-limits) for
conversion semantics and limits. FST retains complete signal histories, and
Kanata retains instructions until EOF; these conversions require those values
to fit in memory.

```sh
cargo test --locked -p vtr-convert
cargo clippy --locked -p vtr-convert --all-targets -- -D warnings
```

Tests cover conversion fidelity, aliases, raw byte strings, repeated events,
time bases, pipeline transactions, compressed inputs, and malformed recordings.
Committed [fixtures](tests/fixtures/README.md) need no simulator or submodule.
