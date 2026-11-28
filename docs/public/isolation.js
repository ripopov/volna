function waitForLoad() {
  if (document.readyState === 'complete') return Promise.resolve();
  return new Promise(resolve => {
    const done = () => resolve();
    window.addEventListener('load', done, { once: true });
    setTimeout(done, 1000);
  });
}

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
  // once ready resolves, wait for the current load to settle before reloading so Chromium does not
  // drop location.reload() during an in-progress hard reload or right after history.replaceState().
  await waitForLoad();
  history.replaceState({ ...state, volnaIsolationReload: true }, '');
  await new Promise((_, reject) => {
    setTimeout(() => location.reload(), 0);
    setTimeout(() => location.replace(location.href), 3000);
    setTimeout(() => {
      history.replaceState({ ...state, volnaIsolationReload: false }, '');
      reject(new Error('Could not reload into cross-origin isolation. Please reload this page.'));
    }, 8000);
  });
}

if (crossOriginIsolated && history.state?.volnaIsolationReload) {
  try { history.replaceState({ ...history.state, volnaIsolationReload: false }, ''); } catch {}
}
if (document.querySelector('iframe[title="Interactive Volna sample recording"]')) {
  ensureIsolation().catch(error => console.error(error.message));
}
