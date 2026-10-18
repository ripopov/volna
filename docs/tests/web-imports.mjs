import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { existsSync } from 'node:fs';
import { mkdir, readFile, stat } from 'node:fs/promises';
import { extname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { gzipSync } from 'node:zlib';
import { chromium } from 'playwright';

const root = fileURLToPath(new URL('../dist/', import.meta.url));
const results = fileURLToPath(new URL('../test-results/imports/', import.meta.url));
const fixtures = new URL('../../vtr/vtr-convert/tests/fixtures/', import.meta.url);
const sampleWorkspace = JSON.parse(
  await readFile(new URL('../../volna/examples/landing-page.volna.json', import.meta.url))
);
const base =
  `/${(process.env.DOCS_BASE || '/').split('/').filter(Boolean).join('/')}`.replace(/\/$/, '') +
  '/';
const mime = {
  '.html': 'text/html',
  '.js': 'text/javascript',
  '.css': 'text/css',
  '.wasm': 'application/wasm'
};
const server = createServer(async (request, response) => {
  try {
    const path = new URL(request.url, 'http://localhost').pathname;
    assert.ok(path.startsWith(base));
    let file = resolve(root, path.slice(base.length));
    assert.ok(
      file.startsWith(root) && (file === root || file.startsWith(root.replace(/\/$/, '') + '/'))
    );
    if ((await stat(file)).isDirectory()) file += '/index.html';
    response.writeHead(200, {
      'Cross-Origin-Opener-Policy': 'same-origin',
      'Cross-Origin-Embedder-Policy': 'require-corp',
      'Content-Type': mime[extname(file)] || 'application/octet-stream'
    });
    response.end(await readFile(file));
  } catch {
    response.writeHead(404).end();
  }
});
await mkdir(results, { recursive: true });
await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
const origin = `http://127.0.0.1:${server.address().port}`;
const chrome = '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';
let browser;
try {
  browser = await chromium.launch({
    executablePath: process.env.CHROME_PATH || (existsSync(chrome) ? chrome : undefined)
  });
  const context = await browser.newContext({ viewport: { width: 1440, height: 1000 } });
  await context.addInitScript(() => {
    Object.defineProperty(navigator, 'hardwareConcurrency', { get: () => 4 });
    const NativeWorker = Worker;
    window.importWorkers = 0;
    window.importSnapshots = [];
    window.convertedImages = [];
    window.Worker = class extends NativeWorker {
      constructor(url, options) {
        super(url, options);
        this.converter = String(url).includes('conversion-worker.js');
        if (this.converter) {
          window.importWorkers++;
          this.addEventListener('message', ({ data }) => {
            if (data.type === 'progress') window.importSnapshots.push(data.snapshot);
            if (data.type === 'complete')
              window.convertedImages.push(Array.from(new Uint8Array(data.bytes)));
          });
        }
      }
      terminate() {
        if (this.converter) {
          window.importWorkers--;
          this.converter = false;
        }
        super.terminate();
      }
    };
  });
  const page = await context.newPage();
  page.setDefaultTimeout(30000);
  const errors = [];
  const converterRequests = [];
  page.on('request', (request) => {
    if (
      new URL(request.url()).pathname.includes('/viewer/converter/') ||
      request.url().includes('conversion-worker.js')
    )
      converterRequests.push(request.url());
  });
  page.on('pageerror', (error) => errors.push(error.message));
  await page.goto(`${origin}${base}app/`);
  await page.locator('body[data-workspace-restored="true"]').waitFor({ timeout: 60000 });
  assert.deepEqual(converterRequests, [], 'startup must not request the converter');
  await page.keyboard.press('Tab');
  const open = async (name, buffer) => {
    await page
      .locator('#recording-file')
      .setInputFiles({ name, mimeType: 'application/octet-stream', buffer });
    await page.waitForFunction((name) => document.body.dataset.localTraceOpened === name, name);
    assert.equal(await page.locator('#viewer-error').isVisible(), false);
    assert.equal(await page.evaluate(() => window.importWorkers), 0);
  };
  const command = (name) =>
    page.evaluate(
      async ({ base, name }) => {
        (await import(`${base}viewer/pkg/volna.js`)).dispatch_command(name);
      },
      { base, name }
    );
  const save = async () => {
    const download = page.waitForEvent('download');
    await command('saveWorkspaceAs');
    return JSON.parse(await readFile(await (await download).path()));
  };
  const restore = async (name, kind, path, range, timescale) => {
    const workspace = structuredClone(sampleWorkspace);
    workspace.traces[0] = {
      ...workspace.traces[0],
      path: name,
      name,
      time_range: range,
      timescale
    };
    workspace.timescale = timescale;
    const panel = workspace.panels.find((panel) => panel.kind === kind);
    workspace.panels = [panel];
    workspace.layout = { center: { tabs: [panel.id], active: panel.id }, right: null, windows: {} };
    workspace.focused = panel.id;
    workspace.timeline_dividers = [];
    workspace.sidebar.expanded = [];
    workspace.sidebar.selected_scope = null;
    if (kind === 'pipeline') panel.track = path;
    else panel.rows = [{ type: 'signal', signal: path, format: 'bit' }];
    await command('openWorkspace');
    await page.getByLabel('Open a Volna workspace').setInputFiles({
      name: `${name}.volna.json`,
      mimeType: 'application/json',
      buffer: Buffer.from(JSON.stringify(workspace))
    });
    return panel.id;
  };

  await open(
    'direct.vtr',
    await readFile(new URL('../../volna-trace/tests/fixtures/values.vtr', import.meta.url))
  );
  assert.deepEqual(converterRequests, [], 'VTR must open without requesting the converter');

  for (const fixture of ['pipeline.fst', 'pipeline.kanata', 'waves.vcd']) {
    const buffer = await readFile(new URL(fixture, fixtures));
    await open(fixture, buffer);
    const image = await page.evaluate(() => window.convertedImages.at(-1));
    assert.deepEqual(image.slice(0, 8), [137, 86, 84, 82, 13, 10, 26, 10]);
    const saved = await save();
    assert.equal(saved.traces[0].name, fixture);
    assert.equal(
      new URL(saved.traces[0].path, `${origin}${base}local/${fixture}.volna.json`).href,
      `${origin}${base}local/${fixture}`
    );
    await open(`${fixture}.gz`, gzipSync(buffer));
    assert.deepEqual(await page.evaluate(() => window.convertedImages.at(-1)), image);
  }
  assert.ok(
    converterRequests.some((url) => /vtr_convert_[a-f0-9]{16}_bg\.wasm$/.test(url)),
    'imports fetch the versioned WASM on demand'
  );
  await open('pipeline.fst', await readFile(new URL('pipeline.fst', fixtures)));
  const fstWaves = await restore('pipeline.fst', 'waves', ['TOP', 'top', 'clk'], [0, 30], -12);
  await page.waitForFunction(
    (id) =>
      window.volnaCurrentState?.includes(
        `panel=${id} focused=true linked=(true,true) items=1 loaded=1`
      ),
    fstWaves
  );
  const kanata = await readFile(new URL('pipeline.kanata', fixtures));
  await open('pipeline.LOG.GZ', gzipSync(kanata));
  const pipeline = await restore('pipeline.LOG.GZ', 'pipeline', ['cpu', 'thread0'], [100, 106], 0);
  await page.waitForFunction(
    (id) =>
      window.volnaCurrentState?.includes(`panel=${id} pipeline track=cpu.thread0`) &&
      window.volnaCurrentState.includes(' ready rows=2 '),
    pipeline
  );
  await page.screenshot({ path: `${results}/kanata-dark.png` });
  await open('waves.vcd', await readFile(new URL('waves.vcd', fixtures)));
  const waves = await restore('waves.vcd', 'waves', ['top', 'clk'], [0, 10], -8);
  await page.waitForFunction(
    (id) =>
      window.volnaCurrentState?.includes(
        `panel=${id} focused=true linked=(true,true) items=1 loaded=1`
      ),
    waves
  );
  console.log(
    'FST, Kanata, LOG, VCD, gzip, waveform loading, pipeline rows, and source workspace identity passed'
  );

  const large = Buffer.from(
    'Kanata\t0004\n' +
      Array.from(
        { length: 250000 },
        (_, i) => `C=\t${i}\nI\t${i}\t${i}\t0\nS\t${i}\t0\tX\nE\t${i}\t0\tX\nR\t${i}\t${i}\t0\n`
      ).join('')
  );
  const startLarge = async () => {
    await page.evaluate(() => {
      window.importSnapshots = [];
    });
    await page
      .locator('#recording-file')
      .setInputFiles({ name: 'large.kanata', mimeType: 'text/plain', buffer: large });
    await page.waitForFunction(() => window.importSnapshots.some((s) => s.phase === 'reading'));
    await page.getByRole('progressbar', { name: 'Conversion progress' }).waitFor();
    assert.notEqual(await page.getByRole('progressbar').getAttribute('aria-valuenow'), null);
  };
  await startLarge();
  await page.waitForFunction(() =>
    window.importSnapshots.some(
      (snapshot) =>
        snapshot.phase === 'reading' && snapshot.completed > 0 && snapshot.output_bytes > 0
    )
  );
  assert.match(await page.locator('#viewer-message').innerText(), /Input .*Output /);
  await page.evaluate(
    () => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)))
  );
  await page.getByRole('button', { name: 'Cancel import' }).focus();
  await page.keyboard.press('Enter');
  await page.waitForFunction(() => window.importWorkers === 0);
  assert.equal(await page.locator('#viewer-status').innerText(), 'Import cancelled.');
  assert.equal(await page.locator('body').getAttribute('data-local-trace-opened'), null);
  await startLarge();
  await open('replacement.LOG', kanata);
  await startLarge();
  await page.getByRole('button', { name: 'Load sample' }).click();
  await page.locator('body[data-workspace-restored="true"]').waitFor();
  assert.equal(await page.evaluate(() => window.importWorkers), 0);
  console.log(
    'Responsive frames during conversion, keyboard cancellation, replacement, and sample cancellation passed'
  );

  await page.locator('#recording-file').setInputFiles({
    name: 'bad.fst',
    mimeType: 'application/octet-stream',
    buffer: Buffer.from('broken')
  });
  await page.locator('#viewer-error').waitFor();
  assert.match(await page.locator('#viewer-error').innerText(), /Could not open bad.fst/);
  assert.equal(await page.evaluate(() => window.importWorkers), 0);
  await open('recovered.kanata', kanata);
  await page.route('**/viewer/converter/*_bg.wasm', (route) => route.abort());
  await page.locator('#recording-file').setInputFiles({
    name: 'missing.fst',
    mimeType: 'application/octet-stream',
    buffer: await readFile(new URL('pipeline.fst', fixtures))
  });
  await page.locator('#viewer-error').waitFor();
  assert.equal(await page.evaluate(() => window.importWorkers), 0);
  await page.unroute('**/viewer/converter/*_bg.wasm');
  await open('retried.fst', await readFile(new URL('pipeline.fst', fixtures)));
  const requestsBeforeVtr = converterRequests.length;
  await open(
    'recovered.vtr',
    Buffer.from(await page.evaluate(() => window.convertedImages.at(-1)))
  );
  assert.equal(converterRequests.length, requestsBeforeVtr);
  const memoryError = await page.evaluate(async (base) => {
    const { Conversion, conversionStatus } = await import(`${base}viewer/conversion.js`);
    try {
      await new Conversion().run(
        { name: 'too-large.fst', size: 256 * 1024 * 1024 + 1 },
        conversionStatus
      );
    } catch (error) {
      return error.message;
    }
  }, base);
  assert.match(memoryError, /exceeds the browser input limit/);
  assert.equal(await page.evaluate(() => window.importWorkers), 0);
  console.log(
    'Malformed input and unavailable WASM report errors; a subsequent VTR opens directly'
  );

  let release;
  let received;
  const gate = new Promise((resolve) => {
    release = resolve;
  });
  const requested = new Promise((resolve) => {
    received = resolve;
  });
  await page.route('**/viewer/converter/*_bg.wasm', async (route) => {
    received();
    await gate;
    await route.continue().catch(() => {});
  });
  for (const [theme, width, zoom] of [
    ['dark', 1440, 1],
    ['light', 1440, 1.25],
    ['dark', 360, 1],
    ['light', 360, 1.25]
  ]) {
    await page.emulateMedia({ reducedMotion: width === 360 ? 'reduce' : 'no-preference' });
    await page.setViewportSize({ width, height: 900 });
    await page.evaluate(
      async ({ base, theme, zoom }) => {
        document.documentElement.dataset.theme = theme;
        (await import(`${base}viewer/pkg/volna.js`)).set_settings(
          JSON.stringify({ 'appearance.theme': `volna-${theme}`, 'appearance.zoom': zoom })
        );
      },
      { base, theme, zoom }
    );
    await page.locator('#recording-file').setInputFiles({
      name: 'recording-with-a-long-name.kanata',
      mimeType: 'text/plain',
      buffer: kanata
    });
    await requested;
    await page.getByRole('button', { name: 'Cancel import' }).waitFor();
    assert.equal(await page.getByRole('progressbar').getAttribute('aria-valuenow'), null);
    if (width === 360)
      assert.equal(
        await page
          .locator('#import-progress > span')
          .evaluate((fill) => getComputedStyle(fill).animationName),
        'none'
      );
    assert.equal(
      await page.evaluate(() => document.documentElement.scrollWidth > innerWidth),
      false
    );
    const bounds = await page.locator('#viewer-status').boundingBox();
    assert.ok(bounds.x >= 0 && bounds.x + bounds.width <= width);
    const actions = await page.locator('#viewer-actions').boundingBox();
    assert.ok(bounds.y + bounds.height <= actions.y, 'progress must remain above wrapped controls');
    await page.screenshot({ path: `${results}/progress-${theme}-${width}-${zoom}.png` });
    await page.getByRole('button', { name: 'Cancel import' }).focus();
    await page.keyboard.press('Enter');
    await page.waitForFunction(() => window.importWorkers === 0);
  }
  release();
  assert.deepEqual(errors, []);
  await context.close();
} finally {
  await browser?.close();
  await new Promise((resolve) => server.close(resolve));
}
