const { randomUUID } = require("node:crypto");
const MAX_WORKSPACE_BYTES = 16 * 1024 * 1024;
const storageKey = (uri) => `volna.workspace:${uri.toString()}`;
const fileTarget = (uri) => ({ kind: "file", uri: uri.toString() });

function storedContent(value) {
  if (value === undefined) return { status: "missing" };
  if (typeof value !== "string") {
    return { status: "error", value: "invalid workspace storage value" };
  }
  if (Buffer.byteLength(value, "utf8") > MAX_WORKSPACE_BYTES) {
    return { status: "error", value: "workspace exceeds size limit" };
  }
  return { status: "bytes", value: Array.from(Buffer.from(value, "utf8")) };
}

function createWorkspaceHost(vscode, context, panel, uri) {
  const webview = panel.webview;
  const sidecar = uri.with({ path: `${uri.path}.volna.json` });
  let disposed = false;
  let writes = Promise.resolve();
  const settings = () => collectSettings(vscode, uri);
  const post = async (message) => {
    if (disposed) return false;
    try {
      return await webview.postMessage(message);
    } catch (error) {
      if (!disposed) {
        vscode.window.showErrorMessage(
          `Volna host message failed: ${error.message}`,
        );
      }
      return false;
    }
  };

  async function readContent(location) {
    try {
      const data = await vscode.workspace.fs.readFile(location);
      if (data.byteLength > MAX_WORKSPACE_BYTES)
        throw new Error("workspace exceeds size limit");
      return { status: "bytes", value: Array.from(data) };
    } catch (error) {
      return error.code === "FileNotFound"
        ? { status: "missing" }
        : { status: "error", value: error.message };
    }
  }
  async function probeWritable(location) {
    if (vscode.workspace.fs.isWritableFileSystem(location.scheme) === false)
      return false;
    const probe = location.with({
      path: `${location.path}.probe-${randomUUID()}`,
    });
    try {
      await vscode.workspace.fs.writeFile(probe, new Uint8Array());
      await vscode.workspace.fs.delete(probe, { useTrash: false });
      return true;
    } catch {
      return false;
    }
  }
  async function candidates(policy) {
    if (policy === "off") return undefined;
    const [sideContent, canWrite] = await Promise.all([
      readContent(sidecar),
      policy === "vscode" ? false : probeWritable(sidecar),
    ]);
    const stored = context.workspaceState.get(storageKey(uri));
    return {
      sidecar: {
        target: fileTarget(sidecar),
        content: sideContent,
        writable: canWrite,
      },
      fallback: {
        target: { kind: "storage", key: storageKey(uri) },
        content: storedContent(stored),
        writable: true,
      },
    };
  }
  async function ready() {
    const config = settings();
    const saved = await candidates(config["workspace.autosave"] ?? "sidecar");
    await post({
      type: "open",
      traceUri: uri.toString(),
      name: uri.path.split("/").pop(),
      candidates: saved,
      settings: config,
    });
  }
  function save(snapshot) {
    const { ticket, json } = snapshot;
    writes = writes.then(async () => {
      let error;
      try {
        if (Buffer.byteLength(json, "utf8") > MAX_WORKSPACE_BYTES)
          throw new Error("workspace exceeds size limit");
        if (ticket.target.kind === "file") {
          await vscode.workspace.fs.writeFile(
            vscode.Uri.parse(ticket.target.uri),
            Buffer.from(json, "utf8"),
          );
        } else if (ticket.target.kind === "storage") {
          await context.workspaceState.update(ticket.target.key, json);
        } else throw new Error("unsupported workspace destination");
      } catch (e) {
        error = e.message;
      }
      if (disposed && error) {
        vscode.window.showErrorMessage(`Volna workspace not saved: ${error}`);
      } else {
        await post({ type: "saved", ticket, ...(error ? { error } : {}) });
      }
    });
    return writes;
  }
  async function receive(message) {
    if (disposed) return;
    switch (message.type) {
      case "ready":
        return ready();
      case "workspace":
        return save(message);
      case "saveWorkspaceAs": {
        const picked = await vscode.window.showSaveDialog({
          defaultUri: sidecar,
          filters: { "Volna workspace": ["volna.json"] },
        });
        if (picked)
          return post({
            type: "workspaceDestination",
            target: fileTarget(picked),
          });
        break;
      }
      case "openWorkspace": {
        const picked = await vscode.window.showOpenDialog({
          canSelectMany: false,
          filters: { "Volna workspace": ["volna.json"] },
          openLabel: "Open Workspace",
        });
        if (picked?.[0])
          return post({
            type: "workspace",
            candidate: {
              target: fileTarget(picked[0]),
              content: await readContent(picked[0]),
              writable: true,
            },
          });
        break;
      }
      case "closeTrace": {
        if (message.traceUri !== uri.toString()) break;
        const tab = vscode.window.tabGroups.all
          .flatMap((group) => group.tabs)
          .find(
            (tab) =>
              tab.input?.uri?.toString() === uri.toString() &&
              tab.input?.viewType === "volna.waveform",
          );
        if (tab) await vscode.window.tabGroups.close(tab);
        break;
      }
      case "notice":
        vscode.window.showWarningMessage(`Volna: ${message.text}`);
        break;
    }
  }
  function hidden() {
    return post({ type: "requestWorkspace" });
  }
  function settingsChanged() {
    return post({ type: "settings", settings: settings() });
  }
  function dispose() {
    disposed = true;
    return writes;
  }
  return { receive, hidden, dispose, settingsChanged };
}
const SETTING_KEYS = Object.keys(
  require("./package.json").contributes.configuration.properties,
)
  .map((key) => key.slice("volna.".length))
  .filter((key) => key !== "remote.serverPath");
function collectSettings(vscode, uri) {
  const config = vscode.workspace.getConfiguration("volna", uri);
  const out = {};
  for (const key of SETTING_KEYS) {
    const value = config.get(key);
    if (value !== undefined) out[key] = value;
  }
  return out;
}
module.exports = { createWorkspaceHost, storageKey };
