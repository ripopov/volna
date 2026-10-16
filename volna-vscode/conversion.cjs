const { spawn } = require("node:child_process");
const fs = require("node:fs/promises");
const { tmpdir } = require("node:os");
const path = require("node:path");

const PHASES = {
  preparing: "Preparing input",
  reading: "Reading input",
  writing: "Writing VTR",
  finalizing: "Finalizing VTR",
  complete: "Complete",
  failed: "Conversion failed",
};

function formatBytes(bytes) {
  let size = bytes;
  let unit = "B";
  for (const next of ["KiB", "MiB", "GiB", "TiB", "PiB", "EiB"]) {
    if (size < 1024) break;
    size /= 1024;
    unit = next;
  }
  return unit === "B" ? `${bytes} B` : `${size.toFixed(1)} ${unit}`;
}

function readSnapshot(line) {
  const value = JSON.parse(line);
  const count = (n) => Number.isSafeInteger(n) && n >= 0;
  if (
    value?.version !== 1 ||
    typeof value.phase !== "string" ||
    !Object.hasOwn(PHASES, value.phase) ||
    !count(value.completed) ||
    !count(value.input_bytes) ||
    !count(value.output_bytes) ||
    !(
      value.total === null ||
      (count(value.total) && value.total > 0 && value.completed <= value.total)
    )
  )
    throw new Error("Invalid or unsupported converter progress record");
  return value;
}

function convert(executable, input, output, signal, progress, spawnChild) {
  signal.throwIfAborted();
  return new Promise((resolve, reject) => {
    const child = spawnChild(
      executable,
      ["convert", "--progress=json", "--", input, output],
      { stdio: ["ignore", "pipe", "pipe"], windowsHide: true, shell: false },
    );
    let pending = Buffer.alloc(0),
      diagnostics = "",
      failure,
      latest;
    const abort = () => child.kill();
    const fail = (error) => {
      failure ??= error;
      child.kill();
    };
    signal.addEventListener("abort", abort, { once: true });
    if (signal.aborted) abort();
    child.on("error", fail);
    child.stdout.on("error", fail);
    child.stderr.on("error", fail);
    child.stderr.on("data", (bytes) => {
      diagnostics = (diagnostics + bytes.toString("utf8")).slice(-16384);
    });
    child.stdout.on("data", (bytes) => {
      if (failure || signal.aborted) return;
      try {
        pending = Buffer.concat([pending, bytes]);
        let newline;
        while ((newline = pending.indexOf(10)) !== -1) {
          if (newline >= 4096)
            throw new Error("Converter progress record exceeds size limit");
          latest = readSnapshot(pending.subarray(0, newline).toString("utf8"));
          pending = pending.subarray(newline + 1);
          const percent =
            latest.phase === "complete"
              ? "100%"
              : latest.total === null
                ? ""
                : `${Math.floor((latest.completed / latest.total) * 100)}%`;
          progress.report({
            message: `${PHASES[latest.phase]} ${percent} · Input ${formatBytes(latest.input_bytes)} · Output ${formatBytes(latest.output_bytes)}`,
          });
        }
        if (pending.length >= 4096)
          throw new Error("Converter progress record exceeds size limit");
      } catch (error) {
        fail(error);
      }
    });
    // close follows pipe drainage and process exit, so cleanup also works on Windows.
    child.on("close", (code, killedBy) => {
      signal.removeEventListener("abort", abort);
      if (signal.aborted) reject(new Error("Conversion cancelled"));
      else if (failure) reject(failure);
      else if (code !== 0)
        reject(
          new Error(
            `Cannot convert ${path.basename(input)} to VTR (${killedBy || code}).\n${diagnostics.trim()}`,
          ),
        );
      else if (pending.length || latest?.phase !== "complete")
        reject(new Error("Converter exited without completed progress"));
      else resolve();
    });
  });
}

async function prepareTrace(vscode, context, uri, signal, spawnChild = spawn) {
  if (!/\.(fst|kanata|log)(\.gz)?$/i.test(uri.fsPath))
    return { path: uri.fsPath, dispose: async () => {} };
  signal.throwIfAborted();
  const directory = await fs.mkdtemp(path.join(tmpdir(), "volna-import-"));
  // The server exposes this basename as the recording name.
  const output = path.join(directory, path.basename(uri.fsPath));
  const dispose = () =>
    fs.rm(directory, { recursive: true, force: true, maxRetries: 5 });
  try {
    await vscode.window.withProgress(
      {
        location: vscode.ProgressLocation.Notification,
        title: `Converting ${path.basename(uri.fsPath)}`,
        cancellable: true,
      },
      async (progress, token) => {
        const cancellation = new AbortController();
        const subscription = token.onCancellationRequested(() =>
          cancellation.abort(),
        );
        try {
          if (token.isCancellationRequested) cancellation.abort();
          await convert(
            path.join(
              context.extensionPath,
              "bin",
              process.platform === "win32" ? "vtr.exe" : "vtr",
            ),
            uri.fsPath,
            output,
            AbortSignal.any([signal, cancellation.signal]),
            progress,
            spawnChild,
          );
        } finally {
          subscription.dispose();
        }
      },
    );
    signal.throwIfAborted();
    return { path: output, dispose };
  } catch (error) {
    await dispose();
    throw error;
  }
}

module.exports = { prepareTrace };
