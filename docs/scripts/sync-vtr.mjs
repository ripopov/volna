import { copyFile, mkdir, readFile, writeFile } from 'node:fs/promises';

// The public favicon is a build input mirrored from the shared canonical brand asset.
await copyFile(
  new URL('../design-system/assets/volna.svg', import.meta.url),
  new URL('../public/assets/volna.svg', import.meta.url),
);

// Keep the crate's normative guides authoritative while adapting repository links to web routes.
const sources = [
  ['README.md', 'getting-started', 'Getting started with VTR', 'Write, close, and query a trace with the Rust library.'],
  ['docs/SPEC.md', 'specification', 'VTR file format specification', 'The normative VTR 1.1 container, encodings, recovery rules, and activity sidecar.'],
  ['docs/RATIONALE.md', 'rationale', 'VTR design rationale', 'Trace storage decisions, library boundaries, and derived indexes.'],
  ['docs/LOGGING.md', 'logging', 'Structured logging', 'Declare typed log sites, write records, and query messages.'],
];
const routes = { 'SPEC.md': '../specification/', 'RATIONALE.md': '../rationale/', 'LOGGING.md': '../logging/' };
const output = new URL('../src/content/docs/vtr/', import.meta.url);
await mkdir(output, { recursive: true });
for (const [source, slug, title, description] of sources) {
  let body = await readFile(new URL(`../../vtr/${source}`, import.meta.url), 'utf8');
  body = body.replace(/^# [^\n]+\n+/, '');
  body = body.replace(/\]\((?:docs\/)?(SPEC|RATIONALE|LOGGING)\.md([^)]*)\)/g,
    (_, name, fragment) => `](${routes[`${name}.md`]}${fragment})`);
  body = body.replace('](../).', '](https://github.com/ripopov/volna/tree/main/vtr).');
  body = body.replace('](../LICENSE)', '](https://github.com/ripopov/volna/blob/main/LICENSE)');
  const header = `---\ntitle: ${JSON.stringify(title)}\ndescription: ${JSON.stringify(description)}\n---\n\n`;
  await writeFile(new URL(`${slug}.md`, output), header + body);
}
console.log(`Synced ${sources.length} VTR guides from the workspace crate.`);
