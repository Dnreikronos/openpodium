use std::collections::BTreeSet;

use iced::theme::palette;
use iced::widget::canvas::{self, Path, Stroke};
use iced::widget::image::Handle;
use iced::{Color, Font, Pixels, Point, Radians, Rectangle, Size, Theme, Vector, font};
use openpodium::domain::{
    AgentProgram, CanvasColor, CanvasNodeContent, ConnectionKind, Node, NodeGroup, NodeId,
    NodeTarget, ShapeKind,
};
use openpodium::portal::{PortalFrame, PortalFrameTransform, PortalRect};

use crate::app::shell;
use crate::terminal::{self, BODY_PADDING, CELL_HEIGHT, CELL_WIDTH, HEADER_HEIGHT};

use super::{Camera, CanvasDocument, NodeKind, ViewportSize, WorldPoint, WorldRect};

const BASE_GRID_STEP: f64 = 40.0;
const MIN_GRID_PIXELS: f64 = 24.0;
/// Below this zoom a node is too small to aim at, so portals stop taking
/// pointer input and the header drops its subtitle. Bodies still draw: a node
/// you cannot read is still a node you need to see.
pub(super) const BODY_MIN_ZOOM: f64 = 0.4;

/// Below this zoom terminal glyphs are too small to read, so cells render as
/// ink bars instead. Shaping thousands of illegible glyphs costs a great deal
/// and shows less than the bars do.
const GLYPH_MIN_ZOOM: f32 = 0.55;

/// How far a partly covered node may fragment before it is drawn whole.
const MAX_VISIBLE_REGIONS: usize = 4;

pub(super) struct TerminalOverlay<'a> {
    pub(super) focused: Option<NodeId>,
    pub(super) preedit: &'a str,
}

pub(super) fn draw(
    frame: &mut canvas::Frame,
    camera: Camera,
    viewport: ViewportSize,
    document: &CanvasDocument,
    selection: &[NodeId],
    terminal_overlay: TerminalOverlay<'_>,
    theme: &Theme,
) {
    let palette = theme.extended_palette();
    frame.fill_rectangle(Point::ORIGIN, frame.size(), shell::surface_color(palette));

    draw_grid(frame, camera, viewport, palette);
    draw_groups(frame, camera, viewport, document, palette);
    draw_connections(frame, camera, viewport, document, palette);
    draw_nodes(
        frame,
        camera,
        viewport,
        document,
        selection,
        terminal_overlay,
        palette,
    );
}

fn draw_grid(
    frame: &mut canvas::Frame,
    camera: Camera,
    viewport: ViewportSize,
    palette: &palette::Extended,
) {
    let visible = camera.visible_world_rect(viewport);
    let step = grid_step(camera.zoom());
    let first_x = (visible.x / step).floor() * step;
    let first_y = (visible.y / step).floor() * step;
    let right = visible.x + visible.width;
    let bottom = visible.y + visible.height;
    let columns = ((right - first_x) / step).ceil() as usize + 1;
    let rows = ((bottom - first_y) / step).ceil() as usize + 1;

    let grid = Path::new(|path| {
        for index in 0..columns {
            let world_x = first_x + index as f64 * step;
            let screen_x = camera
                .world_to_screen(WorldPoint::new(world_x, 0.0), viewport)
                .x as f32;
            path.move_to(Point::new(screen_x, 0.0));
            path.line_to(Point::new(screen_x, viewport.height as f32));
        }
        for index in 0..rows {
            let world_y = first_y + index as f64 * step;
            let screen_y = camera
                .world_to_screen(WorldPoint::new(0.0, world_y), viewport)
                .y as f32;
            path.move_to(Point::new(0.0, screen_y));
            path.line_to(Point::new(viewport.width as f32, screen_y));
        }
    });
    // The grid is orientation, not decoration: it must be visible up close and
    // never compete with the nodes drawn on top of it.
    frame.stroke(
        &grid,
        Stroke::default()
            .with_color(
                shell::sunken_color(palette).scale_alpha(if palette.is_dark { 1.0 } else { 0.38 }),
            )
            .with_width(0.75),
    );

    let shows_vertical_axis = visible.x <= 0.0 && visible.x + visible.width >= 0.0;
    let shows_horizontal_axis = visible.y <= 0.0 && visible.y + visible.height >= 0.0;
    let axes = Path::new(|path| {
        if shows_vertical_axis {
            let x = camera
                .world_to_screen(WorldPoint::new(0.0, visible.y), viewport)
                .x as f32;
            path.move_to(Point::new(x, 0.0));
            path.line_to(Point::new(x, viewport.height as f32));
        }
        if shows_horizontal_axis {
            let y = camera
                .world_to_screen(WorldPoint::new(visible.x, 0.0), viewport)
                .y as f32;
            path.move_to(Point::new(0.0, y));
            path.line_to(Point::new(viewport.width as f32, y));
        }
    });
    frame.stroke(
        &axes,
        Stroke::default()
            .with_color(
                shell::sunken_color(palette).scale_alpha(if palette.is_dark { 1.0 } else { 0.38 }),
            )
            .with_width(0.75),
    );
}

fn draw_groups(
    frame: &mut canvas::Frame,
    camera: Camera,
    viewport: ViewportSize,
    document: &CanvasDocument,
    palette: &palette::Extended,
) {
    for group in document.layout().groups() {
        let Some(bounds) = group_bounds(group, document) else {
            continue;
        };
        let padding = 20.0;
        let top_left = camera.world_to_screen(
            WorldPoint::new(bounds.x - padding, bounds.y - padding),
            viewport,
        );
        let shape = Path::rounded_rectangle(
            Point::new(top_left.x as f32, top_left.y as f32),
            Size::new(
                ((bounds.width + padding * 2.0) * camera.zoom()) as f32,
                ((bounds.height + padding * 2.0) * camera.zoom()) as f32,
            ),
            14.0.into(),
        );
        frame.stroke(
            &shape,
            Stroke::default()
                .with_color(palette.secondary.base.color)
                .with_width(1.5),
        );
    }
}

