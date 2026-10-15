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
try {
  browser = await chromium.launch({ executablePath: process.env.CHROME_PATH || (existsSync(chrome) ? chrome : undefined) });
  const context = await browser.newContext({ viewport: { width: 1440, height: 1000 } });
  await context.addInitScript(() => Object.defineProperty(navigator, 'hardwareConcurrency', { get: () => 4 }));
  const page = await context.newPage();
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  page.on('console', m => { if (m.type() === 'error') errors.push(m.text()); });
  await page.goto(`${origin}${base}app/`);
  await page.locator('body[data-workspace-restored="true"]').waitFor({ timeout: 60000 });
  assert.equal(page.workers().length, 4);
  assert.equal(await page.evaluate(() => crossOriginIsolated), true);
  await page.locator('#recording-file').setInputFiles(fixture);
  await page.locator('body[data-local-trace-opened="rows.vtr"]').waitFor({ timeout: 60000 });
  const workspace = JSON.parse(await readFile(new URL('../../volna/examples/landing-page.volna.json', import.meta.url)));
  workspace.traces[0] = { ...workspace.traces[0], path: 'rows.vtr', name: 'rows.vtr', time_range: [0, 1000000] };
  workspace.panels = [workspace.panels.find(p => p.kind === 'table')];
  workspace.panels[0].id = 4;
  workspace.panels[0].source.path = ['sort', 'rows'];
  workspace.panels[0].widths = [210, 120, 105, 120, 100, 90, 360];
  workspace.layout = { center: { tabs: [4], active: 4 }, right: null };
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
  const settled = () => page.waitForFunction(() => /panel=4 table .*rows=1000000.*status=1000000 rows$/.test(window.volnaCurrentState), {}, { timeout: 30000 });
  await settled();
  await page.mouse.click(350, 132);
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
  await page.mouse.click(350, 108);
  await sorting();
  await page.mouse.wheel(0, 300);
  await settled();
  assert.match(await state(), /column: 0, descending: false.*first_time=Some\(999999\)/);
  assert.equal((await state()).match(/selected=(.*?) sort=/)[1], selected);
  assert.ok(await page.evaluate(() => window.sortFrames) >= 2, 'animation frames must continue during the worker sort');
  await page.screenshot({ path: `${results}/ascending-dark.png` });

  await page.mouse.click(350, 108);
  await sorting();
  await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
  await page.mouse.click(614, 84); // Cancel in the table toolbar.
  await settled();
  assert.match(await state(), /column: 0, descending: false/, 'cancel retains the applied order');

  await page.mouse.click(350, 108);
  await sorting();
  await page.mouse.click(500, 108); // A newer Begin query replaces the pending Label query.
  await settled();
  assert.match(await state(), /column: 1, descending: false.*first_time=Some\(0\)/);

  const settings = values => page.evaluate(async ({ base, values }) => (await import(`${base}viewer/pkg/volna.js`)).set_settings(JSON.stringify(values)), { base, values });
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
} finally {
  await browser?.close();
  await new Promise(resolve => server.close(resolve));
}
