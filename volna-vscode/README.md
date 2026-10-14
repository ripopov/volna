# Volna for VS Code

Open VTR and FST recordings in a read-only custom editor. The GPUI viewer runs
in the webview; the bundled `volna-server` reads traces on the workspace host,
including Remote SSH workspaces. Viewer commands, VS Code themes, resource
settings, and saved `.volna.json` workspaces pass through the extension host.

## Build and install

Use Node 24, Python 3, the repository Rust toolchain, and a C compiler that
supports WebAssembly. Install the matching `wasm-bindgen` CLI from `Cargo.lock`.
The shared build helper enables `RUSTC_BOOTSTRAP` for the unstable atomic wait
API used by `wasm_thread`. From this directory:

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.128 --locked
npm ci
npm run check
npm test
npm run build
npm run test:package
npm run package
code --install-extension volna-0.1.2-linux-x64.vsix
```

Packaging uses the build host's OS and architecture. Linux packages require
glibc. Each VSIX includes WebAssembly, a native trace server, and dependency
license notices. Generated `media/`, `bin/`, `licenses/`, `LICENSE`, and VSIX files are
ignored by Git. The manifest's settings are generated from `volna-core`; run
`npm run settings` after changing the settings registry.

## Verification

`npm test` checks command routing, workspace persistence, theme changes, and
bounded binary transport. `npm run test:package` checks platform identities,
server architectures, asset dependencies, and license integrity.

On Linux, `npm run test:vsix` installs the built package into an isolated VS Code
profile and opens VTR and FST fixtures through their default editor association.
It requires `code` and `xvfb-run`; Playwright connects to that VS Code process.
Set `VOLNA_CODE` to use another VS Code binary.
The test verifies the bundled server, trace metadata, and rendered viewer frames.

The GitHub workflow verifies WebAssembly and all six Linux, macOS, and Windows
x64/arm64 packages, then runs the installed-extension test on Linux x64.
Packages remain workflow artifacts for inspection during development.