fn draw_connections(
    frame: &mut canvas::Frame,
    camera: Camera,
    viewport: ViewportSize,
    document: &CanvasDocument,
    palette: &palette::Extended,
) {
    for connection in document.layout().connections() {
        let Some(source) = document
            .layout()
            .nodes()
            .iter()
            .find(|node| node.id() == connection.source())
        else {
            continue;
        };
        let Some(target) = document
            .layout()
            .nodes()
            .iter()
            .find(|node| node.id() == connection.target())
        else {
            continue;
        };
        let source_rect = node_screen_rect(source, camera, viewport);
        let target_rect = node_screen_rect(target, camera, viewport);
        let from = edge_anchor(source_rect, target_rect.center());
        let to = edge_anchor(target_rect, source_rect.center());
        let path = connection_path(from, to);
        frame.stroke(
            &path,
            Stroke {
                line_dash: canvas::LineDash {
                    segments: &[6.0, 5.0],
                    offset: 0,
                },
                ..Stroke::default()
                    .with_color(connection_color(connection.kind(), palette).scale_alpha(0.7))
                    .with_width((1.5 * camera.zoom() as f32).clamp(1.0, 2.5))
            },
        );
    }
}

pub(super) struct ConnectionPreview {
    pub source: NodeId,
    pub pointer: Point,
    pub target: Option<NodeId>,
}

pub(super) fn draw_connection_preview(
    frame: &mut canvas::Frame,
    camera: Camera,
    viewport: ViewportSize,
    document: &CanvasDocument,
    preview: ConnectionPreview,
    palette: &palette::Extended,
) {
    let Some(source) = document
        .layout()
        .nodes()
        .iter()
        .find(|node| node.id() == preview.source)
    else {
        return;
    };
    let target = preview.target.and_then(|id| {
        document
            .layout()
            .nodes()
            .iter()
            .find(|node| node.id() == id)
    });
    let source_rect = node_screen_rect(source, camera, viewport);
    let target_rect = target.map(|node| node_screen_rect(node, camera, viewport));
    let from = edge_anchor(
        source_rect,
        target_rect.map_or(preview.pointer, |rect| rect.center()),
    );
    let to = target_rect.map_or(preview.pointer, |rect| {
        edge_anchor(rect, source_rect.center())
    });
    let valid = target.is_none_or(|target| {
        target.id() != source.id()
            && ConnectionKind::between_content(source.content(), target.content()).is_some()
            && !document.layout().connections().iter().any(|connection| {
                connection.source() == source.id() && connection.target() == target.id()
            })
    });
    let color = if valid {
        palette.primary.base.color
    } else {
        palette.danger.base.color
    };
    frame.stroke(
        &connection_path(from, to),
        Stroke {
            line_dash: canvas::LineDash {
                segments: &[6.0, 5.0],
                offset: 0,
            },
            ..Stroke::default().with_color(color).with_width(1.5)
        },
    );
    for point in [from, to] {
        frame.fill(&Path::circle(point, 4.0), color);
    }
    if let Some(rect) = target_rect {
        frame.stroke(
            &Path::rounded_rectangle(rect.position(), rect.size(), 6.0.into()),
            Stroke::default().with_color(color).with_width(2.0),
        );
    }
}

fn node_screen_rect(node: &Node, camera: Camera, viewport: ViewportSize) -> Rectangle {
    let origin = camera.world_to_screen(
        WorldPoint::new(
            f64::from(node.position().x()),
            f64::from(node.position().y()),
        ),
        viewport,
    );
    Rectangle::new(
        Point::new(origin.x as f32, origin.y as f32),
        Size::new(
            node.size().width() * camera.zoom() as f32,
            node.size().height() * camera.zoom() as f32,
        ),
    )
}

pub(super) fn rename_button_bounds(
    node: &Node,
    camera: Camera,
    viewport: ViewportSize,
) -> Option<Rectangle> {
    rename_button_in_rect(node, camera, node_screen_rect(node, camera, viewport))
}

fn rename_button_in_rect(node: &Node, camera: Camera, rect: Rectangle) -> Option<Rectangle> {
    if !matches!(node.reference(), Some(NodeTarget::Agent(_))) {
        return None;
    }
    if rect.width < 80.0 {
        return None;
    }
    let header = (HEADER_HEIGHT * camera.zoom() as f32).clamp(28.0, 60.0);
    Some(Rectangle::new(
        Point::new(rect.x + rect.width - 28.0, rect.y + (header - 24.0) * 0.5),
        Size::new(24.0, 24.0),
    ))
}

fn edge_anchor(rect: Rectangle, toward: Point) -> Point {
    let center = rect.center();
    let dx = toward.x - center.x;
    let dy = toward.y - center.y;
    if dx.abs() > dy.abs() {
        Point::new(
            if dx >= 0.0 {
                rect.x + rect.width
            } else {
                rect.x
            },
            center.y,
        )
    } else {
        Point::new(
            center.x,
            if dy >= 0.0 {
                rect.y + rect.height
            } else {
                rect.y
            },
        )
    }
}

fn connection_path(from: Point, to: Point) -> Path {
    let delta = to - from;
    let vertical = delta.y.abs() >= delta.x.abs();
    let reach = (if vertical {
        delta.y.abs()
    } else {
        delta.x.abs()
    } * 0.5)
        .clamp(20.0, 220.0);
    let offset = if vertical {
        Vector::new(0.0, reach * delta.y.signum())
    } else {
        Vector::new(reach * delta.x.signum(), 0.0)
    };
    Path::new(|builder| {
        builder.move_to(from);
        builder.bezier_curve_to(from + offset, to - offset, to);
    })
}

