import init, * as viewer from "./volna.js";
import { watchTheme } from "./theme.mjs";

const host = acquireVsCodeApi();
const status = document.getElementById("viewer-status");
const send = (message) => host.postMessage(message);

window.volnaEmbedded = true;
window.volnaOpen = () => send({ type: "pickFile" });
window.volnaReady = () => {
  status.hidden = true;
  send({ type: "ready" });
};
window.volnaWorkspace = (envelope) => send(JSON.parse(envelope));
window.volnaTraceStart = (connection) =>
  send({ type: "traceStart", connection });
window.volnaTraceSend = (connection, bytes) =>
  send({ type: "traceRequest", connection, bytes });
window.volnaTraceStop = (connection) => send({ type: "traceStop", connection });

const continuation = new MessageChannel();
continuation.port1.onmessage = (event) => viewer.trace_continue(event.data);
window.volnaTraceYield = (connection) =>
  continuation.port2.postMessage(connection);

window.addEventListener("message", ({ data: message }) => {
  if (!message) return;
  try {
    switch (message.type) {
      case "open":
        viewer.open_remote(
          message.name,
          JSON.stringify({
            traceUri: message.traceUri,
            candidates: message.candidates,
            settings: message.settings,
          }),
        );
        break;
      case "traceFrame":
        viewer.trace_frame(message.connection, new Uint8Array(message.bytes));
        break;
      case "traceError":
        viewer.trace_error(message.connection, message.message);
        break;
      case "command":
        viewer.dispatch_command(message.name);
        break;
      case "settings":
        viewer.set_settings(JSON.stringify(message.settings));
        break;
      case "saved":
      case "workspace":
      case "workspaceDestination":
      case "requestWorkspace":
        viewer.workspace_message(JSON.stringify(message));
        break;
    }
  } catch (error) {
    send({ type: "notice", text: error.message });
  }
});

function updatePageTheme(snapshot) {
  const kind = snapshot.split("\n", 1)[0];
  document.documentElement.dataset.theme = kind.includes("light")
    ? "light"
    : "dark";
}

const themes = watchTheme((snapshot) => {
  updatePageTheme(snapshot);
  viewer.set_vscode_theme(snapshot);
});
window.volnaVscodeTheme = () => {
  const snapshot = themes.current();
  updatePageTheme(snapshot);
  return snapshot;
};

const labelInputs = () => {
  for (const input of document.querySelectorAll("textarea:not([aria-label])")) {
    input.setAttribute("aria-label", "Viewer text input");
  }
};
new MutationObserver(labelInputs).observe(document.body, {
  childList: true,
  subtree: true,
});

try {
  await init();
  themes.start();
} catch (error) {
  status.hidden = false;
  status.textContent = `Volna could not start: ${error?.message ?? String(error)}`;
  send({ type: "notice", text: status.textContent });
}
