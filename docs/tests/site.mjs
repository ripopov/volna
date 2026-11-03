import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { existsSync } from 'node:fs';
import { mkdir, readFile, stat } from 'node:fs/promises';
import { resolve, extname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { chromium } from 'playwright';
import AxeBuilder from '@axe-core/playwright';

const sampleWorkspace = JSON.parse(await readFile(new URL('../../volna/examples/landing-page.volna.json', import.meta.url), 'utf8'));
const sampleRecord = sampleWorkspace.panels.find(panel => panel.id === 37).record;
const root = resolve(fileURLToPath(new URL('../dist/', import.meta.url)));
const results = fileURLToPath(new URL('../test-results/', import.meta.url));
const base = `/${(process.env.DOCS_BASE || '/').split('/').filter(Boolean).join('/')}`.replace(/\/$/, '') + '/';
const mime = { '.html': 'text/html', '.css': 'text/css', '.js': 'text/javascript', '.mjs': 'text/javascript', '.svg': 'image/svg+xml', '.woff2': 'font/woff2', '.wasm': 'application/wasm', '.json': 'application/json' };
const server = createServer(async (req, res) => {
  try {
    const path = decodeURIComponent(new URL(req.url, 'http://localhost').pathname);
    if (!path.startsWith(base)) throw new Error('outside base');
    let file = resolve(root, path.slice(base.length));
    if (file !== root && !file.startsWith(root + '/')) throw new Error('outside output');
    if ((await stat(file)).isDirectory()) file += '/index.html';
    const content = await readFile(file);
    res.writeHead(200, { 'Cross-Origin-Opener-Policy': 'same-origin', 'Cross-Origin-Embedder-Policy': 'require-corp', 'Content-Type': mime[extname(file)] || 'application/octet-stream' });
    res.end(content);
  } catch {
    res.writeHead(404); res.end('Not found');
  }
});
await new Promise(done => server.listen(0, '127.0.0.1', done));
await mkdir(results, { recursive: true });
const origin = `http://127.0.0.1:${server.address().port}`;
const chrome = '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';
let browser;
const report = { pages: [], accessibility: [], behavior: [] };
try {
  browser = await chromium.launch({ executablePath: process.env.CHROME_PATH || (existsSync(chrome) ? chrome : undefined) });
  const behaviorContext = await browser.newContext({ viewport: { width: 1440, height: 1000 }, colorScheme: 'dark' });
  await behaviorContext.addInitScript(() => Object.defineProperty(navigator, 'hardwareConcurrency', { get: () => 4 }));
  const page = await behaviorContext.newPage();
  const appErrors = [];
  page.on('pageerror', error => appErrors.push(error.message));
  await page.goto(`${origin}${base}`);
  assert.equal(await page.locator('.site-title').getAttribute('href'), base);
  assert.equal(await page.getByRole('navigation', { name: 'Site' }).getByRole('link', { name: 'Docs' }).getAttribute('href'), `${base}docs/`);
  assert.equal(await page.getByRole('navigation', { name: 'Site' }).getByRole('link', { name: 'Web App' }).getAttribute('href'), `${base}app/`);
  assert.equal(await page.locator('iframe').count(), 0);
  const preview = page.locator('.v-viewer-preview');
  assert.equal(await preview.getAttribute('href'), './app/');
  assert.ok(await preview.locator('img').evaluate(img => img.complete && img.naturalWidth > 0));
  await page.setViewportSize({ width: 360, height: 800 });
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth + 1), false);
  await page.locator('html').evaluate(element => { element.dataset.theme = 'light'; });
  assert.equal(await page.locator('html').getAttribute('data-theme'), 'light');
  await page.setViewportSize({ width: 1440, height: 1000 });
  await page.goto(`${origin}${base}docs/`);
  assert.match(await page.locator('h1').textContent(), /engineering docs/);
  assert.equal(await page.locator('.v-guide').count(), 8);
  let releaseWasm, wasmRequested;
  const wasmRequest = new Promise(resolve => { wasmRequested = resolve; });
  const wasmResponse = new Promise(resolve => { releaseWasm = resolve; });
  const wasmRoute = `${origin}${base}viewer/pkg/volna_bg.wasm`;
  await page.route(wasmRoute, async route => {
    wasmRequested();
    await wasmResponse;
    await route.continue();
  });
  await page.goto(`${origin}${base}app/`);
  await wasmRequest;
  assert.equal(await page.locator('#viewer-message').textContent(), 'Downloading the Volna viewer…');
  assert.equal(await page.getByRole('progressbar', { name: 'Viewer startup progress' }).isVisible(), true);
  await page.screenshot({ path: `${results}/viewer-startup.png` });
  releaseWasm();
  assert.equal(await page.locator('.volna-header, #starlight__sidebar').count(), 0);
  await page.locator('body[data-workspace-restored="true"]').waitFor({ timeout: 60000 });
  await page.unroute(wasmRoute);
  assert.equal(await page.getByRole('progressbar', { name: 'Viewer startup progress', includeHidden: true }).isVisible(), false);
  verifySampleWorkspace(await page.evaluate(() => window.volnaCurrentState));
  const loadedSampleState = await page.evaluate(() => window.volnaCurrentState);
  const appCanvas = await page.locator('body > canvas').boundingBox();
  assert.ok(appCanvas && Math.abs(appCanvas.width - 1440) < 2 && Math.abs(appCanvas.height - 1000) < 2,
    'standalone canvas must fill the browser viewport');
  await page.locator('#recording-file').setInputFiles(fileURLToPath(new URL('../../volna/examples/landing.vtr', import.meta.url)));
  await page.locator('body[data-local-trace-opened="landing.vtr"]').waitFor({ timeout: 30000 });
  await page.evaluate(async base => (await import(`${base}viewer/pkg/volna.js`)).dispatch_command('openWorkspace'), base);
  await page.locator('input[aria-label="Open a Volna workspace"]').setInputFiles(fileURLToPath(new URL('../../volna/examples/landing-page.volna.json', import.meta.url)));
  await page.waitForFunction(() => window.volnaCurrentState?.includes('panel=40 table'));
  const downloadPromise = page.waitForEvent('download');
  await page.evaluate(async base => (await import(`${base}viewer/pkg/volna.js`)).dispatch_command('saveWorkspaceAs'), base);
  assert.equal((await downloadPromise).suggestedFilename(), 'landing.vtr.volna.json');
  let releaseSample, sampleRequested;
  const sampleFetch = new Promise(resolve => { sampleRequested = resolve; });
  const sampleResponse = new Promise(resolve => { releaseSample = resolve; });
  const sampleRoute = `${origin}${base}examples/landing.vtr`;
  await page.route(sampleRoute, async route => {
    sampleRequested();
    await sampleResponse;
    await route.continue();
  });
  await page.getByRole('button', { name: 'Load sample' }).click();
  await sampleFetch;
  await page.evaluate(state => window.volnaViewerState(state), loadedSampleState);
  assert.equal(await page.locator('body').getAttribute('data-workspace-restored'), null,
    'a ready state from the previous recording cannot complete a pending sample fetch');
  assert.equal(await page.locator('#viewer-status').isVisible(), true);
  releaseSample();
  await page.locator('body[data-workspace-restored="true"]').waitFor({ timeout: 60000 });
  await page.unroute(sampleRoute);
  const selectedTransaction = () => page.evaluate(() => window.volnaCurrentState
    .split('\n').find(line => line.startsWith('panel=37 transaction ')));
  const islandTransaction = await selectedTransaction();
  assert.match(islandTransaction, /ready record=Some/);
  for (const [theme, width, zoom] of [
    ['light', 1440, 1], ['light', 1440, 1.25], ['light', 360, 1], ['dark', 1440, 1],
  ]) {
    await page.setViewportSize({ width, height: 1000 });
    await page.evaluate(async ({ base, theme, zoom }) => {
      (await import(`${base}viewer/pkg/volna.js`)).set_settings(JSON.stringify({
        'appearance.theme': `volna-islands-${theme}`, 'appearance.zoom': zoom,
      }));
      await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
    }, { base, theme, zoom });
    assert.equal(await selectedTransaction(), islandTransaction,
      'switching Islands appearance and interface zoom must preserve the selected transaction');
    const screenshot = await page.screenshot({ path: `${results}/islands-${theme}-${width}-${zoom}.png` });
    if (width === 1440 && zoom === 1) await verifySampleIslandContour(page, screenshot);
  }
  report.behavior.push('Islands light and dark switching preserves the selected transaction at desktop, narrow width, and 125% zoom');
  report.behavior.push('Islands tab headers meet their content and match the untabbed sidebar without an outer border');
  const pipelineState = () => page.evaluate(() => window.volnaCurrentState
    .split('\n').find(line => line.startsWith('panel=38 pipeline ')));
  const navigatorRect = state => state.match(/navigator_rect=\(([^)]+)\)/)[1].split(',').map(Number);
  const waitForNavigator = visible => page.waitForFunction(async ({ base, visible }) => {
    (await import(`${base}viewer/pkg/volna.js`)).debug_state();
    const state = window.volnaCurrentState.split('\n').find(line => line.startsWith('panel=38 pipeline '));
    const width = Number(state?.match(/navigator_rect=\([^,]+,[^,]+,([^,]+),/)?.[1]);
    return state?.includes(`navigator=(${visible},104)`) && width === (visible ? 104 : 0);
  }, { base, visible });
  const rowTop = state => Number(state.match(/ top=([^ ]+)/)[1]);
  const pipelineView = state => state.match(/viewport=\(([^)]+)\)/)[1].split(',').map(Number);
  let pipelineBefore = await pipelineState();
  let [navX, navY, navW, navH] = navigatorRect(pipelineBefore);
  assert.ok(navW >= 32 && navH > 0);
  await page.mouse.click(navX + navW / 2, navY + navH * 0.6);
  await page.waitForFunction(before => window.volnaCurrentState
    .split('\n').find(line => line.startsWith('panel=38 pipeline ')) !== before, pipelineBefore);
  let pipelineAfter = await pipelineState();
  assert.ok(rowTop(pipelineAfter) > rowTop(pipelineBefore), 'navigator click reveals a sampled instruction');
  assert.equal(await selectedTransaction(), islandTransaction, 'navigator click preserves record selection');
  assert.equal(pipelineAfter.match(/row_px=([^ ]+)/)[1], pipelineBefore.match(/row_px=([^ ]+)/)[1]);
  pipelineBefore = pipelineAfter;
  await page.mouse.move(navX - 40, navY + navH / 2);
  await page.mouse.wheel(0, 36);
  await page.waitForFunction(before => window.volnaCurrentState
    .split('\n').find(line => line.startsWith('panel=38 pipeline ')).match(/ top=([^ ]+)/)[1] !== before,
  pipelineBefore.match(/ top=([^ ]+)/)[1]);
  pipelineAfter = await pipelineState();
  assert.notEqual(pipelineView(pipelineAfter)[0], pipelineView(pipelineBefore)[0], 'wheel follows instruction time');
  assert.equal(await selectedTransaction(), islandTransaction);
  await page.mouse.move(navX, navY + navH / 2);
  await page.mouse.down();
  await page.mouse.move(navX - 40, navY + navH / 2, { steps: 5 });
  await page.mouse.up();
  await waitForNavigator(true);
  [navX, navY, navW, navH] = navigatorRect(await pipelineState());
  await page.screenshot({ path: `${results}/pipeline-navigator-dark.png` });
  await page.mouse.click(navX, navY + navH / 2);
  await waitForNavigator(false);
  [navX, navY, , navH] = navigatorRect(await pipelineState());
  await page.mouse.click(navX - 2, navY + navH / 2);
  await waitForNavigator(true);
  [navX, navY, navW, navH] = navigatorRect(await pipelineState());
  await page.mouse.move(navX, navY + navH / 2);
  await page.mouse.down();
  await page.mouse.move(navX + navW - 32, navY + navH / 2, { steps: 5 });
  await page.mouse.move(navX + navW + 10, navY + navH / 2, { steps: 5 });
  await page.mouse.up();
  await waitForNavigator(false);
  [navX, navY, , navH] = navigatorRect(await pipelineState());
  await page.mouse.click(navX - 2, navY + navH / 2);
  await waitForNavigator(true);
  report.behavior.push('pipeline navigator picking, stream-following wheel, resizing, collapse and restore preserve selection and zoom');
  const occupancyRect = state => state.match(/occupancy_rect=\(([^)]+)\)/)[1].split(',').map(Number);
  const waitForOccupancy = visible => page.waitForFunction(visible => {
    const state = window.volnaCurrentState.split('\n').find(line => line.startsWith('panel=38 pipeline '));
    const height = Number(state?.match(/occupancy_rect=\(([^)]+)\)/)?.[1].split(',')[3]);
    return state?.includes(`occupancy=(${visible},96)`) && (visible ? height > 0 : height === 0);
  }, visible);
  await waitForOccupancy(true);
  let [plotX, plotY, plotW, plotH] = occupancyRect(await pipelineState());
  assert.ok(plotW > 0 && plotH > 0);
  const plotRows = rowTop(await pipelineState());
  const plotView = pipelineView(await pipelineState());
  await page.mouse.click(plotX + plotW * 0.7, plotY + plotH * 0.6);
  await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
  assert.deepEqual(pipelineView(await pipelineState()), plotView, 'occupancy clicks use the canvas time range');
  assert.equal(await selectedTransaction(), islandTransaction, 'occupancy navigation preserves selected record');
  assert.equal(rowTop(await pipelineState()), plotRows, 'occupancy navigation preserves row position');
  for (const theme of ['dark', 'light']) {
    await page.evaluate(async ({ base, theme }) => {
      (await import(`${base}viewer/pkg/volna.js`)).set_settings(JSON.stringify({ 'appearance.theme': `volna-islands-${theme}` }));
      await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
    }, { base, theme });
    await page.screenshot({ path: `${results}/pipeline-occupancy-${theme}.png` });
  }
  const beforePlotZoom = await pipelineState();
  const beforePlotZoomView = pipelineView(beforePlotZoom);
  const beforePlotZoomCursor = beforePlotZoom.match(/cursor=([^ ]+)/)[1];
  await page.keyboard.down('Control');
  await page.mouse.move(plotX + plotW * 0.2, plotY + plotH * 0.5);
  await page.mouse.down();
  await page.mouse.move(plotX + plotW * 0.8, plotY + plotH * 0.5, { steps: 5 });
  await page.screenshot({ path: `${results}/pipeline-occupancy-zoom-light.png` });
  await page.mouse.up();
  await page.keyboard.up('Control');
  await page.waitForFunction(width => {
    const state = window.volnaCurrentState.split('\n').find(line => line.startsWith('panel=38 pipeline '));
    const view = state.match(/viewport=\(([^)]+)\)/)[1].split(',').map(Number);
    return view[1] - view[0] < width * 0.7;
  }, beforePlotZoomView[1] - beforePlotZoomView[0]);
  const afterPlotZoom = await pipelineState();
  assert.equal(afterPlotZoom.match(/cursor=([^ ]+)/)[1], beforePlotZoomCursor, 'occupancy Ctrl-drag preserves cursor');
  assert.equal(rowTop(afterPlotZoom), rowTop(beforePlotZoom), 'occupancy Ctrl-drag preserves row position');
  assert.equal(afterPlotZoom.match(/row_px=([^ ]+)/)[1], beforePlotZoom.match(/row_px=([^ ]+)/)[1], 'occupancy Ctrl-drag preserves row height');
  assert.equal(await selectedTransaction(), islandTransaction, 'occupancy Ctrl-drag preserves selected record');
  await page.mouse.click(plotX + plotW / 2, plotY - 5);
  await waitForOccupancy(false);
  [plotX, plotY, plotW] = occupancyRect(await pipelineState());
  await page.mouse.click(plotX + plotW / 2, plotY - 5);
  await waitForOccupancy(true);
  report.behavior.push('pipeline occupancy follows canvas time; cursor placement, collapse and restore preserve rows and selection in both themes');
  const navigatorThemes = [
    'volna', 'volna-dark', 'volna-light', 'volna-mixed', 'volna-islands-dark',
    'volna-islands-light', 'one-dark', 'dracula', 'catppuccin-mocha',
    'catppuccin-latte', 'github-dark', 'github-light', 'vscode-dark', 'vscode-light',
  ];
  await page.mouse.move(600, 400);
  for (const theme of navigatorThemes) {
    await page.evaluate(async ({ base, theme }) => {
      (await import(`${base}viewer/pkg/volna.js`)).set_settings(JSON.stringify({ 'appearance.theme': theme }));
      await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
    }, { base, theme });
    await waitForNavigator(true);
    [navX, navY, navW, navH] = navigatorRect(await pipelineState());
    assert.equal(await selectedTransaction(), islandTransaction, `${theme}: theme switch preserves selection`);
    await page.screenshot({ path: `${results}/minimap-${theme}.png`, clip: {
      x: navX - 120, y: navY - 20, width: navW + 120, height: navH + 20,
    } });
  }
  for (const theme of ['dark-modern', 'light-modern', 'dark-high-contrast', 'light-high-contrast']) {
    const snapshot = await readFile(new URL(`../../volna-core/tests/fixtures/vscode/${theme}.txt`, import.meta.url), 'utf8');
    await page.evaluate(async ({ base, snapshot }) => {
      (await import(`${base}viewer/pkg/volna.js`)).set_vscode_theme(snapshot);
      await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
    }, { base, snapshot });
    await waitForNavigator(true);
    [navX, navY, navW, navH] = navigatorRect(await pipelineState());
    await page.screenshot({ path: `${results}/minimap-host-${theme}.png`, clip: {
      x: navX - 120, y: navY - 20, width: navW + 120, height: navH + 20,
    } });
  }
  await page.evaluate(async base => {
    (await import(`${base}viewer/pkg/volna.js`)).set_settings(JSON.stringify({ 'appearance.theme': 'volna-islands-dark' }));
  }, base);
  report.behavior.push('minimap viewport, aborted status marks and grip rendered in every bundled theme and four host palettes');
  await page.mouse.move(550, 620);
  await page.mouse.down();
  await page.mouse.move(600, 620, { steps: 3 });
  await page.mouse.move(950, 500, { steps: 3 });
  await page.mouse.up();
  assert.deepEqual(appErrors, [], 'dragging the Table panel must not panic in the browser');
  report.behavior.push('landing screenshot, header destinations, docs overview, standalone app default workspace, local file, workspace import and export, Table panel drag');
  await page.goto(`${origin}${base}vtr/`);
  const guide = await readFile(new URL('../../vtr/README.md', import.meta.url), 'utf8');
  const commands = [...guide.matchAll(/```sh\n([\s\S]*?)\n```/g)].map(match => match[1]);
  const renderedCommands = page.locator('.expressive-code pre');
  assert.equal(await renderedCommands.count(), commands.length, 'guide must render all current command blocks');
  for (const [index, command] of commands.entries()) {
    const lines = (await renderedCommands.nth(index).locator('.ec-line').allTextContents()).map(line => line === '\n' ? '' : line);
    assert.equal(lines.join('\n'), command, 'rendered commands must match the canonical guide');
  }
  assert.equal(await page.getByRole('link', { name: 'waveform example', exact: true }).getAttribute('href'),
    'https://github.com/ripopov/vtr/blob/main/vtr-rs/examples/waveform.rs');
  await page.goto(`${origin}${base}authoring/`);
  await verifyDiagrams(page);
  await page.keyboard.press('Tab');
  const focus = await page.evaluate(() => ({ width: getComputedStyle(document.activeElement).outlineWidth, style: getComputedStyle(document.activeElement).outlineStyle }));
  assert.notEqual(focus.width, '0px', 'keyboard focus must be visible');
  assert.notEqual(focus.style, 'none');

  await page.locator('starlight-theme-select select').first().selectOption('light');
  assert.equal(await page.locator('html').getAttribute('data-theme'), 'light');
  await page.reload();
  assert.equal(await page.locator('html').getAttribute('data-theme'), 'light', 'theme must persist');
  await page.locator('starlight-theme-select select').first().selectOption('dark');
  for (const route of ['vtr/logging/', 'volna-server/']) {
    await page.goto(`${origin}${base}${route}`);
    const title = (await page.locator('h1').textContent()).trim();
    await page.locator('button[data-open-modal]').click();
    await page.locator('.pagefind-ui__search-input').fill(title);
    await page.locator(`.pagefind-ui__result-link[href="${base}${route}"]`).waitFor();
    await page.keyboard.press('Escape');
  }
  report.behavior.push('static Mermaid SVG, keyboard focus, persistent theme, local Pagefind search');

  for (const theme of ['dark', 'light']) {
    await page.locator('starlight-theme-select select').first().selectOption(theme);
    await page.goto(`${origin}${base}vtr/`);
    const leftToggle = page.getByRole('button', { name: 'Documentation navigator', exact: true });
    const rightToggle = page.getByRole('button', { name: 'On this page', exact: true });
    const leftPanel = page.locator('#starlight__sidebar');
    const rightPanel = page.locator('.right-sidebar-container');
    const initial = await page.locator('.main-pane').boundingBox();
    assert.equal(await leftToggle.getAttribute('aria-expanded'), 'true');
    assert.equal(await rightToggle.getAttribute('aria-expanded'), 'true');
    await leftToggle.focus();
    await page.keyboard.press('Enter');
    assert.equal(await leftToggle.getAttribute('aria-expanded'), 'false');
    assert.equal(await leftPanel.isVisible(), false);
    assert.equal(await rightPanel.isVisible(), true, 'left toggle must leave the right panel visible');
    assert.ok((await page.locator('.main-pane').boundingBox()).x < initial.x, 'left panel space must be released');
    await leftToggle.click();
    await rightToggle.click();
    assert.equal(await leftPanel.isVisible(), true, 'right toggle must leave the left panel visible');
    assert.equal(await rightPanel.isVisible(), false);
    assert.ok((await page.locator('.main-pane').boundingBox()).width > initial.width, 'right panel space must be released');
    await leftToggle.click();
    await page.goto(`${origin}${base}vtr/specification/`);
    await page.reload();
    assert.equal(await leftPanel.isVisible(), false, 'left preference must persist');
    assert.equal(await rightPanel.isVisible(), false, 'right preference must persist');
    assert.equal(await leftToggle.getAttribute('aria-expanded'), 'false');
    assert.equal(await rightToggle.getAttribute('aria-expanded'), 'false');
    await page.screenshot({ path: `${results}/panels-hidden-${theme}-1440.png` });
    const audit = await new AxeBuilder({ page }).withTags(['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa']).analyze();
    assert.deepEqual(audit.violations.map(v => v.id), [], 'collapsed panels must remain accessible');
    await page.setViewportSize({ width: 900, height: 1000 });
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth + 1), false, 'intermediate header must fit');
    assert.equal(await leftToggle.isVisible(), true);
    assert.equal(await rightToggle.isVisible(), false, 'right toggle is only available with the desktop contents panel');
    await page.setViewportSize({ width: 360, height: 800 });
    assert.equal(await leftToggle.isVisible(), false);
    await page.locator('button[popovertarget="starlight__sidebar"]').click();
    assert.equal(await leftPanel.isVisible(), true, 'saved desktop preference must not hide the mobile menu');
    await page.locator('button[popovertarget="starlight__sidebar"]').click();
    const mobileContents = page.locator('#starlight__on-this-page--mobile');
    assert.equal(await mobileContents.isVisible(), true, 'saved desktop preference must not hide mobile page navigation');
    await mobileContents.click();
    assert.equal(await page.locator('#starlight__mobile-toc').getAttribute('open'), '');
    await mobileContents.click();
    await page.setViewportSize({ width: 1440, height: 1000 });
    await leftToggle.click();
    await rightToggle.click();
  }
  const unavailableStorage = await browser.newContext({ viewport: { width: 1440, height: 1000 } });
  await unavailableStorage.addInitScript(() => {
    Object.defineProperty(window, 'localStorage', { get() { throw new DOMException('Storage unavailable', 'SecurityError'); } });
  });
  const storagePage = await unavailableStorage.newPage();
  await storagePage.goto(`${origin}${base}vtr/`);
  await storagePage.getByRole('button', { name: 'Documentation navigator', exact: true }).click();
  assert.equal(await storagePage.locator('#starlight__sidebar').isVisible(), false);
  await storagePage.getByRole('button', { name: 'Documentation navigator', exact: true }).click();
  assert.equal(await storagePage.locator('#starlight__sidebar').isVisible(), true);
  await unavailableStorage.close();
  report.behavior.push('independent panel toggles, keyboard activation, layout space, persisted preferences, mobile resizing, unavailable storage');

  await page.goto(`${origin}${base}authoring/`);
  const ticks = page.locator('[data-ticks]');
  const scale = page.locator('[data-scale]');
  const output = page.locator('[data-time-unit] output');
  await ticks.fill('9007199254740993');
  assert.equal(await output.textContent(), '9007199.254740993 s', 'timestamps above 2^53 must stay exact');
  await ticks.fill('18446744073709551615');
  await scale.selectOption('-15');
  assert.equal(await output.textContent(), '18446.744073709551615 s');
  await ticks.fill('18446744073709551616');
  assert.equal(await ticks.getAttribute('aria-invalid'), 'true');
  await ticks.fill('-1');
  assert.equal(await ticks.getAttribute('aria-invalid'), 'true');
  await ticks.fill('0');
  assert.equal(await ticks.getAttribute('aria-invalid'), null);
  assert.equal(await output.textContent(), '0.000000000000000 s');
  report.behavior.push('exact u64 conversion, maximum value, overflow/negative rejection, zero');

  const noJs = await browser.newContext({ javaScriptEnabled: false });
  const staticPage = await noJs.newPage();
  await staticPage.goto(`${origin}${base}`);
  assert.match(await staticPage.locator('.sl-markdown-content').textContent(), /sample workspace shows waveforms/);
  assert.equal(await staticPage.getByRole('link', { name: 'Read the Docs' }).first().getAttribute('href'), './docs/');
  await staticPage.goto(`${origin}${base}vtr/`);
  assert.equal(await staticPage.getByRole('button', { name: 'Documentation navigator', exact: true }).isVisible(), false);
  assert.equal(await staticPage.locator('#starlight__sidebar').isVisible(), true);
  assert.equal(await staticPage.locator('.right-sidebar-container').isVisible(), true);
  await staticPage.goto(`${origin}${base}authoring/`);
  await verifyDiagrams(staticPage);
  for (const route of ['docs/', 'vtr/', 'vtr/specification/', 'authoring/', 'volna-trace/', 'volna-server/']) {
    await staticPage.goto(`${origin}${base}${route}`);
    assert.equal(await staticPage.locator('h1').count(), 1, `${route}: static page title`);
    assert.ok((await staticPage.locator('.sl-markdown-content').textContent()).trim().length > 20, `${route}: static content must not be empty`);
    assert.ok(await staticPage.locator('.sl-markdown-content h2').count() > 0, `${route}: sections must render without JavaScript`);
  }
  await noJs.close();
  report.behavior.push('readable docs and diagrams without JavaScript');
  await page.setViewportSize({ width: 360, height: 800 });
  await page.goto(`${origin}${base}vtr/`);
  await page.locator('button[popovertarget="starlight__sidebar"]').click();
  const sidebar = page.locator('#starlight__sidebar');
  assert.equal(await sidebar.evaluate(element => element.matches(':popover-open')), true);
  await sidebar.locator(`a[href="${base}vtr/logging/"]`).click();
  await page.waitForURL(`${origin}${base}vtr/logging/`);
  await page.locator('button[popovertarget="starlight__sidebar"]').click();
  await sidebar.locator(`a[href="${base}volna-server/"]`).click();
  await page.waitForURL(`${origin}${base}volna-server/`);
  await page.locator('main a[href$="volna-trace/"]').first().click();
  await page.waitForURL(`${origin}${base}volna-trace/`);
  report.behavior.push('mobile menu, server navigation, and canonical crate cross-links');
  await page.close();
  await behaviorContext.close();
  const routes = ['', 'docs/', 'vtr/', 'vtr/specification/', 'vtr/rationale/', 'vtr/logging/', 'volna-trace/', 'volna-server/', 'volna-core/', 'volna/', 'authoring/', 'design/', '404.html'];
  for (const theme of ['dark', 'light']) {
    for (const width of [1440, 360]) {
      const context = await browser.newContext({ viewport: { width, height: 1000 }, colorScheme: theme, reducedMotion: 'reduce' });
      await context.addInitScript(() => Object.defineProperty(navigator, 'hardwareConcurrency', { get: () => 4 }));
      const page = await context.newPage();
      const errors = [];
      page.on('pageerror', error => errors.push(error.message));
      page.on('response', response => { if (response.status() >= 400) errors.push(`${response.status()}: ${response.url()}`); });
      for (const route of routes) {
        const response = await page.goto(`${origin}${base}${route}`);
        assert.equal(response.status(), 200, route);
        await page.evaluate(() => document.fonts.ready);
        await page.waitForFunction(() => [...document.querySelectorAll('.expressive-code pre')].every(pre => pre.scrollWidth <= pre.clientWidth || pre.tabIndex === 0));
        const state = await page.evaluate(() => ({
          overflow: document.documentElement.scrollWidth > innerWidth + 1,
          theme: document.documentElement.dataset.theme,
          headings: document.querySelectorAll('h1').length,
          badImages: [...document.images].filter(img => !img.complete || img.naturalWidth === 0).map(img => img.src),
          font: getComputedStyle(document.body).fontFamily,
          inter: document.fonts.check('16px Inter'),
          movement: [...document.querySelectorAll('*')].some(el => {
            const style = getComputedStyle(el);
            return style.animationName !== 'none' && style.animationDuration !== '0s';
          }),
        }));
        assert.equal(state.overflow, false, `${route} ${theme} ${width}: page overflow`);
        assert.equal(state.theme, theme);
        assert.equal(state.headings, 1, route);
        assert.deepEqual(state.badImages, [], route);
        assert.match(state.font, /Inter/);
        assert.equal(state.inter, true);
        assert.equal(state.movement, false);
        const content = await page.locator('.sl-markdown-content').textContent();
        assert.ok(content.trim().length > 20, `${route}: rendered content must not be empty`);
        assert.deepEqual(errors, [], `${route}: browser errors`);
        if (width === 1440) {
          const audit = await new AxeBuilder({ page }).withTags(['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa']).analyze();
          report.accessibility.push({ route, theme, violations: audit.violations });
          assert.deepEqual(audit.violations.map(v => `${v.id}: ${v.nodes.map(n => n.target).join(', ')}`), [], `${route} ${theme}: accessibility`);
        }
        if (['', 'docs/', 'vtr/', 'authoring/', 'vtr/specification/', 'volna-trace/', 'volna-server/'].includes(route)) {
          await page.screenshot({ path: `${results}/${route.replaceAll('/', '-') || 'landing'}-${theme}-${width}.png`, fullPage: true });
        }
        report.pages.push({ route, theme, width, ...state });
      }
      await context.close();
    }
  }

  console.log(`Verified ${report.pages.length} route/theme/viewport combinations, ${report.accessibility.length} accessibility audits, search, widgets, and static reading.`);
} finally {
  await writeReport();
  if (browser) await browser.close();
  await new Promise(done => server.close(done));
}
async function writeReport() {
  const { writeFile } = await import('node:fs/promises');
  await writeFile(`${results}/report.json`, JSON.stringify(report, null, 2) + '\n');
}

