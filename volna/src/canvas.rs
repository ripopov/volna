//! `PanelCanvas`: the GPUI element that hosts a core-painted panel (waves or
//! pipeline). It asks the core for the frame's layout (for hitboxes), paints
//! the display list the core produces, and forwards pointer input as commands.

use gpui_kit::{
    App, Bounds, ContentMask, CursorStyle, DispatchPhase, Element, ElementId, Entity, Font,
    FontStyle, FontWeight, GlobalElementId, Hitbox, HitboxBehavior, Hsla, InspectorElementId,
    IntoElement, LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PathBuilder,
    PinchEvent, Pixels, ScrollDelta, ScrollWheelEvent, ShapedLine, SharedString, Style, TextAlign,
    TextRun, Window, fill, point, px, quad, size,
};
use volna_core::FontRole;
use volna_core::app::{Command, PanelLayout};
use volna_core::geometry::{CursorIcon, Point as CPoint, Rect as CRect};
use volna_core::scene::{Prim, TextMeasure};
use volna_core::wave::PointerEvent;

use crate::app::{ShapedCache, TextStyleKey, Workspace, to_modifiers};
use crate::theme::{Theme, canvas_theme, core_theme, hsla, theme};
use std::{
    collections::{HashMap, VecDeque},
    sync::Arc,
};

/// Keep two submissions per panel alive. GPUI owns the atlas, and obsolete
/// images are explicitly removed before the next submission to bound it.
#[derive(Default)]
pub(crate) struct PresenceImages {
    panels: HashMap<volna_core::panels::PanelId, VecDeque<Vec<Arc<gpui_kit::RenderImage>>>>,
}

impl PresenceImages {
    fn begin(
        &mut self,
        panel: volna_core::panels::PanelId,
        panels: &volna_core::panels::Panels,
        window: &mut Window,
    ) {
        self.panels.retain(|id, frames| {
            if panels.get(*id).is_some() {
                return true;
            }
            for image in frames.drain(..).flatten() {
                window.drop_image(image).ok();
            }
            false
        });
        let frames = self.panels.entry(panel).or_default();
        if frames.len() == 2 {
            for image in frames.pop_front().unwrap() {
                window.drop_image(image).ok();
            }
        }
    }

    fn finish(
        &mut self,
        panel: volna_core::panels::PanelId,
        images: Vec<Arc<gpui_kit::RenderImage>>,
    ) {
        self.panels.entry(panel).or_default().push_back(images);
    }
}

fn presence_image(
    width: usize,
    height: usize,
    runs: impl Iterator<Item = (usize, std::ops::Range<usize>)>,
    color: volna_core::Color,
) -> gpui_kit::RenderImage {
    let [r, g, b, a] = color.to_rgba8();
    let mut image = image::RgbaImage::new(width as u32, height as u32);
    let bytes: &mut [u8] = image.as_mut();
    // RenderImage's upload format is BGRA, despite the image crate's container name.
    for (y, x) in runs {
        let start = (y * width + x.start) * 4;
        let end = (y * width + x.end) * 4;
        for pixel in bytes[start..end].chunks_exact_mut(4) {
            pixel.copy_from_slice(&[b, g, r, a]);
        }
    }
    gpui_kit::RenderImage::new(vec![image::Frame::new(image)])
}

pub struct PanelCanvas {
    ws: Entity<Workspace>,
    panel: volna_core::panels::PanelId,
    generation: u64,
    table_focus: Option<gpui_kit::FocusHandle>,
    /// A waveform panel: its rows are exposed as a tree.
    waves: bool,
}

impl PanelCanvas {
    pub fn new(ws: Entity<Workspace>, panel: volna_core::panels::PanelId, generation: u64) -> Self {
        PanelCanvas {
            ws,
            panel,
            generation,
            table_focus: None,
            waves: false,
        }
    }

