//! Core-painted panels with native GPUI interaction overlays.

use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, AvailableSpace, Bounds, ContentMask, CursorStyle, DispatchPhase, Element,
    ElementId, Entity, Font, FontStyle, FontWeight, GlobalElementId, Hitbox, HitboxBehavior, Hsla,
    InspectorElementId, IntoElement, LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, PathBuilder, PinchEvent, Pixels, ScrollDelta, ScrollWheelEvent, ShapedLine,
    SharedString, Style, TextAlign, TextRun, Window, div, fill, point, px, quad, size,
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
    regions: Vec<(CRect, Hitbox)>,
    row_h: f32,
    accessible: Vec<(
        volna_core::table::AccessibleRow,
        Option<gpui_kit::accesskit::NodeId>,
    )>,
    table_status: Option<String>,
    table_splitters: Option<AnyElement>,
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
        f64::from(row.bounds.left()) * scale,
        f64::from(row.bounds.top()) * scale,
        f64::from(row.bounds.right()) * scale,
        f64::from(row.bounds.bottom()) * scale,
    ));
    node
}

struct GpuiMeasure<'a> {
    window: &'a Window,
    theme: &'a Theme,
}

impl TextMeasure for GpuiMeasure<'_> {
    fn text_width(&mut self, text: &str, font: FontRole, size: f32) -> f32 {
        f32::from(shape(self.window, self.theme, text, font, size, Hsla::default()).width())
    }
}

fn fit_table_column(
    owner: &Entity<Workspace>,
    panel: volna_core::panels::PanelId,
    source_index: usize,
    window: &Window,
    cx: &App,
) -> Option<Command> {
    let t = *theme(cx);
    let core = *core_theme(cx);
    let width = owner
        .read(cx)
        .app
        .panels
        .get(panel)?
        .kind
        .table()?
        .column_fit_width(source_index, &core, &mut GpuiMeasure { window, theme: &t })?;
    Some(Command::Table(
        panel,
        volna_core::table::TableCommand::ResizeColumn {
            source_index,
            width,
        },
    ))
}

