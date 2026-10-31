import { ensureIsolation } from '../isolation.js';
import { Conversion, conversionStatus } from './conversion.js';
import { trackDownload } from './startup.js';

const base = new URL('../', import.meta.url);
const status = document.getElementById('viewer-status');
const statusText = document.getElementById('viewer-message');
const startupProgress = document.getElementById('startup-progress');
const importProgress = document.getElementById('import-progress');
const error = document.getElementById('viewer-error');
const fileInput = document.getElementById('recording-file');
const workspaceInput = document.createElement('input');
workspaceInput.type = 'file';
workspaceInput.accept = '.volna.json,application/json';
workspaceInput.hidden = true;
workspaceInput.setAttribute('aria-label', 'Open a Volna workspace');
document.body.append(workspaceInput);
const sampleUrl = new URL('examples/landing.vtr', base);
const sampleWorkspaceUrl = new URL('examples/landing-page.volna.json', base);
let viewer;
let pendingLocal = null;
let pendingSample = false;
let currentTraceUri = null;
const conversion = new Conversion();
const cancelImport = document.getElementById('cancel-import');
let openRequest = 0;
let openAbort;
const labelViewerInputs = () => {
  document.querySelectorAll('textarea:not([aria-label])').forEach(input => {
    input.setAttribute('aria-label', 'Viewer text input');
  });
};
new MutationObserver(labelViewerInputs).observe(document.body, { childList: true, subtree: true });
const startupTimer = setTimeout(() => showError('The viewer did not start. Check browser graphics support and reload this page.'), 30000);

function showError(message) {
  startupProgress.hidden = true;
  importProgress.hidden = true;
  error.textContent = message;
  error.hidden = false;
  status.hidden = true;
}

function bytesContent(bytes) {
  return { status: 'bytes', value: Array.from(bytes) };
}