    /// The theme this panel paints with: the canvas theme for the panels
    /// drawn against time (waves, pipeline), the chrome for the table.
    pub(crate) fn core_theme(&self, cx: &App) -> crate::theme::CoreTheme {
        let data = self
            .ws
            .read(cx)
            .app
            .panels
            .get(self.panel)
            .is_some_and(|panel| {
                matches!(
                    panel.kind,
                    volna_core::panels::PanelKind::Waves(_)
                        | volna_core::panels::PanelKind::Pipeline(_)
                )
            });
        if data {
            *canvas_theme(cx)
        } else {
            *core_theme(cx)
        }
    }

    pub fn waves(mut self, waves: bool) -> Self {
        self.waves = waves;
        self
    }

    pub fn table(mut self, focus: gpui_kit::FocusHandle) -> Self {
        self.table_focus = Some(focus);
        self
    }
}

pub struct CanvasPrepaint {
    hitbox: Hitbox,
    /// Small interactive rectangles, so GPUI resolves pointer shapes per position.
    regions: Vec<(CRect, Hitbox)>,
    row_h: f32,
    accessible: Vec<(
        volna_core::table::AccessibleRow,
        Option<gpui_kit::accesskit::NodeId>,
    )>,
    table_status: Option<String>,
    /// Wave rows on screen, while assistive technology is active.
    wave_rows: Vec<volna_core::wave::model::AccessibleRow>,
    scale: f64,
}

impl IntoElement for PanelCanvas {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

fn gbounds(r: CRect) -> Bounds<Pixels> {
    Bounds::new(
        point(px(r.origin.x), px(r.origin.y)),
        size(px(r.size.width), px(r.size.height)),
    )
}

fn cbounds(b: Bounds<Pixels>) -> CRect {
    CRect::from_xywh(
        f32::from(b.origin.x),
        f32::from(b.origin.y),
        f32::from(b.size.width),
        f32::from(b.size.height),
    )
}

fn cpoint(p: gpui_kit::Point<Pixels>) -> CPoint {
    CPoint {
        x: f32::from(p.x),
        y: f32::from(p.y),
    }
}

fn font(t: &Theme, role: FontRole) -> Font {
    let weight = match role {
        FontRole::Ui | FontRole::Mono => FontWeight::NORMAL,
        FontRole::UiMedium => FontWeight::MEDIUM,
        FontRole::UiSemibold => FontWeight::SEMIBOLD,
    };
    Font {
        family: t.font_family(role).into(),
        features: Default::default(),
        fallbacks: None,
        weight,
        style: FontStyle::Normal,
    }
}

fn shape(
    window: &Window,
    t: &Theme,
    text: &str,
    role: FontRole,
    size: f32,
    color: Hsla,
) -> ShapedLine {
    let text: SharedString = text.to_owned().into();
    let run = TextRun {
        len: text.len(),
        font: font(t, role),
        color,
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    window
        .text_system()
        .shape_line(text, px(size), &[run], None)
}

/// A wave row as a tree item: its name, level, selection and, for a group,
/// whether it is expanded.
pub(crate) fn wave_row_node(
    row: &volna_core::wave::model::AccessibleRow,
    scale: f64,
) -> gpui_kit::accesskit::Node {
    use gpui_kit::accesskit::{Node, Rect, Role};
    let mut node = Node::new(Role::TreeItem);
    node.set_label(row.label.clone());
    node.set_level(row.level);
    node.set_selected(row.selected);
    if let Some(expanded) = row.expanded {
        node.set_expanded(expanded);
    }
    node.set_bounds(Rect::new(
        row.bounds.left() as f64 * scale,
        row.bounds.top() as f64 * scale,
        row.bounds.right() as f64 * scale,
        row.bounds.bottom() as f64 * scale,
    ));
    node
}

/// Width measurement through GPUI's text system.
struct GpuiMeasure<'a> {
    window: &'a Window,
    theme: &'a Theme,
}

impl TextMeasure for GpuiMeasure<'_> {
    fn text_width(&mut self, text: &str, font: FontRole, size: f32) -> f32 {
        f32::from(shape(self.window, self.theme, text, font, size, Hsla::default()).width())
    }
}

