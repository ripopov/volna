---
name: volna-design
description: Apply Volna's shared Instrument design system to engineering documentation and application UI. Use when adding or restyling Volna-branded pages, components, diagrams, or interface mockups.
---

Read [readme.md](readme.md), then the relevant files in `tokens/` and
`components/components.css`. Paths in this skill are relative to this directory.

Use `styles.css` as the shared web entry point. Set `data-theme="dark|light"`.
Compose reusable `v-` classes; extend them here rather than introducing a page
palette. Starlight integration belongs in `../src/styles/starlight.css` and must
map existing semantic tokens onto framework roles. Native frontends map
`tokens/theme.css` and `tokens/viewer.css` into toolkit themes.

Use bundled Inter and JetBrains Mono fonts and canonical SVG brand assets.
Keep dark and light themes, keyboard focus, native labels, 360px layouts, and
reduced motion working. Use SVG or Mermaid for engineering models and small
HTML/JS widgets only when interaction explains a concept. Include accessible
text descriptions and a meaningful JavaScript-disabled document.

Write precise engineering prose about this checkout. Label proposed behavior.
VTR contains runtime facts; design semantics and presentation belong to separate
VDB/application layers. Avoid promotional claims, unverified comparisons,
screenshot galleries, benchmark panels, and prototype site content.

Add a specimen to the design foundations guide or an adopting page for every new
component. Run the validation commands in readme.md and review browser screenshots.
