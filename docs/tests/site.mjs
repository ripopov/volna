import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { existsSync } from 'node:fs';
import { mkdir, readFile, stat } from 'node:fs/promises';
import { resolve, extname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { chromium } from 'playwright';
import AxeBuilder from '@axe-core/playwright';

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
    res.writeHead(200, { 'Content-Type': mime[extname(file)] || 'application/octet-stream' });
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
  const page = await behaviorContext.newPage();
  const appErrors = [];
  page.on('pageerror', error => appErrors.push(error.message));
  await page.goto(`${origin}${base}`);
  assert.equal(await page.locator('.site-title').getAttribute('href'), base);
  assert.equal(await page.getByRole('navigation', { name: 'Site' }).getByRole('link', { name: 'Docs' }).getAttribute('href'), `${base}docs/`);
  assert.equal(await page.getByRole('navigation', { name: 'Site' }).getByRole('link', { name: 'Web App' }).getAttribute('href'), `${base}app/`);
  const sample = page.frameLocator('iframe[title="Interactive Volna sample recording"]');
  await sample.locator('body[data-workspace-restored="true"]').waitFor({ timeout: 60000 });
  const sampleState = await sample.locator('body').evaluate(() => window.volnaCurrentState);
  assert.match(sampleState, /panel=1 .*items=\d+/);
  assert.match(sampleState, /panel=4 table .*rows=\d+/);
  assert.match(sampleState, /panel=3 transaction/);
  await page.screenshot({ path: `${results}/landing-restored-dark-1440.png`, fullPage: true });
  await page.setViewportSize({ width: 1440, height: 1600 });
  const frameBounds = await page.locator('.v-viewer-frame').boundingBox();
  assert.ok(frameBounds && Math.abs(frameBounds.x - 72) < 2 && Math.abs(frameBounds.width - 1296) < 2,
    'landing viewer must occupy 90% of the viewport width');
  assert.ok(Math.abs(1600 - frameBounds.y - frameBounds.height - 24) < 2,
    'landing viewer must grow to fill available viewport height');
  await page.setViewportSize({ width: 360, height: 800 });
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth + 1), false);
  const frameScroll = page.locator('.v-viewer-frame');
  assert.ok(await frameScroll.evaluate(element => element.scrollWidth > element.clientWidth));
  await page.screenshot({ path: `${results}/landing-restored-dark-360.png`, fullPage: true });
  await page.getByRole('button', { name: 'Show right viewer panels' }).click();
  await page.waitForFunction(() => document.querySelector('.v-viewer-frame').scrollLeft > 0);
  await page.locator('html').evaluate(element => { element.dataset.theme = 'light'; });
  assert.equal(await page.locator('html').getAttribute('data-theme'), 'light');
  await page.evaluate(() => scrollTo(0, 0));
  await page.screenshot({ path: `${results}/landing-restored-light-360.png`, fullPage: true });
  await page.setViewportSize({ width: 1440, height: 1000 });
  await page.goto(`${origin}${base}docs/`);
  assert.match(await page.locator('h1').textContent(), /engineering docs/);
  assert.equal(await page.locator('.v-guide').count(), 10);
  await page.goto(`${origin}${base}app/`);
  assert.equal(await page.locator('.volna-header, #starlight__sidebar').count(), 0);
  await page.locator('#viewer-status').waitFor({ state: 'hidden', timeout: 60000 });
  const appCanvas = await page.locator('body > canvas').boundingBox();
  assert.ok(appCanvas && Math.abs(appCanvas.width - 1440) < 2 && Math.abs(appCanvas.height - 1000) < 2,
    'standalone canvas must fill the browser viewport');
  await page.locator('#recording-file').setInputFiles(fileURLToPath(new URL('../../volna/examples/landing.vtr', import.meta.url)));
  await page.locator('body[data-local-trace-opened="landing.vtr"]').waitFor({ timeout: 30000 });
  await page.evaluate(async base => (await import(`${base}viewer/pkg/volna.js`)).dispatch_command('openWorkspace'), base);
  await page.locator('input[aria-label="Open a Volna workspace"]').setInputFiles(fileURLToPath(new URL('../../volna/examples/landing.vtr.volna.json', import.meta.url)));
  await page.waitForFunction(() => window.volnaCurrentState?.includes('panel=4 table'));
  const downloadPromise = page.waitForEvent('download');
  await page.evaluate(async base => (await import(`${base}viewer/pkg/volna.js`)).dispatch_command('saveWorkspaceAs'), base);
  assert.equal((await downloadPromise).suggestedFilename(), 'landing.vtr.volna.json');
  await page.getByRole('button', { name: 'Load sample' }).click();
  await page.locator('body[data-workspace-restored="true"]').waitFor({ timeout: 60000 });
  await page.mouse.move(1130, 632);
  await page.mouse.down();
  await page.mouse.move(1180, 632, { steps: 3 });
  await page.mouse.move(950, 500, { steps: 3 });
  await page.mouse.up();
  assert.deepEqual(appErrors, [], 'dragging the Table panel must not panic in the browser');
  report.behavior.push('landing workspace restore, header destinations, docs overview, standalone app sample, local file, workspace import and export, Table panel drag');
  const failedViewer = await behaviorContext.newPage();
  await failedViewer.route('**/viewer/pkg/volna_bg.wasm', route => route.abort());
  await failedViewer.goto(`${origin}${base}`);
  await failedViewer.frameLocator('iframe[title="Interactive Volna sample recording"]').getByRole('alert').waitFor();
  assert.equal(await failedViewer.getByRole('link', { name: 'Read the Docs' }).first().getAttribute('href'), './docs/');
  await failedViewer.close();
  report.behavior.push('viewer initialization failure keeps landing documentation links usable');
  await page.goto(`${origin}${base}vtr/`);
  const waveform = await readFile(new URL('../../vtr/examples/waveform.rs', import.meta.url), 'utf8');
  const example = page.locator('.expressive-code pre').filter({ hasText: 'Minimal waveform round trip.' });
  // Expressive Code represents an empty line with a newline text node.
  const exampleLines = (await example.locator('.ec-line').allTextContents()).map(line => line === '\n' ? '' : line);
  assert.equal(exampleLines.join('\n'), waveform.trimEnd(), 'quick start must embed the current waveform source');
  assert.equal(await page.getByRole('link', { name: 'runnable waveform API example' }).getAttribute('href'),
    'https://github.com/ripopov/volna/blob/main/vtr/examples/waveform.rs');
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
  assert.match(await staticPage.locator('.sl-markdown-content').textContent(), /sample below opens a VTR recording/);
  assert.equal(await staticPage.getByRole('link', { name: 'Read the Docs' }).first().getAttribute('href'), './docs/');
  await staticPage.goto(`${origin}${base}vtr/`);
  assert.equal(await staticPage.getByRole('button', { name: 'Documentation navigator', exact: true }).isVisible(), false);
  assert.equal(await staticPage.locator('#starlight__sidebar').isVisible(), true);
  assert.equal(await staticPage.locator('.right-sidebar-container').isVisible(), true);
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
  const routes = ['', 'docs/', 'vtr/', 'vtr/specification/', 'vtr/rationale/', 'vtr/logging/', 'vtr-capi/', 'vtr-guard/', 'volna-trace/', 'volna-server/', 'volna-core/', 'volna/', 'authoring/', 'design/', '404.html'];
  for (const theme of ['dark', 'light']) {
    for (const width of [1440, 360]) {
      const context = await browser.newContext({ viewport: { width, height: 1000 }, colorScheme: theme, reducedMotion: 'reduce' });
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
        if (['', 'vtr/', 'authoring/', 'vtr/specification/', 'volna-trace/', 'volna-server/'].includes(route)) {
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
