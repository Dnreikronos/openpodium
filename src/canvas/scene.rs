use iced::theme::palette;
use iced::widget::canvas::{self, Path, Stroke};
use iced::{Color, Pixels, Point, Size, Theme};

use super::{Camera, ViewportSize, WorldPoint, WorldRect};

const BASE_GRID_STEP: f64 = 40.0;
const MIN_GRID_PIXELS: f64 = 24.0;

#[derive(Debug, Clone, Copy)]
enum Accent {
    Primary,
    Success,
    Warning,
}

#[derive(Debug, Clone, Copy)]
struct CanvasNode {
    title: &'static str,
    subtitle: &'static str,
    bounds: WorldRect,
    accent: Accent,
}

const REPRESENTATIVE_NODES: [CanvasNode; 5] = [
    CanvasNode {
        title: "Lead agent",
        subtitle: "Planning",
        bounds: WorldRect::new(-390.0, -210.0, 230.0, 132.0),
        accent: Accent::Primary,
    },
    CanvasNode {
        title: "Builder",
        subtitle: "Implementing canvas",
        bounds: WorldRect::new(-70.0, -75.0, 250.0, 140.0),
        accent: Accent::Success,
    },
    CanvasNode {
        title: "Reviewer",
        subtitle: "Waiting",
        bounds: WorldRect::new(285.0, -195.0, 220.0, 130.0),
        accent: Accent::Warning,
    },
    CanvasNode {
        title: "Task queue",
        subtitle: "3 open tasks",
        bounds: WorldRect::new(-300.0, 185.0, 220.0, 120.0),
        accent: Accent::Warning,
    },
    CanvasNode {
        title: "Test agent",
        subtitle: "Ready",
        bounds: WorldRect::new(150.0, 210.0, 230.0, 126.0),
        accent: Accent::Primary,
    },
];

pub(super) fn draw(
    frame: &mut canvas::Frame,
    camera: Camera,
    viewport: ViewportSize,
    theme: &Theme,
) {
    let palette = theme.extended_palette();
    frame.fill_rectangle(Point::ORIGIN, frame.size(), palette.background.base.color);

    draw_grid(frame, camera, viewport, palette);
    draw_nodes(frame, camera, viewport, palette);
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
    if !shows_vertical_axis && !shows_horizontal_axis {
        return;
    }

    let axes = Path::new(|path| {
        if shows_vertical_axis {
            let screen_x = camera
                .world_to_screen(WorldPoint::new(0.0, visible.y), viewport)
                .x as f32;
            path.move_to(Point::new(screen_x, 0.0));
            path.line_to(Point::new(screen_x, viewport.height as f32));
        }
        if shows_horizontal_axis {
            let screen_y = camera
                .world_to_screen(WorldPoint::new(visible.x, 0.0), viewport)
                .y as f32;
            path.move_to(Point::new(0.0, screen_y));
            path.line_to(Point::new(viewport.width as f32, screen_y));
        }
    });
    frame.stroke(
        &axes,
        Stroke::default()
            .with_color(palette.background.strong.color)
            .with_width(1.5),
    );
}

fn draw_nodes(
    frame: &mut canvas::Frame,
    camera: Camera,
    viewport: ViewportSize,
    palette: &palette::Extended,
) {
    let visible = camera.visible_world_rect(viewport);

    for node in visible_nodes(&REPRESENTATIVE_NODES, visible) {
        let top_left =
            camera.world_to_screen(WorldPoint::new(node.bounds.x, node.bounds.y), viewport);
        let top_left = Point::new(top_left.x as f32, top_left.y as f32);
        let size = Size::new(
            (node.bounds.width * camera.zoom()) as f32,
            (node.bounds.height * camera.zoom()) as f32,
        );
        let radius = (12.0 * camera.zoom() as f32).clamp(4.0, 16.0);
        let shape = Path::rounded_rectangle(top_left, size, radius.into());
        let accent = accent_color(node.accent, palette);

        frame.fill(&shape, palette.background.strong.color);
        frame.stroke(&shape, Stroke::default().with_color(accent).with_width(2.0));
        frame.fill_rectangle(
            Point::new(top_left.x, top_left.y + radius),
            Size::new(
                (5.0 * camera.zoom() as f32).clamp(2.0, 6.0),
                size.height - radius * 2.0,
            ),
            accent,
        );

        let padding = (18.0 * camera.zoom() as f32).clamp(8.0, 20.0);
        frame.fill_text(canvas::Text {
            content: node.title.to_owned(),
            position: Point::new(top_left.x + padding, top_left.y + padding),
            color: palette.background.strong.text,
            size: Pixels((18.0 * camera.zoom() as f32).clamp(11.0, 20.0)),
            ..canvas::Text::default()
        });
        frame.fill_text(canvas::Text {
            content: node.subtitle.to_owned(),
            position: Point::new(top_left.x + padding, top_left.y + padding * 2.5),
            color: palette.secondary.base.color,
            size: Pixels((13.0 * camera.zoom() as f32).clamp(9.0, 15.0)),
            ..canvas::Text::default()
        });
    }
}

fn visible_nodes(nodes: &[CanvasNode], visible: WorldRect) -> impl Iterator<Item = &CanvasNode> {
    nodes
        .iter()
        .filter(move |node| visible.intersects(node.bounds))
}

fn grid_step(zoom: f64) -> f64 {
    let mut step = BASE_GRID_STEP;
    while step * zoom < MIN_GRID_PIXELS {
        step *= 2.0;
    }
    step
}

fn accent_color(accent: Accent, palette: &palette::Extended) -> Color {
    match accent {
        Accent::Primary => palette.primary.base.color,
        Accent::Success => palette.success.base.color,
        Accent::Warning => palette.warning.base.color,
    }
}

#[cfg(test)]
mod tests {
    use super::{CanvasNode, REPRESENTATIVE_NODES, grid_step, visible_nodes};
    use crate::canvas::WorldRect;

    fn titles(nodes: impl Iterator<Item = &'static CanvasNode>) -> Vec<&'static str> {
        nodes.map(|node| node.title).collect()
    }

    #[test]
    fn off_screen_nodes_are_culled_before_drawing() {
        let visible = WorldRect::new(-100.0, -100.0, 300.0, 200.0);

        assert_eq!(
            titles(visible_nodes(&REPRESENTATIVE_NODES, visible)),
            vec!["Builder"]
        );
    }

    #[test]
    fn grid_spacing_never_becomes_visually_dense() {
        assert_eq!(grid_step(4.0), 40.0);
        assert_eq!(grid_step(1.0), 40.0);
        assert_eq!(grid_step(0.5), 80.0);
        assert_eq!(grid_step(0.25), 160.0);
    }
}
