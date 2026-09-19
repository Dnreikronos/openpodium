use std::collections::BTreeSet;

use iced::theme::palette;
use iced::widget::canvas::{self, Path, Stroke};
use iced::{Color, Pixels, Point, Size, Theme};
use openpodium::domain::{AgentProgram, ConnectionKind, Node, NodeGroup, NodeId};

use super::{Camera, CanvasDocument, NodeKind, ViewportSize, WorldPoint, WorldRect};

const BASE_GRID_STEP: f64 = 40.0;
const MIN_GRID_PIXELS: f64 = 24.0;

pub(super) fn draw(
    frame: &mut canvas::Frame,
    camera: Camera,
    viewport: ViewportSize,
    document: &CanvasDocument,
    selection: &[NodeId],
    theme: &Theme,
) {
    let palette = theme.extended_palette();
    frame.fill_rectangle(Point::ORIGIN, frame.size(), palette.background.base.color);

    draw_grid(frame, camera, viewport, palette);
    draw_groups(frame, camera, viewport, document, palette);
    draw_connections(frame, camera, viewport, document, palette);
    draw_nodes(frame, camera, viewport, document, selection, palette);
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
        let header_height = (48.0 * camera.zoom() as f32).clamp(28.0, 60.0);
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
            let padding = (15.0 * camera.zoom() as f32).clamp(8.0, 18.0);
            frame.fill_text(canvas::Text {
                content: label.title.clone(),
                position: Point::new(top_left.x + padding, top_left.y + padding * 0.75),
                color: palette.background.strong.text,
                size: Pixels((16.0 * camera.zoom() as f32).clamp(10.0, 19.0)),
                ..canvas::Text::default()
            });
            frame.fill_text(canvas::Text {
                content: label.subtitle.clone(),
                position: Point::new(
                    top_left.x + padding,
                    top_left.y + header_height + padding * 1.4,
                ),
                color: palette.secondary.base.color,
                size: Pixels((13.0 * camera.zoom() as f32).clamp(9.0, 15.0)),
                ..canvas::Text::default()
            });
            frame.fill_text(canvas::Text {
                content: "$ terminal will connect in runtime setup".to_owned(),
                position: Point::new(
                    top_left.x + padding,
                    top_left.y + header_height + padding * 3.0,
                ),
                color: palette.background.base.text,
                size: Pixels((12.0 * camera.zoom() as f32).clamp(8.0, 14.0)),
                ..canvas::Text::default()
            });
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
        NodeKind::Agent(AgentProgram::Codex) => palette.primary.base.color,
        NodeKind::Agent(AgentProgram::Claude) => palette.warning.base.color,
        NodeKind::Agent(AgentProgram::Shell) => palette.success.base.color,
        NodeKind::Task => palette.secondary.base.color,
        NodeKind::Handoff => palette.danger.base.color,
    }
}

fn connection_color(kind: ConnectionKind, palette: &palette::Extended) -> Color {
    match kind {
        ConnectionKind::Coordination => palette.primary.base.color,
        ConnectionKind::Assignment => palette.success.base.color,
        ConnectionKind::Dependency => palette.warning.base.color,
        ConnectionKind::Handoff => palette.danger.base.color,
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
