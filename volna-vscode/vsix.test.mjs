import assert from "node:assert/strict";
import { spawn, execFile } from "node:child_process";
import { promisify } from "node:util";
import { gzipSync } from "node:zlib";
import {
  access,
  copyFile,
  mkdir,
  mkdtemp,
  readFile,
  rm,
  writeFile,
} from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { createServer } from "node:net";
import { fileURLToPath } from "node:url";
import { test } from "node:test";
import { chromium } from "playwright";
import { PNG } from "pngjs";
import { traces, until } from "./test-support.cjs";

const ext = dirname(fileURLToPath(import.meta.url));
const exec = promisify(execFile);

async function freePort() {
  const server = createServer();
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  const port = server.address().port;
  await new Promise((resolve) => server.close(resolve));
  return port;
}

test(
  "installed VSIX opens VTR, FST, and Kanata with its bundled tools",
  { timeout: 300000 },
  async (t) => {
    assert.equal(
      process.platform,
      "linux",
      "this headless host requires Linux and xvfb-run",
    );
    const manifest = JSON.parse(
      await readFile(join(ext, "package.json"), "utf8"),
    );
    const { version } = manifest;
    const target = `linux-${process.arch}`;
    const vsix = resolve(
      process.env.VOLNA_VSIX ?? join(ext, `volna-${version}-${target}.vsix`),
    );
    await access(vsix).catch(() => {
      throw new Error(`Missing ${vsix}; run package_vsix.py first`);
    });
    const root = await mkdtemp(join(tmpdir(), "volna-vsix-test-"));
    let child,
      browser,
      page,
      output = "",
      spawnError;
    const results = join(ext, "test-results");
    await mkdir(results, { recursive: true });
    async function hostExited() {
      const detail = await readFile(
        join(root, "driver-error.txt"),
        "utf8",
      ).catch(() => "");
      return new Error(
        `VS Code exited (${child.exitCode}): ${detail}\n${output}`,
      );
    }
    t.after(async () => {
      await browser?.close();
      if (
        child &&
        child.exitCode === null &&
        child.signalCode === null &&
        !spawnError
      ) {
        const exited = new Promise((resolve) => child.once("exit", resolve));
        process.kill(-child.pid, "SIGTERM");
        const force = setTimeout(() => {
          try {
            process.kill(-child.pid, "SIGKILL");
          } catch {}
        }, 2000);
        await exited;
        clearTimeout(force);
      }
      await rm(root, { recursive: true, force: true, maxRetries: 5 });
    });
    const code = process.env.VOLNA_CODE ?? "code";
    const profile = join(root, "profile"),
      extensions = join(root, "extensions"),
      probe = join(root, "probe");
    await mkdir(join(profile, "User"), { recursive: true });
    await mkdir(probe);
    await writeFile(
      join(profile, "User", "settings.json"),
      JSON.stringify({
        "workbench.startupEditor": "none",
        "window.restoreWindows": "none",
        "security.workspace.trust.enabled": false,
        "volna.workspace.autosave": "off",
        "update.mode": "none",
        "extensions.autoUpdate": false,
      }),
    );
    await writeFile(
      join(probe, "package.json"),
      JSON.stringify({
        name: "volna-vsix-probe",
        version: "0.0.0",
        publisher: "vtr-test",
        engines: { vscode: manifest.engines.vscode },
        main: "./probe.cjs",
      }),
    );
    await writeFile(join(probe, "probe.cjs"), "exports.activate = () => {};");
    await copyFile(join(ext, "vsix-driver.cjs"), join(probe, "run.cjs"));
    await copyFile(
      join(ext, "test-support.cjs"),
      join(probe, "test-support.cjs"),
    );
    for (const { name, source, gzip } of traces) {
      if (gzip)
        await writeFile(
          join(root, name),
          gzipSync(await readFile(join(ext, "..", source))),
        );
      else await copyFile(join(ext, "..", source), join(root, name));
    }
    await exec(
      code,
      [
        "--user-data-dir",
        profile,
        "--extensions-dir",
        extensions,
        "--install-extension",
        vsix,
        "--force",
      ],
      { timeout: 30000 },
    );
    const port = await freePort();
    child = spawn(
      "xvfb-run",
      [
        "-a",
        code,
        "--new-window",
        "--wait",
        "--user-data-dir",
        profile,
        "--extensions-dir",
        extensions,
        `--extensionDevelopmentPath=${probe}`,
        `--extensionTestsPath=${join(probe, "run.cjs")}`,
        "--skip-welcome",
        "--skip-release-notes",
        "--disable-workspace-trust",
        `--remote-debugging-port=${port}`,
        "--enable-unsafe-swiftshader",
        "--use-angle=swiftshader",
        root,
      ],
      {
        detached: true,
        env: {
          ...process.env,
          VOLNA_VSIX_TEST_DIR: root,
          VOLNA_EXTENSION_ID: `${manifest.publisher}.${manifest.name}`,
        },
        stdio: ["ignore", "pipe", "pipe"],
      },
    );
    child.stdout.on("data", (bytes) => {
      output = (output + bytes).slice(-12000);
    });
    child.stderr.on("data", (bytes) => {
      output = (output + bytes).slice(-12000);
    });
    child.on("error", (error) => {
      spawnError = error;
    });
    await until(
      async () => {
        if (spawnError) throw spawnError;
        if (child.exitCode !== null) throw await hostExited();
        try {
          return (await fetch(`http://127.0.0.1:${port}/json/version`)).ok;
        } catch {
          return false;
        }
      },
      "VS Code debugging endpoint",
      30000,
    );
    browser = await chromium.connectOverCDP(`http://127.0.0.1:${port}`);
    page = await until(
      () => browser.contexts()[0]?.pages()[0],
      "VS Code window",
    );
    await page.bringToFront();
    const errors = [];
    page.on("pageerror", (error) => errors.push(error.message));
    page.on("console", (message) => {
      if (message.type() === "error" || message.type() === "warning")
        errors.push(message.text());
    });
    for (const [index, { name, viewport }] of traces.entries()) {
      await until(
        async () => {
          if (child.exitCode !== null) throw await hostExited();
          try {
            return (
              JSON.parse(await readFile(join(root, "current.json"), "utf8"))
                .index === index
            );
          } catch {
            return false;
          }
        },
        `default custom editor for ${name}`,
        30000,
      );
      let lastState, viewer;
      try {
        viewer = await until(
          async () => {
            for (const frame of page.frames()) {
              try {
                if (
                  await frame.evaluate(() =>
                    Array.from(document.scripts).some((script) =>
                      script.src.endsWith("/webview.mjs"),
                    ),
                  )
                )
                  return frame;
              } catch {}
            }
          },
          `${name} webview`,
          10000,
        );
        await viewer.waitForSelector("canvas", { timeout: 60000 });
        await viewer.evaluate(async () => {
          const script = Array.from(document.scripts).find((script) =>
            script.src.endsWith("/webview.mjs"),
          );
          window.volnaTestModule = await import(
            new URL("./volna.js", script.src)
          );
        });
        let requestedWaves = false;
        const state = await until(
          async () => {
            const nextState = page
              .waitForEvent("console", {
                predicate: (message) => message.text().includes("STATE "),
                timeout: 1000,
              })
              .then(
                (message) => message.text(),
                () => null,
              );
            await viewer.evaluate(() => window.volnaTestModule.debug_state());
            const state = await nextState;
            lastState = state;
            if (state?.includes("trace_loaded=true") && !requestedWaves) {
              await writeFile(join(root, `trace-loaded-${index}`), "ok");
              requestedWaves = true;
              return null;
            }
            return state?.includes(`viewport=(${viewport.join(",")})`) &&
              /frames .*total=[1-9]\d*/.test(state)
              ? state
              : null;
          },
          `${name} trace metadata loaded through bundled server`,
          30000,
        );
        console.log(`${name}: ${state}`);
        const rendered = await until(async () => {
          const screenshot = await viewer.locator("canvas").screenshot();
          const { data } = PNG.sync.read(screenshot);
          const colors = new Set();
          for (let offset = 0; offset < data.length; offset += 64) {
            colors.add(data.readUInt32LE(offset));
            if (colors.size > 16) return screenshot;
          }
        }, `${name} nonblank viewer pixels`);
        await writeFile(join(results, `${name}-canvas.png`), rendered);
        await page.screenshot({ path: join(results, `${name}.png`) });
        await writeFile(join(root, `passed-${index}`), "ok");
      } catch (error) {
        const hostText = await page
          .locator("body")
          .innerText()
          .catch(() => "");
        await page
          .screenshot({ path: join(results, `${name}-failure.png`) })
          .catch(() => {});
        throw new Error(
          `${error.message}\nLast viewer state:\n${lastState}\nWebview errors:\n${errors.join("\n")}\nVS Code:\n${output}\nHost UI:\n${hostText}`,
          { cause: error },
        );
      }
    }
    await until(
      () => child.exitCode !== null,
      "VS Code test host shutdown",
      15000,
    );
    assert.equal(child.exitCode, 0, output);
    await access(join(root, "done"));
  },
);
