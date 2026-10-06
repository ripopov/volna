# Volna

Volna is a hardware debug environment under development.

## Workspace

| Crate | Responsibility |
| --- | --- |
| [`vtr`](vtr/README.md) | Versatile Trace Record: runtime trace format, reader, writer, and derived indexes |
| [`vtr-capi`](vtr-capi/README.md) | C ABI, public C/C++ headers, and simulator integration helpers for VTR |
| [`vtr-guard`](vtr-guard/README.md) | Linux crash and stop guard for VTR writers, used by the C API |
| [`volna-trace`](volna-trace/README.md) | Immutable VTR/FST sessions, complete objects, memory admission, and remote client |
| [`vtr-cli`](vtr-cli/README.md) | Terminal trace inspection, queries, activity indexing, and Kanata/VCD/FST conversion |
| [`volna-server`](volna-server/README.md) | Headless process serving one recording over framed stdin/stdout |
| [`volna-core`](volna-core/README.md) | Toolkit-independent viewer state, commands, layouts, and display lists |
| [`volna`](volna/README.md) | GPUI desktop viewer for VTR and FST recordings |

## VTR/VDB boundary

VTR stores runtime signal values, transactions, logs, clocks, hierarchy, and identities with no dependency on VDB; design semantics and layout live in the VDB/application layer.

## Development

Use Rust 1.97.1 or newer and a C compiler (for vendored Zstandard and
C smoke tests). The C++ smoke tests also need a C++17 compiler.
Core libraries need no simulator, submodules, GUI SDK, or old repository checkout.
The GPUI viewer requires its platform SDK and display libraries; see the
[viewer guide](volna/README.md).

```sh
cargo build --workspace --locked
cargo test --workspace --locked                 # unit, integration, and rustdoc tests
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked
```

CI runs these automated, headless checks. To select the library, use
`cargo test -p vtr`; VTR alone requires Rust 1.97.1 or newer. See the
[crate guide](vtr/README.md),
[format specification](vtr/docs/SPEC.md), and [design rationale](vtr/docs/RATIONALE.md).

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
