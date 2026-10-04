# Volna

Volna is a hardware debug environment under development.

## Workspace

| Crate | Responsibility |
| --- | --- |
| [`vtr`](vtr/README.md) | Versatile Trace Record: runtime trace format, reader, writer, and derived indexes |
| [`volna-trace`](volna-trace/README.md) | Immutable VTR/FST sessions, complete objects, memory admission, and remote client |
| [`volna-server`](volna-server/README.md) | Headless process serving one recording over framed stdin/stdout |

## VTR/VDB boundary

VTR stores runtime signal values, transactions, logs, clocks, hierarchy, and identities with no dependency on VDB; design semantics and layout live in the VDB/application layer.

## Development

Use stable Rust 1.96 or newer and a C compiler (for vendored Zstandard).
No simulator, submodules, GUI SDK, or old repository checkout is required.

```sh
cargo build --workspace --locked
cargo test --workspace --locked                 # unit, integration, and rustdoc tests
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked
```

CI runs these automated, headless checks. To select the library, use
`cargo test -p vtr`; VTR alone supports Rust 1.85 or newer. See the
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
