import assert from "node:assert/strict";
import { readFile, writeFile } from "node:fs/promises";
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { format } from "prettier";

const root = fileURLToPath(new URL("../../", import.meta.url));
const path = new URL("../package.json", import.meta.url);
const manifest = JSON.parse(await readFile(path, "utf8"));
const configuration = JSON.parse(
  execFileSync(
    "cargo",
    [
      "run",
      "--quiet",
      "--locked",
      "-p",
      "volna-core",
      "--example",
      "vscode_configuration",
    ],
    { cwd: root, encoding: "utf8" },
  ),
);

if (process.argv.includes("--check")) {
  assert.deepEqual(
    manifest.contributes.configuration,
    configuration,
    "Run npm run settings to update the VS Code settings.",
  );
} else {
  manifest.contributes.configuration = configuration;
  await writeFile(
    path,
    await format(JSON.stringify(manifest), { filepath: fileURLToPath(path) }),
  );
}