fn resize_table_column(
    owner: &Entity<Workspace>,
    panel: volna_core::panels::PanelId,
    generation: u64,
    source_index: usize,
    direction: f32,
    window: &mut Window,
    cx: &mut App,
) {
    cx.stop_propagation();
    let table = owner
        .read(cx)
        .app
        .panels
        .get(panel)
        .and_then(|panel| panel.kind.table());
    let Some(table) = table else { return };
    let Some(column) = table
        .layout
        .columns
        .iter()
        .find(|column| column.source_index == source_index)
    else {
        return;
    };
    let width = table.widths[source_index].unwrap_or(column.rect.width() / theme(cx).zoom);
    let step = if window.modifiers().shift { 32.0 } else { 8.0 };
    let command = Command::Table(
        panel,
        volna_core::table::TableCommand::ResizeColumn {
            source_index,
            width: width + direction * step,
        },
    );
    owner.update(cx, |ws, cx| {
        ws.dispatch_if_current(generation, command, Some(window), cx);
    });
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
                    paint_prims(prims, i, t, shaped, images, window, cx);
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
                f64::from(row.bounds.left()) * prepaint.scale,
                f64::from(row.bounds.top()) * prepaint.scale,
                f64::from(row.bounds.right()) * prepaint.scale,
                f64::from(row.bounds.bottom()) * prepaint.scale,
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
        (): &mut (),
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
                Some(PanelLayout::Table(_)) | None => Vec::new(),
            }
        });
        let regions = rects
            .into_iter()
            .map(|r| (r, window.insert_hitbox(gbounds(r), HitboxBehavior::Normal)))
            .collect();
        let mut table_splitters = self.table_splitters(cx);
        if let Some((edges, header)) = &mut table_splitters {
            edges.prepaint_as_root(
                gbounds(*header).origin,
                size(
                    AvailableSpace::Definite(px(header.width())),
                    AvailableSpace::Definite(px(header.height())),
                ),
                window,
                cx,
            );
        }
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
            table_splitters: table_splitters.map(|(edges, _)| edges),
            wave_rows: if self.waves && window.is_a11y_active() {
                let app = &self.ws.read(cx).app;
                app.panels
                    .waves(self.panel)
                    .map(|w| w.accessible_rows(&app.doc).collect())
                    .unwrap_or_default()
            } else {
                Vec::new()
            },
            scale: f64::from(window.scale_factor()),
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        (): &mut (),
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
                            );
                        });
                        window.focus(&focus, cx);
                    });
                }
            }
        }
        let t = *theme(cx);
        let core = self.core_theme(cx);
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
        if let Some(edges) = &mut prepaint.table_splitters {
            edges.paint(window, cx);
            if self
                .ws
                .read(cx)
                .app
                .panels
                .get(self.panel)
                .and_then(|panel| panel.kind.table())
                .is_some_and(|table| table.resizing_column().is_some())
            {
                window.set_window_cursor_style(CursorStyle::ResizeColumn);
            }
        }
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
    fn table_splitters(&self, cx: &App) -> Option<(AnyElement, CRect)> {
        let table = self.ws.read(cx).app.panels.get(self.panel)?.kind.table()?;
        let header = table.layout.header;
        let t = *theme(cx);
        let mut edges = div()
            .id(("table-column-edges", self.panel.0))
            .tab_group()
            .tab_index(0)
            .tab_stop(false)
            .relative()
            .w(px(header.width()))
            .h(px(header.height()))
            .overflow_hidden();
        for column in &table.layout.columns {
            if column.rect.right() <= header.left() || column.rect.left() >= header.right() {
                continue;
            }
            let source_index = column.source_index;
            let sort_owner = self.ws.clone();
            let generation = self.generation;
            let panel = self.panel;
            let current = table
                .applied_sort()
                .filter(|sort| sort.column == source_index);
            let direction = current.map_or("source order", |sort| {
                if sort.descending {
                    "descending"
                } else {
                    "ascending"
                }
            });
            let enabled = table.can_sort_column(source_index);
            let key_owner = sort_owner.clone();
            edges = edges.child(
                div()
                    .id(("table-sort", source_index))
                    .occlude()
                    .when(enabled, gpui_kit::Styled::cursor_pointer)
                    .role(gpui_kit::accesskit::Role::Button)
                    .tab_index((source_index * 2) as isize)
                    .tab_stop(enabled)
                    .focus(move |style| style.border_1().border_color(t.border_focused))
                    .absolute()
                    .top_0()
                    .left(px(
                        column.rect.left() - header.left() + column.resize.width() * 0.5
                    ))
                    .w(px((column.rect.width() - column.resize.width()).max(0.0)))
                    .h_full()
                    .aria_label(format!(
                        "Sort {}: {direction}",
                        table.column_title(source_index)
                    ))
                    .aria_description(if enabled {
                        "Enter or Space cycles ascending, descending, then source order"
                    } else {
                        "Sorting unavailable: insufficient memory or source not ready"
                    })
                    .key_context("TableColumnSort")
                    .on_action(move |_: &crate::table_panel::SortColumn, window, cx| {
                        cx.stop_propagation();
                        key_owner.update(cx, |ws, cx| {
                            ws.dispatch_if_current(
                                generation,
                                Command::Table(
                                    panel,
                                    volna_core::table::TableCommand::SortColumn(source_index),
                                ),
                                Some(window),
                                cx,
                            );
                        });
                    })
                    .on_click(move |_, window, cx| {
                        sort_owner.update(cx, |ws, cx| {
                            ws.dispatch_if_current(
                                generation,
                                Command::Table(
                                    panel,
                                    volna_core::table::TableCommand::SortColumn(source_index),
                                ),
                                Some(window),
                                cx,
                            );
                        });
                    }),
            );
            if column.resize.right() <= header.left() || column.resize.left() >= header.right() {
                continue;
            }
            let group: SharedString =
                format!("table-{}-column-edge-{source_index}", self.panel.0).into();
            let active = table.resizing_column() == Some(source_index);
            let highlight = div()
                .w(px(1.0))
                .h_full()
                .bg(if active { t.border_focused } else { t.border })
                .group_hover(group.clone(), move |style| style.bg(t.border_focused));
            let owner = self.ws.clone();
            let narrow_owner = owner.clone();
            let widen_owner = owner.clone();
            let fit_owner = owner.clone();
            let generation = self.generation;
            let panel = self.panel;
            let focus = self.table_focus.clone();
            let width = column.rect.width() / t.zoom;
            edges = edges.child(
                div()
                    .id(("table-column-resize", source_index))
                    .group(group)
                    .absolute()
                    .top_0()
                    .left(px(column.resize.left() - header.left()))
                    .w(px(column.resize.width()))
                    .h_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .occlude()
                    .cursor_col_resize()
                    .role(gpui_kit::accesskit::Role::Splitter)
                    .aria_label(format!("Resize {}", table.column_title(source_index)))
                    .aria_description(
                        "Left and Right resize the column. Enter fits its header and visible rows.",
                    )
                    .aria_orientation(gpui_kit::accesskit::Orientation::Vertical)
                    .aria_numeric_value(f64::from(width))
                    .aria_min_numeric_value(f64::from(volna_core::table::layout::MIN_COLUMN_WIDTH))
                    .aria_max_numeric_value(f64::from(volna_core::table::layout::MAX_COLUMN_WIDTH))
                    .aria_numeric_value_step(8.0)
                    .tab_index((source_index * 2 + 1) as isize)
                    .focus(move |style| style.bg(t.hover.bg))
                    .child(highlight)
                    .key_context("TableColumnResize")
                    .on_action(move |_: &crate::table_panel::NarrowColumn, window, cx| {
                        resize_table_column(
                            &narrow_owner,
                            panel,
                            generation,
                            source_index,
                            -1.0,
                            window,
                            cx,
                        );
                    })
                    .on_action(move |_: &crate::table_panel::WidenColumn, window, cx| {
                        resize_table_column(
                            &widen_owner,
                            panel,
                            generation,
                            source_index,
                            1.0,
                            window,
                            cx,
                        );
                    })
                    .on_action(move |_: &crate::table_panel::FitColumn, window, cx| {
                        cx.stop_propagation();
                        if let Some(command) =
                            fit_table_column(&fit_owner, panel, source_index, window, cx)
                        {
                            fit_owner.update(cx, |ws, cx| {
                                ws.dispatch_if_current(generation, command, Some(window), cx);
                            });
                        }
                    })
                    .on_mouse_down(MouseButton::Left, move |event, window, cx| {
                        cx.stop_propagation();
                        if owner.read(cx).app.doc.generation() != generation {
                            return;
                        }
                        if let Some(focus) = &focus {
                            window.focus(focus, cx);
                        }
                        let command = if event.click_count >= 2 {
                            let Some(command) =
                                fit_table_column(&owner, panel, source_index, window, cx)
                            else {
                                return;
                            };
                            command
                        } else {
                            Command::Pointer(
                                panel,
                                PointerEvent::Down {
                                    position: cpoint(event.position),
                                    button: volna_core::geometry::MouseButton::Left,
                                    modifiers: to_modifiers(event.modifiers),
                                },
                            )
                        };
                        owner.update(cx, |ws, cx| {
                            ws.dispatch_if_current(generation, command, Some(window), cx);
                        });
                    }),
            );
        }
        Some((edges.into_any_element(), header))
    }

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
                ws.dispatch_if_current(generation, Command::Pointer(panel, ev), Some(window), cx);
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
                    MouseButton::Navigate(_) => return,
                };
                if ws.read(cx).app.doc.generation() != generation {
                    return;
                }
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
                let group_name = ws
                    .read(cx)
                    .app
                    .panels
                    .waves(panel)
                    .and_then(|w| w.group_name_at(cpoint(ev.position)))
                    .is_some();
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
                        );
                    });
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
                        );
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
                        );
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
                        );
                    });
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
                        );
                    });
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
                        );
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
                        );
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
                    .is_some_and(volna_core::panels::Panel::dragging)
                {
                    send(&ws, PointerEvent::Up, window, cx);
                }
            }
        });

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
            volna_core::Color::rgb(0x0012_3456),
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
                let phase = f64::from(frame) * 0.125 * end / 2000.0;
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
