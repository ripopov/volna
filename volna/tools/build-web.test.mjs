import assert from "node:assert/strict";
import test from "node:test";
import { wasmBuildEnvironment } from "./build-web.mjs";

const unavailable = () => {
  throw new Error("unexpected tool discovery");
};

test("preserves platform defaults and explicit cross compilers", () => {
  const env = { PATH: "/tools" };
  assert.equal(wasmBuildEnvironment(env, "linux", unavailable), env);
  for (const key of [
    "CC_wasm32-unknown-unknown",
    "CC_wasm32_unknown_unknown",
    "TARGET_CC",
    "CC",
  ]) {
    const configured = { ...env, [key]: "/custom/clang" };
    assert.equal(
      wasmBuildEnvironment(configured, "darwin", unavailable),
      configured,
    );
  }
});

test("uses a WebAssembly-capable clang on PATH without Homebrew", () => {
  const env = { PATH: "/llvm/bin" };
  const result = wasmBuildEnvironment(env, "darwin", (command, args) => {
    assert.equal(command, "clang");
    assert.deepEqual(args, ["--print-targets"]);
    return { status: 0, stdout: "wasm32 - WebAssembly 32-bit" };
  });
  assert.equal(result, env);
});

function homebrew(command, args, options) {
  if (command === "clang") return { status: 0, stdout: "aarch64 x86-64" };
  if (command === "brew") {
    assert.equal(options.env.HOMEBREW_NO_AUTO_UPDATE, "1");
    if (args[0] === "list")
      return { status: 0, stdout: "llvm@9\nllvm@20\nllvm\nnode\n" };
    return { status: 0, stdout: `/brew/${args[1]}\n` };
  }
  if (command === "/brew/llvm/bin/clang") return { status: 1, stdout: "" };
  assert.equal(command, "/brew/llvm@20/bin/clang");
  return { status: 0, stdout: "wasm32 - WebAssembly 32-bit" };
}

test("selects an installed LLVM with WebAssembly support and matching archiver", () => {
  const env = { PATH: "/usr/bin", KEEP: "value" };
  assert.deepEqual(wasmBuildEnvironment(env, "darwin", homebrew), {
    ...env,
    CC_wasm32_unknown_unknown: "/brew/llvm@20/bin/clang",
    AR_wasm32_unknown_unknown: "/brew/llvm@20/bin/llvm-ar",
  });
  assert.deepEqual(env, { PATH: "/usr/bin", KEEP: "value" });
});

test("preserves an explicitly configured archiver during LLVM discovery", () => {
  const env = { AR_wasm32_unknown_unknown: "/custom/ar" };
  assert.equal(
    wasmBuildEnvironment(env, "darwin", homebrew).AR_wasm32_unknown_unknown,
    "/custom/ar",
  );
});

test("reports the missing WebAssembly toolchain before starting cargo", () => {
  for (const failure of [
    { status: 1, stdout: "" },
    { status: null, error: new Error("brew is unavailable") },
  ]) {
    assert.throws(
      () => wasmBuildEnvironment({}, "darwin", () => failure),
      /Apple Clang cannot compile WebAssembly.*CC_wasm32_unknown_unknown/,
    );
  }
});
