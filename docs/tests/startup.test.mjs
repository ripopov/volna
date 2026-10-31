import assert from 'node:assert/strict';
import { test } from 'node:test';
import { trackDownload } from '../public/viewer/startup.js';

function response(headers = {}) {
  return new Response(new ReadableStream({
    start(controller) {
      controller.enqueue(new Uint8Array([1, 2]));
      controller.enqueue(new Uint8Array([3, 4]));
      controller.close();
    },
  }), { headers });
}

test('tracks downloaded bytes and then switches to startup', async () => {
  const events = [];
  const tracked = trackDownload(response({ 'content-length': '4', 'content-type': 'application/wasm' }),
    value => events.push(value), () => events.push('starting'));
  assert.equal(tracked.headers.get('content-type'), 'application/wasm');
  assert.deepEqual([...new Uint8Array(await tracked.arrayBuffer())], [1, 2, 3, 4]);
  assert.deepEqual(events, [0.5, 1, 'starting']);
});

test('keeps progress indeterminate for missing or compressed lengths', async () => {
  for (const headers of [{}, { 'content-length': '2', 'content-encoding': 'gzip' }]) {
    const events = [];
    await trackDownload(response(headers), value => events.push(value), () => events.push('starting')).arrayBuffer();
    assert.deepEqual(events, [null, null, 'starting']);
  }
});

test('reports failed downloads', () => {
  assert.throws(() => trackDownload(new Response(null, { status: 404 }), () => {}, () => {}), /404/);
});
