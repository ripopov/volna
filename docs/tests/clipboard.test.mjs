import assert from 'node:assert/strict';
import test from 'node:test';
import { existsSync } from 'node:fs';
import { mkdir, readFile } from 'node:fs/promises';
import { chromium } from 'playwright';

test('clipboard recovery preserves text, retries, and restores focus', async () => {
  const source = await readFile(new URL('../../volna/src/clipboard.js', import.meta.url));
  const chrome = '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';
  const browser = await chromium.launch({
    executablePath: process.env.CHROME_PATH || (existsSync(chrome) ? chrome : undefined),
  });
  try {
    const page = await browser.newPage();
    await page.setContent('<button id="source">Copy selection</button>');
    await page.locator('#source').focus();
    const text = 'Time\t125 ns\ntop.cpu.pc\t80000000';
    await page.evaluate(async ({ source, text }) => {
      const { copyText } = await import(`data:text/javascript;base64,${source}`);
      window.written = [];
      window.allowCopy = false;
      const host = {
        document,
        navigator: { clipboard: { writeText(value) {
          if (!window.allowCopy) throw new Error('Clipboard blocked');
          window.written.push(value);
          return Promise.resolve();
        } } },
      };
      window.copy = value => copyText(value, host);
      window.copy(text);
    }, { source: source.toString('base64'), text });

    const dialog = page.getByRole('dialog', { name: 'Copy text', exact: true });
    await dialog.waitFor();
    const area = dialog.getByRole('textbox', { name: 'Text to copy' });
    assert.equal(await area.inputValue(), text);
    assert.equal(await area.evaluate(element => document.activeElement === element), true);
    await dialog.getByRole('button', { name: 'Retry copy' }).click();
    await page.getByRole('status').filter({ hasText: 'Copying is still blocked' }).waitFor();
    assert.equal(await area.inputValue(), text);

    const results = new URL('../test-results/clipboard/', import.meta.url);
    await mkdir(results, { recursive: true });
    await page.screenshot({ path: new URL('fallback.png', results).pathname });
    await page.evaluate(() => { window.allowCopy = true; });
    await dialog.getByRole('button', { name: 'Retry copy' }).click();
    await dialog.waitFor({ state: 'detached' });
    assert.deepEqual(await page.evaluate(() => window.written), [text]);
    assert.equal(await page.locator('#source').evaluate(element => document.activeElement === element), true);

    await page.evaluate(() => window.copy('marker at 125 ns'));
    assert.deepEqual(await page.evaluate(() => window.written), [text, 'marker at 125 ns']);
    assert.equal(await dialog.count(), 0);

    await page.evaluate(() => { window.allowCopy = false; window.copy('marker at 125 ns'); });
    await dialog.waitFor();
    await page.keyboard.press('Escape');
    await dialog.waitFor({ state: 'detached' });
    assert.equal(await page.locator('#source').evaluate(element => document.activeElement === element), true);
  } finally {
    await browser.close();
  }
});
