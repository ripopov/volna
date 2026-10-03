# Volna

Volna is a hardware debug environment under development.

## Workspace

| Crate | Responsibility |
| --- | --- |
| [`vtr`](vtr/README.md) | Volna Trace Record: runtime trace format, reader, writer, and derived indexes |

## VTR/VDB boundary

VTR stores runtime signal values, transactions, logs, clocks, hierarchy, and identities with no dependency on VDB; design semantics and layout live in the VDB/application layer.

## Development

Use stable Rust 1.85 or newer and a C compiler (for vendored Zstandard).
No simulator, submodules, GUI SDK, or old repository checkout is required.

```sh
cargo build --workspace --locked
cargo test --workspace --locked                 # unit, integration, and rustdoc tests
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked
```

CI runs these automated, headless checks. To select the library, use
`cargo test -p vtr`. See the [crate guide](vtr/README.md),
[format specification](vtr/docs/SPEC.md), and [design rationale](vtr/docs/RATIONALE.md).
