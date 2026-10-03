# VTR — Volna Trace Record

A Rust library for the VTR hardware simulation trace format. One file stores
signal waveforms, transactions, elaborated runtime hierarchy, and runtime
relations on a shared time base.

## Scope

- 2-, 4-, and 9-state bit vectors, reals, byte strings, aliases, and event signals.
- Dynamically declared scopes, variables, streams, generators, and enum tables.
- Transactions with typed attributes, events, stages, parent links, and relations.
- Structured timestamped logs and declared clock stretches.
- Streaming writing with optional background encoding, LZ4/Zstandard compression,
  checksums, and recovery of complete sections after an interrupted write.
- Memory-mapped random-access reading, owned immutable histories, hierarchy
  counts, and a rebuildable activity-index sidecar.

This crate implements trace storage and queries only. It does **not** implement
VDB, source indexing, presentation, a viewer, conversion tools, a server, or a C
ABI. Stable names and runtime identity allow separate consumers to bind design
metadata without changing a trace. Log-site source provenance is the only
source-location exception.

The format remains experimental. Version fields are not a compatibility
promise; the [specification](docs/SPEC.md) defines this checkout's encoding.

## Quick start

From another crate in this workspace:

```toml
[dependencies]
vtr = { path = "../vtr" }
```

Write a waveform, close it explicitly to check errors, then query it:

```rust
use vtr::{Direction, Reader, ScopeType, SignalKind, VarType, Writer};

fn main() -> vtr::Result<()> {
    let mut writer = Writer::create("counter.vtr")?;
    writer.set_timescale(-9)?; // one time unit is one nanosecond
    let top = writer.add_scope(None, "top", ScopeType::Module, "counter")?;
    let (_, count) = writer.add_var(
        Some(top), "count", VarType::Reg, Direction::Output,
        SignalKind::Bits { width: 8, states: 4 },
    )?;
    writer.set_time(0)?;
    writer.emit_u64(count, 0)?;
    writer.set_time(10)?;
    writer.emit_u64(count, 42)?;
    writer.close()?;

    let reader = Reader::open("counter.vtr")?;
    let count = reader.find_signal("top.count", '.').expect("declared signal");
    assert_eq!(reader.value_at(count, 10)?.borrow().as_u64(), Some(42));
    assert_eq!(reader.changes(count, 0, 10)?.len(), 2);
    Ok(())
}
```

`Writer` accepts non-decreasing waveform times; transaction/log timestamps
have their own ordering rules. `Reader` is `Send + Sync`. Do not modify or
truncate a file while it is memory-mapped by a reader; see the safety contract
in `Reader::open_with`. Explicitly call `Writer::close` to observe write errors
rather than relying on `Drop`.

## Documentation and examples

- `cargo doc -p vtr --no-deps --open`: build and open the Rust API documentation,
  including query costs, ordering, and ownership.
- [File-format specification](docs/SPEC.md): container, encodings, recovery,
  and the activity sidecar.
- [Design rationale](docs/RATIONALE.md): format and library boundaries.
- [Structured logging](docs/LOGGING.md): typed sites, records, and queries.
- `examples/waveform.rs`: minimal waveform round trip.
- `examples/logging.rs`: logs alongside transactions, read back three ways.

Run examples from the workspace root (the output path is optional):

```sh
cargo run -p vtr --example waveform -- /tmp/waveform.vtr
cargo run -p vtr --example logging -- /tmp/logging.vtr
```

## Building and testing

Requires Rust 1.85+ and a C compiler for Zstandard. All Rust dependencies come
from crates.io; there are no path dependencies on other project components.

```sh
cargo test -p vtr --locked
cargo test -p vtr --all-targets --locked
cargo fmt --all -- --check
cargo clippy -p vtr --all-targets --locked -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc -p vtr --no-deps --locked
```

`src/` contains unit tests and executable rustdoc examples. `tests/` covers
round trips, aliases, late hierarchy declarations, logs, clocks, recovery,
sealing, and activity-index correctness and memory bounds. Historical Verilator
output is checked in under `tests/fixtures/verilator`; tests need neither
Verilator nor the prototype repository.

## License

Licensed under MIT; see the repository root [LICENSE](../LICENSE).