/// Emits text only where a node is actually exposed.
///
/// The renderer batches canvas text separately from canvas geometry and draws
/// it last, so a covered node's text would otherwise paint over the node
/// covering it. Geometry needs no such help: meshes honour draw order. Clipping
/// meshes here would in fact lose them, which is why only text goes through
/// this path.
fn clip_text(
    frame: &mut canvas::Frame,
    regions: &[Rectangle],
    bounds: Rectangle,
    mut draw: impl FnMut(&mut canvas::Frame),
) {
    for region in regions {
        let Some(clip) = region.intersection(&bounds) else {
            continue;
        };
        if clip.width <= 0.0 || clip.height <= 0.0 {
            continue;
        }
        frame.with_clip(clip, &mut draw);
    }
}

/// The parts of `rect` that `occluder` leaves uncovered.
fn subtract(rect: Rectangle, occluder: Rectangle) -> Vec<Rectangle> {
    let Some(overlap) = rect.intersection(&occluder) else {
        return vec![rect];
    };
    if overlap.width <= 0.0 || overlap.height <= 0.0 {
        return vec![rect];
    }

    let mut parts = Vec::new();
    let rect_bottom = rect.y + rect.height;
    let rect_right = rect.x + rect.width;
    let overlap_bottom = overlap.y + overlap.height;
    let overlap_right = overlap.x + overlap.width;

    if overlap.y > rect.y {
        parts.push(Rectangle::new(
            Point::new(rect.x, rect.y),
            Size::new(rect.width, overlap.y - rect.y),
        ));
    }
    if overlap_bottom < rect_bottom {
        parts.push(Rectangle::new(
            Point::new(rect.x, overlap_bottom),
            Size::new(rect.width, rect_bottom - overlap_bottom),
        ));
    }
    if overlap.x > rect.x {
        parts.push(Rectangle::new(
            Point::new(rect.x, overlap.y),
            Size::new(overlap.x - rect.x, overlap.height),
        ));
    }
    if overlap_right < rect_right {
        parts.push(Rectangle::new(
            Point::new(overlap_right, overlap.y),
            Size::new(rect_right - overlap_right, overlap.height),
        ));
    }
    parts
}

/// The parts of `rect` still visible: inside `within`, and not taken by an
/// occluder.
///
/// The renderer batches all canvas text and draws it above all canvas
/// geometry, so a node covered by another cannot be hidden by draw order, and
/// a node hanging off the edge is not clipped to the widget either. Its text
/// would paint straight through the node above it and across the workspace
/// rail beside it. Clipping each node to what it actually shows is what keeps
/// both honest.
fn visible_regions(rect: Rectangle, within: Rectangle, occluders: &[Rectangle]) -> Vec<Rectangle> {
    let Some(rect) = rect.intersection(&within) else {
        return Vec::new();
    };
    let mut regions = vec![rect];
    for occluder in occluders {
        if regions.is_empty() {
            break;
        }
        let mut split = regions
            .iter()
            .flat_map(|region| subtract(*region, *occluder))
            .collect::<Vec<_>>();
        // Bound the clipping cost without exposing text under higher cards.
        split.truncate(MAX_VISIBLE_REGIONS);
        regions = split;
    }
    regions
        .into_iter()
        .filter(|region| region.width > 0.0 && region.height > 0.0)
        .collect()
}

