# Volna

Volna is a hardware debug environment under development.

## Workspace

| Package | Responsibility |
| --- | --- |
| [`vtr`](vtr/vtr/README.md) | Versatile Trace Record: runtime trace format, reader, writer, and derived indexes |
| [`vtr-cpp`](vtr/vtr-cpp/README.md) | Native C++20 VTR reader/writer, C headers, and simulator integration helpers |
| [`vtr-guard`](vtr/vtr-guard/README.md) | Linux crash and stop guard for Rust VTR writers |
| [`volna-trace`](volna-trace/README.md) | Immutable VTR sessions, complete objects, memory admission, and remote client |
| [`vtr-convert`](vtr/vtr-convert/README.md) | Shared FST, Kanata, and VCD importers |
| [`vtr-convert-wasm`](vtr/vtr-convert-wasm/README.md) | Browser worker host for in-memory trace conversion |
| [`vtr-progress`](vtr/vtr-progress/README.md) | Conversion progress counters and the CLI/native reporting protocol |
| [`vtr-cli`](vtr/vtr-cli/README.md) | Terminal trace inspection, queries, activity indexing, and Kanata/VCD/FST conversion |
| [`volna-server`](volna-server/README.md) | Headless process serving one recording over framed stdin/stdout |
| [`volna-core`](volna-core/README.md) | Toolkit-independent viewer state, commands, layouts, and display lists |
| [`volna`](volna/README.md) | GPUI desktop viewer with automatic FST and Kanata import into VTR |
| [`volna-vscode`](volna-vscode/README.md) | VS Code custom editor with bundled trace server, automatic FST/Kanata conversion, and platform VSIX packaging |

## VTR/VDB boundary

VTR stores runtime signal values, transactions, logs, clocks, hierarchy, and identities with no dependency on VDB; design semantics and layout live in the VDB/application layer.

## Development

Use the pinned nightly toolchain in `rust-toolchain.toml` and a C compiler
for vendored Zstandard. The native VTR library additionally requires a C++20
compiler, CMake, and LZ4, Zstandard, and zlib development packages; see its
[build instructions](vtr/vtr-cpp/README.md).
VTR is developed in [ripopov/vtr](https://github.com/ripopov/vtr) and pinned by
this repository's `vtr` submodule. Initialize it before building:

```sh
git submodule update --init vtr
```

The VTR workspace contains `vtr/vtr`, `vtr/vtr-cpp`, the converter crates,
and `vtr/ext/verilator`. Its Rust libraries need no simulator or GUI SDK.
Initialize its nested Verilator submodules only for simulator development:

```sh
git -C vtr submodule update --init --recursive ext/verilator
```

Develop VTR on its `main` branch, push changes there, then stage the updated
`vtr` submodule revision in Volna.
The GPUI viewer requires its platform SDK and display libraries; see the
[viewer guide](volna/README.md).

```sh
cargo build --workspace --locked
cargo test --workspace --locked                 # Volna tests
cargo test --manifest-path vtr/Cargo.toml --workspace --locked # VTR tests
cargo fmt --all -- --check
cargo fmt --manifest-path vtr/Cargo.toml --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked
```

CI runs these automated, headless checks. To select the library, use
`cargo test --manifest-path vtr/Cargo.toml -p vtr`. The workspace enables Cargo artifact dependencies so building
Volna also builds its matching `vtr` executable. The standalone VTR crate still
requires Rust 1.97.1 or newer; CI tests its packaged source with that compiler.
See the
[crate guide](vtr/vtr/README.md),
[format specification](vtr/vtr/docs/SPEC.md), and [design rationale](vtr/vtr/docs/RATIONALE.md).

To run the CI Clippy check before each push, enable the tracked Git hook once
per clone:

```sh
git config core.hooksPath .githooks
```

The hook checks the current working tree with the same Clippy command as CI.

## Documentation website

Engineering documentation builds to a static Astro/Starlight website in
[`docs/`](docs/README.md). It uses Markdown, SVG, build-time Mermaid diagrams,
and small HTML/JavaScript widgets. The shared
[Volna design system](docs/design-system/readme.md) also supplies application UI
tokens and assets.

```sh
cd docs
npm ci
npx playwright install chromium
npm run build
npm run preview
```

The output is `docs/dist/`. See the docs README for checks, browser verification,
and hosting under a URL prefix.

## License

Licensed under MIT; see [LICENSE](LICENSE).
