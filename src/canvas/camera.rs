const MIN_ZOOM: f64 = 0.25;
const MAX_ZOOM: f64 = 4.0;

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct WorldPoint {
    pub(crate) x: f64,
    pub(crate) y: f64,
}

impl WorldPoint {
    pub(crate) const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct ScreenPoint {
    pub(crate) x: f64,
    pub(crate) y: f64,
}

impl ScreenPoint {
    pub(crate) const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct ViewportSize {
    pub(crate) width: f64,
    pub(crate) height: f64,
}

impl ViewportSize {
    pub(crate) const fn new(width: f64, height: f64) -> Self {
        Self { width, height }
    }

    fn center(self) -> ScreenPoint {
        ScreenPoint::new(self.width / 2.0, self.height / 2.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct WorldRect {
    pub(crate) x: f64,
    pub(crate) y: f64,
    pub(crate) width: f64,
    pub(crate) height: f64,
}

impl WorldRect {
    pub(crate) const fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub(crate) fn intersects(self, other: Self) -> bool {
        self.x <= other.x + other.width
            && self.x + self.width >= other.x
            && self.y <= other.y + other.height
            && self.y + self.height >= other.y
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Camera {
    position: WorldPoint,
    zoom: f64,
}

impl Camera {
    pub(crate) fn position(self) -> WorldPoint {
        self.position
    }

    pub(crate) fn zoom(self) -> f64 {
        self.zoom
    }

    pub(crate) fn zoom_percent(self) -> u16 {
        (self.zoom * 100.0).round() as u16
    }

    pub(crate) fn world_to_screen(self, point: WorldPoint, viewport: ViewportSize) -> ScreenPoint {
        let center = viewport.center();

        ScreenPoint::new(
            center.x + (point.x - self.position.x) * self.zoom,
            center.y + (point.y - self.position.y) * self.zoom,
        )
    }

    pub(crate) fn screen_to_world(self, point: ScreenPoint, viewport: ViewportSize) -> WorldPoint {
        let center = viewport.center();

        WorldPoint::new(
            self.position.x + (point.x - center.x) / self.zoom,
            self.position.y + (point.y - center.y) / self.zoom,
        )
    }

    pub(crate) fn pan_by_screen(self, delta_x: f64, delta_y: f64) -> Self {
        if !delta_x.is_finite() || !delta_y.is_finite() {
            return self;
        }

        Self {
            position: WorldPoint::new(
                self.position.x - delta_x / self.zoom,
                self.position.y - delta_y / self.zoom,
            ),
            ..self
        }
    }

    pub(crate) fn zoom_at(self, factor: f64, anchor: ScreenPoint, viewport: ViewportSize) -> Self {
        if !factor.is_finite() || factor <= 0.0 {
            return self;
        }

        let zoom = (self.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        if zoom == self.zoom {
            return self;
        }

        let anchor_world = self.screen_to_world(anchor, viewport);
        let viewport_center = viewport.center();

        Self {
            position: WorldPoint::new(
                anchor_world.x - (anchor.x - viewport_center.x) / zoom,
                anchor_world.y - (anchor.y - viewport_center.y) / zoom,
            ),
            zoom,
        }
    }

    pub(crate) fn visible_world_rect(self, viewport: ViewportSize) -> WorldRect {
        let top_left = self.screen_to_world(ScreenPoint::new(0.0, 0.0), viewport);
        let bottom_right =
            self.screen_to_world(ScreenPoint::new(viewport.width, viewport.height), viewport);

        WorldRect::new(
            top_left.x,
            top_left.y,
            bottom_right.x - top_left.x,
            bottom_right.y - top_left.y,
        )
    }
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            position: WorldPoint::default(),
            zoom: 1.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Camera, MAX_ZOOM, MIN_ZOOM, ScreenPoint, ViewportSize, WorldPoint, WorldRect};

    const VIEWPORT: ViewportSize = ViewportSize::new(1200.0, 800.0);
    const EPSILON: f64 = 1.0e-9;

    fn assert_close(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() < EPSILON,
            "expected {expected}, got {actual}"
        );
    }

    #[test]
    fn camera_starts_centered_at_one_hundred_percent_zoom() {
        let camera = Camera::default();

        assert_eq!(camera.position(), WorldPoint::default());
        assert_eq!(camera.zoom_percent(), 100);
        assert_eq!(
            camera.world_to_screen(WorldPoint::default(), VIEWPORT),
            ScreenPoint::new(600.0, 400.0)
        );
    }

    #[test]
    fn world_and_screen_transforms_round_trip_without_mutating_world_coordinates() {
        let original = WorldPoint::new(318.25, -741.75);
        let mut camera = Camera::default();

        for _ in 0..500 {
            camera = camera.pan_by_screen(13.5, -7.25).zoom_at(
                1.002,
                ScreenPoint::new(917.0, 263.0),
                VIEWPORT,
            );
            let restored =
                camera.screen_to_world(camera.world_to_screen(original, VIEWPORT), VIEWPORT);

            assert_close(restored.x, original.x);
            assert_close(restored.y, original.y);
        }

        assert_eq!(original, WorldPoint::new(318.25, -741.75));
    }

    #[test]
    fn zoom_keeps_the_world_point_under_the_pointer_fixed() {
        let pointer = ScreenPoint::new(975.0, 185.0);
        let camera = Camera::default().pan_by_screen(-240.0, 90.0);
        let world_before = camera.screen_to_world(pointer, VIEWPORT);
        let zoomed = camera.zoom_at(1.75, pointer, VIEWPORT);
        let screen_after = zoomed.world_to_screen(world_before, VIEWPORT);

        assert_close(screen_after.x, pointer.x);
        assert_close(screen_after.y, pointer.y);
    }

    #[test]
    fn zoom_is_clamped_to_deterministic_bounds() {
        let center = ScreenPoint::new(600.0, 400.0);
        let minimum = Camera::default().zoom_at(0.0001, center, VIEWPORT);
        let maximum = Camera::default().zoom_at(10_000.0, center, VIEWPORT);

        assert_eq!(minimum.zoom(), MIN_ZOOM);
        assert_eq!(maximum.zoom(), MAX_ZOOM);
        assert_eq!(minimum.zoom_percent(), 25);
        assert_eq!(maximum.zoom_percent(), 400);
    }

    #[test]
    fn visible_world_rect_tracks_camera_position_and_zoom() {
        let camera = Camera::default().pan_by_screen(-200.0, 100.0).zoom_at(
            2.0,
            ScreenPoint::new(600.0, 400.0),
            VIEWPORT,
        );

        assert_eq!(
            camera.visible_world_rect(VIEWPORT),
            WorldRect::new(-100.0, -300.0, 600.0, 400.0)
        );
    }

    #[test]
    fn rectangle_intersection_includes_edges() {
        let viewport = WorldRect::new(-100.0, -100.0, 200.0, 200.0);

        assert!(viewport.intersects(WorldRect::new(100.0, 20.0, 50.0, 50.0)));
        assert!(!viewport.intersects(WorldRect::new(100.1, 20.0, 50.0, 50.0)));
    }
}
