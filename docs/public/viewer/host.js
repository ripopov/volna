const base = new URL('../', import.meta.url);
const status = document.getElementById('viewer-status');
const error = document.getElementById('viewer-error');
const fileInput = document.getElementById('recording-file');
const workspaceInput = document.createElement('input');
workspaceInput.type = 'file';
workspaceInput.accept = '.volna.json,application/json';
workspaceInput.hidden = true;
workspaceInput.setAttribute('aria-label', 'Open a Volna workspace');
document.body.append(workspaceInput);
const sampleUrl = new URL('examples/landing.vtr', base);
let viewer;
let pendingLocal = null;
let pendingSample = false;
let currentTraceUri = null;
// GPUI adds an IME mirror textarea asynchronously; keep it named for assistive technology.
const labelViewerInputs = () => {
  document.querySelectorAll('textarea:not([aria-label])').forEach(input => {
    input.setAttribute('aria-label', 'Viewer text input');
  });
};
new MutationObserver(labelViewerInputs).observe(document.body, { childList: true, subtree: true });
const startupTimer = setTimeout(() => showError('The viewer did not start. Check browser graphics support and reload this page.'), 30000);

function showError(message) {
  error.textContent = message;
  error.hidden = false;
  status.hidden = true;
}

function bytesContent(bytes) {
  return { status: 'bytes', value: Array.from(bytes) };
}

function candidates(traceUri, sidecarBytes) {
  const key = `volna.workspace.${traceUri}`;
  let saved = null;
  if (!sidecarBytes) {
    try { saved = localStorage.getItem(key); } catch { /* Private browsing can disable storage. */ }
  }
  return {
    sidecar: {
      target: { kind: 'file', uri: `${traceUri}.volna.json` },
      content: sidecarBytes ? bytesContent(sidecarBytes) : { status: 'missing' },
      writable: false,
    },
    fallback: {
      target: { kind: 'storage', key },
      content: saved ? bytesContent(new TextEncoder().encode(saved)) : { status: 'missing' },
      writable: true,
    },
  };
}

async function openSample() {
  pendingSample = true;
  window.volnaCurrentState = '';
  status.textContent = 'Loading the sample recording and workspace…';
  status.hidden = false;
  error.hidden = true;
  try {
    const [traceResponse, workspaceResponse] = await Promise.all([
      fetch(sampleUrl), fetch(new URL('examples/landing.vtr.volna.json', base)),
    ]);
    if (!traceResponse.ok || !workspaceResponse.ok) throw new Error('Sample files are unavailable.');
    const [trace, workspace] = await Promise.all([
      traceResponse.arrayBuffer(), workspaceResponse.arrayBuffer(),
    ]);
    currentTraceUri = sampleUrl.href;
    viewer.open_resource('landing.vtr', new Uint8Array(trace), JSON.stringify({
      traceUri: sampleUrl.href,
      candidates: candidates(sampleUrl.href, new Uint8Array(workspace)),
    }));
    status.textContent = 'Opening the sample workspace…';
  } catch (cause) {
    pendingSample = false;
    showError(`Could not load the sample: ${cause.message}`);
  }
}

async function openFile(file) {
  if (!file) return;
  window.volnaCurrentState = '';
  status.textContent = `Opening ${file.name}…`;
  status.hidden = false;
  error.hidden = true;
  try {
    const traceUri = new URL(`local/${encodeURIComponent(file.name)}`, base).href;
    pendingLocal = file.name;
    currentTraceUri = traceUri;
    viewer.open_resource(file.name, new Uint8Array(await file.arrayBuffer()), JSON.stringify({
      traceUri, candidates: candidates(traceUri),
    }));
  } catch (cause) {
    pendingLocal = null;
    showError(`Could not open ${file.name}: ${cause.message}`);
  }
}

window.volnaOpen = () => fileInput.click();
fileInput.addEventListener('change', () => {
  void openFile(fileInput.files[0]);
  fileInput.value = '';
});
workspaceInput.addEventListener('change', async () => {
  const file = workspaceInput.files[0];
  workspaceInput.value = '';
  if (!file || !currentTraceUri) return;
  try {
    viewer.workspace_message(JSON.stringify({
      type: 'workspace',
      candidate: {
        target: { kind: 'file', uri: `${currentTraceUri}.volna.json` },
        content: bytesContent(new Uint8Array(await file.arrayBuffer())),
        writable: false,
      },
    }));
  } catch (cause) { showError(`Could not open workspace: ${cause.message}`); }
});
document.getElementById('open-recording')?.addEventListener('click', window.volnaOpen);
document.getElementById('load-sample')?.addEventListener('click', () => void openSample());

window.volnaWorkspace = json => {
  const message = JSON.parse(json);
  if (message.type === 'workspace') {
    let failure = null;
    try {
      if (message.ticket.target.kind === 'storage') {
        localStorage.setItem(message.ticket.target.key, message.json);
      } else {
        const url = URL.createObjectURL(new Blob([message.json], { type: 'application/json' }));
        const link = document.createElement('a');
        link.href = url;
        link.download = `${decodeURIComponent(new URL(message.ticket.target.uri).pathname.split('/').pop())}`;
        link.click();
        setTimeout(() => URL.revokeObjectURL(url), 60000);
      }
    } catch (cause) { failure = `Workspace storage failed: ${cause.message}`; }
    viewer.workspace_message(JSON.stringify({ type: 'saved', ticket: message.ticket, error: failure }));
  } else if (message.type === 'notice') {
    showError(message.text);
  } else if (message.type === 'openWorkspace') {
    if (currentTraceUri) workspaceInput.click();
    else showError('Open a recording before opening its workspace.');
  } else if (message.type === 'saveWorkspaceAs') {
    if (currentTraceUri) viewer.workspace_message(JSON.stringify({
      type: 'workspaceDestination',
      target: { kind: 'file', uri: `${currentTraceUri}.volna.json` },
    }));
    else showError('Open a recording before saving its workspace.');
  }
};

window.volnaReady = () => {
  clearTimeout(startupTimer);
  status.hidden = true;
  document.querySelectorAll('#viewer-actions button').forEach(button => { button.disabled = false; });
  if (document.body.dataset.sample === 'true') void openSample();
};

window.volnaViewerState = state => {
  window.volnaCurrentState = state;
  if (pendingLocal) {
    document.body.dataset.localTraceOpened = pendingLocal;
    pendingLocal = null;
    status.hidden = true;
  }
  if (pendingSample && state.includes('panel=2 pipeline') && state.includes('panel=3 transaction') && state.includes('panel=4 table')) {
    pendingSample = false;
    status.hidden = true;
    document.body.dataset.workspaceRestored = 'true';
  }
};

try {
  viewer = await import(new URL('viewer/pkg/volna.js', base));
  await viewer.default(new URL('viewer/pkg/volna_bg.wasm', base));
} catch (cause) {
  clearTimeout(startupTimer);
  showError(`The viewer could not start: ${cause.message}`);
}