function verifySampleWorkspace(state) {
  assert.match(state, /panel=39 .*items=15 loaded=13 .*cursor=Some\(69\).*viewport=\(46,95\)/);
  assert.match(state, /panel=38 pipeline track=soc.cpu0.pipeline.instruction .*ready rows=420/);
  assert.match(state, /panel=40 table .*rows=420/);
  const transaction = state.split('\n').find(line => line.includes('panel=37 transaction '));
  assert.ok(transaction?.includes(`ready record=Some(("${sampleRecord.track.join('.')}", ${sampleRecord.id}))`), state);
}

async function verifySampleIslandContour(page, screenshot) {
  const colors = await page.evaluate(async ({ bytes, sidebarWidth }) => {
    const bitmap = await createImageBitmap(new Blob([new Uint8Array(bytes)], { type: 'image/png' }));
    const canvas = new OffscreenCanvas(bitmap.width, bitmap.height);
    const context = canvas.getContext('2d');
    context.drawImage(bitmap, 0, 0);
    bitmap.close();
    const points = { untabbed: [4, 58], headerLeft: [sidebarWidth + 4, 58], contentLeft: [sidebarWidth + 4, 78], headerTop: [sidebarWidth + 20, 44] };
    return Object.fromEntries(Object.entries(points).map(([name, [x, y]]) =>
      [name, [...context.getImageData(x, y, 1, 1).data]]));
  }, { bytes: [...screenshot], sidebarWidth: 423 });
  for (const edge of ['headerLeft', 'contentLeft', 'headerTop']) {
    assert.deepEqual(colors[edge], colors.untabbed, `${edge}: tabbed and untabbed island contours must match`);
  }
}

async function verifyDiagrams(page) {
  const diagrams = page.locator('svg[id^="mermaid"]');
  assert.ok(await diagrams.count() > 0, 'Mermaid must render as static SVG');
  for (const diagram of await diagrams.all()) {
    const state = await diagram.evaluate(svg => ({
      width: svg.viewBox.baseVal.width,
      height: svg.viewBox.baseVal.height,
      labelled: ['aria-labelledby', 'aria-describedby'].every(attribute => {
        const ids = svg.getAttribute(attribute)?.trim().split(/\s+/);
        return ids?.length && ids.every(id => svg.querySelector(`[id="${id}"]`)?.textContent.trim());
      }),
    }));
    assert.ok(state.width > 0 && state.height > 0, 'diagram must have drawable dimensions');
    assert.ok(state.labelled, 'diagram must reference nonempty accessible title and description');
  }
}
