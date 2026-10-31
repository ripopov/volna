export function trackDownload(response, onProgress, onComplete) {
  if (!response.ok) throw new Error(`Viewer download failed (${response.status}).`);
  const encoding = response.headers.get('content-encoding');
  const length = Number(response.headers.get('content-length'));
  const total = (!encoding || encoding === 'identity') && Number.isSafeInteger(length) && length > 0 ? length : null;
  if (!response.body) {
    onComplete();
    return response;
  }
  let received = 0;
  const body = response.body.pipeThrough(new TransformStream({
    transform(chunk, controller) {
      received += chunk.byteLength;
      onProgress(total && received <= total ? received / total : null);
      controller.enqueue(chunk);
    },
    flush() { onComplete(); },
  }));
  return new Response(body, { status: response.status, statusText: response.statusText, headers: response.headers });
}