function candidates(traceUri, sidecarBytes, workspaceUri = `${traceUri}.volna.json`) {
  const key = `volna.workspace.${traceUri}`;
  let saved = null;
  if (!sidecarBytes) {
    try { saved = localStorage.getItem(key); } catch {}
  }
  return {
    sidecar: {
      target: { kind: 'file', uri: workspaceUri },
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

function beginOpen() {
  conversion.cancel();
  openAbort?.abort();
  openAbort = new AbortController();
  pendingLocal = null;
  pendingSample = false;
  cancelImport.hidden = true;
  importProgress.hidden = true;
  delete document.body.dataset.localTraceOpened;
  delete document.body.dataset.workspaceRestored;
  window.volnaCurrentState = '';
  error.hidden = true;
  return ++openRequest;
}

async function openSample() {
  const request = beginOpen();
  statusText.textContent = 'Loading the sample recording and workspace…';
  status.hidden = false;
  error.hidden = true;
  try {
    const [traceResponse, workspaceResponse] = await Promise.all([
      fetch(sampleUrl, { signal: openAbort.signal }), fetch(sampleWorkspaceUrl, { signal: openAbort.signal }),
    ]);
    if (!traceResponse.ok || !workspaceResponse.ok) throw new Error('Sample files are unavailable.');
    const [trace, workspace] = await Promise.all([
      traceResponse.arrayBuffer(), workspaceResponse.arrayBuffer(),
    ]);
    if (request !== openRequest) return;
    currentTraceUri = sampleUrl.href;
    viewer.open_resource('landing.vtr', new Uint8Array(trace), JSON.stringify({
      traceUri: sampleUrl.href,
      candidates: candidates(sampleUrl.href, new Uint8Array(workspace), sampleWorkspaceUrl.href),
    }));
    pendingSample = true;
    statusText.textContent = 'Opening the sample workspace…';
  } catch (cause) {
    if (request !== openRequest) return;
    pendingSample = false;
    showError(`Could not load the sample: ${cause.message}`);
  }
}

function showConversion(name, snapshot) {
  statusText.textContent = `${name}: ${conversionStatus(snapshot)}`;
  importProgress.hidden = false;
  const fraction = snapshot.phase === 'complete' ? 1 : snapshot.total ? snapshot.completed / snapshot.total : null;
  if (fraction === null) {
    importProgress.removeAttribute('aria-valuenow');
    importProgress.style.removeProperty('--progress');
  } else {
    importProgress.setAttribute('aria-valuenow', String(Math.floor(fraction * 100)));
    importProgress.style.setProperty('--progress', `${fraction * 100}%`);
  }
}

async function openFile(file) {
  if (!file) return;
  const request = beginOpen();
  statusText.textContent = `Opening ${file.name}…`;
  status.hidden = false;
  error.hidden = true;
  try {
    const direct = /\.vtr$/i.test(file.name);
    if (!direct && !/\.(fst|kanata|log|vcd)(\.gz)?$/i.test(file.name)) {
      throw new Error('Choose a VTR, FST, Kanata, LOG, or VCD recording.');
    }
    cancelImport.hidden = direct;
    if (!direct) showConversion(file.name, {
      version: 1, phase: 'preparing', completed: 0, total: null, input_bytes: file.size, output_bytes: 0,
    });
    const bytes = direct ? new Uint8Array(await file.arrayBuffer()) : await conversion.run(file, snapshot => {
      if (request === openRequest) showConversion(file.name, snapshot);
    });
    if (request !== openRequest) return;
    cancelImport.hidden = true;
    importProgress.hidden = true;
    statusText.textContent = `Opening ${file.name}…`;
    const traceUri = new URL(`local/${encodeURIComponent(file.name)}`, base).href;
    pendingLocal = file.name;
    currentTraceUri = traceUri;
    viewer.open_resource(file.name, bytes, JSON.stringify({
      traceUri, candidates: candidates(traceUri),
    }));
  } catch (cause) {
    if (request !== openRequest) return;
    pendingLocal = null;
    cancelImport.hidden = true;
    showError(`Could not open ${file.name}: ${cause.message}`);
  }
}

cancelImport.addEventListener('click', () => {
  beginOpen();
  statusText.textContent = 'Import cancelled.';
  status.hidden = false;
});
window.addEventListener('pagehide', () => { beginOpen(); });

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
  startupProgress.hidden = true;
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
  if (pendingSample && /^panel=\d+ pipeline .* ready /m.test(state)
      && /^panel=\d+ transaction .* ready /m.test(state)
      && /^panel=\d+ table .*rows=[1-9]\d*/m.test(state)) {
    pendingSample = false;
    status.hidden = true;
    document.body.dataset.workspaceRestored = 'true';
  }
};

try {
  await ensureIsolation();
  if (!crossOriginIsolated || typeof SharedArrayBuffer === 'undefined') {
    throw new Error('Background workers require HTTPS and cross-origin isolation. Configure Cross-Origin-Opener-Policy: same-origin and Cross-Origin-Embedder-Policy: require-corp.');
  }
  if (typeof Atomics.waitAsync !== 'function') {
    throw new Error('Background workers require Atomics.waitAsync. Update to a browser that supports it.');
  }
  viewer = await import(new URL('viewer/pkg/volna.js', base));
  statusText.textContent = 'Downloading the Volna viewer…';
  const response = await fetch(new URL('viewer/pkg/volna_bg.wasm', base));
  const wasm = trackDownload(response, fraction => {
    if (fraction === null) {
      startupProgress.removeAttribute('aria-valuenow');
      startupProgress.style.removeProperty('--progress');
    } else {
      startupProgress.setAttribute('aria-valuenow', String(Math.floor(fraction * 100)));
      startupProgress.style.setProperty('--progress', `${fraction * 100}%`);
    }
  }, () => {
    statusText.textContent = 'Starting the Volna viewer…';
    startupProgress.removeAttribute('aria-valuenow');
    startupProgress.style.removeProperty('--progress');
  });
  await viewer.default({ module_or_path: wasm });
} catch (cause) {
  clearTimeout(startupTimer);
  showError(`The viewer could not start: ${cause.message}`);
}
