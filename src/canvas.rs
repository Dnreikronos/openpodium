#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Camera {
    zoom: f32,
}

impl Camera {
    pub(crate) fn zoom_percent(self) -> u16 {
        (self.zoom * 100.0).round() as u16
    }
}

impl Default for Camera {
    fn default() -> Self {
        Self { zoom: 1.0 }
    }
}

#[cfg(test)]
mod tests {
    use super::Camera;

    #[test]
    fn camera_starts_at_one_hundred_percent_zoom() {
        assert_eq!(Camera::default().zoom_percent(), 100);
    }
}
