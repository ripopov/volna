const test = require("node:test");
const assert = require("node:assert/strict");
const { spawn } = require("node:child_process");
const { EventEmitter } = require("node:events");
const fs = require("node:fs/promises");
const { tmpdir } = require("node:os");
const path = require("node:path");
const { prepareTrace } = require("./conversion.cjs");
const { createTraceHost } = require("./trace-host.cjs");
const { until } = require("./test-support.cjs");

const snapshot = (phase = "complete", completed = 0, total = null) => ({
  version: 1,
  phase,
  completed,
  total,
  input_bytes: 4096,
  output_bytes: 2048,
});
const record = (value) =>
  `process.stdout.write(${JSON.stringify(JSON.stringify(value) + "\n")});`;
const success = `require('node:fs').writeFileSync(process.argv.at(-1), 'converted');${record(snapshot())}`;
const request = () => {
  const bytes = Buffer.alloc(13);
  bytes.write("VLNA");
  bytes.writeUInt32LE(1, 4);
  bytes.writeUInt32LE(1, 8);
  return bytes;
};

function fixture(script = success) {
  const reports = [],
    launches = [],
    children = [],
    messages = [],
    notifications = [];
  const cancellation = new EventEmitter();
  let cancelled = false;
  const vscode = {
    ProgressLocation: { Notification: 15 },
    window: {
      showWarningMessage: (message) => assert.fail(message),
      withProgress: async (options, task) => {
        notifications.push(options);
        return task(
          { report: (value) => reports.push(value) },
          {
            get isCancellationRequested() {
              return cancelled;
            },
            onCancellationRequested(callback) {
              cancellation.on("cancel", callback);
              return { dispose: () => cancellation.off("cancel", callback) };
            },
          },
        );
      },
    },
    workspace: { getConfiguration: () => ({ get: () => "" }) },
  };
  const context = {
    extensionPath: path.join(tmpdir(), "extension with spaces"),
  };
  const uri = {
    scheme: "vscode-remote",
    fsPath: path.join(tmpdir(), "a file '$(test).fst"),
  };
  const panel = {
    webview: {
      postMessage: async (message) => {
        messages.push(message);
        return true;
      },
    },
  };
  const server = `
    require('node:fs').accessSync(process.argv[1]);
    const bytes = Buffer.alloc(13);
    bytes.write('VLNA'); bytes.writeUInt32LE(1, 4); bytes.writeUInt32LE(1, 8);
    process.stdin.once('data', (request) => {
      require('node:assert/strict').equal(request.toString('utf8', 0, 4), 'VLNA');
      process.stdout.write(bytes);
    });
  `;
  const spawnChild = (executable, args, options) => {
    launches.push({ executable, args, options });
    const child = spawn(
      process.execPath,
      ["-e", args[0] === "convert" ? script : server, ...args],
      options,
    );
    children.push(child);
    return child;
  };
  return {
    vscode,
    context,
    uri,
    reports,
    launches,
    children,
    messages,
    notifications,
    spawnChild,
    cancel: () => {
      cancelled = true;
      cancellation.emit("cancel");
    },
    host: createTraceHost(vscode, context, panel, uri, spawnChild),
  };
}

test("conversion uses the bundled CLI and retains output until the remote server closes", async (t) => {
  const f = fixture(`${record(snapshot("reading", 42, 100))}${success}`);
  t.after(() => f.host.dispose());
  const opening = f.host.receive({ type: "traceStart", connection: "A" });
  const queued = f.host.receive({
    type: "traceRequest",
    connection: "A",
    bytes: request(),
  });
  await opening;
  await queued;
  await until(
    () => f.messages.some((message) => message.type === "traceFrame"),
    "server frame",
  );
  assert.match(f.launches[0].executable, /[/\\]bin[/\\]vtr(?:\.exe)?$/);
  assert.deepEqual(f.launches[0].args.slice(0, 4), [
    "convert",
    "--progress=json",
    "--",
    f.uri.fsPath,
  ]);
  assert.equal(f.launches[0].options.shell, false);
  assert.equal(f.launches[0].options.windowsHide, true);
  const output = f.launches[0].args.at(-1);
  assert.equal(path.basename(output), path.basename(f.uri.fsPath));
  assert.equal(await fs.readFile(output, "utf8"), "converted");
  assert.deepEqual(f.launches[1].args, [output]);
  assert.match(
    f.launches[1].executable,
    /[/\\]bin[/\\]volna-server(?:\.exe)?$/,
  );
  assert.match(
    f.reports[0].message,
    /Reading input 42%.*Input 4.0 KiB.*Output 2.0 KiB/,
  );
  assert.equal(f.notifications[0].cancellable, true);
  await f.host.dispose();
  await assert.rejects(fs.access(path.dirname(output)), { code: "ENOENT" });
  assert.notEqual(f.children[1].signalCode, null);
});

