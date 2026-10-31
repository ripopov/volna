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

Browser import notices use `v-progress` with a percentage for measured phases.
Omit `aria-valuenow` while work is indeterminate; reduced motion leaves its fill
static. The text accompanying the bar names the phase and input/output sizes.

<div class="v-status">
<span>Reading input 42% · Input 128.0 MiB · Output 38.7 MiB</span>
<div class="v-progress" role="progressbar" aria-label="Import progress specimen" aria-valuemin="0" aria-valuemax="100" aria-valuenow="42" style="--progress:42%;width:100%"><span></span></div>
</div>

The landing page uses `v-viewer-preview` to present a screenshot linking to the
standalone Web App.

Controls have visible keyboard focus and native labels. Tables and code blocks
scroll within their own containers. Motion changes only color or opacity and
respects the reduced-motion preference. Backgrounds are flat, borders are thin,
and radii follow the shared 3/4/6/8/10px scale.

The sample viewer's title bar and panel toolbars are specimens of native action
buttons: a 24px control contains a 16px icon at 100% interface zoom, with a 1.5-unit
SVG stroke. Both scale with interface zoom. Enabled icons use the primary
foreground (`--viewer-text`), which is near white in dark themes and dark in light
themes. Muted foregrounds remain available for secondary information and disabled
controls.

The sample viewer's Table panel is the specimen for native column dividers.
Its numeric-format toolbar button pairs a number-sign icon with the current mode
(for example, `hex`) and defaults numeric data to hexadecimal.
Headers use a 40px row with a centered title, sort arrow, and format chip.
Enable the ID column or a numeric attribute in Columns to expose its format chip.
Inherited formats use a neutral chip; explicit column overrides use an accent
border and a pin. The chip opens compatible translators with sample values;
Use table default restores inheritance. Long titles end in an ellipsis, and
columns retain enough width for the sort arrow, format chip, and resize handle.
An upward or downward arrow marks the applied sort direction; paired arrows mark
an unsorted column. These controls scale with interface zoom.
Time columns retain their time representation, and format changes preserve row
selection, scroll position, and sorting. These preferences are saved with the table.

The Table toolbar's Filter button follows Columns. It reveals a 36px entry row
below the headers, with each editor aligned to the same column geometry as the
cells. Its count remains visible when the row is collapsed. Each editor exposes
matching modes, case sensitivity, and missing-value handling through its compact
mode button. Enter applies the draft; Escape restores the applied query. Clear
filters removes the rules without collapsing the entry row. The previous results
remain visible while the background worker evaluates a new query.


Each column has a 1px divider with an 8px grab area extending through the header
and body, including the trailing Attributes edge. Hover and drag use the
focused-border role and GPUI's column resize cursor. Dragging changes that
column's width and moves later columns;
overflow scrolls within the table. Double-click fits the header and visible
values. Focus a divider with Tab, resize with Left or Right, and fit with Enter.
Attributes fills unused space until given an explicit width. Workspace files
remember widths, including hidden columns; resetting columns restores defaults.

The native waveform point tracks in `volna/examples/landing.vtr` are a specimen
of inline labels, exact-time groups, and time-range groups. Labels use the viewer
panel surface and border; a severity-colored edge remains visible beside a
selected outline. Timestamp anchors and range brackets locate records independently
of label width. Group boundaries and lanes stay fixed while panning; edge labels
are clipped without moving their anchors. Three lanes bound track density; counts
retain individual occurrences, including rare errors. Logs and located events share the geometry
and hit targets. Record details reuses one panel for individual records and bounded,
searchable group pages, with log arguments separated from runtime source metadata.

The desktop header is a live specimen of the shared ghost icon buttons
(`v-btn v-btn--ghost v-btn--icon`). The left and right panel icons independently
hide or show the documentation navigator and page navigation. Each button reports
its panel's expanded state and remembers the preference across pages and visits
when browser storage is available. Hiding a panel releases its layout space.
At narrower widths, Starlight's mobile navigation controls remain available;
desktop preferences do not hide them. Without JavaScript, the desktop buttons
are hidden and the navigation panels remain visible.

## Engineering content

Use diagrams to explain structure and widgets to inspect exact relationships.
Describe implemented behavior and constraints. Distinguish the current library
from future application layers. Keep screenshot galleries, promotional claims,
and benchmark panels out of engineering guides.

The design import includes foundation tokens, reusable documentation components,
local fonts and their OFL licenses, and canonical SVG branding. The prototype's
website pages, UI-kit site content, screenshots, and benchmark components are
excluded.
