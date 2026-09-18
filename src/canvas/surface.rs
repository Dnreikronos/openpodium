use iced::keyboard::{self, Key, Modifiers, key::Named};
use iced::mouse;
use iced::widget::canvas::{self, Action};
use iced::{Element, Fill, Point, Rectangle, Renderer, Theme};

use super::{Camera, ScreenPoint, ViewportSize, scene};

const KEYBOARD_PAN_PIXELS: f64 = 80.0;
const KEYBOARD_ZOOM_FACTOR: f64 = 1.2;
const LINE_SCROLL_PIXELS: f64 = 48.0;
const LINE_ZOOM_SENSITIVITY: f64 = 0.18 / LINE_SCROLL_PIXELS;
const PIXEL_ZOOM_SENSITIVITY: f64 = 0.003;

#[derive(Debug, Clone, Copy)]
pub(crate) enum Message {
    CameraChanged(Camera),
}

pub(crate) fn view(camera: Camera) -> Element<'static, Message> {
    canvas::Canvas::new(Surface { camera })
        .width(Fill)
        .height(Fill)
        .into()
}

#[derive(Debug, Clone, Copy)]
struct Surface {
    camera: Camera,
}

#[derive(Debug, Default)]
struct State {
    geometry: canvas::Cache,
    drag: Option<Drag>,
    modifiers: Modifiers,
}

#[derive(Debug, Clone, Copy)]
struct Drag {
    button: mouse::Button,
    last_position: Point,
}

impl canvas::Program<Message> for Surface {
    type State = State;

    fn update(
        &self,
        state: &mut State,
        event: &canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<Action<Message>> {
        match event {
            canvas::Event::Keyboard(keyboard::Event::ModifiersChanged(modifiers)) => {
                state.modifiers = *modifiers;
                None
            }
            canvas::Event::Keyboard(keyboard::Event::KeyPressed { key, modifiers, .. })
                if cursor.is_over(bounds) =>
            {
                state.modifiers = *modifiers;
                self.handle_key(state, key, bounds)
            }
            canvas::Event::Mouse(mouse::Event::ButtonPressed(button))
                if matches!(button, mouse::Button::Left | mouse::Button::Middle) =>
            {
                let position = cursor.position_in(bounds)?;
                state.drag = Some(Drag {
                    button: *button,
                    last_position: position,
                });
                Some(Action::capture())
            }
            canvas::Event::Mouse(mouse::Event::CursorMoved { position })
                if state.drag.is_some() =>
            {
                let position = Point::new(position.x - bounds.x, position.y - bounds.y);
                let drag = state.drag.as_mut().expect("drag was checked above");
                let delta = position - drag.last_position;
                drag.last_position = position;
                self.publish_camera(
                    state,
                    self.camera
                        .pan_by_screen(f64::from(delta.x), f64::from(delta.y)),
                )
            }
            canvas::Event::Mouse(mouse::Event::ButtonReleased(button))
                if state.drag.is_some_and(|drag| drag.button == *button) =>
            {
                state.drag = None;
                Some(Action::capture())
            }
            canvas::Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                let anchor = cursor.position_in(bounds)?;
                let (x, y, zoom_sensitivity) = scroll_delta(*delta);
                let camera = if state.modifiers.command() || state.modifiers.control() {
                    self.camera.zoom_at(
                        (y * zoom_sensitivity).exp(),
                        ScreenPoint::new(f64::from(anchor.x), f64::from(anchor.y)),
                        viewport(bounds),
                    )
                } else {
                    self.camera.pan_by_screen(x, y)
                };

                self.publish_camera(state, camera)
            }
            _ => None,
        }
    }

    fn draw(
        &self,
        state: &State,
        renderer: &Renderer,
        theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let viewport = viewport(bounds);
        let geometry = state.geometry.draw(renderer, bounds.size(), |frame| {
            scene::draw(frame, self.camera, viewport, theme);
        });

        vec![geometry]
    }

    fn mouse_interaction(
        &self,
        state: &State,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        if state.drag.is_some() {
            mouse::Interaction::Grabbing
        } else if cursor.is_over(bounds) {
            mouse::Interaction::Grab
        } else {
            mouse::Interaction::default()
        }
    }
}

impl Surface {
    fn handle_key(self, state: &State, key: &Key, bounds: Rectangle) -> Option<Action<Message>> {
        let viewport = viewport(bounds);
        let center = ScreenPoint::new(viewport.width / 2.0, viewport.height / 2.0);
        let camera = match key.as_ref() {
            Key::Named(Named::ArrowLeft) => self.camera.pan_by_screen(KEYBOARD_PAN_PIXELS, 0.0),
            Key::Named(Named::ArrowRight) => self.camera.pan_by_screen(-KEYBOARD_PAN_PIXELS, 0.0),
            Key::Named(Named::ArrowUp) => self.camera.pan_by_screen(0.0, KEYBOARD_PAN_PIXELS),
            Key::Named(Named::ArrowDown) => self.camera.pan_by_screen(0.0, -KEYBOARD_PAN_PIXELS),
            Key::Character("+" | "=") => {
                self.camera.zoom_at(KEYBOARD_ZOOM_FACTOR, center, viewport)
            }
            Key::Character("-" | "_") => {
                self.camera
                    .zoom_at(1.0 / KEYBOARD_ZOOM_FACTOR, center, viewport)
            }
            Key::Character("0") => Camera::default(),
            _ => return None,
        };

        self.publish_camera(state, camera)
    }

    fn publish_camera(self, state: &State, camera: Camera) -> Option<Action<Message>> {
        if camera == self.camera {
            return Some(Action::capture());
        }

        state.geometry.clear();
        Some(Action::publish(Message::CameraChanged(camera)).and_capture())
    }
}

fn viewport(bounds: Rectangle) -> ViewportSize {
    ViewportSize::new(f64::from(bounds.width), f64::from(bounds.height))
}

fn scroll_delta(delta: mouse::ScrollDelta) -> (f64, f64, f64) {
    match delta {
        mouse::ScrollDelta::Lines { x, y } => (
            f64::from(x) * LINE_SCROLL_PIXELS,
            f64::from(y) * LINE_SCROLL_PIXELS,
            LINE_ZOOM_SENSITIVITY,
        ),
        mouse::ScrollDelta::Pixels { x, y } => (f64::from(x), f64::from(y), PIXEL_ZOOM_SENSITIVITY),
    }
}
