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
    ],
    {
      env: { ...process.env, RUSTC_BOOTSTRAP: "1" },
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
