const test = require("node:test");
const assert = require("node:assert/strict");
const { activate } = require("./extension");

function activated() {
  const commands = new Map();
  let provider;
  const disposable = { dispose() {} };
  const vscode = {
    Uri: {
      joinPath: (base, ...parts) => ({ path: [base.path, ...parts].join("/") }),
    },
    commands: {
      registerCommand: (name, callback) => {
        commands.set(name, callback);
        return disposable;
      },
      executeCommand: async () => {},
    },
    window: {
      registerCustomEditorProvider: (_type, value) => {
        provider = value;
        return disposable;
      },
      showErrorMessage() {},
    },
    workspace: { onDidChangeConfiguration: () => disposable },
  };
  activate(
    {
      subscriptions: [],
      extensionUri: { path: "/extension" },
    },
    vscode,
  );
  return { commands, provider };
}

function panel(active) {
  const posted = [];
  const stateListeners = [];
  const disposeListeners = [];
  const disposable = { dispose() {} };
  return {
    active,
    visible: true,
    posted,
    onDidChangeViewState: (callback) => {
      stateListeners.push(callback);
      return disposable;
    },
    onDidDispose: (callback) => {
      disposeListeners.push(callback);
      return disposable;
    },
    focus(value) {
      this.active = value;
      for (const callback of stateListeners) callback();
    },
    dispose() {
      for (const callback of disposeListeners) callback();
    },
    webview: {
      cspSource: "vscode-webview:",
      asWebviewUri: (uri) => uri.path,
      onDidReceiveMessage: () => disposable,
      postMessage: async (message) => {
        posted.push(message);
        return true;
      },
    },
  };
}

test("volna.undo and volna.redo post their names to the active viewer only", async () => {
  const { commands, provider } = activated();
  const background = panel(false);
  const active = panel(true);
  const uri = {
    path: "/trace.vtr",
    toString: () => "file:///trace.vtr",
    with: () => uri,
  };
  await provider.resolveCustomEditor({ uri }, background);
  await provider.resolveCustomEditor({ uri }, active);
  await commands.get("volna.undo")();
  await commands.get("volna.redo")();
  assert.deepEqual(active.posted, [
    { type: "command", name: "undo" },
    { type: "command", name: "redo" },
  ]);
  assert.deepEqual(background.posted, []);
  active.focus(false);
  background.focus(true);
  await commands.get("volna.undo")();
  assert.deepEqual(background.posted, [{ type: "command", name: "undo" }]);
  background.dispose();
  await commands.get("volna.redo")();
  assert.equal(background.posted.length, 1);
  assert.equal(active.posted.length, 2);
});

test("waveform navigation follows the active editor and stops when it loses focus", async () => {
  const { commands, provider } = activated();
  const first = panel(true);
  const second = panel(false);
  const uri = {
    path: "/trace.vtr",
    toString: () => "file:///trace.vtr",
    with: () => uri,
  };
  await provider.resolveCustomEditor({ uri }, first);
  await provider.resolveCustomEditor({ uri }, second);
  await commands.get("volna.zoomIn")();
  first.focus(false);
  second.focus(true);
  await commands.get("volna.panRight")();
  await commands.get("volna.toggleSidebar")();
  assert.deepEqual(first.posted, [{ type: "command", name: "zoomIn" }]);
  assert.deepEqual(second.posted, [
    { type: "command", name: "panRight" },
    { type: "command", name: "toggleSidebar" },
  ]);
  second.focus(false);
  await commands.get("volna.zoomFit")();
  assert.equal(first.posted.length, 1);
  assert.equal(second.posted.length, 2);
});
