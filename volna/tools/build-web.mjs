import { mkdir, rm } from "node:fs/promises";
import { spawnSync } from "node:child_process";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("../../", import.meta.url));

function run(command, args, options = {}) {
  const result = spawnSync(command, args, {
    cwd: root,
    stdio: "inherit",
    ...options,
  });
  if (result.error) throw result.error;
  if (result.status !== 0)
    throw new Error(`${command} exited with ${result.status}`);
  return result.stdout;
}

export async function buildWebViewer(destination) {
  const metadata = JSON.parse(
    run(
      "cargo",
      ["metadata", "--no-deps", "--format-version", "1", "--locked"],
      {
        stdio: ["ignore", "pipe", "inherit"],
        encoding: "utf8",
      },
    ),
  );
  // wasm-bindgen initializes a separate stack and TLS block for each shared-memory worker.
  run(
    "cargo",
    [
      "build",
      "-p",
      "volna",
      "--lib",
      "--target",
      "wasm32-unknown-unknown",
      "--release",
      "--locked",
      "-Zbuild-std=std,panic_abort",
    ],
    {
      env: {
        ...process.env,
        RUSTC_BOOTSTRAP: "1",
        CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUSTFLAGS:
          "-C target-feature=+atomics,+bulk-memory,+mutable-globals -C link-arg=--shared-memory -C link-arg=--import-memory -C link-arg=--max-memory=4294967296 -C link-arg=--export=__wasm_init_tls -C link-arg=--export=__tls_size -C link-arg=--export=__tls_align -C link-arg=--export=__tls_base",
      },
    },
  );
  await rm(destination, { recursive: true, force: true });
  await mkdir(destination, { recursive: true });
  run("wasm-bindgen", [
    "--target",
    "web",
    "--out-dir",
    destination,
    "--out-name",
    "volna",
    join(
      metadata.target_directory,
      "wasm32-unknown-unknown",
      "release",
      "volna.wasm",
    ),
  ]);
}
