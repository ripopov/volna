export async function ensureIsolation() {
  if (crossOriginIsolated) return;
  if (window.top !== window) {
    // The top-level page must reload into isolation before any iframe can share memory.
    return new Promise(() => {});
  }
  if (!isSecureContext || !navigator.serviceWorker) {
    throw new Error('Background workers require HTTPS and cross-origin isolation.');
  }
  const reloadKey = 'volna-isolation-reload';
  if (sessionStorage.getItem(reloadKey)) {
    sessionStorage.removeItem(reloadKey);
    throw new Error('Cross-origin isolation is unavailable. Enable COOP: same-origin and COEP: require-corp on the server.');
  }
  await navigator.serviceWorker.register(new URL('isolation-worker.js', import.meta.url));
  await navigator.serviceWorker.ready;
  if (!navigator.serviceWorker.controller) {
    await new Promise(resolve => navigator.serviceWorker.addEventListener('controllerchange', resolve, { once: true }));
  }
  sessionStorage.setItem(reloadKey, '1');
  location.reload();
  await new Promise(() => {});
}

if (crossOriginIsolated) {
  try { sessionStorage.removeItem('volna-isolation-reload'); } catch {}
}
if (document.querySelector('iframe[title="Interactive Volna sample recording"]')) {
  ensureIsolation().catch(error => console.error(error.message));
}