const SHAPED_CACHE_LIMIT: usize = 4096;

fn cursor_style(icon: CursorIcon) -> CursorStyle {
    match icon {
        CursorIcon::Default => CursorStyle::Arrow,
        CursorIcon::PointingHand => CursorStyle::PointingHand,
        CursorIcon::ResizeLeftRight => CursorStyle::ResizeLeftRight,
        CursorIcon::ResizeUpDown => CursorStyle::ResizeUpDown,
        CursorIcon::Grabbing => CursorStyle::ClosedHand,
    }
}

/// Paint `prims[*i..]` until the matching `PopClip`, nesting clips as content masks.
fn paint_prims(
    prims: &[Prim],
    i: &mut usize,
    t: &Theme,
    shaped: &mut ShapedCache,
    images: &mut Vec<Arc<gpui_kit::RenderImage>>,
    window: &mut Window,
    cx: &mut App,
) {
    while *i < prims.len() {
        let prim = &prims[*i];
        *i += 1;
        match prim {
            Prim::Presence { rect, mask, color } => {
                let image = Arc::new(presence_image(
                    mask.width(),
                    mask.height(),
                    mask.runs(),
                    *color,
                ));
                let bounds = gbounds(*rect);
                window
                    .paint_image(bounds, bounds, Default::default(), image.clone(), 0, false)
                    .ok();
                images.push(image);
            }
            Prim::Quad {
                rect,
                fill: color,
                radius,
                border_width,
                border_color,
            } => {
                let b = gbounds(*rect);
                if *radius == 0.0 && *border_width == 0.0 {
                    window.paint_quad(fill(b, hsla(*color)));
                } else {
                    window.paint_quad(quad(
                        b,
                        px(*radius),
                        hsla(*color),
                        px(*border_width),
                        hsla(*border_color),
                        gpui_kit::BorderStyle::default(),
                    ));
                }
            }
            Prim::Lines {
                segments,
                color,
                width,
            } => {
                let mut builder = PathBuilder::stroke(px(*width));
                for [a, b] in segments {
                    builder.move_to(point(px(a.x), px(a.y)));
                    builder.line_to(point(px(b.x), px(b.y)));
                }
                if let Ok(path) = builder.build() {
                    window.paint_path(path, hsla(*color));
                }
            }
            Prim::Text {
                origin,
                height,
                text,
                font: role,
                size,
                color,
            } => {
                let color = hsla(*color);
                let style = TextStyleKey {
                    font: *role,
                    size: size.to_bits(),
                    color: [color.h, color.s, color.l, color.a].map(f32::to_bits),
                };
                let line = shaped.line(style, text, SHAPED_CACHE_LIMIT, || {
                    shape(window, t, text, *role, *size, color)
                });
                line.paint(
                    point(px(origin.x), px(origin.y)),
                    px(*height),
                    TextAlign::Left,
                    None,
                    window,
                    cx,
                )
                .ok();
            }
            Prim::Icon { name, rect, color } => {
                window
                    .paint_svg(
                        gbounds(*rect),
                        name.path().into(),
                        None,
                        gpui_kit::TransformationMatrix::unit(),
                        hsla(*color),
                        cx,
                    )
                    .ok();
            }
            Prim::PushClip(rect) => {
                let mask = ContentMask {
                    bounds: gbounds(*rect),
                };
                window.with_content_mask(Some(mask), |window| {
                    paint_prims(prims, i, t, shaped, images, window, cx)
                });
            }
            Prim::PopClip => return,
        }
    }
}

impl Element for PanelCanvas {
    type RequestLayoutState = ();
    type PrepaintState = CanvasPrepaint;

    fn id(&self) -> Option<ElementId> {
        if self.table_focus.is_some() {
            Some(ElementId::Name("table-rows".into()))
        } else {
            self.waves.then(|| ElementId::Name("wave-rows".into()))
        }
    }

