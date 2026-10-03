import { readFile } from 'node:fs/promises';
import { relative } from 'node:path';
import { fileURLToPath } from 'node:url';
import type { Loader } from 'astro/loaders';
import { docsLoader } from '@astrojs/starlight/loaders';

// VTR pages are rendered straight from the crate tree; the README is the
// introduction and the crate guide at once, so there is no authored copy to
// drift from. Image paths are rewritten to public assets mirrored by the
// Astro config.
const sources = [
  ['README.md', 'vtr', 'VTR trace library', 'What a VTR recording contains, and how to write, read, and recover one.'],
  ['docs/SPEC.md', 'vtr/specification', 'VTR file format specification', 'The normative VTR 1.1 container, encodings, recovery rules, and activity sidecar.'],
  ['docs/RATIONALE.md', 'vtr/rationale', 'VTR design rationale', 'Trace storage decisions, library boundaries, and derived indexes.'],
  ['docs/LOGGING.md', 'vtr/logging', 'Structured logging', 'Declare typed log sites, write records, and query messages.'],
] as const;
// Cross-page links are written relative to the nested guide pages; links
// rewritten into the README render one level higher, so they need no prefix.
const routes = { SPEC: 'specification/', RATIONALE: 'rationale/', LOGGING: 'logging/' };

export function volnaDocsLoader(): Loader {
  const authored = docsLoader();
  return {
    name: 'volna-docs-loader',
    async load(context) {
      await authored.load(context);
      // The authored loader removes entries outside its directory; load crate guides afterwards.
      const guides = new Map(sources.map(([source, slug, title, description]) => {
        const fileURL = new URL(`../../vtr/${source}`, import.meta.url);
        return [fileURLToPath(fileURL), { fileURL, id: slug, title, description }];
      }));
      async function loadGuide(path: string) {
        const guide = guides.get(path)!;
        const prefix = guide.id.includes('/') ? '../' : '';
        let body = await readFile(guide.fileURL, 'utf8');
        body = body.replace(/^# [^\n]+\n+/, '');
        body = body.replace(/\]\((?:docs\/)?(SPEC|RATIONALE|LOGGING)\.md([^)]*)\)/g,
          (_, name: keyof typeof routes, fragment: string) => `](${prefix}${routes[name]}${fragment})`);
        body = body.replace('](../).', '](https://github.com/ripopov/volna/tree/main/vtr).');
        body = body.replace('](../LICENSE)', '](https://github.com/ripopov/volna/blob/main/LICENSE)');
        // README-relative repo path of the container schematic -> page-relative public asset.
        body = body.replace('src="docs/container.svg"', 'src="../assets/container.svg"');
        const data = await context.parseData({
          id: guide.id, filePath: path,
          data: { title: guide.title, description: guide.description },
        });
        const rendered = await context.renderMarkdown(body, { fileURL: guide.fileURL });
        context.store.set({ id: guide.id, data, body, filePath: relative(fileURLToPath(context.config.root), path), rendered });
      }
      for (const path of guides.keys()) await loadGuide(path);
      // Astro can log a render error and cache an entry without its HTML. Reject incomplete Markdown.
      for (const [id, entry] of context.store.entries()) {
        if (entry.filePath?.endsWith('.md') && !entry.rendered) {
          throw new Error(`Documentation failed to render: ${id}`);
        }
      }
      if (context.watcher) {
        context.watcher.add([...guides.keys()]);
        // Serialize reloads so an older render cannot overwrite a newer source revision.
        let pending = Promise.resolve();
        const reload = (path: string) => {
          if (!guides.has(path)) return;
          pending = pending.then(() => loadGuide(path)).catch((error) => {
            context.store.delete(guides.get(path)!.id);
            context.logger.error(`Failed to load ${path}: ${error.message}`);
          });
        };
        context.watcher.on('change', reload);
        context.watcher.on('add', reload);
        context.watcher.on('unlink', (path) => {
          if (guides.has(path)) {
            pending = pending.then(() => { context.store.delete(guides.get(path)!.id); });
          }
        });
      }
    },
  };
}
