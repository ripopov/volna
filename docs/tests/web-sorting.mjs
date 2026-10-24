import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { existsSync } from 'node:fs';
import { readFile, stat, mkdir } from 'node:fs/promises';
import { spawnSync } from 'node:child_process';
import { extname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { chromium } from 'playwright';

const root = resolve(fileURLToPath(new URL('../dist/', import.meta.url)));
const results = fileURLToPath(new URL('../test-results/sorting/', import.meta.url));
await mkdir(results, { recursive: true });
const fixture = resolve(results, 'rows.vtr');
const generated = spawnSync('cargo', ['run', '-q', '-p', 'volna-core', '--example', 'table_sort_fixture', '--release', '--', fixture, '1000000'], {
  cwd: fileURLToPath(new URL('../../', import.meta.url)), stdio: 'inherit',
});
assert.equal(generated.status, 0);
const base = `/${(process.env.DOCS_BASE || '/').split('/').filter(Boolean).join('/')}`.replace(/\/$/, '') + '/';
const mime = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.wasm': 'application/wasm', '.svg': 'image/svg+xml', '.woff2': 'font/woff2' };
let headers = true;
const server = createServer(async (req, res) => {
  try {
    const path = new URL(req.url, 'http://localhost').pathname;
    assert.ok(path.startsWith(base));
    let file = resolve(root, path.slice(base.length));
    assert.ok(file.startsWith(root + '/') || file === root);
    if ((await stat(file)).isDirectory()) file += '/index.html';
    if (headers) {
      res.setHeader('Cross-Origin-Opener-Policy', 'same-origin');
      res.setHeader('Cross-Origin-Embedder-Policy', 'require-corp');
    }
    res.setHeader('Content-Type', mime[extname(file)] || 'application/octet-stream');
    res.end(await readFile(file));
  } catch { res.writeHead(404).end(); }
});
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
const origin = `http://127.0.0.1:${server.address().port}`;
const chrome = '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';
let browser;
let page;
try {
  browser = await chromium.launch({ executablePath: process.env.CHROME_PATH || (existsSync(chrome) ? chrome : undefined) });
  const context = await browser.newContext({ viewport: { width: 1440, height: 1000 } });
  await context.addInitScript(() => Object.defineProperty(navigator, 'hardwareConcurrency', { get: () => 4 }));
  page = await context.newPage();
  const settings = values => page.evaluate(async ({ base, values }) => (await import(`${base}viewer/pkg/volna.js`)).set_settings(JSON.stringify(values)), { base, values });
  page.setDefaultTimeout(30000);
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  page.on('console', m => { if (m.type() === 'error') errors.push(m.text()); });
  await page.goto(`${origin}${base}app/`);
  await page.locator('body[data-workspace-restored="true"]').waitFor({ timeout: 60000 });
  assert.equal(page.workers().length, 4);
  assert.equal(await page.evaluate(() => crossOriginIsolated), true);
  // The complete typed million-row track exceeds the default per-object cap.
  await settings({ 'memory.objectMiB': 512 });
  await page.locator('#recording-file').setInputFiles(fixture);
  await page.locator('body[data-local-trace-opened="rows.vtr"]').waitFor({ timeout: 60000 });
  const workspace = JSON.parse(await readFile(new URL('../../volna/examples/landing-page.volna.json', import.meta.url)));
  workspace.traces[0] = { ...workspace.traces[0], path: 'rows.vtr', name: 'rows.vtr', time_range: [0, 1000000] };
  workspace.panels = [workspace.panels.find(p => p.kind === 'table')];
  workspace.panels[0].id = 4;
  workspace.panels[0].source.path = ['sort', 'rows'];
  workspace.panels[0].widths = [210, 120, 105, 120, 100, 90, 360];
  workspace.layout = { center: { tabs: [4], active: 4 }, right: null, windows: {} };
  workspace.focused = 4;
  workspace.timeline_dividers = [];
  workspace.timescale = -12;
  workspace.traces[0].timescale = -12;
  workspace.sidebar.expanded = [];
  workspace.sidebar.selected_scope = null;
  workspace.sidebar.width = 190;
  await page.evaluate(async base => (await import(`${base}viewer/pkg/volna.js`)).dispatch_command('openWorkspace'), base);
  await page.locator('input[aria-label="Open a Volna workspace"]').setInputFiles({
    name: 'rows.volna.json', mimeType: 'application/json', buffer: Buffer.from(JSON.stringify(workspace)),
  });
  const state = () => page.evaluate(() => window.volnaCurrentState);
  const sorting = () => page.waitForFunction(() => window.volnaCurrentState?.includes('sorting…'), {}, { timeout: 10000 });
  const debuggerSession = await browser.newBrowserCDPSession();
  const { targetInfos } = await debuggerSession.send('Target.getTargets');
  const workerUrls = new Set(page.workers().map(worker => worker.url()));
  const workerSessions = await Promise.all(targetInfos
    .filter(target => target.type === 'worker' && workerUrls.has(target.url))
    .map(async target => (await debuggerSession.send('Target.attachToTarget', {
      targetId: target.targetId, flatten: false
    })).sessionId));
  assert.equal(workerSessions.length, 4);
  let nextMessage = 0;
  const responses = new Map();
  debuggerSession.on('Target.receivedMessageFromTarget', event => {
    const message = JSON.parse(event.message);
    responses.get(message.id)?.(message);
  });
  const workerCommand = (sessionId, method) => new Promise((resolve, reject) => {
    const id = ++nextMessage;
    const timeout = setTimeout(() => {
      responses.delete(id);
      reject(new Error(`Worker command timed out: ${method}`));
    }, 30000);
    const finish = message => {
      clearTimeout(timeout);
      responses.delete(id);
      if (message.error) reject(new Error(message.error.message));
      else resolve(message.result);
    };
    responses.set(id, finish);
    debuggerSession.send('Target.sendMessageToTarget', {
      sessionId, message: JSON.stringify({ id, method })
    }).catch(error => finish({ error }));
  });
  // Worker completion can beat a frame; hold it until pending UI assertions finish.
  const withPausedWorkers = async action => {
    await Promise.all(workerSessions.map(id => workerCommand(id, 'Debugger.enable')));
    await Promise.all(workerSessions.map(id => workerCommand(id, 'Debugger.pause')));
    try {
      await action();
    } finally {
      await Promise.all(workerSessions.map(id => workerCommand(id, 'Debugger.disable')));
    }
  };
  const settled = async () => {
    try {
      await page.waitForFunction(() => /panel=4 table .*rows=1000000.*status=1000000 rows$/.test(window.volnaCurrentState), {}, { timeout: 30000 });
    } catch (error) {
      console.error(await state());
      throw error;
    }
  };
  await settled();
  await page.mouse.click(350, 148);
  const selected = (await state()).match(/selected=(.*?) sort=/)[1];
  assert.notEqual(selected, 'None');
  await page.evaluate(() => {
    window.sortFrames = 0;
    const frame = () => {
      if (window.volnaCurrentState?.includes('sorting…')) window.sortFrames++;
      requestAnimationFrame(frame);
    };
    requestAnimationFrame(frame);
  });
  await withPausedWorkers(async () => {
    await page.mouse.click(350, 108);
    await sorting();
    await page.mouse.wheel(0, 300);
    await page.waitForFunction(() => window.sortFrames >= 2);
  });
  await settled();
  assert.match(await state(), /column: 0, descending: false.*first_time=Some\(999999\)/);
  assert.equal((await state()).match(/selected=(.*?) sort=/)[1], selected);
  assert.ok(await page.evaluate(() => window.sortFrames) >= 2, 'animation frames must continue during the worker sort');
  await page.screenshot({ path: `${results}/ascending-dark.png` });

  await withPausedWorkers(async () => {
    await page.mouse.click(350, 108);
    await sorting();
    await page.screenshot({ path: `${results}/sorting-pending-dark.png` });
    await page.mouse.click(678, 84); // Cancel in the table toolbar.
  });
  await settled();
  assert.match(await state(), /column: 0, descending: false/, 'cancel retains the applied order');

  await withPausedWorkers(async () => {
    await page.mouse.click(350, 108);
    await sorting();
    await page.mouse.click(500, 108); // A newer Begin query replaces the pending Label query.
  });
  await settled();
  assert.match(await state(), /column: 1, descending: false.*first_time=Some\(0\)/);

  await settings({ 'memory.budgetMiB': 1, 'appearance.theme': 'volna-light' });
  // Wait for the rendered settings change before exercising the disabled header.
  await page.waitForTimeout(100);
  await page.mouse.click(500, 108);
  await page.keyboard.press('Enter');
  await page.waitForTimeout(100);
  assert.match(await state(), /column: 1, descending: false/);
  assert.ok(!(await state()).includes('sorting…'));
  await page.screenshot({ path: `${results}/memory-refused-light.png` });
  await settings({ 'memory.budgetMiB': 512 });
  await page.mouse.click(500, 108);
  await settled();
  await page.waitForFunction(() => window.volnaCurrentState?.includes('column: 1, descending: true'));
  assert.match(await state(), /first_time=Some\(999999\)/);
  await page.keyboard.press('Space');
  await page.waitForFunction(() => window.volnaCurrentState?.includes('sort=None'));
  assert.match(await state(), /first_time=Some\(0\)/);
  console.log('1M rows: worker sorting, responsive frames, cancellation, newest query, selection, RAM refusal, and keyboard cycling passed');
  await page.evaluate(() => {
    window.filterFrames = 0;
    const frame = () => {
      if (window.volnaCurrentState?.includes('filtering…')) window.filterFrames++;
      requestAnimationFrame(frame);
    };
    requestAnimationFrame(frame);
  });
  await page.mouse.click(485, 80); // Filter follows Columns.
  await page.waitForTimeout(100);
  await page.mouse.click(535, 150); // Begin's filter input, below its header.
  await page.waitForTimeout(60);
  await withPausedWorkers(async () => {
    await page.keyboard.type('> 500000');
    await page.keyboard.press('Enter');
    await page.waitForFunction(() => window.filterFrames >= 2);
  });
  await page.waitForFunction(() => window.volnaCurrentState?.includes('status=499999 of 1000000 rows'));
  assert.match(await state(), /first_time=Some\(500001\)/);
  assert.ok(await page.evaluate(() => window.filterFrames) >= 1, 'frames must continue during the worker scan');
  await page.screenshot({ path: `${results}/filtered-dark.png` });
  await page.mouse.click(485, 80); // Collapsing preserves the result.
  await page.waitForTimeout(60);
  assert.match(await state(), /rows=499999/);
  await page.mouse.click(485, 80);
  await page.waitForTimeout(60);
  await page.mouse.click(500, 108); // Sort matches by Begin.
  await page.waitForFunction(() => /column: 1, descending: false.*status=499999 of 1000000 rows$/.test(window.volnaCurrentState));
  await page.mouse.click(535, 150);
  await page.waitForTimeout(60);
  // The web viewer's text-editing keymap uses Control on every host.
  await page.keyboard.press('Control+A');
  await page.keyboard.type('>= 999990');
  await page.keyboard.press('Enter');
  await page.waitForFunction(() => window.volnaCurrentState?.includes('status=10 of 1000000 rows'));
  assert.match(await state(), /first_time=Some\(999990\)/);
  await page.mouse.click(565, 80); // Clear filters preserves sorting.
  await settled();
  assert.match(await state(), /column: 1, descending: false.*first_time=Some\(0\)/);
  console.log('1M rows: background filtering, responsive frames, collapsed state, exact ranges, sorting matches, and clearing passed');

  assert.deepEqual(errors, []);
  await context.close();

  headers = false;
  const staticContext = await browser.newContext();
  await staticContext.addInitScript(() => Object.defineProperty(navigator, 'hardwareConcurrency', { get: () => 4 }));
  const staticPage = await staticContext.newPage();
  await staticPage.goto(`${origin}${base}`);
  await staticPage.frameLocator('iframe[title="Interactive Volna sample recording"]').locator('body[data-workspace-restored="true"]').waitFor({ timeout: 60000 });
  assert.equal(await staticPage.evaluate(() => crossOriginIsolated), true);
  assert.equal(await staticPage.evaluate(() => !!navigator.serviceWorker.controller), true);
  await staticContext.close();
  console.log('Static hosting without response headers: isolation bootstrap and embedded worker pool passed');

  headers = true;
  const unsupported = await browser.newContext();
  await unsupported.addInitScript(() => { Atomics.waitAsync = undefined; });
  const unsupportedPage = await unsupported.newPage();
  await unsupportedPage.goto(`${origin}${base}app/`);
  await unsupportedPage.getByRole('alert').waitFor();
  assert.match(await unsupportedPage.getByRole('alert').innerText(), /Background workers require Atomics.waitAsync/);
  assert.equal(unsupportedPage.workers().length, 0);
  await unsupported.close();
  console.log('Unsupported browser reports the missing capability without starting the viewer');
} catch (error) {
  if (page && !page.isClosed()) {
    console.error(await page.evaluate(() => window.volnaCurrentState));
    await page.screenshot({ path: `${results}/failure.png` });
  }
  throw error;
} finally {
  await browser?.close();
  await new Promise(resolve => server.close(resolve));
}
