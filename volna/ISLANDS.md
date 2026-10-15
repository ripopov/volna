# Volna Islands

**Volna Islands Dark** is the default theme. Select it in Settings → Appearance,
or set `"appearance.theme": "volna-islands-dark"`. Choose `volna` to follow the
system appearance. Select **Volna Islands Light** or set
`"appearance.theme": "volna-islands-light"` for the light variant. Neither variant
has a separate density setting.

## Appearance and ownership

The core theme registry resolves both variants through the same settings and
persistence path as every other built-in theme. `Theme::islands_dark` and
`Theme::islands_light` start from Volna Dark and Volna Light respectively, changing
only workspace surfaces and the optional typed `Islands` style. Dark islands use
Instrument's viewer canvas (`#1f2228`) on neutral chrome (`#2f343e`); light islands
use the white viewer canvas on pale blue-gray chrome (`#e6e9ef`). Waveform states,
markers, pipeline stages, fonts, and data row sizes retain their base theme values.

One window-wide backdrop blends shared `chrome-ocean-blue` and
`chrome-ocean-teal` viewer tokens into the shared `chrome-background` token. The
light palette uses soft sky blue and sea glass with a neutral fade. The glow is
anchored near the upper-left corner, peaks at 200 logical px horizontally, and
fades to neutral by 900 px horizontally and 280 px vertically. These distances
follow interface zoom. Transparent title/status bars and dock gutters reveal
the same backdrop behind opaque islands. Window controls and drag handlers keep
their existing behavior. Text and icon contrast is checked against each color
stop; other themes retain their solid chrome.

`dock.rs` delegates the classic appearance to GPUI-KIT's `DockSkin`. For Islands,
the same dock entities use the frame, tab-group, divider, and drop-indicator
renderer hooks. `islands.rs` supplies the custom tab strip using base `Tab` and
`Tabs` for accessibility and themed buttons and menus for actions. Panel titles,
toolbars, menus, selection, closing, zoom, and drag/drop call the existing panel
handles and `TabGroupContext` actions. Panels do not select skins or draw island
frames. The core's common canvas frame omits its full-panel focus outline;
selected tabs carry the active group's focus treatment instead.

Theme switching does not reconstruct the dock layout or panel entities. Tabs sit
edge-to-edge with rounded upper corners. The selected tab joins the panel surface;
unselected tabs use a 12% black tint in dark mode or the viewer hover surface in
light mode, with a subtle bottom border. Tab strokes stay on internal edges;
the island's outer top and left edges use the same borderless contour as an
island without tabs.
The focused group's selected title uses the primary text color; inactive titles
are muted. Keyboard focus has its own inset accent outline that does not change
tab geometry. Losing window focus mutes
the selected title without dimming data colors.

## Geometry

[JetBrains's official Islands specification](https://plugins.jetbrains.com/docs/intellij/supporting-islands-theme.html)
defines normal-mode `Island.arc = 20` as a rendered **10 logical px radius**, and
`Island.borderWidth = 5` as a **4 logical px visible gap**. It also recommends
dark islands darker than surrounding chrome, at least **1.20:1** background
contrast, and no noticeable island border. The dark Instrument pair has
approximately **1.28:1** contrast. Light islands reverse the surface hierarchy:
white content sits above slightly darker chrome with approximately **1.22:1**
contrast. Both variants share the same geometry.

Other dimensions below are Volna choices or retained toolkit/application values,
not undocumented JetBrains measurements. Values are at 100% interface zoom;
display scaling converts logical pixels to physical pixels separately.

| Dimension | Logical px | Basis |
| --- | ---: | --- |
| Island radius | 10 | Official normal-mode geometry |
| Visible gap | 4 | Official normal-mode geometry |
| Outer workspace inset | 4 | Volna choice: two 2 px insets |
| Tab-group slot inset | 2 per edge | Half of the visible gap |
| Main toolbar / status bar | 32 / 32 | Existing toolbar; Volna choice for status bar |
| Status bar vertical / horizontal padding | 4 / 12 | Volna choice; content vertically centered |
| Status text line height | 16 | Volna choice |
| Tab control height / upper radius | 32 / 10 | Volna choice |
| Tab strip vertical / horizontal padding | 0 / 0 | Volna choice |
| Tab spacing / label inset | 0 / 12 | Volna choice |
| Dense toolbar breakpoint | 640 | Volna choice, per group |
| Additional toolbar row spacing | 3 | Volna choice |
| Data header / default data row | 32 / 24 | Existing Volna values |
| Pipeline row | 18 by default | Existing independently zoomable data geometry |
| Workspace sidebar resize band | 8 | Existing Volna hit area |
| Dock split / dock-edge resize bands | 9 / 5 | Installed toolkit hit areas |
| All workspace splitter highlights, hover / drag | 1 / 4 | Volna choice; existing hit bands retained |

The tab group's inset is inside its existing layout slot. Split containers remain
transparent and receive no flex gap. Two adjacent 2 px insets therefore expose
4 px while split extents, resize bounds, and persisted sizes retain the toolkit's
coordinate system. At fractional display scales the full gap is rounded once
in physical pixels, then split into complementary leading/trailing insets. At
125%, for example, these are 2 and 3 physical pixels rather than two rounded-down
halves. The outer padding uses the opposite pair to keep its total inset equal
to one gap. The workspace sidebar's otherwise separate 1 px divider
collapses to zero layout width for Islands, retaining its 8 px hit area.
Dock edges and internal splits share the same highlight renderer. Edge handles
retain the toolkit's hit band and call `DockContext` for sizing and release;
release also emits the layout-change event used for persistence. Their active
highlight paints above the dock's rectangular clip so both halves remain visible
at the boundary. Drop rectangles come directly from the content frame's resolved
coordinates.

Dense panel toolbars occupy a second row below 640 px, leaving the tab labels
usable. The toolbar itself can scroll in very small groups. Tab strips scroll
horizontally and provide an all-tabs menu; arrow keys, Home, End, Enter, and Space
activate tabs while keyboard focus is in the strip.

## Clipping and validation

The pinned GPUI-KIT **0.7.3** includes `RoundedFrameCover`, whose backdrop is a
solid color. GPUI's rectangular descendant clip does not follow the parent's
rounded corners. Islands use the same quarter-arc notch geometry with
window-coordinate gradient fills, so stationary covers match the backdrop
behind scrolled trees, tables, and canvas content. No upgrade is needed.
The dock's separate header and content hooks cover the top and bottom corners
respectively, keeping their internal junction square. Covers stay in ordinary
paint order; menus, tooltips, dialogs, and drag previews retain their toolkit
overlay layer.

Relevant automated checks include the theme contrast/data-color invariant and
`islands_docking_and_live_switching_preserve_panel_identity`. The latter switches
through all registered themes and checks dock nodes, panel identities, layout,
and selected transaction state, then exercises resize, movement, undo, collapse,
reopening, and a narrow window. Native captures should additionally inspect
corners and gutters at the actual display scale, including scrolled content and
drag indicators. UI zoom and display scaling are distinct checks.
