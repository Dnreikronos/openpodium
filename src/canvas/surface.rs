use std::cell::Cell;

use iced::keyboard::{self, Key, Modifiers, key::Named};
use iced::mouse;
use iced::widget::canvas::{self, Action};
use iced::{Element, Fill, Point, Rectangle, Renderer, Theme};
use openpodium::domain::{CanvasLayout, Node, NodeId};

use super::{Camera, CanvasDocument, ScreenPoint, ViewportSize, editor, scene};

const KEYBOARD_PAN_PIXELS: f64 = 80.0;
const KEYBOARD_ZOOM_FACTOR: f64 = 1.2;
const LINE_SCROLL_PIXELS: f64 = 48.0;
const LINE_ZOOM_SENSITIVITY: f64 = 0.18 / LINE_SCROLL_PIXELS;
const PIXEL_ZOOM_SENSITIVITY: f64 = 0.003;
const RESIZE_HANDLE_PIXELS: f32 = 18.0;

#[derive(Debug, Clone)]
pub(crate) enum Message {
    CameraChanged(Camera),
    SelectionChanged(Vec<NodeId>),
    PreviewLayout(CanvasLayout),
    CommitLayout {
        before: CanvasLayout,
        after: CanvasLayout,
    },
    UndoRequested,
    RedoRequested,
    DeleteRequested,
    DuplicateRequested,
}

pub(crate) fn view(
    camera: Camera,
    document: CanvasDocument,
    selection: Vec<NodeId>,
    revision: u64,
) -> Element<'static, Message> {
    canvas::Canvas::new(Surface {
        camera,
        document,
        selection,
        revision,
    })
    .width(Fill)
    .height(Fill)
    .into()
}

#[derive(Debug, Clone)]
struct Surface {
    camera: Camera,
    document: CanvasDocument,
    selection: Vec<NodeId>,
    revision: u64,
}

#[derive(Debug, Default)]
struct State {
    geometry: canvas::Cache,
    drag: Option<Drag>,
    modifiers: Modifiers,
    rendered_revision: Cell<u64>,
}

#[derive(Debug, Clone)]
enum Drag {
    Pan {
        button: mouse::Button,
        last_position: Point,
    },
    Move {
        button: mouse::Button,
        start_position: Point,
        before: CanvasLayout,
        selection: Vec<NodeId>,
        preview: CanvasLayout,
    },
    Resize {
        button: mouse::Button,
        start_position: Point,
        before: CanvasLayout,
        node_id: NodeId,
        initial_width: f32,
        initial_height: f32,
        preview: CanvasLayout,
    },
}

impl Drag {
    fn button(&self) -> mouse::Button {
        match self {
            Self::Pan { button, .. } | Self::Move { button, .. } | Self::Resize { button, .. } => {
                *button
            }
        }
    }
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
                self.handle_key(state, key, *modifiers, bounds)
            }
            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Middle)) => {
                let position = cursor.position_in(bounds)?;
                state.drag = Some(Drag::Pan {
                    button: mouse::Button::Middle,
                    last_position: position,
                });
                Some(Action::capture())
            }
            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let position = cursor.position_in(bounds)?;
                self.begin_left_drag(state, position, bounds)
            }
            canvas::Event::Mouse(mouse::Event::CursorMoved { position })
                if state.drag.is_some() =>
            {
                let position = Point::new(position.x - bounds.x, position.y - bounds.y);
                self.update_drag(state, position)
            }
            canvas::Event::Mouse(mouse::Event::ButtonReleased(button))
                if state
                    .drag
                    .as_ref()
                    .is_some_and(|drag| drag.button() == *button) =>
            {
                let drag = state.drag.take().expect("drag was checked above");
                match drag {
                    Drag::Move {
                        before, preview, ..
                    }
                    | Drag::Resize {
                        before, preview, ..
                    } if before != preview => Some(
                        Action::publish(Message::CommitLayout {
                            before,
                            after: preview,
                        })
                        .and_capture(),
                    ),
                    _ => Some(Action::capture()),
                }
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
        if state.rendered_revision.get() != self.revision {
            state.geometry.clear();
            state.rendered_revision.set(self.revision);
        }
        let viewport = viewport(bounds);
        let geometry = state.geometry.draw(renderer, bounds.size(), |frame| {
            scene::draw(
                frame,
                self.camera,
                viewport,
                &self.document,
                &self.selection,
                theme,
            );
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
            return mouse::Interaction::Grabbing;
        }
        let Some(position) = cursor.position_in(bounds) else {
            return mouse::Interaction::default();
        };
        if self.resize_hit(position, bounds).is_some() {
            mouse::Interaction::Pointer
        } else if self.hit_node(position, bounds).is_some() {
            mouse::Interaction::Grab
        } else {
            mouse::Interaction::default()
        }
    }
}

