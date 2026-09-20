use std::collections::BTreeSet;

use iced::theme::palette;
use iced::widget::canvas::{self, Path, Stroke};
use iced::{Color, Font, Pixels, Point, Radians, Size, Theme, Vector, font};
use openpodium::domain::{
    AgentProgram, CanvasColor, CanvasNodeContent, ConnectionKind, Node, NodeGroup, NodeId,
    ShapeKind,
};

use crate::terminal::{self, BODY_PADDING, CELL_HEIGHT, CELL_WIDTH, HEADER_HEIGHT};

use super::{Camera, CanvasDocument, NodeKind, ViewportSize, WorldPoint, WorldRect};

const BASE_GRID_STEP: f64 = 40.0;
const MIN_GRID_PIXELS: f64 = 24.0;

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
    frame.fill_rectangle(Point::ORIGIN, frame.size(), palette.background.base.color);

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
    frame.stroke(
        &grid,
        Stroke::default()
            .with_color(palette.background.weak.color)
            .with_width(1.0),
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
            .with_color(palette.background.strong.color)
            .with_width(1.5),
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
        let from = camera.world_to_screen(node_center(source), viewport);
        let to = camera.world_to_screen(node_center(target), viewport);
        let path = Path::line(
            Point::new(from.x as f32, from.y as f32),
            Point::new(to.x as f32, to.y as f32),
        );
        frame.stroke(
            &path,
            Stroke::default()
                .with_color(connection_color(connection.kind(), palette))
                .with_width((2.0 * camera.zoom() as f32).clamp(1.0, 3.0)),
        );
    }
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

    for node in nodes {
        let label = document.label(node.id());
        let top_left = camera.world_to_screen(
            WorldPoint::new(
                f64::from(node.position().x()),
                f64::from(node.position().y()),
            ),
            viewport,
        );
        let top_left = Point::new(top_left.x as f32, top_left.y as f32);
        let size = Size::new(
            (f64::from(node.size().width()) * camera.zoom()) as f32,
            (f64::from(node.size().height()) * camera.zoom()) as f32,
        );
        let radius = (10.0 * camera.zoom() as f32).clamp(4.0, 14.0);
        let shape = Path::rounded_rectangle(top_left, size, radius.into());
        let accent = node_color(label.kind, palette);
        let is_selected = selected.contains(&node.id());

        frame.fill(&shape, Color::from_rgb(0.055, 0.065, 0.08));
        frame.stroke(
            &shape,
            Stroke::default()
                .with_color(if is_selected {
                    palette.primary.strong.color
                } else {
                    accent
                })
                .with_width(if is_selected { 3.0 } else { 1.5 }),
        );
        let zoom = camera.zoom() as f32;
        let header_height = (HEADER_HEIGHT * zoom).clamp(28.0, 60.0);
        frame.fill_rectangle(
            top_left,
            Size::new(size.width, header_height),
            palette.background.strong.color,
        );
        frame.fill_rectangle(
            top_left,
            Size::new((5.0 * camera.zoom() as f32).clamp(2.0, 6.0), header_height),
            accent,
        );

        if camera.zoom() >= 0.4 {
            let padding = (12.0 * zoom).clamp(7.0, 16.0);
            frame.fill_text(canvas::Text {
                content: label.title.clone(),
                position: Point::new(top_left.x + padding, top_left.y + padding * 0.45),
                color: palette.background.strong.text,
                size: Pixels((14.0 * zoom).clamp(9.0, 17.0)),
                ..canvas::Text::default()
            });
            frame.fill_text(canvas::Text {
                content: label.subtitle.clone(),
                position: Point::new(top_left.x + padding, top_left.y + header_height * 0.56),
                color: palette.secondary.base.color,
                size: Pixels((10.0 * zoom).clamp(7.0, 12.0)),
                ..canvas::Text::default()
            });
            if let Some(terminal) = document.terminal(node.id()) {
                draw_terminal(
                    frame,
                    top_left,
                    header_height,
                    zoom,
                    terminal,
                    (terminal_overlay.focused == Some(node.id()))
                        .then_some(terminal_overlay.preedit),
                );
            } else {
                let body_padding = (12.0 * zoom).clamp(7.0, 16.0);
                draw_content(
                    frame,
                    Point::new(
                        top_left.x + body_padding,
                        top_left.y + header_height + body_padding,
                    ),
                    Size::new(
                        (size.width - body_padding * 2.0).max(1.0),
                        (size.height - header_height - body_padding * 2.0).max(1.0),
                    ),
                    zoom,
                    node.content(),
                    document.body(node.id()),
                    palette.background.base.text,
                );
            }
        }

        if is_selected {
            let handle = (12.0 * camera.zoom() as f32).clamp(8.0, 16.0);
            frame.fill_rectangle(
                Point::new(
                    top_left.x + size.width - handle,
                    top_left.y + size.height - handle,
                ),
                Size::new(handle, handle),
                palette.primary.strong.color,
            );
        }
    }
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

