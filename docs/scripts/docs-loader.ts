import { readFile } from 'node:fs/promises';
import { posix, relative, sep } from 'node:path';
import { fileURLToPath } from 'node:url';
import type { Loader } from 'astro/loaders';
import { docsLoader } from '@astrojs/starlight/loaders';

// Render crate guides directly from their canonical Markdown sources. Local
// links between guides map to website routes; other repository links retain
// their source locations. The container schematic is mirrored by Astro.
const sources = [
  ['vtr/README.md', 'vtr', 'VTR trace library', 'What a VTR recording contains, and how to write, read, and recover one.'],
  ['vtr/docs/SPEC.md', 'vtr/specification', 'VTR file format specification', 'The normative VTR 1.1 container, encodings, recovery rules, and activity sidecar.'],
  ['vtr/docs/RATIONALE.md', 'vtr/rationale', 'VTR design rationale', 'Trace storage decisions, library boundaries, and derived indexes.'],
  ['vtr/docs/LOGGING.md', 'vtr/logging', 'Structured logging', 'Declare typed log sites, write records, and query messages.'],
  ['vtr-capi/README.md', 'vtr-capi', 'VTR C API', 'Build and link libvtr, use opaque handles, and write a trace from C.'],
  ['vtr-guard/README.md', 'vtr-guard', 'VTR crash guard', 'Finish watched VTR writers after Linux faults, stop requests, and exit.'],
  ['volna-trace/README.md', 'volna-trace', 'Volna trace loading', 'Read immutable VTR/FST recordings locally or through a cooperative remote client.'],
  ['volna-server/README.md', 'volna-server', 'Volna server', 'Host one immutable recording over framed pipes, with complete objects and activity sidecars.'],
  ['volna-core/README.md', 'volna-core', 'Volna viewer core', 'Toolkit-independent viewer state, commands, panel models, and display lists.'],
  ['volna/README.md', 'volna', 'Volna desktop viewer', 'Build and run the GPUI viewer for VTR and FST recordings.'],
] as const;
export function volnaDocsLoader(): Loader {
  const authored = docsLoader();
  return {
    name: 'volna-docs-loader',
    async load(context) {
      await authored.load(context);
      // The authored loader removes entries outside its directory; load crate guides afterwards.
      const guides = new Map(sources.map(([source, slug, title, description]) => {
        const fileURL = new URL(`../../${source}`, import.meta.url);
        return [fileURLToPath(fileURL), { fileURL, id: slug, title, description }];
      }));
      const dependencies = new Map<string, Set<string>>();
      async function loadGuide(path: string) {
        const guide = guides.get(path)!;
        let body = await readFile(guide.fileURL, 'utf8');
        body = body.replace(/^# [^\n]+\n+/, '');
        body = body.replace(/\]\(([^)]+)\)/g, (match, href: string) => {
          if (/^(?:[a-z]+:|#|\/)/i.test(href)) return match;
          const target = new URL(href, guide.fileURL);
          const fragment = target.hash;
          target.hash = '';
          const path = fileURLToPath(target);
          const linkedGuide = guides.get(path) || (href.split('#')[0].endsWith('/')
            ? guides.get(fileURLToPath(new URL('README.md', target))) : undefined);
          if (linkedGuide) {
            const route = posix.relative(guide.id, linkedGuide.id) || '.';
            return `](${route}/${fragment})`;
          }
          const repoPath = relative(fileURLToPath(new URL('../../', import.meta.url)), path).split(sep).join('/');
          return `](https://github.com/ripopov/volna/blob/main/${repoPath}${fragment})`;
        });
        // README-relative repo path of the container schematic -> page-relative public asset.
        body = body.replace('src="docs/container.svg"', 'src="../assets/container.svg"');
        // Hidden Markdown markers embed source files on the website without
        // duplicating code in repository guides. Paths are relative to the guide.
        const includes = [...body.matchAll(/^<!-- include-code: (\S+) (\w+) -->[ \t]*$/gm)];
        const includedPaths = new Set(includes.map((match) => fileURLToPath(new URL(match[1], guide.fileURL))));
        dependencies.set(path, includedPaths);
        context.watcher?.add([...includedPaths]);
        for (const match of includes) {
          const code = await readFile(new URL(match[1], guide.fileURL), 'utf8');
          const fence = '`'.repeat(Math.max(3, ...[...code.matchAll(/`+/g)].map((run) => run[0].length + 1)));
          body = body.replace(match[0], () => `${fence}${match[2]}\n${code.trimEnd()}\n${fence}`);
        }
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
          const affected = [...guides.keys()].filter((guidePath) => guidePath === path || dependencies.get(guidePath)?.has(path));
          for (const guidePath of affected) {
            pending = pending.then(() => loadGuide(guidePath)).catch((error) => {
              context.store.delete(guides.get(guidePath)!.id);
              context.logger.error(`Failed to load ${guidePath}: ${error.message}`);
            });
          }
        };
        context.watcher.on('change', reload);
        context.watcher.on('add', reload);
        context.watcher.on('unlink', (path) => {
          if (guides.has(path)) {
            pending = pending.then(() => { context.store.delete(guides.get(path)!.id); });
          } else reload(path);
        });
      }
    },
  };
}