fn draw_nodes(
    frame: &mut canvas::Frame,
    camera: Camera,
    viewport: ViewportSize,
    document: &CanvasDocument,
    selection: &[NodeId],
    terminal_overlay: TerminalOverlay<'_>,
    palette: &palette::Extended,
) {
    let visible = camera.visible_world_rect(viewport);
    let selected = selection.iter().copied().collect::<BTreeSet<_>>();
    let mut nodes = document
        .layout()
        .nodes()
        .iter()
        .filter(|node| visible.intersects(node_bounds(node)))
        .collect::<Vec<_>>();
    nodes.sort_by_key(|node| (node.z_index(), node.id()));

    let canvas_bounds = Rectangle::new(
        Point::ORIGIN,
        Size::new(viewport.width as f32, viewport.height as f32),
    );

    // Screen rectangles in the same back-to-front order, so each node can be
    // clipped to the part of it that the nodes above have not covered.
    let rects = nodes
        .iter()
        .map(|node| {
            let origin = camera.world_to_screen(
                WorldPoint::new(
                    f64::from(node.position().x()),
                    f64::from(node.position().y()),
                ),
                viewport,
            );
            Rectangle::new(
                Point::new(origin.x as f32, origin.y as f32),
                Size::new(
                    (f64::from(node.size().width()) * camera.zoom()) as f32,
                    (f64::from(node.size().height()) * camera.zoom()) as f32,
                ),
            )
        })
        .collect::<Vec<_>>();

    for (index, node) in nodes.iter().enumerate() {
        let regions = visible_regions(rects[index], canvas_bounds, &rects[index + 1..]);
        if regions.is_empty() {
            continue;
        }
        draw_node(
            frame,
            camera,
            document,
            node,
            rects[index],
            &regions,
            selected.contains(&node.id()),
            terminal_overlay.focused,
            terminal_overlay.preedit,
            palette,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_node(
    frame: &mut canvas::Frame,
    camera: Camera,
    document: &CanvasDocument,
    node: &Node,
    rect: Rectangle,
    regions: &[Rectangle],
    is_selected: bool,
    focused_terminal: Option<NodeId>,
    preedit: &str,
    palette: &palette::Extended,
) {
    {
        let label = document.label(node.id());
        let top_left = rect.position();
        let size = rect.size();
        let radius = (6.0 * camera.zoom() as f32).clamp(3.0, 10.0);
        let shape = Path::rounded_rectangle(top_left, size, radius.into());
        let accent = node_color(label.kind, palette);

        if !palette.is_dark {
            for spread in [3.0, 2.0, 1.0] {
                frame.fill(
                    &Path::rounded_rectangle(
                        Point::new(top_left.x - spread, top_left.y - spread + 2.0),
                        Size::new(size.width + spread * 2.0, size.height + spread * 2.0),
                        (radius + spread).into(),
                    ),
                    Color::BLACK.scale_alpha(0.018),
                );
            }
        }
        let is_note = matches!(node.content(), CanvasNodeContent::Note { .. });
        let surface = if is_note && !palette.is_dark {
            Color::from_rgb8(255, 253, 218)
        } else {
            shell::surface_color(palette)
        };
        frame.fill(&shape, surface);
        let zoom = camera.zoom() as f32;
        let header_height = (HEADER_HEIGHT * zoom).clamp(28.0, 60.0);
        let header_color = if is_note && !palette.is_dark {
            Color::from_rgb8(249, 245, 202)
        } else {
            shell::sunken_color(palette)
        };
        frame.fill(
            &Path::rounded_rectangle(
                top_left,
                Size::new(size.width, header_height),
                radius.into(),
            ),
            header_color,
        );
        frame.fill_rectangle(
            Point::new(top_left.x, top_left.y + header_height - radius),
            Size::new(size.width, radius),
            header_color,
        );
        frame.fill_rectangle(
            Point::new(top_left.x, top_left.y + header_height - 1.0),
            Size::new(size.width, 1.0),
            shell::hairline_color(palette),
        );
        frame.stroke(
            &shape,
            if is_selected {
                Stroke {
                    line_dash: canvas::LineDash {
                        segments: &[5.0, 4.0],
                        offset: 0,
                    },
                    ..Stroke::default()
                        .with_color(palette.primary.base.color)
                        .with_width(1.5)
                }
            } else {
                Stroke::default()
                    .with_color(shell::hairline_color(palette))
                    .with_width(1.0)
            },
        );

        let padding = (10.0 * zoom).clamp(7.0, 16.0);
        let marker_size = (7.0 * zoom).clamp(5.0, 10.0);
        frame.stroke(
            &Path::rounded_rectangle(
                Point::new(
                    top_left.x + padding,
                    top_left.y + (header_height - marker_size) * 0.5,
                ),
                Size::new(marker_size, marker_size),
                2.0.into(),
            ),
            Stroke::default().with_color(accent).with_width(1.4),
        );
        let shows_detail = camera.zoom() >= BODY_MIN_ZOOM && size.width >= 300.0;
        let rename_button = is_selected
            .then(|| rename_button_in_rect(node, camera, rect))
            .flatten();
        let name_inset = if rename_button.is_some() { 28.0 } else { 0.0 };
        if let Some(button) = rename_button {
            let center = button.center();
            frame.fill(
                &Path::rounded_rectangle(button.position(), button.size(), 5.0.into()),
                shell::surface_color(palette),
            );
            let pencil = Path::new(|path| {
                path.move_to(Point::new(center.x - 5.0, center.y + 5.0));
                path.line_to(Point::new(center.x - 4.0, center.y + 1.0));
                path.line_to(Point::new(center.x + 3.0, center.y - 6.0));
                path.line_to(Point::new(center.x + 6.0, center.y - 3.0));
                path.line_to(Point::new(center.x - 1.0, center.y + 4.0));
                path.close();
                path.move_to(Point::new(center.x + 1.0, center.y - 4.0));
                path.line_to(Point::new(center.x + 4.0, center.y - 1.0));
            });
            frame.stroke(
                &pencil,
                Stroke::default()
                    .with_color(shell::muted_color(palette))
                    .with_width(1.2),
            );
        }
        let title_size = (12.0 * zoom).clamp(9.0, 16.0);
        let title_x = top_left.x + padding + marker_size + 6.0;
        let title_width = if shows_detail {
            size.width * 0.45
        } else {
            size.width - name_inset
        };
        let title_bounds = Rectangle::new(
            Point::new(title_x, top_left.y),
            Size::new(
                (title_width - (title_x - top_left.x) - padding).max(1.0),
                header_height,
            ),
        );
        clip_text(frame, regions, title_bounds, |frame| {
            frame.fill_text(canvas::Text {
                content: label.title.clone(),
                position: Point::new(
                    title_x,
                    top_left.y + (header_height - title_size * 1.3) * 0.5,
                ),
                color: palette.background.base.text,
                size: Pixels(title_size),
                ..canvas::Text::default()
            });
        });
        if shows_detail {
            let detail_bounds = Rectangle::new(
                Point::new(top_left.x + title_width, top_left.y),
                Size::new(
                    (size.width - title_width - padding - name_inset).max(1.0),
                    header_height,
                ),
            );
            clip_text(frame, regions, detail_bounds, |frame| {
                let detail_size = (10.0 * zoom).clamp(8.0, 12.0);
                frame.fill_text(canvas::Text {
                    content: label.subtitle.clone(),
                    position: Point::new(
                        detail_bounds.x,
                        top_left.y + (header_height - detail_size * 1.3) * 0.5,
                    ),
                    color: shell::muted_color(palette),
                    size: Pixels(detail_size),
                    ..canvas::Text::default()
                });
            });
        }

        if let Some(terminal) = document.terminal(node.id()) {
            draw_terminal(
                frame,
                top_left,
                header_height,
                zoom,
                terminal,
                (focused_terminal == Some(node.id())).then_some(preedit),
                regions,
                rect,
                palette,
            );
        } else {
            let body_padding = (12.0 * zoom).clamp(7.0, 16.0);
            let body_top_left = Point::new(
                top_left.x + body_padding,
                top_left.y + header_height + body_padding,
            );
            let body_size = Size::new(
                (size.width - body_padding * 2.0).max(1.0),
                (size.height - header_height - body_padding * 2.0).max(1.0),
            );
            let rendered_portal = document
                .portal_frame(node.id())
                .is_some_and(|portal_frame| {
                    draw_portal_frame(
                        frame,
                        node.content(),
                        portal_frame,
                        body_top_left,
                        body_size,
                    )
                });
            if !rendered_portal {
                clip_text(frame, regions, rect, |frame| {
                    draw_content(
                        frame,
                        body_top_left,
                        body_size,
                        zoom,
                        node.content(),
                        document.body(node.id()),
                        palette.background.base.text,
                    );
                });
            }
        }

        if is_selected {
            // A small filled grip in the corner, sized to stay grabbable at
            // low zoom without turning into a block over the node body.
            let handle = (10.0 * camera.zoom() as f32).clamp(7.0, 13.0);
            let grip = Path::rounded_rectangle(
                Point::new(
                    top_left.x + size.width - handle - 2.0,
                    top_left.y + size.height - handle - 2.0,
                ),
                Size::new(handle, handle),
                3.0.into(),
            );
            frame.fill(&grip, palette.primary.base.color);
            frame.stroke(
                &grip,
                Stroke::default()
                    .with_color(shell::surface_color(palette))
                    .with_width(1.5),
            );
        }
    }
}

fn draw_portal_frame(
    frame: &mut canvas::Frame,
    content: &CanvasNodeContent,
    portal_frame: &PortalFrame,
    body_top_left: Point,
    body_size: Size,
) -> bool {
    let CanvasNodeContent::Portal(config) = content else {
        return false;
    };
    let Some(node_bounds) = PortalRect::new(
        f64::from(body_top_left.x),
        f64::from(body_top_left.y),
        f64::from(body_size.width),
        f64::from(body_size.height),
    ) else {
        return false;
    };
    let transform = PortalFrameTransform::new(
        node_bounds,
        portal_frame.viewport(),
        config.presentation().preserve_aspect_ratio(),
    );
    let destination = transform.destination();
    let image = Handle::from_bytes(portal_frame.bytes().to_vec());
    frame.draw_image(
        Rectangle::new(
            Point::new(destination.x() as f32, destination.y() as f32),
            Size::new(destination.width() as f32, destination.height() as f32),
        ),
        &image,
    );
    true
}

fn draw_content(
    frame: &mut canvas::Frame,
    body_top_left: Point,
    body_size: Size,
    zoom: f32,
    content: &CanvasNodeContent,
    body: Option<&str>,
    text_color: Color,
) {
    match content {
        CanvasNodeContent::Reference(_) => {}
        CanvasNodeContent::Note { path, .. } => draw_body_text(
            frame,
            body_top_left,
            body.unwrap_or(path.as_str()),
            zoom,
            text_color,
        ),
        CanvasNodeContent::FileTree { root } => draw_body_text(
            frame,
            body_top_left,
            body.unwrap_or(root.as_str()),
            zoom,
            text_color,
        ),
        CanvasNodeContent::Artifact { path } => draw_body_text(
            frame,
            body_top_left,
            body.unwrap_or(path.as_str()),
            zoom,
            text_color,
        ),
        CanvasNodeContent::Diff { path, .. } => draw_body_text(
            frame,
            body_top_left,
            body.unwrap_or(path.as_str()),
            zoom,
            text_color,
        ),
        CanvasNodeContent::Text { markdown } => {
            draw_body_text(frame, body_top_left, markdown.as_str(), zoom, text_color)
        }
        CanvasNodeContent::Portal(config) => draw_body_text(
            frame,
            body_top_left,
            body.unwrap_or(config.target().selector()),
            zoom,
            text_color,
        ),
        CanvasNodeContent::Shape(shape) => {
            let path = match shape.kind() {
                ShapeKind::Rectangle => Path::rounded_rectangle(
                    body_top_left,
                    body_size,
                    (8.0 * zoom).clamp(3.0, 12.0).into(),
                ),
                ShapeKind::Ellipse => Path::new(|path| {
                    path.ellipse(canvas::path::arc::Elliptical {
                        center: Point::new(
                            body_top_left.x + body_size.width / 2.0,
                            body_top_left.y + body_size.height / 2.0,
                        ),
                        radii: Vector::new(body_size.width / 2.0, body_size.height / 2.0),
                        rotation: Radians(0.0),
                        start_angle: Radians(0.0),
                        end_angle: Radians(std::f32::consts::TAU),
                    });
                    path.close();
                }),
            };
            frame.fill(&path, canvas_color(shape.fill()));
            frame.stroke(
                &path,
                Stroke::default()
                    .with_color(canvas_color(shape.stroke()))
                    .with_width((shape.stroke_width().get() * zoom).max(1.0)),
            );
        }
        CanvasNodeContent::Arrow(arrow) => {
            let start = normalized_screen_point(body_top_left, body_size, arrow.start());
            let end = normalized_screen_point(body_top_left, body_size, arrow.end());
            let color = canvas_color(arrow.stroke());
            let width = (arrow.stroke_width().get() * zoom).max(1.0);
            frame.stroke(
                &Path::line(start, end),
                Stroke::default().with_color(color).with_width(width),
            );
            let angle = (end.y - start.y).atan2(end.x - start.x);
            let head = 12.0 * zoom;
            let left = Point::new(
                end.x - head * (angle - 0.55).cos(),
                end.y - head * (angle - 0.55).sin(),
            );
            let right = Point::new(
                end.x - head * (angle + 0.55).cos(),
                end.y - head * (angle + 0.55).sin(),
            );
            let arrowhead = Path::new(|path| {
                path.move_to(end);
                path.line_to(left);
                path.line_to(right);
                path.close();
            });
            frame.fill(&arrowhead, color);
        }
        CanvasNodeContent::Freehand(freehand) => {
            let points = freehand.points();
            let path = Path::new(|path| {
                if let Some(first) = points.first() {
                    path.move_to(normalized_screen_point(body_top_left, body_size, *first));
                    for point in &points[1..] {
                        path.line_to(normalized_screen_point(body_top_left, body_size, *point));
                    }
                }
            });
            frame.stroke(
                &path,
                Stroke::default()
                    .with_color(canvas_color(freehand.stroke()))
                    .with_width((freehand.stroke_width().get() * zoom).max(1.0)),
            );
        }
    }
}

fn normalized_screen_point(
    top_left: Point,
    size: Size,
    point: openpodium::domain::NormalizedPoint,
) -> Point {
    Point::new(
        top_left.x + size.width * point.x(),
        top_left.y + size.height * point.y(),
    )
}

fn draw_body_text(
    frame: &mut canvas::Frame,
    position: Point,
    content: &str,
    zoom: f32,
    color: Color,
) {
    let content = content
        .lines()
        .take(12)
        .map(|line| line.chars().take(80).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n");
    frame.fill_text(canvas::Text {
        content,
        position,
        color,
        size: Pixels((12.0 * zoom).clamp(8.0, 15.0)),
        ..canvas::Text::default()
    });
}

fn canvas_color(color: CanvasColor) -> Color {
    let [red, green, blue, alpha] = color.channels();
    Color::from_rgba8(red, green, blue, f32::from(alpha) / 255.0)
}

#[allow(clippy::too_many_arguments)]
fn draw_terminal(
    frame: &mut canvas::Frame,
    top_left: Point,
    header_height: f32,
    zoom: f32,
    terminal: &terminal::View,
    preedit: Option<&str>,
    regions: &[Rectangle],
    bounds: Rectangle,
    palette: &palette::Extended,
) {
    let origin = Point::new(
        top_left.x + BODY_PADDING * zoom,
        top_left.y + header_height + BODY_PADDING * zoom,
    );
    let cell_size = Size::new(CELL_WIDTH * zoom, CELL_HEIGHT * zoom);
    terminal_background_runs(&terminal.cells, palette, |row, column, columns, color| {
        frame.fill_rectangle(
            Point::new(
                origin.x + column as f32 * cell_size.width,
                origin.y + row as f32 * cell_size.height,
            ),
            Size::new(columns as f32 * cell_size.width, cell_size.height),
            color,
        );
    });
    for cell in terminal.cells.iter() {
        let position = Point::new(
            origin.x + cell.column as f32 * cell_size.width,
            origin.y + cell.row as f32 * cell_size.height,
        );
        if cell.text != " " && zoom < GLYPH_MIN_ZOOM {
            // Too small to read. An ink bar keeps the shape of the output,
            // which is the only thing a glyph could convey at this size.
            frame.fill_rectangle(
                Point::new(position.x, position.y + cell_size.height * 0.25),
                Size::new(cell_size.width, (cell_size.height * 0.5).max(1.0)),
                terminal_color(cell.foreground, palette).scale_alpha(0.85),
            );
        }
        if cell.underline || cell.hyperlink.is_some() {
            frame.fill_rectangle(
                Point::new(position.x, position.y + cell_size.height - zoom.max(1.0)),
                Size::new(cell_size.width, zoom.max(1.0)),
                terminal_color(cell.foreground, palette),
            );
        }
        if cell.strikeout {
            frame.fill_rectangle(
                Point::new(position.x, position.y + cell_size.height * 0.52),
                Size::new(cell_size.width, zoom.max(1.0)),
                terminal_color(cell.foreground, palette),
            );
        }
    }

    if zoom >= GLYPH_MIN_ZOOM {
        clip_text(frame, regions, bounds, |frame| {
            for cell in terminal.cells.iter() {
                if cell.text == " " {
                    continue;
                }
                let position = Point::new(
                    origin.x + cell.column as f32 * cell_size.width,
                    origin.y + cell.row as f32 * cell_size.height,
                );
                frame.fill_text(canvas::Text {
                    content: cell.text.clone(),
                    position: Point::new(position.x, position.y - cell_size.height * 0.04),
                    color: terminal_color(cell.foreground, palette),
                    size: Pixels((13.0 * zoom).max(5.0)),
                    font: Font {
                        family: font::Family::Monospace,
                        weight: if cell.bold {
                            font::Weight::Bold
                        } else {
                            font::Weight::Normal
                        },
                        style: if cell.italic {
                            font::Style::Italic
                        } else {
                            font::Style::Normal
                        },
                        ..Font::MONOSPACE
                    },
                    ..canvas::Text::default()
                });
            }
        });
    }

    if let Some(cursor) = terminal.cursor {
        let cursor_color = terminal
            .cells
            .iter()
            .find(|cell| cell.row == cursor.row && cell.column == cursor.column)
            .map(|cell| terminal_color(cell.foreground, palette))
            .unwrap_or(palette.background.base.text);
        let position = Point::new(
            origin.x + cursor.column as f32 * cell_size.width,
            origin.y + cursor.row as f32 * cell_size.height,
        );
        let (cursor_position, cursor_size) = match cursor.style {
            terminal::CursorStyle::Block => (position, cell_size),
            terminal::CursorStyle::Underline => (
                Point::new(
                    position.x,
                    position.y + cell_size.height - (2.0 * zoom).max(1.0),
                ),
                Size::new(cell_size.width, (2.0 * zoom).max(1.0)),
            ),
            terminal::CursorStyle::Beam => {
                (position, Size::new((2.0 * zoom).max(1.0), cell_size.height))
            }
        };
        frame.fill_rectangle(cursor_position, cursor_size, cursor_color.scale_alpha(0.45));

        if let Some(preedit) = preedit.filter(|preedit| !preedit.is_empty()) {
            clip_text(frame, regions, bounds, |frame| {
                frame.fill_text(canvas::Text {
                    content: preedit.to_owned(),
                    position,
                    color: cursor_color,
                    size: Pixels((13.0 * zoom).max(5.0)),
                    font: Font::MONOSPACE,
                    ..canvas::Text::default()
                });
            });
        }
    }
}

fn terminal_background_runs(
    cells: &[terminal::CellView],
    palette: &palette::Extended,
    mut paint: impl FnMut(usize, usize, usize, Color),
) {
    let surface = shell::surface_color(palette);
    let mut cells = cells.iter().peekable();
    while let Some(cell) = cells.next() {
        let color = terminal_color(cell.background, palette);
        // The card already paints this surface, including empty terminal cells.
        if color == surface {
            continue;
        }
        let mut columns = 1;
        while cells.peek().is_some_and(|next| {
            next.row == cell.row
                && next.column == cell.column + columns
                && terminal_color(next.background, palette) == color
        }) {
            cells.next();
            columns += 1;
        }
        paint(cell.row, cell.column, columns, color);
    }
}

fn terminal_color(color: terminal::CellColor, palette: &palette::Extended) -> Color {
    use terminal::DefaultColor;
    let mut resolved = match color.default_role {
        Some(DefaultColor::Foreground | DefaultColor::DimForeground) => {
            palette.background.base.text
        }
        Some(DefaultColor::Background | DefaultColor::DimBackground) => {
            shell::surface_color(palette)
        }
        None => return Color::from_rgb8(color.red, color.green, color.blue),
    };
    if matches!(
        color.default_role,
        Some(DefaultColor::DimForeground | DefaultColor::DimBackground)
    ) {
        resolved.r *= 2.0 / 3.0;
        resolved.g *= 2.0 / 3.0;
        resolved.b *= 2.0 / 3.0;
    }
    resolved
}

fn group_bounds(group: &NodeGroup, document: &CanvasDocument) -> Option<WorldRect> {
    let mut nodes = group.members().filter_map(|node_id| {
        document
            .layout()
            .nodes()
            .iter()
            .find(|node| node.id() == node_id)
    });
    let first = nodes.next()?;
    let mut left = f64::from(first.position().x());
    let mut top = f64::from(first.position().y());
    let mut right = left + f64::from(first.size().width());
    let mut bottom = top + f64::from(first.size().height());
    for node in nodes {
        left = left.min(f64::from(node.position().x()));
        top = top.min(f64::from(node.position().y()));
        right = right.max(f64::from(node.position().x() + node.size().width()));
        bottom = bottom.max(f64::from(node.position().y() + node.size().height()));
    }
    Some(WorldRect::new(left, top, right - left, bottom - top))
}

fn node_bounds(node: &Node) -> WorldRect {
    WorldRect::new(
        f64::from(node.position().x()),
        f64::from(node.position().y()),
        f64::from(node.size().width()),
        f64::from(node.size().height()),
    )
}

fn grid_step(zoom: f64) -> f64 {
    let mut step = BASE_GRID_STEP;
    while step * zoom < MIN_GRID_PIXELS {
        step *= 2.0;
    }
    step
}

fn node_color(kind: NodeKind, palette: &palette::Extended) -> Color {
    match kind {
        NodeKind::Agent {
            role_color: Some(color),
            ..
        } => color,
        NodeKind::Agent {
            program: AgentProgram::Codex,
            ..
        } => palette.primary.base.color,
        NodeKind::Agent {
            program: AgentProgram::Claude,
            ..
        } => palette.warning.base.color,
        NodeKind::Agent {
            program: AgentProgram::OpenCode,
            ..
        } => palette.secondary.base.color,
        NodeKind::Agent {
            program: AgentProgram::Custom(_),
            ..
        } => palette.primary.weak.color,
        NodeKind::Agent {
            program: AgentProgram::Shell,
            ..
        } => palette.success.base.color,
        NodeKind::Task => palette.secondary.base.color,
        NodeKind::Handoff => palette.danger.base.color,
        NodeKind::Context => palette.primary.weak.color,
    }
}

fn connection_color(kind: ConnectionKind, palette: &palette::Extended) -> Color {
    match kind {
        ConnectionKind::Coordination => palette.primary.base.color,
        ConnectionKind::Assignment => palette.success.base.color,
        ConnectionKind::Dependency => palette.warning.base.color,
        ConnectionKind::Handoff => palette.danger.base.color,
        ConnectionKind::Reference => palette.primary.weak.color,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn terminal_backgrounds_skip_default_cells_and_batch_solid_rows() {
        let mut model = crate::terminal::Model::new(crate::terminal::GridSize {
            columns: 80,
            rows: 30,
        });
        for theme in [super::shell::theme(), iced::Theme::Dark] {
            let mut rectangles = 0;
            super::terminal_background_runs(
                &model.view(crate::terminal::Status::Running).cells,
                theme.extended_palette(),
                |_, _, _, _| rectangles += 1,
            );
            assert_eq!(rectangles, 0);
        }
        model.feed(b"\x1b[41m\x1b[2J");
        let mut runs = Vec::new();
        super::terminal_background_runs(
            &model.view(crate::terminal::Status::Running).cells,
            iced::Theme::Dark.extended_palette(),
            |row, column, columns, _| runs.push((row, column, columns)),
        );
        assert_eq!(runs, (0..30).map(|row| (row, 0, 80)).collect::<Vec<_>>());
    }

    #[test]
    fn terminal_background_runs_preserve_colors_selection_and_gaps() {
        let mut model = crate::terminal::Model::new(crate::terminal::GridSize {
            columns: 20,
            rows: 4,
        });
        model.feed(
            "plain\x1b[41mred界red\x1b[0;7minverse\r\n\x1b[48;2;255;255;255mwhite".as_bytes(),
        );
        model.begin_selection(1, 1, false);
        model.update_selection(1, 3, true);
        let view = model.view(crate::terminal::Status::Running);
        for theme in [super::shell::theme(), iced::Theme::Dark] {
            let palette = theme.extended_palette();
            let mut painted = std::collections::BTreeMap::new();
            super::terminal_background_runs(&view.cells, palette, |row, column, columns, color| {
                for column in column..column + columns {
                    assert!(painted.insert((row, column), color).is_none());
                }
            });
            let expected = view
                .cells
                .iter()
                .filter_map(|cell| {
                    let color = super::terminal_color(cell.background, palette);
                    (color != super::shell::surface_color(palette))
                        .then_some(((cell.row, cell.column), color))
                })
                .collect();
            assert_eq!(painted, expected);
        }
    }

    #[test]
    fn terminal_theme_changes_defaults_but_not_explicit_colors() {
        let mut model = crate::terminal::Model::new(crate::terminal::GridSize {
            columns: 20,
            rows: 4,
        });
        model.feed(b"A\x1b[38;2;39;42;47;48;2;255;255;255mB\x1b[0;7mC");
        let view = model.view(crate::terminal::Status::Running);
        let cell = |text| view.cells.iter().find(|cell| cell.text == text).unwrap();
        for theme in [super::shell::theme(), iced::Theme::Dark] {
            let palette = theme.extended_palette();
            assert_eq!(
                super::terminal_color(cell("A").foreground, palette),
                palette.background.base.text
            );
            assert_eq!(
                super::terminal_color(cell("A").background, palette),
                super::shell::surface_color(palette)
            );
            assert_eq!(
                super::terminal_color(cell("B").foreground, palette),
                iced::Color::from_rgb8(39, 42, 47)
            );
            assert_eq!(
                super::terminal_color(cell("B").background, palette),
                iced::Color::WHITE
            );
            assert_eq!(
                super::terminal_color(cell("C").foreground, palette),
                super::shell::surface_color(palette)
            );
            assert_eq!(
                super::terminal_color(cell("C").background, palette),
                palette.background.base.text
            );
        }
    }

    use super::{MAX_VISIBLE_REGIONS, grid_step, visible_regions};
    use iced::{Point, Rectangle, Size};

    fn rect(x: f32, y: f32, width: f32, height: f32) -> Rectangle {
        Rectangle::new(Point::new(x, y), Size::new(width, height))
    }

    fn canvas() -> Rectangle {
        rect(-1_000.0, -1_000.0, 4_000.0, 4_000.0)
    }

    fn area(regions: &[Rectangle]) -> f32 {
        regions
            .iter()
            .map(|region| region.width * region.height)
            .sum()
    }

    #[test]
    fn grid_spacing_never_becomes_visually_dense() {
        assert_eq!(grid_step(4.0), 40.0);
        assert_eq!(grid_step(1.0), 40.0);
        assert_eq!(grid_step(0.5), 80.0);
        assert_eq!(grid_step(0.25), 160.0);
    }

    #[test]
    fn an_unobstructed_node_is_drawn_whole() {
        let node = rect(0.0, 0.0, 100.0, 100.0);

        assert_eq!(visible_regions(node, canvas(), &[]), vec![node]);
        assert_eq!(
            visible_regions(node, canvas(), &[rect(200.0, 200.0, 50.0, 50.0)]),
            vec![node]
        );
    }

    /// Dragging one node fully over another is the case that made text from the
    /// covered node paint straight through the one on top.
    #[test]
    fn a_fully_covered_node_is_not_drawn() {
        let node = rect(10.0, 10.0, 80.0, 80.0);

        assert!(visible_regions(node, canvas(), &[rect(0.0, 0.0, 200.0, 200.0)]).is_empty());
        assert!(visible_regions(node, canvas(), &[node]).is_empty());
        assert!(
            visible_regions(
                node,
                canvas(),
                &[rect(0.0, 0.0, 200.0, 50.0), rect(0.0, 50.0, 200.0, 150.0)]
            )
            .is_empty(),
            "coverage by several nodes together still hides it"
        );
    }

    #[test]
    fn a_partly_covered_node_keeps_only_its_exposed_area() {
        let node = rect(0.0, 0.0, 100.0, 100.0);

        // Covered from the right half.
        let regions = visible_regions(node, canvas(), &[rect(50.0, -10.0, 100.0, 120.0)]);
        assert_eq!(regions, vec![rect(0.0, 0.0, 50.0, 100.0)]);

        // Covered through the middle, leaving a band above and below.
        let regions = visible_regions(node, canvas(), &[rect(-10.0, 40.0, 120.0, 20.0)]);
        assert_eq!(area(&regions), 8_000.0);
        assert!(regions.iter().all(|region| region.height > 0.0));
    }

    /// The text batch is not clipped to the canvas widget, so a node hanging
    /// off the edge would otherwise print across the workspace rail.
    #[test]
    fn a_node_past_the_canvas_edge_is_cut_at_it() {
        let node = rect(-200.0, 50.0, 300.0, 100.0);
        let canvas = rect(0.0, 0.0, 800.0, 600.0);

        assert_eq!(
            visible_regions(node, canvas, &[]),
            vec![rect(0.0, 50.0, 100.0, 100.0)]
        );
        assert!(visible_regions(rect(-400.0, 0.0, 100.0, 100.0), canvas, &[]).is_empty());
    }

    #[test]
    fn fragmented_regions_remain_occluded_after_the_limit_is_reached() {
        let node = rect(0.0, 0.0, 100.0, 100.0);
        let occluders = [
            rect(20.0, 20.0, 10.0, 10.0),
            rect(60.0, 20.0, 10.0, 10.0),
            rect(20.0, 60.0, 10.0, 10.0),
            rect(60.0, 60.0, 10.0, 10.0),
        ];

        let regions = visible_regions(node, canvas(), &occluders);
        assert!(!regions.is_empty());
        assert!(regions.len() <= MAX_VISIBLE_REGIONS);
        assert!(regions.iter().all(|region| occluders.iter().all(|cover| {
            region
                .intersection(cover)
                .is_none_or(|overlap| overlap.width * overlap.height == 0.0)
        })));
        let mut fully_covered = occluders.to_vec();
        fully_covered.push(node);
        assert!(visible_regions(node, canvas(), &fully_covered).is_empty());
    }
}
#[test]
fn connection_anchors_stay_on_the_edges_facing_the_other_card() {
    let rect = Rectangle::new(Point::new(100.0, 100.0), Size::new(300.0, 200.0));
    assert_eq!(
        edge_anchor(rect, Point::new(250.0, 600.0)),
        Point::new(250.0, 300.0)
    );
    assert_eq!(
        edge_anchor(rect, Point::new(250.0, -200.0)),
        Point::new(250.0, 100.0)
    );
    assert_eq!(
        edge_anchor(rect, Point::new(800.0, 200.0)),
        Point::new(400.0, 200.0)
    );
    assert_eq!(
        edge_anchor(rect, Point::new(-200.0, 200.0)),
        Point::new(100.0, 200.0)
    );
}
