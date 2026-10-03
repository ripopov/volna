# Volna documentation

A static Astro/Starlight site authored in Markdown, with build-time Mermaid SVGs,
original SVG figures, and small HTML/JavaScript widgets. The shared Volna design
system lives in `design-system/`; the agent entry is
`../.agents/skills/volna-design`.

## Build

Requires Node.js 22.19+ and Chromium for Mermaid. From this directory:

```sh
npm ci
npx playwright install chromium
npm run check
npm run build
npm test
npm run preview
```

On macOS, installed Google Chrome is used automatically. `CHROME_PATH` overrides
the executable; otherwise Playwright's installed Chromium is used. Linux CI
installs Chromium and its system dependencies with
`npx playwright install --with-deps chromium`.

The complete static website is in `dist/`, including the local Pagefind search
index. Serve that directory from any static host. No backend, old checkout, CDN,
or network request is required for fonts, diagrams, or widgets.

Use `npm run dev` for live authoring. Set `DOCS_BASE=/volna/` on the build and test
commands for hosting under a prefix. Use the same environment when previewing.
Set `DOCS_SITE` to the public site URL when building for a known host to generate
a sitemap. Without it, the sitemap integration is skipped.

## GitHub Pages

The [documentation workflow](../.github/workflows/docs.yml) builds and tests with
`DOCS_BASE=/volna/` and `DOCS_SITE=https://ripopov.github.io`. Successful builds
on `main` deploy to [ripopov.github.io/volna](https://ripopov.github.io/volna/).
Pull requests run the same checks without deploying.

Pushes affecting documentation, VTR guides, the design skill, or the workflow
trigger the build. The workflow also supports manual runs for redeployment.
The repository's Pages publishing source must be set to **GitHub Actions**.
Deployment uses the `github-pages` environment and grants Pages/OIDC write
permissions only to the deployment job.

## Sources

The content loader reads `../vtr/README.md` as the VTR introduction and
`../vtr/docs/{SPEC,RATIONALE,LOGGING}.md` as the remaining VTR pages. There are
no authored VTR pages; edit those crate files for VTR changes, and the
development server watches them for updates. The loader supplies page metadata,
removes the top-level title, and adapts repository links and image paths to web
routes in memory without generating Markdown copies.
The specification is normative; the extracted crate's current documentation
supersedes the prototype's broader documentation and application proposals.

Authored pages live in `src/content/docs/`. See the
[authoring guide](src/content/docs/authoring.md) for SVG, Mermaid, widget,
navigation, and accessibility conventions. The
[design system guide](design-system/readme.md) documents shared design ownership
and import scope. The sync step also mirrors the canonical brand SVG and the VTR container
schematic into `public/assets/`; these generated assets are ignored by Git.

## Verification

`npm run check` checks Astro and TypeScript. The build also checks local links,
anchors, and static assets. `npm test` serves the built output and tests all docs
routes in desktop/mobile light/dark layouts, accessibility, search, no-JavaScript
reading, SVG rendering, focus, reduced motion, and exact timestamp conversion.
Screenshots are written to `test-results/` for visual review.