    fn a11y_role(&self) -> Option<gpui_kit::Role> {
        if self.table_focus.is_some() {
            Some(gpui_kit::Role::ListBox)
        } else {
            self.waves.then_some(gpui_kit::Role::Tree)
        }
    }

    fn a11y_synthetic_children(
        &mut self,
        prepaint: &mut CanvasPrepaint,
        builder: &mut gpui_kit::A11ySubtreeBuilder,
    ) {
        use gpui_kit::accesskit::{Action, Node, Rect, Role};
        if self.waves {
            builder.parent_node().set_label("Waveform rows");
            builder
                .parent_node()
                .set_description("Groups fold with Left and Right. Only visible rows are exposed.");
            for row in &prepaint.wave_rows {
                let node_id = builder.synthetic_node_id((self.generation, self.panel.0, row.entry));
                builder.push_child(node_id, wave_row_node(row, prepaint.scale));
            }
            return;
        }
        builder
            .parent_node()
            .set_label(prepaint.table_status.as_deref().unwrap_or("Table rows"));
        builder.parent_node().set_description(
            "Arrow keys navigate rows. Enter opens record details. Only visible rows are exposed.",
        );
        for (row, id) in &mut prepaint.accessible {
            let node_id = builder.synthetic_node_id((self.generation, self.panel.0, row.ordinal));
            let mut node = Node::new(Role::ListBoxOption);
            node.set_label(row.label.clone());
            node.set_selected(row.selected);
            node.set_row_index_text(row.ordinal.saturating_add(1).to_string());
            node.set_bounds(Rect::new(
                row.bounds.left() as f64 * prepaint.scale,
                row.bounds.top() as f64 * prepaint.scale,
                row.bounds.right() as f64 * prepaint.scale,
                row.bounds.bottom() as f64 * prepaint.scale,
            ));
            node.add_action(Action::Click);
            node.add_action(Action::Focus);
            if row.selected {
                builder.parent_node().set_active_descendant(node_id);
            }
            builder.push_child(node_id, node);
            *id = Some(node_id);
        }
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size.width = gpui_kit::relative(1.0).into();
        style.size.height = gpui_kit::relative(1.0).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> CanvasPrepaint {
        let t = self.core_theme(cx);
        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
        let rects = self.ws.update(cx, |ws, _| {
            match ws.app.layout_panel(
                self.panel,
                cbounds(bounds),
                &t,
                f64::from(window.scale_factor()),
            ) {
                Some(PanelLayout::Waves(layout)) => {
                    let mut rects = vec![layout.names_split, layout.values_split];
                    rects.extend(layout.badges.iter().map(|(_, b)| *b));
                    rects.extend(layout.marker_lane.chips.iter().map(|c| c.rect));
                    rects
                }
                Some(PanelLayout::Pipeline(layout)) => {
                    let mut rects = vec![layout.label_split];
                    rects.extend(layout.marker_lane.chips.iter().map(|c| c.rect));
                    rects.extend(layout.retry);
                    rects.extend(layout.activity_controls.iter().map(|(_, rect, _)| *rect));
                    rects
                }
                Some(PanelLayout::Table(_)) => Vec::new(),
                None => Vec::new(),
            }
        });
        let regions = rects
            .into_iter()
            .map(|r| (r, window.insert_hitbox(gbounds(r), HitboxBehavior::Normal)))
            .collect();
        let table = window
            .is_a11y_active()
            .then(|| {
                self.ws
                    .read(cx)
                    .app
                    .panels
                    .get(self.panel)
                    .and_then(|panel| panel.kind.table())
            })
            .flatten();
        CanvasPrepaint {
            hitbox,
            regions,
            row_h: t.row_height,
            accessible: table
                .map(|table| table.accessible_rows().map(|row| (row, None)).collect())
                .unwrap_or_default(),
            table_status: table.map(|table| format!("Table rows. {}", table.status())),
            wave_rows: if self.waves && window.is_a11y_active() {
                let app = &self.ws.read(cx).app;
                app.panels
                    .waves(self.panel)
                    .map(|w| w.accessible_rows(&app.doc).collect())
                    .unwrap_or_default()
            } else {
                Vec::new()
            },
            scale: window.scale_factor() as f64,
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _: &mut (),
        prepaint: &mut CanvasPrepaint,
        window: &mut Window,
        cx: &mut App,
    ) {
        if let Some(focus) = &self.table_focus {
            for (row, node_id) in &prepaint.accessible {
                let Some(node_id) = node_id else { continue };
                for action in [
                    gpui_kit::AccessibleAction::Click,
                    gpui_kit::AccessibleAction::Focus,
                ] {
                    let owner = self.ws.clone();
                    let focus = focus.clone();
                    let generation = self.generation;
                    let panel = self.panel;
                    let ordinal = row.ordinal;
                    window.on_a11y_action(*node_id, action, move |_, window, cx| {
                        owner.update(cx, |ws, cx| {
                            ws.dispatch_if_current(
                                generation,
                                Command::Table(
                                    panel,
                                    volna_core::table::TableCommand::Select(ordinal),
                                ),
                                Some(window),
                                cx,
                            )
                        });
                        window.focus(&focus, cx);
                    });
                }
            }
        }
        let t = *theme(cx);
        let core = self.core_theme(cx);
        // Build the display list, then paint it. The scene buffer and the
        // shaped-text cache live on the workspace so they persist across frames.
        let (mut scene, mut shaped, mut presence_images) = self.ws.update(cx, |ws, _| {
            let mut scene = std::mem::take(&mut ws.scene);
            let mut measure = GpuiMeasure { window, theme: &t };
            ws.app
                .render_panel_into(self.panel, &core, &mut measure, &mut scene);
            let mut images = std::mem::take(&mut ws.presence_images);
            images.begin(self.panel, &ws.app.panels, window);
            (scene, std::mem::take(&mut ws.shaped), images)
        });
        let mut i = 0;
        let mut images = Vec::new();
        paint_prims(
            &scene.prims,
            &mut i,
            &t,
            &mut shaped,
            &mut images,
            window,
            cx,
        );
        presence_images.finish(self.panel, images);
        // Pointer shapes.
        if let Some(icon) = scene.window_cursor {
            window.set_window_cursor_style(cursor_style(icon));
        } else {
            for (rect, icon) in &scene.cursors {
                if let Some((_, hitbox)) = prepaint.regions.iter().find(|(r, _)| r == rect) {
                    window.set_cursor_style(cursor_style(*icon), hitbox);
                }
            }
        }
        scene.clear();
        self.register_mouse_handlers(prepaint, window);
        self.ws.update(cx, |ws, _| {
            ws.scene = scene;
            ws.shaped = shaped;
            ws.presence_images = presence_images;
            if let Some(panel) = ws.app.panels.get_mut(self.panel) {
                panel.record_paint();
            }
        });
    }
}

impl PanelCanvas {
    fn register_mouse_handlers(&self, prepaint: &CanvasPrepaint, window: &mut Window) {
        let ws = self.ws.clone();
        let hitbox = prepaint.hitbox.clone();
        let row_h = prepaint.row_h;
        let panel = self.panel;
        let generation = self.generation;
        let send = move |ws: &Entity<Workspace>,
                         ev: PointerEvent,
                         window: &mut Window,
                         cx: &mut App| {
            ws.update(cx, |ws, cx| {
                ws.dispatch_if_current(generation, Command::Pointer(panel, ev), Some(window), cx)
            });
        };

        window.on_mouse_event({
            let ws = ws.clone();
            let hitbox = hitbox.clone();
            move |ev: &MouseDownEvent, phase, window, cx| {
                if phase != DispatchPhase::Bubble || !hitbox.is_hovered(window) {
                    return;
                }
                let button = match ev.button {
                    MouseButton::Left => volna_core::geometry::MouseButton::Left,
                    MouseButton::Middle => volna_core::geometry::MouseButton::Middle,
                    MouseButton::Right => volna_core::geometry::MouseButton::Right,
                    _ => return,
                };
                if ws.read(cx).app.doc.generation() != generation {
                    return;
                }
                // A press anywhere else keeps the name typed so far.
                ws.update(cx, |ws, cx| ws.commit_rename(window, cx));
                let focus = ws
                    .read(cx)
                    .dock
                    .as_ref()
                    .and_then(|dock| dock.focus(panel, cx))
                    .unwrap_or_else(|| ws.read(cx).waves_focus.clone());
                window.focus(&focus, cx);
                send(
                    &ws,
                    PointerEvent::Down {
                        position: cpoint(ev.position),
                        button,
                        modifiers: to_modifiers(ev.modifiers),
                    },
                    window,
                    cx,
                );
                // Pipeline labels navigate to their record's start. Other
                // record-bearing areas open what the first click selected.
                let label_row = {
                    let app = &ws.read(cx).app;
                    app.panels
                        .pipeline(panel)
                        .and_then(|p| p.label_row_at(&app.doc, cpoint(ev.position)))
                };
                let selects = ws.read(cx).app.panels.get(panel).is_some_and(|p| {
                    p.kind.pipeline().is_some_and(|pipeline| {
                        let layout = pipeline.last_layout();
                        !matches!(
                            pipeline.drag,
                            Some(volna_core::pipeline::model::Drag::BoxZoom { .. })
                        ) && layout.cells.contains(cpoint(ev.position))
                            && !layout.is_overview()
                            && layout.row_at(cpoint(ev.position).y).is_some()
                    }) || p.kind.table().is_some()
                        || p.kind.waves().is_some_and(|w| w.pressed_record)
                });
                // A second click on a group's name renames it.
                let group_name = ws
                    .read(cx)
                    .app
                    .panels
                    .waves(panel)
                    .and_then(|w| w.group_name_at(cpoint(ev.position)))
                    .is_some();
                // A second click on a marker's chip names it; on a span it
                // zooms to the span, and on the live span to the measurement.
                let (chip_marker, span, live) = {
                    let app = &ws.read(cx).app;
                    let markers = app.doc.markers();
                    let id = |ix: usize| markers.get(ix).map(|m| m.id);
                    match app
                        .panels
                        .get(panel)
                        .and_then(|p| p.kind.marker_lane())
                        .and_then(|lane| lane.hit(cpoint(ev.position)))
                    {
                        Some(volna_core::marker::LaneHit::Chip(ix)) => (id(ix), None, false),
                        Some(volna_core::marker::LaneHit::Span(ix)) => {
                            (None, id(ix).zip(id(ix + 1)), false)
                        }
                        Some(volna_core::marker::LaneHit::Live) => (None, None, true),
                        _ => (None, None, false),
                    }
                };
                // On the time header or an empty part of the Markers lane,
                // where the first click put the cursor, a second one adds a
                // marker (or names the one already there).
                let adds_marker = ws.read(cx).app.adds_marker_at(panel, cpoint(ev.position));
                if ev.click_count >= 2
                    && button == volna_core::geometry::MouseButton::Left
                    && adds_marker
                {
                    ws.update(cx, |ws, cx| {
                        ws.dispatch_if_current(
                            generation,
                            Command::Action(volna_core::app::Action::AddOrRenameMarker),
                            Some(window),
                            cx,
                        )
                    });
                    // A name field may open, and it keeps the keys.
                    window.prevent_default();
                } else if ev.click_count >= 2
                    && button == volna_core::geometry::MouseButton::Left
                    && live
                {
                    ws.update(cx, |ws, cx| {
                        ws.dispatch_if_current(
                            generation,
                            Command::Action(volna_core::app::Action::ZoomToMeasurement),
                            Some(window),
                            cx,
                        )
                    });
                } else if ev.click_count >= 2
                    && button == volna_core::geometry::MouseButton::Left
                    && let Some((from, to)) = span
                {
                    ws.update(cx, |ws, cx| {
                        ws.dispatch_if_current(
                            generation,
                            Command::ZoomToSpan(panel, from, to),
                            Some(window),
                            cx,
                        )
                    });
                } else if ev.click_count >= 2
                    && button == volna_core::geometry::MouseButton::Left
                    && let Some(id) = chip_marker
                {
                    ws.update(cx, |ws, cx| {
                        ws.dispatch_if_current(
                            generation,
                            Command::RenameMarker(panel, id),
                            Some(window),
                            cx,
                        )
                    });
                    // The field now has the keys; the panel's own
                    // focus-on-press must not take them back.
                    window.prevent_default();
                } else if ev.click_count >= 2
                    && button == volna_core::geometry::MouseButton::Left
                    && group_name
                {
                    ws.update(cx, |ws, cx| {
                        ws.dispatch_if_current(
                            generation,
                            Command::Action(volna_core::app::Action::RenameGroup),
                            Some(window),
                            cx,
                        )
                    });
                    // The field now has the keys; the panel's own
                    // focus-on-press must not take them back.
                    window.prevent_default();
                } else if ev.click_count >= 2
                    && button == volna_core::geometry::MouseButton::Left
                    && let Some(row) = label_row
                {
                    ws.update(cx, |ws, cx| {
                        ws.dispatch_if_current(
                            generation,
                            Command::PanToTransactionStart { panel, row },
                            Some(window),
                            cx,
                        )
                    });
                } else if ev.click_count >= 2
                    && button == volna_core::geometry::MouseButton::Left
                    && selects
                {
                    ws.update(cx, |ws, cx| {
                        ws.dispatch_if_current(
                            generation,
                            Command::ShowTransaction { from: panel },
                            Some(window),
                            cx,
                        )
                    });
                }
            }
        });

        window.on_mouse_event({
            let ws = ws.clone();
            move |ev: &MouseMoveEvent, phase, window, cx| {
                if phase != DispatchPhase::Bubble {
                    return;
                }
                send(
                    &ws,
                    PointerEvent::Move {
                        position: cpoint(ev.position),
                    },
                    window,
                    cx,
                );
            }
        });

        window.on_mouse_event({
            let ws = ws.clone();
            move |_: &MouseUpEvent, phase, window, cx| {
                if phase != DispatchPhase::Bubble {
                    return;
                }
                if ws
                    .read(cx)
                    .app
                    .panels
                    .get(panel)
                    .is_some_and(|p| p.dragging())
                {
                    send(&ws, PointerEvent::Up, window, cx);
                }
            }
        });

        // Trackpad pinch: zoom about the gesture centre.
        window.on_mouse_event({
            let ws = ws.clone();
            let hitbox = hitbox.clone();
            move |ev: &PinchEvent, phase, window, cx| {
                if phase != DispatchPhase::Bubble || !hitbox.is_hovered(window) {
                    return;
                }
                send(
                    &ws,
                    PointerEvent::Pinch {
                        position: cpoint(ev.position),
                        delta: ev.delta,
                    },
                    window,
                    cx,
                );
            }
        });

        // The core resolves wheel zoom, time pan and row scroll from the hit
        // region and from whether the deltas are a trackpad's exact pixels.
        window.on_mouse_event({
            move |ev: &ScrollWheelEvent, phase, window, cx| {
                if phase != DispatchPhase::Bubble || !hitbox.is_hovered(window) {
                    return;
                }
                let delta = ev.delta.pixel_delta(px(row_h));
                send(
                    &ws,
                    PointerEvent::Wheel {
                        position: cpoint(ev.position),
                        dx: f32::from(delta.x),
                        dy: f32::from(delta.y),
                        modifiers: to_modifiers(ev.modifiers),
                        precise: matches!(ev.delta, ScrollDelta::Pixels(_)),
                    },
                    window,
                    cx,
                );
            }
        });
    }
}

#[cfg(test)]
mod presence_tests {
    use super::*;

    #[test]
    fn mask_upload_preserves_bgra_ink_and_transparent_gaps() {
        let image = presence_image(
            3,
            2,
            [(0, 1..3), (1, 0..1)].into_iter(),
            volna_core::Color::rgb(0x123456),
        );
        assert_eq!(image.size(0).width.0, 3);
        assert_eq!(image.size(0).height.0, 2);
        assert_eq!(
            image.as_bytes(0).unwrap(),
            &[
                0, 0, 0, 0, 0x56, 0x34, 0x12, 255, 0x56, 0x34, 0x12, 255, 0x56, 0x34, 0x12, 255, 0,
                0, 0, 0, 0, 0, 0, 0,
            ]
        );
    }
}

#[cfg(all(test, not(target_family = "wasm")))]
mod presence_atlas_tests {
    use gpui_kit::{TestAppContext, VisualTestContext};
    use volna_core::{
        app::Command,
        pipeline::{FollowActivity, RowView},
        wave::viewport::Viewport,
    };

    #[gpui_kit::test]
    fn fragmented_pipeline_uploads_and_releases_obsolete_atlas_images(cx: &mut TestAppContext) {
        let file = tempfile::Builder::new().suffix(".vtr").tempfile().unwrap();
        let mut writer = vtr::Writer::create(file.path()).unwrap();
        let stream = writer.add_stream(None, "pipeline", "PIPELINE").unwrap();
        let generator = writer.add_generator(stream, "instruction").unwrap();
        let name = writer.intern("stage");
        let lane = writer.intern("0");
        for row in 0..600 {
            let tx = writer.begin_tx(generator, row).unwrap();
            for column in 0..80 {
                writer
                    .tx_stage(tx, name, lane, column * 24, column * 24, &[])
                    .unwrap();
            }
            writer.end_tx(tx, row + 1, vtr::TxStatus::Ok).unwrap();
        }
        writer.set_time(2000).unwrap();
        writer.close().unwrap();
        cx.update(crate::init_app);
        let host = cx.add_window(crate::app::Workspace::new);
        host.update(cx, |ws, _, cx| ws.open_path(file.path().into(), cx))
            .unwrap();
        cx.run_until_parked();
        let panel = host
            .update(cx, |ws, window, cx| {
                ws.dispatch(
                    Command::OpenPipeline {
                        track: ws.app.pipeline_streams()[0].1,
                    },
                    Some(window),
                    cx,
                );
                ws.app.panels.focused_id()
            })
            .unwrap();
        cx.run_until_parked();
        let mut vcx = VisualTestContext::from_window(host.into(), cx);
        vcx.simulate_resize(gpui_kit::size(gpui_kit::px(1200.0), gpui_kit::px(800.0)));
        let mut first = None;
        for frame in 0..4 {
            host.update(&mut vcx, |ws, _, cx| {
                let end = ws.app.doc.limits().1 as f64;
                let phase = frame as f64 * 0.125 * end / 2000.0;
                ws.app.doc.shared.viewport.set(Viewport {
                    start: phase,
                    end: end + phase,
                });
                let pipeline = ws.app.panels.pipeline_mut(panel).unwrap();
                pipeline.follow = FollowActivity::Off;
                pipeline.rows.set(RowView {
                    top: 0.0,
                    row_px: 0.5,
                });
                cx.notify();
            })
            .unwrap();
            vcx.run_until_parked();
            vcx.update(|window, cx| {
                window.refresh();
                window.draw(cx).clear(cx);
            });
            host.update(&mut vcx, |ws, window, _| {
                let frames = ws
                    .presence_images
                    .panels
                    .get(&panel)
                    .expect("painted pipeline");
                assert!(frames.len() <= 2);
                let latest = frames.back().unwrap();
                assert_eq!(latest.len(), 1, "fragmented output uses one image");
                assert!(window.has_image_atlas_entry(&latest[0]));
                if frame == 0 {
                    first = Some(latest[0].clone());
                }
                if frame >= 2 {
                    assert!(!window.has_image_atlas_entry(first.as_ref().unwrap()));
                }
            })
            .unwrap();
        }
    }
}
