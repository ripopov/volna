export class Conversion {
  #pending = null;
  constructor(
    createWorker = () =>
      new Worker(new URL('./conversion-worker.js', import.meta.url), { type: 'module' })
  ) {
    this.createWorker = createWorker;
  }

  run(file, progress) {
    this.cancel();
    return new Promise((resolve, reject) => {
      const worker = this.createWorker();
      const finish = (error, bytes) => {
        if (this.#pending?.worker !== worker) return;
        this.#pending = null;
        worker.terminate();
        if (error) reject(error);
        else resolve(bytes);
      };
      this.#pending = { worker, finish };
      worker.onmessage = ({ data }) => {
        if (this.#pending?.worker !== worker) return;
        if (!data || typeof data !== 'object') {
          finish(new Error('Invalid converter response.'));
          return;
        }
        if (data.type === 'progress') {
          try {
            progress(data.snapshot);
          } catch (error) {
            finish(error);
          }
        } else if (data.type === 'complete' && data.bytes instanceof ArrayBuffer)
          finish(null, new Uint8Array(data.bytes));
        else finish(new Error(data.message || 'Invalid converter response.'));
      };
      worker.onerror = (event) => {
        event.preventDefault();
        finish(new Error(event.message || 'The converter worker failed.'));
      };
      worker.onmessageerror = () => finish(new Error('Could not read the converter response.'));
      try {
        worker.postMessage({ file });
      } catch (error) {
        finish(error);
      }
    });
  }

  cancel() {
    this.#pending?.finish(new DOMException('Conversion cancelled.', 'AbortError'));
  }
}

const labels = {
  preparing: 'Preparing input',
  reading: 'Reading input',
  writing: 'Writing VTR',
  finalizing: 'Finalizing VTR',
  complete: 'Complete',
  failed: 'Conversion failed'
};

export function conversionStatus(snapshot) {
  const validCount = (value) => Number.isInteger(value) && value >= 0;
  if (
    !snapshot ||
    snapshot.version !== 1 ||
    typeof snapshot.phase !== 'string' ||
    !Object.hasOwn(labels, snapshot.phase) ||
    !validCount(snapshot.completed) ||
    !validCount(snapshot.input_bytes) ||
    !validCount(snapshot.output_bytes) ||
    (snapshot.total !== null &&
      (!validCount(snapshot.total) || snapshot.total === 0 || snapshot.completed > snapshot.total))
  ) {
    throw new Error('Invalid conversion progress.');
  }
  const percent = snapshot.total
    ? ` ${Math.floor((snapshot.completed * 100) / snapshot.total)}%`
    : '';
  return `${labels[snapshot.phase]}${percent} · Input ${formatBytes(snapshot.input_bytes)} · Output ${formatBytes(snapshot.output_bytes)}`;
}

function formatBytes(bytes) {
  const units = ['B', 'KiB', 'MiB', 'GiB'];
  let unit = 0;
  while (bytes >= 1024 && unit < units.length - 1) {
    bytes /= 1024;
    unit++;
  }
  return `${unit ? bytes.toFixed(1) : bytes} ${units[unit]}`;
}
