# Volna for VS Code

Open VTR, FST, and Kanata recordings in a read-only custom editor. The GPUI viewer
runs in the webview; the bundled `volna-server` reads VTR on the workspace host,
including Remote SSH workspaces. The bundled `vtr` CLI automatically converts FST
and Kanata before the server starts. Kanata imports accept `.kanata` and `.log`,
including their gzip-compressed forms; FST accepts `.fst` and `.fst.gz`.

Conversion runs on the workspace host. A cancellable VS Code progress
notification shows the phase, phase percentage when measurable, input file size,
and current VTR size on disk. The converted VTR stays in a private temporary
directory until the server stops. Closing the editor, replacing an open request,
or encountering an error stops its child processes and removes temporary files.
VTR opens directly. Source files are never overwritten.

Viewer commands, VS Code themes, resource settings, and saved `.volna.json`
workspaces pass through the extension host. Settings and saved workspaces retain
the original source URI, so reopening an import converts that source again.

## Build and install

Use Node 24, Python 3, the repository Rust toolchain, and a C compiler that
supports WebAssembly. Install the matching `wasm-bindgen` CLI from `Cargo.lock`.
The pinned nightly toolchain supports the atomic wait API used by `wasm_thread`
and rebuilding the standard library for shared-memory WebAssembly. From this directory:

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
glibc. Packaging builds matching `volna-server` and `vtr` executables from the
checkout. Each VSIX includes both native tools, WebAssembly, and dependency
license notices. Generated `media/`, `bin/`, `licenses/`, `LICENSE`, and VSIX files are
ignored by Git. The manifest's settings are generated from `volna-core`; run
`npm run settings` after changing the settings registry.

## Verification

`npm test` checks command routing, workspace persistence, theme changes, and
bounded binary transport, conversion failures, cancellation, and temporary-file
lifetimes. `npm run test:package` checks platform identities, both executable
architectures and contents, asset dependencies, and license integrity.

On Linux, `npm run test:vsix` installs the built package into an isolated VS Code
profile and opens VTR, FST, Kanata, and compressed Kanata fixtures through their
default editor associations.
It requires `code` and `xvfb-run`; Playwright connects to that VS Code process.
Set `VOLNA_CODE` to use another VS Code binary.
The test verifies the bundled tools, trace metadata, rendered viewer frames,
and cleanup of servers and converted recordings after each editor closes.

The GitHub workflow verifies WebAssembly and all six Linux, macOS, and Windows
x64/arm64 packages, then runs the installed-extension test on Linux x64.
Packages remain workflow artifacts for inspection during development.
