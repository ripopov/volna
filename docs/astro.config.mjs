import { copyFile, mkdir } from 'node:fs/promises';
import { existsSync } from 'node:fs';
import { defineConfig } from 'astro/config';
import { unified } from '@astrojs/markdown-remark';
import starlight from '@astrojs/starlight';
import rehypeMermaid from 'rehype-mermaid';
import { focusScrollRegions, rehypeFocusScrollRegions } from './scripts/focus-scroll-regions.mjs';

// Mirror the canonical brand asset and the VTR container schematic into the
// static public directory; both are ignored by Git.
await mkdir(new URL('./public/assets/', import.meta.url), { recursive: true });
await copyFile(new URL('./design-system/assets/volna.svg', import.meta.url),
  new URL('./public/assets/volna.svg', import.meta.url));
await copyFile(new URL('../vtr/docs/container.svg', import.meta.url),
  new URL('./public/assets/container.svg', import.meta.url));

const chrome = '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';
const executablePath = process.env.CHROME_PATH || (existsSync(chrome) ? chrome : undefined);

export default defineConfig({
  output: 'static',
  site: process.env.DOCS_SITE,
  base: process.env.DOCS_BASE || '/',
  trailingSlash: 'always',
  markdown: {
    processor: unified({
      rehypePlugins: [[rehypeMermaid, {
        strategy: 'inline-svg',
        css: new URL('./design-system/tokens/fonts.css', import.meta.url),
        launchOptions: { executablePath },
        mermaidConfig: { theme: 'base', securityLevel: 'strict', fontFamily: 'Inter, sans-serif' },
        errorFallback(_element, _diagram, error, file) { file.fail(error); },
      }], rehypeFocusScrollRegions],
    }),
  },
  integrations: [starlight({
    title: 'Volna',
    description: 'Engineering documentation for Volna and the VTR trace library.',
    logo: { src: './design-system/assets/volna.svg', replacesTitle: false },
    favicon: '/assets/volna.svg',
    customCss: ['./src/styles/starlight.css'],
    components: {
      Footer: './src/components/Footer.astro',
      Header: './src/components/Header.astro',
    },
    expressiveCode: {
      themes: ['one-dark-pro', 'github-light'],
      useStarlightUiTheme: true,
      plugins: [{ name: 'Static scroll focus', hooks: {
        postprocessRenderedBlock({ renderData }) { focusScrollRegions(renderData.blockAst); },
      } }],
    },
    sidebar: [
      { label: 'Overview', link: '/' },
      { label: 'VTR trace library', items: [
        { label: 'Introduction', slug: 'vtr' },
        { label: 'File format', slug: 'vtr/specification' },
        { label: 'Design rationale', slug: 'vtr/rationale' },
        { label: 'Structured logging', slug: 'vtr/logging' },
      ] },
      { label: 'C and simulator integration', items: [
        { label: 'C API', slug: 'vtr-capi' },
        { label: 'Crash guard', slug: 'vtr-guard' },
      ] },
      { label: 'Trace access', items: [
        { label: 'Local and remote loading', slug: 'volna-trace' },
        { label: 'Headless server', slug: 'volna-server' },
      ] },
      { label: 'Contributing', items: [
        { label: 'Writing documentation', slug: 'authoring' },
        { label: 'Design foundations', slug: 'design' },
      ] },
    ],
    tableOfContents: { minHeadingLevel: 2, maxHeadingLevel: 3 },
    pagination: true,
    credits: false,
  })],
});
