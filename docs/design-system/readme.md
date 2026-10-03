# Volna design system

Instrument is the shared visual foundation for Volna engineering documentation
and application UI. This directory is maintained in this repository. Its initial
tokens, font binaries, brand assets, and reusable component CSS were imported
from the [prototype design system](https://github.com/ripopov/vtr/tree/4b4fd692e7e03a7ebea4daf7a89787203eed9cb6/docs/design-system)
and its canonical Volna assets. No runtime dependency on that checkout remains.

## Ownership

- `tokens/`: palettes, semantic dark/light roles, viewer data colors, typography,
  spacing, radii, motion, and elevation. Preserve semantic roles across consumers.
- `fonts/`: Inter 400/600 and JetBrains Mono 400 WOFF2, with original OFL licenses.
- `assets/`: canonical Volna app icon and monochrome mark.
- `components/components.css`: reusable `v-` classes, including documentation
  navigation, content, guide cards, figures, and widgets.
- `styles.css`: shared entry point, usable by web application consumers.
- `../src/styles/starlight.css`: framework adapter. Starlight supplies layout,
  search, navigation, and theme persistence; this adapter supplies Volna styling.
- `../src/content/docs/design.md`: rendered foundation guide and component
  specimen links. Add specimens there or on an adopting page when adding classes.

The source repository's marketing pages, UI kits, React exports, screenshot
assets, benchmark components, and old visual baselines are not part of this
system. Application layers are under development; imported viewer tokens are a
shared design vocabulary, not proof that a viewer is implemented here.

## Direction

One Dark neutrals; blue accent (`--accent`); flat surfaces; 1px borders; radii
3/4/6/8/10px; Inter for prose/UI; JetBrains Mono for code/data. Both themes are
first-class. Do not recolor the app icon. Use waveform state, marker, and stage
roles from `tokens/viewer.css` for time-based application panels.

Keep prose precise and calm. State behavior, limitations, and status. Use sentence
case. Diagrams should explain engineering models. Comparative performance claims
require reproducible measurements and are absent from the current docs.

## Validation

From `docs/`, run `npm run check`, `npm run build`, and `npm test` after changes.
The static link check validates local links, anchors, and assets. Browser checks
cover both themes at desktop and 360px widths, SVG diagrams, search, keyboard
focus, reduced motion, widget precision, and accessibility. Inspect screenshots
under `docs/test-results/` before accepting visual changes.

Shared styles use tokens for colors, fonts, radii, and elevation. Avoid page-local
palette literals and external font/icon CDNs. Widget code must preserve exact
trace values and native keyboard interaction. Narrow screens must scroll wide
content inside its frame, never at page level.

The public container schematic uses its own fixed, accessible light palette;
SVG images embedded through `img` cannot inherit page CSS variables.
