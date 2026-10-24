import { mkdir, readFile, rm, writeFile } from "node:fs/promises";
import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("../../", import.meta.url));

export function wasmBuildEnvironment(
  env = process.env,
  platform = process.platform,
  execute = spawnSync,
) {
  const compilerKeys = [
    "CC_wasm32-unknown-unknown",
    "CC_wasm32_unknown_unknown",
    "TARGET_CC",
    "CC",
  ];
  if (platform !== "darwin" || compilerKeys.some((key) => env[key])) return env;

  const supportsWasm = (compiler) => {
    const result = execute(compiler, ["--print-targets"], {
      env,
      encoding: "utf8",
    });
    return result.status === 0 && /\bwasm32\b/.test(result.stdout);
  };
  if (supportsWasm("clang")) return env;

  const brewEnv = { ...env, HOMEBREW_NO_AUTO_UPDATE: "1" };
  const installed = execute("brew", ["list", "--formula"], {
    env: brewEnv,
    encoding: "utf8",
  });
  const formulas = (installed.stdout || "")
    .split(/\s+/)
    .filter((name) => /^llvm(?:@\d+)?$/.test(name))
    .sort((a, b) => {
      const version = (name) => Number(name.split("@")[1] || Infinity);
      return version(b) - version(a);
    });
  for (const formula of formulas) {
    const prefix = execute("brew", ["--prefix", formula], {
      env: brewEnv,
      encoding: "utf8",
    });
    if (prefix.status !== 0) continue;
    const bin = join(prefix.stdout.trim(), "bin");
    const compiler = join(bin, "clang");
    if (!supportsWasm(compiler)) continue;
    const result = { ...env, CC_wasm32_unknown_unknown: compiler };
    const archiverKeys = [
      "AR_wasm32-unknown-unknown",
      "AR_wasm32_unknown_unknown",
      "TARGET_AR",
      "AR",
    ];
    if (!archiverKeys.some((key) => env[key]))
      result.AR_wasm32_unknown_unknown = join(bin, "llvm-ar");
    return result;
  }
  throw new Error(
    "Apple Clang cannot compile WebAssembly. Install Homebrew LLVM (brew install llvm), or set CC_wasm32_unknown_unknown and AR_wasm32_unknown_unknown to a WebAssembly-capable toolchain.",
  );
}

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
        ...wasmBuildEnvironment(),
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

export async function buildWebConverter(destination) {
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
      "--manifest-path",
      "vtr/Cargo.toml",
      "--target-dir",
      metadata.target_directory,
      "-p",
      "vtr-convert-wasm",
      "--target",
      "wasm32-unknown-unknown",
      "--release",
      "--locked",
    ],
    {
      env: {
        ...wasmBuildEnvironment(),
        CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUSTFLAGS:
          "-C link-arg=--max-memory=1073741824",
      },
    },
  );
  await rm(destination, { recursive: true, force: true });
  await mkdir(destination, { recursive: true });
  const wasm = join(
    metadata.target_directory,
    "wasm32-unknown-unknown",
    "release",
    "vtr_convert_wasm.wasm",
  );
  const digest = createHash("sha256")
    .update(await readFile(wasm))
    .digest("hex")
    .slice(0, 16);
  const name = `vtr_convert_${digest}`;
  run("wasm-bindgen", [
    "--target",
    "web",
    "--out-dir",
    destination,
    "--out-name",
    name,
    wasm,
  ]);
  await writeFile(
    join(destination, "manifest.json"),
    JSON.stringify({ module: `${name}.js` }) + "\n",
  );
}
