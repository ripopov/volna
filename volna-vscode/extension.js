const { randomBytes } = require("node:crypto");
const { createWorkspaceHost } = require("./workspace");
const { createTraceHost } = require("./trace-host.cjs");
const manifest = require("./package.json");

const FORWARDED_COMMANDS = manifest.contributes.commands
  .map(({ command }) => command.slice("volna.".length))
  .filter((name) => !["open", "openSettings"].includes(name));

function webviewHtml(vscode, webview, extensionUri) {
  const media = vscode.Uri.joinPath(extensionUri, "media");
  const resource = (name) =>
    webview.asWebviewUri(vscode.Uri.joinPath(media, name));
  const nonce = randomBytes(24).toString("base64");
  const policy = [
    "default-src 'none'",
    `img-src ${webview.cspSource} blob: data:`,
    `style-src ${webview.cspSource} 'unsafe-inline'`,
    `script-src 'nonce-${nonce}' 'wasm-unsafe-eval' ${webview.cspSource}`,
    `connect-src ${webview.cspSource}`,
    `font-src ${webview.cspSource}`,
  ].join("; ");
  return `<!doctype html>
<html lang="en" data-theme="dark">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <meta http-equiv="Content-Security-Policy" content="${policy}">
  <title>Volna Waveform Viewer</title>
  <link rel="stylesheet" href="${resource("design/styles.css")}">
  <link rel="stylesheet" href="${resource("webview.css")}">
</head>
<body>
  <p id="viewer-status" role="status">Starting Volna…</p>
  <noscript>Enable JavaScript to use the waveform viewer.</noscript>
  <script type="module" nonce="${nonce}" src="${resource("webview.mjs")}"></script>
</body>
</html>`;
}

async function pickTrace(vscode) {
  const picked = await vscode.window.showOpenDialog({
    canSelectMany: false,
    filters: { "Waveform traces": ["vtr", "fst"] },
    openLabel: "Open",
  });
  if (picked?.[0]) {
    await vscode.commands.executeCommand(
      "vscode.openWith",
      picked[0],
      "volna.waveform",
    );
  }
}

function openSettings(vscode) {
  return vscode.commands.executeCommand(
    "workbench.action.openSettings",
    `@ext:${manifest.publisher}.${manifest.name}`,
  );
}

function attachViewer(vscode, panel, context, uri) {
  const workspace = createWorkspaceHost(vscode, context, panel, uri);
  const trace = createTraceHost(vscode, context, panel, uri);
  const configuration = vscode.workspace.onDidChangeConfiguration((event) => {
    if (event.affectsConfiguration("volna", uri)) workspace.settingsChanged();
  });
  const visibility = panel.onDidChangeViewState(() => {
    if (!panel.visible) workspace.hidden();
  });
  const messages = panel.webview.onDidReceiveMessage(async (message) => {
    try {
      if (message.type === "pickFile") await pickTrace(vscode);
      else if (message.type === "openSettings") await openSettings(vscode);
      else if (
        ["traceStart", "traceRequest", "traceStop"].includes(message.type)
      )
        await trace.receive(message);
      else await workspace.receive(message);
    } catch (error) {
      vscode.window.showErrorMessage(`Volna: ${error.message}`);
    }
  });
  panel.onDidDispose(() => {
    configuration.dispose();
    visibility.dispose();
    messages.dispose();
    trace.dispose();
    workspace.dispose();
  });
  panel.webview.options = {
    enableScripts: true,
    localResourceRoots: [vscode.Uri.joinPath(context.extensionUri, "media")],
  };
  panel.webview.html = webviewHtml(vscode, panel.webview, context.extensionUri);
}

function activate(context, vscode = require("vscode")) {
  let activePanel;
  context.subscriptions.push(
    vscode.commands.registerCommand("volna.open", () => pickTrace(vscode)),
  );
  context.subscriptions.push(
    vscode.commands.registerCommand("volna.openSettings", () =>
      openSettings(vscode),
    ),
  );
  for (const name of FORWARDED_COMMANDS) {
    context.subscriptions.push(
      vscode.commands.registerCommand(`volna.${name}`, () =>
        activePanel?.webview.postMessage({ type: "command", name }),
      ),
    );
  }
  context.subscriptions.push(
    vscode.window.registerCustomEditorProvider(
      "volna.waveform",
      {
        async openCustomDocument(uri) {
          return { uri, dispose() {} };
        },
        async resolveCustomEditor(document, panel) {
          if (panel.active) activePanel = panel;
          panel.onDidChangeViewState(() => {
            if (panel.active) activePanel = panel;
            else if (activePanel === panel) activePanel = undefined;
          });
          panel.onDidDispose(() => {
            if (activePanel === panel) activePanel = undefined;
          });
          attachViewer(vscode, panel, context, document.uri);
        },
      },
      {
        webviewOptions: { retainContextWhenHidden: true },
        supportsMultipleEditorsPerDocument: false,
      },
    ),
  );
}

module.exports = { activate };
