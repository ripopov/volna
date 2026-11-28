export async function ensureIsolation() {
  if (crossOriginIsolated) return;
  if (window.top !== window) {
    // The top-level page must reload into isolation before any iframe can share memory.
    return new Promise(() => {});
  }
  if (!isSecureContext || !navigator.serviceWorker) {
    throw new Error('Background workers require HTTPS and cross-origin isolation.');
  }
  // Track the reload guard in history.state rather than sessionStorage: COOP swaps browsing context
  // groups, leaving a stale sessionStorage flag in the non-isolated group after a later hard reload.
  const state = history.state && typeof history.state === 'object' ? history.state : {};
  if (state.volnaIsolationReload) {
    history.replaceState({ ...state, volnaIsolationReload: false }, '');
    throw new Error('Cross-origin isolation is unavailable. Enable COOP: same-origin and COEP: require-corp on the server.');
  }
  await navigator.serviceWorker.register(new URL('isolation-worker.js', import.meta.url));
  await navigator.serviceWorker.ready;
  // A hard reload bypasses the active worker, leaving controller null without firing controllerchange;
  // once ready resolves, a normal reload attaches the active worker.
  history.replaceState({ ...state, volnaIsolationReload: true }, '');
  location.reload();
  await new Promise(() => {});
}

if (crossOriginIsolated && history.state?.volnaIsolationReload) {
  try { history.replaceState({ ...history.state, volnaIsolationReload: false }, ''); } catch {}
}
if (document.querySelector('iframe[title="Interactive Volna sample recording"]')) {
  ensureIsolation().catch(error => console.error(error.message));
}
