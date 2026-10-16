import { cp, mkdir } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { buildWebViewer, buildWebConverter } from '../../volna/tools/build-web.mjs';

const pkg = fileURLToPath(new URL('../public/viewer/pkg/', import.meta.url));
const examples = fileURLToPath(new URL('../public/examples/', import.meta.url));

await mkdir(examples, { recursive: true });
await buildWebViewer(pkg);
await buildWebConverter(fileURLToPath(new URL('../public/viewer/converter/', import.meta.url)));
for (const name of ['landing.vtr', 'landing-page.volna.json']) {
  await cp(new URL(`../../volna/examples/${name}`, import.meta.url), new URL(`../public/examples/${name}`, import.meta.url));
}
