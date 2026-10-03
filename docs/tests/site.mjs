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
const mime = { '.html': 'text/html', '.css': 'text/css', '.js': 'text/javascript', '.svg': 'image/svg+xml', '.woff2': 'font/woff2', '.wasm': 'application/wasm', '.json': 'application/json' };
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
  const routes = ['', 'vtr/', 'vtr/getting-started/', 'vtr/specification/', 'vtr/rationale/', 'vtr/logging/', 'authoring/', 'design/', '404.html'];
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
        if (['', 'vtr/', 'authoring/', 'vtr/specification/'].includes(route)) {
          await page.screenshot({ path: `${results}/${route.replaceAll('/', '-') || 'landing'}-${theme}-${width}.png`, fullPage: true });
        }
        report.pages.push({ route, theme, width, ...state });
      }
      await context.close();
    }
  }

  const page = await browser.newPage({ viewport: { width: 1440, height: 1000 } });
  await page.goto(`${origin}${base}vtr/`);
  assert.equal(await page.locator('svg[id^="mermaid"]').count(), 1, 'Mermaid must be a built SVG');
  assert.equal(await page.locator('svg[id^="mermaid"] title').textContent(), 'Trace and application boundaries');
  await page.keyboard.press('Tab');
  const focus = await page.evaluate(() => ({ width: getComputedStyle(document.activeElement).outlineWidth, style: getComputedStyle(document.activeElement).outlineStyle }));
  assert.notEqual(focus.width, '0px', 'keyboard focus must be visible');
  assert.notEqual(focus.style, 'none');

  await page.locator('starlight-theme-select select').first().selectOption('light');
  assert.equal(await page.locator('html').getAttribute('data-theme'), 'light');
  await page.reload();
  assert.equal(await page.locator('html').getAttribute('data-theme'), 'light', 'theme must persist');
  await page.locator('starlight-theme-select select').first().selectOption('dark');
  await page.locator('button[data-open-modal]').click();
  const search = page.locator('.pagefind-ui__search-input');
  await search.fill('log site');
  await page.locator('.pagefind-ui__result-link').first().waitFor();
  assert.ok((await page.locator('.pagefind-ui__result-link').allTextContents()).some(title => /logging|log|format/i.test(title)), 'search must find VTR content');
  await page.keyboard.press('Escape');
  report.behavior.push('static Mermaid SVG, keyboard focus, persistent theme, local Pagefind search');

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
  await staticPage.goto(`${origin}${base}vtr/`);
  assert.equal(await staticPage.locator('svg[id^="mermaid"]').count(), 1);
  assert.ok((await staticPage.locator('main').textContent()).includes('Trace and application boundaries'));
  await staticPage.goto(`${origin}${base}vtr/specification/`);
  assert.ok((await staticPage.locator('main').textContent()).includes('Conformance checklist'));
  await staticPage.goto(`${origin}${base}authoring/`);
  assert.ok((await staticPage.locator('main').textContent()).includes('ticks × 10^timescale'));
  await noJs.close();
  report.behavior.push('readable docs and diagrams without JavaScript');
  await page.setViewportSize({ width: 360, height: 800 });
  await page.goto(`${origin}${base}vtr/`);
  await page.locator('button[popovertarget="starlight__sidebar"]').click();
  const sidebar = page.locator('#starlight__sidebar');
  assert.equal(await sidebar.evaluate(element => element.matches(':popover-open')), true);
  await sidebar.getByRole('link', { name: 'Structured logging', exact: true }).click();
  await page.waitForURL(`${origin}${base}vtr/logging/`);
  report.behavior.push('mobile menu and guide navigation');
  await page.close();
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
