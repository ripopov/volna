import { readdir, readFile, stat } from 'node:fs/promises';
import { resolve, join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(fileURLToPath(new URL('../dist/', import.meta.url)));
const base = `/${(process.env.DOCS_BASE || '/').split('/').filter(Boolean).join('/')}`.replace(/\/$/, '') + '/';
const origin = 'https://docs.invalid';
async function files(dir) {
  return (await Promise.all((await readdir(dir, { withFileTypes: true })).map(entry => {
    const path = join(dir, entry.name);
    return entry.isDirectory() ? files(path) : [path];
  }))).flat();
}
const htmlFiles = (await files(root)).filter(path => path.endsWith('.html'));
const documents = new Map(await Promise.all(htmlFiles.map(async path => [path, await readFile(path, 'utf8')])));
let count = 0;
const errors = [];
for (const [path, html] of documents) {
  const pageUrl = new URL(base + relative(root, path).replace(/index\.html$/, ''), origin);
  const references = [...html.matchAll(/<(?:a|img|link|script|source)\b[^>]*?\b(?:href|src)="([^"]+)"/g)].map(match => match[1]);
  for (const reference of references) {
    const url = new URL(reference.replaceAll('&amp;', '&'), pageUrl);
    if (url.origin !== origin) continue;
    count++;
    if (!url.pathname.startsWith(base)) {
      errors.push(`${relative(root, path)}: reference outside base ${reference}`);
      continue;
    }
    let target = resolve(root, decodeURIComponent(url.pathname.slice(base.length)));
    if (target !== root && !target.startsWith(root + '/')) throw new Error(`Reference outside output directory: ${reference}`);
    try {
      if ((await stat(target)).isDirectory()) target = join(target, 'index.html');
      await stat(target);
      if (url.hash && target.endsWith('.html')) {
        const doc = documents.get(target) || await readFile(target, 'utf8');
        const ids = new Set([...doc.matchAll(/\bid="([^"]+)"/g)].map(match => match[1]));
        if (!ids.has(decodeURIComponent(url.hash.slice(1)))) throw new Error('missing anchor');
      }
    } catch (error) {
      errors.push(`${relative(root, path)}: ${reference} (${error.message})`);
    }
  }
}
if (errors.length) throw new Error(`Broken local references:\n${errors.join('\n')}`);
console.log(`Verified ${count} local links and assets across ${htmlFiles.length} HTML pages.`);
