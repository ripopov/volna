import test from 'node:test';
import assert from 'node:assert/strict';
import { Conversion, conversionStatus } from '../public/viewer/conversion.js';

function fixture() {
  const workers = [];
  const conversion = new Conversion(() => {
    const worker = {
      postMessage(message) {
        this.message = message;
      },
      terminate() {
        this.terminated = true;
      },
      send(data) {
        this.onmessage({ data });
      }
    };
    workers.push(worker);
    return worker;
  });
  return { conversion, workers };
}

test('conversion transfers complete output and terminates the worker before resolving', async () => {
  const { conversion, workers } = fixture();
  const snapshots = [];
  const file = { name: 'trace.fst' };
  const result = conversion.run(file, (snapshot) => snapshots.push(snapshot));
  const worker = workers[0];
  assert.equal(worker.message.file, file);
  worker.send({ type: 'progress', snapshot: { phase: 'reading' } });
  worker.send({ type: 'complete', bytes: new Uint8Array([1, 2, 3]).buffer });
  assert.deepEqual(await result, new Uint8Array([1, 2, 3]));
  assert.equal(worker.terminated, true);
  worker.send({ type: 'progress', snapshot: { phase: 'failed' } });
  assert.deepEqual(snapshots, [{ phase: 'reading' }]);
});

test('cancel and replacement reject pending work and ignore late messages', async () => {
  const { conversion, workers } = fixture();
  const first = conversion.run({ name: 'old.fst' }, () => assert.fail('stale progress'));
  const rejected = assert.rejects(first, { name: 'AbortError' });
  const second = conversion.run({ name: 'new.log' }, () => {});
  await rejected;
  assert.equal(workers[0].terminated, true);
  workers[0].send({ type: 'progress', snapshot: {} });
  workers[0].send({ type: 'complete', bytes: new ArrayBuffer(0) });
  assert.notEqual(workers[1].terminated, true);
  const cancelled = assert.rejects(second, { name: 'AbortError' });
  conversion.cancel();
  await cancelled;
  assert.equal(workers[1].terminated, true);
  conversion.cancel();
});

test('conversion, worker, message, and progress failures clean up pending work', async () => {
  for (const fail of [
    (worker) => worker.send(null),
    (worker) => worker.send({ type: 'error', message: 'invalid FST' }),
    (worker) => worker.onerror({ message: 'load failed', preventDefault() {} }),
    (worker) => worker.onmessageerror(),
    (worker) => worker.send({ type: 'complete', bytes: 'not an image' }),
    (worker) => worker.send({ type: 'progress', snapshot: {} })
  ]) {
    const { conversion, workers } = fixture();
    const result = conversion.run({}, conversionStatus);
    const rejected = assert.rejects(result);
    fail(workers[0]);
    await rejected;
    assert.equal(workers[0].terminated, true);
  }
});

test('progress uses phase totals, byte sizes, and indeterminate finalization', () => {
  const snapshot = {
    version: 1,
    phase: 'reading',
    completed: 42,
    total: 100,
    input_bytes: 4096,
    output_bytes: 2048
  };
  assert.equal(conversionStatus(snapshot), 'Reading input 42% · Input 4.0 KiB · Output 2.0 KiB');
  assert.match(
    conversionStatus({ ...snapshot, phase: 'finalizing', total: null }),
    /^Finalizing VTR ·/
  );
  assert.match(
    conversionStatus({ ...snapshot, completed: 0, total: Number(0xffffffffffffffffn) }),
    /Reading input 0%/
  );
  for (const invalid of [
    { version: 2 },
    { completed: 101 },
    { total: 0 },
    { input_bytes: -1 },
    { phase: 'unknown' },
    { phase: 'constructor' },
    { phase: ['reading'] }
  ]) {
    assert.throws(
      () => conversionStatus({ ...snapshot, ...invalid }),
      /Invalid conversion progress/
    );
  }
});
