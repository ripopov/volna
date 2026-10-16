globalThis.onmessage = async ({ data: { file } }) => {
  let lastPhase;
  let lastTime = -Infinity;
  globalThis.volnaConversionProgress = (json) => {
    const snapshot = JSON.parse(json);
    const now = performance.now();
    if (snapshot.phase !== lastPhase || now - lastTime >= 100) {
      postMessage({ type: 'progress', snapshot });
      lastPhase = snapshot.phase;
      lastTime = now;
    }
  };
  try {
    const directory = new URL('./converter/', import.meta.url);
    const response = await fetch(new URL('manifest.json', directory), { cache: 'no-cache' });
    if (!response.ok) throw new Error('The converter download is unavailable. Retry the import.');
    const manifest = await response.json();
    if (
      typeof manifest.module !== 'string' ||
      !/^vtr_convert_[a-f0-9]{16}\.js$/.test(manifest.module)
    )
      throw new Error('Invalid converter manifest. Rebuild the browser app.');
    const converter = await import(new URL(manifest.module, directory));
    await converter.default();
    const limit = converter.max_input_bytes();
    if (file.size > limit)
      throw new Error(
        `The recording exceeds the browser input limit (${limit / 1024 / 1024} MiB). Use the native app or vtr CLI for larger recordings.`
      );
    const bytes = converter.convert(file.name, new Uint8Array(await file.arrayBuffer()));
    postMessage({ type: 'complete', bytes: bytes.buffer }, [bytes.buffer]);
  } catch (error) {
    const message =
      error instanceof WebAssembly.RuntimeError
        ? 'Conversion stopped before completion. The recording may be malformed or exceed the converter’s 1 GiB memory limit. Use the native app or vtr CLI for larger recordings.'
        : error?.message || String(error);
    postMessage({ type: 'error', message });
  }
};