fn draw_terminal(
    frame: &mut canvas::Frame,
    top_left: Point,
    header_height: f32,
    zoom: f32,
    terminal: &terminal::View,
    preedit: Option<&str>,
) {
    let origin = Point::new(
        top_left.x + BODY_PADDING * zoom,
        top_left.y + header_height + BODY_PADDING * zoom,
    );
    let cell_size = Size::new(CELL_WIDTH * zoom, CELL_HEIGHT * zoom);
    for cell in &terminal.cells {
        let position = Point::new(
            origin.x + cell.column as f32 * cell_size.width,
            origin.y + cell.row as f32 * cell_size.height,
        );
        let mut background = terminal_color(cell.background);
        if cell.selected {
            background.a = 1.0;
        }
        frame.fill_rectangle(position, cell_size, background);
        if cell.text != " " {
            frame.fill_text(canvas::Text {
                content: cell.text.clone(),
                position: Point::new(position.x, position.y - cell_size.height * 0.04),
                color: terminal_color(cell.foreground),
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
        if cell.underline || cell.hyperlink.is_some() {
            frame.fill_rectangle(
                Point::new(position.x, position.y + cell_size.height - zoom.max(1.0)),
                Size::new(cell_size.width, zoom.max(1.0)),
                terminal_color(cell.foreground),
            );
        }
        if cell.strikeout {
            frame.fill_rectangle(
                Point::new(position.x, position.y + cell_size.height * 0.52),
                Size::new(cell_size.width, zoom.max(1.0)),
                terminal_color(cell.foreground),
            );
        }
    }

    if let Some(cursor) = terminal.cursor {
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
        frame.fill_rectangle(
            cursor_position,
            cursor_size,
            Color::from_rgba(0.9, 0.93, 0.98, 0.55),
        );

        if let Some(preedit) = preedit.filter(|preedit| !preedit.is_empty()) {
            frame.fill_text(canvas::Text {
                content: preedit.to_owned(),
                position,
                color: Color::WHITE,
                size: Pixels((13.0 * zoom).max(5.0)),
                font: Font::MONOSPACE,
                ..canvas::Text::default()
            });
        }
    }
}

fn terminal_color(color: terminal::CellColor) -> Color {
    Color::from_rgb8(color.red, color.green, color.blue)
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

fn node_center(node: &Node) -> WorldPoint {
    WorldPoint::new(
        f64::from(node.position().x() + node.size().width() / 2.0),
        f64::from(node.position().y() + node.size().height() / 2.0),
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
    use super::grid_step;

    #[test]
    fn grid_spacing_never_becomes_visually_dense() {
        assert_eq!(grid_step(4.0), 40.0);
        assert_eq!(grid_step(1.0), 40.0);
        assert_eq!(grid_step(0.5), 80.0);
        assert_eq!(grid_step(0.25), 160.0);
    }
}
