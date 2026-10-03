---
title: Design foundations
description: Shared Volna typography, surfaces, semantic colors, and engineering UI conventions.
---

The **Instrument** design direction connects documentation and application UI:
One Dark neutrals, a blue accent, Inter for prose and controls, and JetBrains Mono
for code and data. Both light and dark themes are supported.

## Shared source

`docs/design-system/` is the maintained source for semantic colors, viewer data
colors, typography, spacing, radii, elevation, motion, fonts, and branding.
The tokens and assets originate in the prototype repository's Volna design
system. The documentation adapter lives in `src/styles/starlight.css`.

Web application code can import `design-system/styles.css`, set
`data-theme="dark"` or `data-theme="light"`, and use the `v-` component classes.
Native frontends should map semantic roles from `tokens/theme.css` and
`tokens/viewer.css` into their toolkit's theme. Keep the shared token files
as the source, rather than duplicating color values in application components.

## Surfaces and type

<div class="v-swatches">
<div class="v-swatch v-swatch--raised">Raised surface<br /><code>--bg-raised</code></div>
<div class="v-swatch v-swatch--selected">Selected surface<br /><code>--accent-soft</code></div>
<div class="v-swatch v-swatch--code">Code surface<br /><code>--code-bg</code></div>
</div>

Body text uses Inter at 16px with a 1.55 line height. Headings use weight 600
and a 1.3 line height. The scale stays consistent across viewport widths:
30px page titles, 24px section headings, 20px subsections, and 16px minor
headings. Landing titles use 32px. Sizes use rem units relative to the browser's
default root size.
Code uses JetBrains Mono with ligatures disabled. Use the monochrome brand mark
or the canonical app icon without recoloring it.

| Role | Tokens |
| --- | --- |
| Main / secondary / faint text | `--text-1`, `--text-2`, `--text-3` |
| Page / panel / inset | `--bg`, `--bg-raised`, `--bg-sunken` |
| Interactive accent / keyboard focus | `--accent`, `--focus-ring` |
| Borders | `--border`, `--border-subtle`, `--border-strong` |
| State | `--success`, `--warning`, `--danger`, `--info` |
| Viewer data | `--viewer-signal`, `--viewer-undef`, `--viewer-highimp`, markers and stages |

## Components

Guide links, a project status line, figures, and the timestamp widget extend the
shared `v-` classes. Their specimens are the [landing page](../), the
[VTR introduction](../vtr/), and the [authoring calculator](../authoring/#html-and-javascript-widgets).
They use semantic tokens; they introduce no independent color palette.

Controls have visible keyboard focus and native labels. Tables and code blocks
scroll within their own containers. Motion changes only color or opacity and
respects the reduced-motion preference. Backgrounds are flat, borders are thin,
and radii follow the shared 3/4/6/8/10px scale.

## Engineering content

Use diagrams to explain structure and widgets to inspect exact relationships.
Describe implemented behavior and constraints. Distinguish the current library
from future application layers. Keep screenshot galleries, promotional claims,
and benchmark panels out of engineering guides.

The design import includes foundation tokens, reusable documentation components,
local fonts and their OFL licenses, and canonical SVG branding. The prototype's
website pages, UI-kit site content, screenshots, and benchmark components are
excluded.
