---
title: Writing documentation
description: Build Markdown documentation with SVG, Mermaid, and small JavaScript widgets.
---

The docs website is a static Astro/Starlight build. Write prose in Markdown;
add a visual when it explains a data model, layout, or relationship more clearly
than text. Interaction should answer a specific engineering question.

## Run the site

From the repository root, with Node.js 22.19 or newer:

```sh
cd docs
npm ci
npx playwright install chromium
npm run dev
```

Mermaid renders in Chromium during the build. On macOS, an installed Google
Chrome is used automatically. Set `CHROME_PATH` to use another executable;
otherwise install Playwright Chromium with the command above.

```sh
npm run check
npm run build
npm run preview
npm test
```

`npm run build` writes the static website to `docs/dist/` and checks internal
links, anchors, and assets. `npm test` starts its own server for that built output
and verifies desktop/mobile layouts, themes, search, diagrams, accessibility,
and widget behavior. No application server is needed to serve `dist/`.

For a host below a URL prefix, build with `DOCS_BASE=/volna/ npm run build`.
Use the same prefix when serving and testing the build:
`DOCS_BASE=/volna/ npm test`. Internal authored links are relative.

## Content ownership

| Location | Responsibility |
| --- | --- |
| `vtr/README.md`, `vtr/docs/*.md` | Authoritative VTR guides |
| `docs/scripts/sync-vtr.mjs` | Copy the crate guide and three format guides, adapting web links |
| `docs/src/content/docs/` | Landing page and authored site guides |
| `docs/design-system/` | Shared design tokens, fonts, branding, and component foundations |
| `docs/src/styles/starlight.css` | Map the shared design roles onto Starlight |
| `docs/public/assets/` | Static engineering figures |
| `docs/src/scripts/` | Small bundled widget scripts |

The generated `vtr/getting-started.md`, `vtr/specification.md`,
`vtr/rationale.md`, and `vtr/logging.md` pages are ignored by Git and refreshed
before development, checking, and building. Edit their crate sources, then run
`npm run sync`. The specification stays intact apart from its page title and
repository-to-website links.

## Markdown

Each authored page begins with a `title` and `description` in YAML frontmatter.
Use sentence case headings and language-tagged code fences. Keep tables within
the content column; wide format tables and code scroll inside their own frame.
Add new navigation entries in `astro.config.mjs`.

Technical statements must describe the current workspace. Label proposals and
planned features. Keep performance claims tied to reproducible measurements;
this site currently makes no comparative benchmark claims.

## SVG figures

Store original SVGs under `public/assets/` and reference them with a relative
Markdown image or an HTML `figure`. Give each image useful alternative text.
Include `title` and `desc` inside the SVG. Label schematic layouts so their
geometry cannot be mistaken for measured data.

The [VTR introduction](../vtr/#container-structure) includes a schematic
container figure.

## Mermaid diagrams

Use a fenced block tagged `mermaid`, with `accTitle` and `accDescr` for an
accessible description. The build converts it to inline SVG, so a diagram remains
visible when browser JavaScript is disabled. Invalid diagrams fail the build.
The shared style adapter applies the selected theme to the SVG.

See the [component boundary diagram](../vtr/#trace-and-application-boundaries)
for an example. Keep diagrams compact enough to read on a phone; split large
models across figures.

## HTML and JavaScript widgets

Markdown accepts semantic HTML. Use labeled native controls, a textual result,
and a script in `src/scripts/`. `src/components/Footer.astro` bundles widget
scripts through Astro; no CDN or framework is needed. Provide the formula or
explanation in prose so the document still works without JavaScript.

The calculator below converts unsigned VTR timestamps to seconds for selected
time units. VTR time is `ticks × 10^timescale` seconds. A nonzero time-zero offset
is outside this calculator. Integer arithmetic preserves exact `u64` values.

<div class="v-widget" data-time-unit>
<div class="v-widget__fields">
<label>Timestamp (ticks)<input data-ticks type="text" inputmode="numeric" value="1000" pattern="[0-9]+" aria-describedby="time-help" /></label>
<label>Timescale<select data-scale><option value="0">0 · seconds</option><option value="-3">−3 · milliseconds</option><option value="-6">−6 · microseconds</option><option value="-9" selected>−9 · nanoseconds</option><option value="-12">−12 · picoseconds</option><option value="-15">−15 · femtoseconds</option></select></label>
</div>
<output aria-live="polite" aria-label="Time in seconds">0.000001000 s</output>
</div>

<p id="time-help">Example: 1,000 ticks at timescale −9 equals 0.000001 seconds (1 µs). With JavaScript enabled, changing either control updates the exact result.</p>