impl Surface {
    fn begin_left_drag(
        &self,
        state: &mut State,
        position: Point,
        bounds: Rectangle,
    ) -> Option<Action<Message>> {
        let before = self.document.layout().clone();
        if let Some(node) = self.resize_hit(position, bounds) {
            state.drag = Some(Drag::Resize {
                button: mouse::Button::Left,
                start_position: position,
                before: before.clone(),
                node_id: node.id(),
                initial_width: node.size().width(),
                initial_height: node.size().height(),
                preview: before,
            });
            return Some(Action::capture());
        }
        if let Some(node) = self.hit_node(position, bounds) {
            let selection = editor::selection_for_click(
                &before,
                &self.selection,
                node.id(),
                state.modifiers.shift(),
            );
            state.drag = Some(Drag::Move {
                button: mouse::Button::Left,
                start_position: position,
                before: before.clone(),
                selection: selection.clone(),
                preview: before,
            });
            state.geometry.clear();
            return Some(Action::publish(Message::SelectionChanged(selection)).and_capture());
        }

        state.drag = Some(Drag::Pan {
            button: mouse::Button::Left,
            last_position: position,
        });
        state.geometry.clear();
        Some(Action::publish(Message::SelectionChanged(Vec::new())).and_capture())
    }

    fn update_drag(&self, state: &mut State, position: Point) -> Option<Action<Message>> {
        let message = match state.drag.as_mut()? {
            Drag::Pan { last_position, .. } => {
                let delta = position - *last_position;
                *last_position = position;
                return self.publish_camera(
                    state,
                    self.camera
                        .pan_by_screen(f64::from(delta.x), f64::from(delta.y)),
                );
            }
            Drag::Move {
                start_position,
                before,
                selection,
                preview,
                ..
            } => {
                let delta = position - *start_position;
                *preview = editor::move_nodes(
                    before,
                    selection,
                    delta.x / self.camera.zoom() as f32,
                    delta.y / self.camera.zoom() as f32,
                );
                Message::PreviewLayout(preview.clone())
            }
            Drag::Resize {
                start_position,
                before,
                node_id,
                initial_width,
                initial_height,
                preview,
                ..
            } => {
                let delta = position - *start_position;
                *preview = editor::resize_node(
                    before,
                    *node_id,
                    *initial_width + delta.x / self.camera.zoom() as f32,
                    *initial_height + delta.y / self.camera.zoom() as f32,
                );
                Message::PreviewLayout(preview.clone())
            }
        };
        state.geometry.clear();
        Some(Action::publish(message).and_capture())
    }

    fn handle_key(
        &self,
        state: &State,
        key: &Key,
        modifiers: Modifiers,
        bounds: Rectangle,
    ) -> Option<Action<Message>> {
        let command = modifiers.command() || modifiers.control();
        match key.as_ref() {
            Key::Character("z") if command && modifiers.shift() => {
                return Some(Action::publish(Message::RedoRequested).and_capture());
            }
            Key::Character("z") if command => {
                return Some(Action::publish(Message::UndoRequested).and_capture());
            }
            Key::Character("y") if command => {
                return Some(Action::publish(Message::RedoRequested).and_capture());
            }
            Key::Character("d") if command => {
                return Some(Action::publish(Message::DuplicateRequested).and_capture());
            }
            Key::Named(Named::Delete | Named::Backspace) if !self.selection.is_empty() => {
                return Some(Action::publish(Message::DeleteRequested).and_capture());
            }
            _ => {}
        }

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

    fn hit_node(&self, position: Point, bounds: Rectangle) -> Option<&Node> {
        let world = self.camera.screen_to_world(
            ScreenPoint::new(f64::from(position.x), f64::from(position.y)),
            viewport(bounds),
        );
        self.document
            .layout()
            .nodes()
            .iter()
            .filter(|node| {
                world.x >= f64::from(node.position().x())
                    && world.x <= f64::from(node.position().x() + node.size().width())
                    && world.y >= f64::from(node.position().y())
                    && world.y <= f64::from(node.position().y() + node.size().height())
            })
            .max_by_key(|node| (node.z_index(), node.id()))
    }

    fn resize_hit(&self, position: Point, bounds: Rectangle) -> Option<&Node> {
        self.document
            .layout()
            .nodes()
            .iter()
            .filter(|node| self.selection.contains(&node.id()))
            .filter(|node| {
                let bottom_right = self.camera.world_to_screen(
                    super::WorldPoint::new(
                        f64::from(node.position().x() + node.size().width()),
                        f64::from(node.position().y() + node.size().height()),
                    ),
                    viewport(bounds),
                );
                (f64::from(position.x) - bottom_right.x).abs() <= f64::from(RESIZE_HANDLE_PIXELS)
                    && (f64::from(position.y) - bottom_right.y).abs()
                        <= f64::from(RESIZE_HANDLE_PIXELS)
            })
            .max_by_key(|node| (node.z_index(), node.id()))
    }

    fn publish_camera(&self, state: &State, camera: Camera) -> Option<Action<Message>> {
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