test("failed or malformed conversion never starts the server and removes its private directory", async () => {
  for (const script of [
    "process.stderr.write('unsupported input'); process.exitCode = 3;",
    record({ ...snapshot(), version: 999 }),
    record({ ...snapshot(), completed: 101, total: 100 }),
    "process.stdout.write('x'.repeat(5000)); setInterval(() => {}, 1000);",
    "process.stdout.write('{');",
    record(snapshot("reading", 1, 2)),
  ]) {
    const f = fixture(script);
    await f.host.receive({ type: "traceStart", connection: "A" });
    await until(() => f.messages.length, "conversion failure");
    assert.equal(f.messages[0].type, "traceError");
    assert.equal(f.launches.length, 1);
    await assert.rejects(fs.access(path.dirname(f.launches[0].args.at(-1))), {
      code: "ENOENT",
    });
    if (script.includes("unsupported input"))
      assert.match(f.messages[0].message, /unsupported input/);
    await f.host.dispose();
  }
});

test("notification cancellation and editor disposal kill conversion before deleting its output", async () => {
  for (const cancelNotification of [true, false]) {
    const f = fixture(
      `${record(snapshot("reading", 1, 2))}setInterval(() => {}, 1000);`,
    );
    const opening = f.host.receive({ type: "traceStart", connection: "A" });
    await until(() => f.reports.length, "conversion progress");
    if (cancelNotification) f.cancel();
    else await f.host.dispose();
    await opening;
    assert.notEqual(f.children[0].signalCode, null);
    assert.equal(f.launches.length, 1);
    await assert.rejects(fs.access(path.dirname(f.launches[0].args.at(-1))), {
      code: "ENOENT",
    });
    if (cancelNotification) assert.match(f.messages[0].message, /cancelled/);
    else assert.equal(f.messages.length, 0);
  }
});

test("missing converter and server executables report errors and remove converted files", async (t) => {
  for (const missing of ["converter", "server"]) {
    const f = fixture();
    const launches = [];
    const spawnChild = (executable, args, options) => {
      launches.push(args);
      if ((args[0] === "convert") === (missing === "converter"))
        return spawn(
          path.join(f.context.extensionPath, "missing-executable"),
          args,
          options,
        );
      return f.spawnChild(executable, args, options);
    };
    const panel = {
      webview: {
        postMessage: async (message) => {
          f.messages.push(message);
          return true;
        },
      },
    };
    const host = createTraceHost(f.vscode, f.context, panel, f.uri, spawnChild);
    t.after(() => host.dispose());
    await host.receive({ type: "traceStart", connection: "A" });
    await until(() => f.messages.length, "executable startup failure");
    assert.equal(f.messages[0].type, "traceError");
    assert.match(f.messages[0].message, /ENOENT/);
    const directory = path.dirname(launches[0].at(-1));
    await until(
      async () =>
        fs.access(directory).then(
          () => false,
          () => true,
        ),
      "temporary recording cleanup",
    );
  }
});

test("superseded conversions cannot launch a stale server or remove the new recording", async (t) => {
  const f = fixture(
    `${record(snapshot("reading", 1, 2))}setTimeout(() => {${success}}, 500);`,
  );
  t.after(() => f.host.dispose());
  const first = f.host.receive({ type: "traceStart", connection: "A" });
  const oldRequest = f.host.receive({
    type: "traceRequest",
    connection: "A",
    bytes: request(),
  });
  await until(() => f.reports.length, "first conversion progress");
  const replacement = f.host.receive({ type: "traceStart", connection: "B" });
  const newRequest = f.host.receive({
    type: "traceRequest",
    connection: "B",
    bytes: request(),
  });
  await replacement;
  await newRequest;
  await first;
  await oldRequest;
  await until(
    () => f.messages.some((message) => message.type === "traceFrame"),
    "replacement server",
  );
  assert.equal(f.launches.length, 3);
  assert.ok(f.messages.every((message) => message.connection === "B"));
  const oldOutput = f.launches[0].args.at(-1),
    newOutput = f.launches[1].args.at(-1);
  assert.notEqual(oldOutput, newOutput);
  await assert.rejects(fs.access(path.dirname(oldOutput)), { code: "ENOENT" });
  await fs.access(newOutput);
  await f.host.receive({ type: "traceStop", connection: "A" });
  await fs.access(newOutput);
  await f.host.dispose();
  await assert.rejects(fs.access(path.dirname(newOutput)), { code: "ENOENT" });
});

test("VTR opens directly and all supported compressed import extensions convert", async () => {
  const f = fixture();
  const direct = { ...f.uri, fsPath: "/remote/direct.vtr" };
  const recording = await prepareTrace(
    f.vscode,
    f.context,
    direct,
    new AbortController().signal,
    () => assert.fail("converter started for VTR"),
  );
  assert.equal(recording.path, direct.fsPath);
  await recording.dispose();
  for (const suffix of [
    "fst",
    "fst.gz",
    "kanata",
    "kanata.gz",
    "log",
    "log.gz",
    "KANATA.GZ",
  ]) {
    const uri = { ...f.uri, fsPath: `/remote/trace.${suffix}` };
    const converted = await prepareTrace(
      f.vscode,
      f.context,
      uri,
      new AbortController().signal,
      f.spawnChild,
    );
    assert.equal(await fs.readFile(converted.path, "utf8"), "converted");
    await converted.dispose();
  }
});
