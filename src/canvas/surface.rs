use std::cell::Cell;

use iced::advanced::widget::{self, Tree};
use iced::advanced::{
    Clipboard, InputMethod, Layout, Shell, Widget, input_method, layout, renderer,
};
use iced::keyboard::key::Named;
use iced::keyboard::{self, Key, Modifiers};
use iced::mouse;
use iced::widget::canvas::{self, Action};
use iced::{Element, Fill, Length, Point, Rectangle, Renderer, Size, Theme};
use openpodium::domain::{CanvasLayout, CanvasNodeContent, Node, NodeId};
use openpodium::navigation::Shortcut;
use openpodium::portal::{PortalFrameTransform, PortalKeyInput, PortalPoint, PortalRect};

use crate::terminal::{self, BODY_PADDING, CELL_HEIGHT, CELL_WIDTH, HEADER_HEIGHT};

use super::{Camera, CanvasDocument, ScreenPoint, ViewportSize, WorldPoint, editor, scene};

const LINE_SCROLL_PIXELS: f64 = 48.0;
const LINE_ZOOM_SENSITIVITY: f64 = 0.18 / LINE_SCROLL_PIXELS;
const PIXEL_ZOOM_SENSITIVITY: f64 = 0.003;
const RESIZE_HANDLE_PIXELS: f32 = 18.0;

#[cfg(test)]
mod scroll_tests;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum ConnectionMode {
    #[default]
    Off,
    PickSource,
    PickTarget(NodeId),
}

pub(crate) struct Interaction {
    pub selection: Vec<NodeId>,
    pub focused_terminal: Option<NodeId>,
    pub focused_portal: Option<NodeId>,
    pub connection_mode: ConnectionMode,
}

#[derive(Debug, Clone)]
pub(crate) enum Message {
    CameraChanged(Camera),
    SelectionChanged(Vec<NodeId>),
    EditRequested(NodeId),
    RenameRequested(NodeId),
    ConnectionSourceSelected(NodeId),
    ConnectNodes {
        source: NodeId,
        target: NodeId,
    },
    CancelConnection,
    PreviewLayout(CanvasLayout),
    CommitLayout {
        before: CanvasLayout,
        after: CanvasLayout,
    },
    CopyRequested,
    PasteRequested,
    TerminalFocused(Option<NodeId>),
    TerminalInput {
        node_id: NodeId,
        bytes: Vec<u8>,
    },
    TerminalPasteRequested(NodeId),
    TerminalCopyRequested(NodeId),
    TerminalScrolled {
        node_id: NodeId,
        lines: i32,
    },
    TerminalSelectionStarted {
        node_id: NodeId,
        row: usize,
        column: usize,
        right_side: bool,
    },
    TerminalSelectionUpdated {
        node_id: NodeId,
        row: usize,
        column: usize,
        right_side: bool,
    },
    PortalClicked {
        node_id: NodeId,
        observation_revision: u64,
        x: u32,
        y: u32,
    },
    PortalScrolled {
        node_id: NodeId,
        observation_revision: u64,
        x: u32,
        y: u32,
        delta_x: i32,
        delta_y: i32,
    },
    PortalText {
        node_id: NodeId,
        observation_revision: u64,
        text: String,
    },
    PortalKey {
        node_id: NodeId,
        observation_revision: u64,
        key: PortalKeyInput,
        shift: bool,
    },
}

pub(crate) fn view(
    camera: Camera,
    document: CanvasDocument,
    interaction: Interaction,
    application_shortcuts: Vec<Shortcut>,
    revision: u64,
) -> Element<'static, Message> {
    TerminalCanvas::element(Surface {
        camera,
        document,
        selection: interaction.selection,
        focused_terminal: interaction.focused_terminal,
        focused_portal: interaction.focused_portal,
        connection_mode: interaction.connection_mode,
        application_shortcuts,
        revision,
    })
}

struct TerminalCanvas {
    inner: canvas::Canvas<Surface, Message>,
    surface: Surface,
}

impl TerminalCanvas {
    fn element(surface: Surface) -> Element<'static, Message> {
        Element::new(Self {
            inner: canvas::Canvas::new(surface.clone())
                .width(Fill)
                .height(Fill),
            surface,
        })
    }
}

impl Widget<Message, Theme, Renderer> for TerminalCanvas {
    fn tag(&self) -> widget::tree::Tag {
        self.inner.tag()
    }

    fn state(&self) -> widget::tree::State {
        self.inner.state()
    }

    fn size(&self) -> Size<Length> {
        self.inner.size()
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        self.inner.layout(tree, renderer, limits)
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &canvas::Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        self.inner.update(
            tree, event, layout, cursor, renderer, clipboard, shell, viewport,
        );
        let state = tree.state.downcast_ref::<State>();
        let input_method = if (self.surface.focused_terminal.is_some()
            || self.surface.focused_portal.is_some())
            && cursor.is_over(layout.bounds())
        {
            InputMethod::Enabled {
                cursor: self.surface.ime_cursor(layout.bounds()),
                purpose: input_method::Purpose::Normal,
                preedit: (!state.preedit.is_empty()).then_some(input_method::Preedit {
                    content: state.preedit.as_str(),
                    selection: None,
                    text_size: None,
                }),
            }
        } else {
            InputMethod::Disabled
        };
        shell.request_input_method(&input_method);
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        self.inner
            .mouse_interaction(tree, layout, cursor, viewport, renderer)
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        self.inner
            .draw(tree, renderer, theme, style, layout, cursor, viewport);
    }
}

#[derive(Debug, Clone)]
struct Surface {
    camera: Camera,
    document: CanvasDocument,
    selection: Vec<NodeId>,
    focused_terminal: Option<NodeId>,
    focused_portal: Option<NodeId>,
    connection_mode: ConnectionMode,
    application_shortcuts: Vec<Shortcut>,
    revision: u64,
}

#[derive(Debug, Default)]
struct State {
    geometry: canvas::Cache,
    drag: Option<Drag>,
    modifiers: Modifiers,
    rendered_revision: Cell<u64>,
    preedit: String,
    connection_cursor: Option<Point>,
    connection_press: Option<(NodeId, ConnectionMode)>,
    terminal_scroll: Option<(NodeId, f64)>,
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
    TerminalSelection {
        button: mouse::Button,
        node_id: NodeId,
    },
}

impl Drag {
    fn button(&self) -> mouse::Button {
        match self {
            Self::Pan { button, .. }
            | Self::Move { button, .. }
            | Self::Resize { button, .. }
            | Self::TerminalSelection { button, .. } => *button,
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
            canvas::Event::Mouse(mouse::Event::CursorMoved { position })
                if self.connection_mode != ConnectionMode::Off =>
            {
                state.connection_cursor =
                    Some(Point::new(position.x - bounds.x, position.y - bounds.y));
                Some(Action::request_redraw())
            }
            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))
                if self.connection_mode != ConnectionMode::Off =>
            {
                let position = cursor.position_in(bounds)?;
                state.drag = None;
                state.connection_cursor = Some(position);
                let Some(node) = self.hit_node(position, bounds) else {
                    return Some(Action::publish(Message::CancelConnection).and_capture());
                };
                state.connection_press = Some((node.id(), self.connection_mode));
                Some(
                    Action::publish(match self.connection_mode {
                        ConnectionMode::PickSource => Message::ConnectionSourceSelected(node.id()),
                        ConnectionMode::PickTarget(source) if source != node.id() => {
                            Message::ConnectNodes {
                                source,
                                target: node.id(),
                            }
                        }
                        _ => return Some(Action::capture()),
                    })
                    .and_capture(),
                )
            }
            canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
                if self.connection_mode != ConnectionMode::Off =>
            {
                let pressed = state.connection_press.take();
                if let ConnectionMode::PickTarget(source) = self.connection_mode
                    && let Some((pressed_node, started_in)) = pressed
                    && pressed_node == source
                    && let Some(target) = cursor
                        .position_in(bounds)
                        .and_then(|position| self.hit_node(position, bounds))
                    && (target.id() != source || started_in == ConnectionMode::PickTarget(source))
                {
                    return Some(
                        Action::publish(Message::ConnectNodes {
                            source,
                            target: target.id(),
                        })
                        .and_capture(),
                    );
                }
                Some(Action::capture())
            }
            canvas::Event::Keyboard(keyboard::Event::ModifiersChanged(modifiers)) => {
                state.modifiers = *modifiers;
                None
            }
            canvas::Event::Keyboard(keyboard::Event::KeyPressed {
                key,
                text,
                modifiers,
                ..
            }) if cursor.is_over(bounds) => {
                state.modifiers = *modifiers;
                if let Some(node_id) = self.focused_terminal {
                    self.handle_terminal_key(state, node_id, key, text.as_deref(), *modifiers)
                } else if let Some(node_id) = self.focused_portal {
                    self.handle_portal_key(node_id, key, text.as_deref(), *modifiers)
                } else {
                    None
                }
            }
            canvas::Event::InputMethod(input_method::Event::Preedit(text, _))
                if (self.focused_terminal.is_some() || self.focused_portal.is_some())
                    && cursor.is_over(bounds) =>
            {
                state.preedit.clone_from(text);
                state.geometry.clear();
                Some(Action::request_redraw().and_capture())
            }
            canvas::Event::InputMethod(input_method::Event::Commit(text))
                if (self.focused_terminal.is_some() || self.focused_portal.is_some())
                    && cursor.is_over(bounds) =>
            {
                state.preedit.clear();
                state.geometry.clear();
                if let Some(node_id) = self.focused_terminal {
                    Some(
                        Action::publish(Message::TerminalInput {
                            node_id,
                            bytes: text.as_bytes().to_vec(),
                        })
                        .and_capture(),
                    )
                } else {
                    self.portal_text_action(self.focused_portal?, text.clone())
                }
            }
            canvas::Event::InputMethod(input_method::Event::Closed)
                if self.focused_terminal.is_some() || self.focused_portal.is_some() =>
            {
                state.preedit.clear();
                state.geometry.clear();
                Some(Action::request_redraw())
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
                self.update_drag(state, position, bounds)
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
                    Drag::TerminalSelection { .. } => Some(Action::capture()),
                    _ => Some(Action::capture()),
                }
            }
            canvas::Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                let anchor = cursor.position_in(bounds)?;
                let (x, y, zoom_sensitivity) = scroll_delta(*delta);
                if state.modifiers.alt()
                    && let Some((node_id, observation_revision, point)) =
                        self.portal_point_at(anchor, bounds)
                {
                    return Some(
                        Action::publish(Message::PortalScrolled {
                            node_id,
                            observation_revision,
                            x: point.x().floor() as u32,
                            y: point.y().floor() as u32,
                            delta_x: saturating_i32(-x),
                            delta_y: saturating_i32(-y),
                        })
                        .and_capture(),
                    );
                }
                if let Some(node) = self.hit_node(anchor, bounds)
                    && let Some(terminal) = self.document.terminal(node.id())
                {
                    let node_id = node.id();
                    let remainder = match state.terminal_scroll {
                        Some((previous, remainder)) if previous == node_id => remainder,
                        _ => 0.0,
                    };
                    let total = remainder + y / f64::from(CELL_HEIGHT);
                    let lines = total.trunc() as i32;
                    state.terminal_scroll = Some((node_id, total - f64::from(lines)));
                    if lines != 0 {
                        if terminal.mode.mouse_reporting {
                            let (_, row, column, _) = self
                                .terminal_cell_at(anchor, bounds)
                                .unwrap_or((node_id, 0, 0, false));
                            return Some(
                                Action::publish(Message::TerminalInput {
                                    node_id,
                                    bytes: terminal::encode_mouse_wheel(row, column, lines > 0)
                                        .repeat(lines.unsigned_abs().min(100) as usize),
                                })
                                .and_capture(),
                            );
                        }
                        return Some(
                            Action::publish(Message::TerminalScrolled { node_id, lines })
                                .and_capture(),
                        );
                    }
                    return Some(Action::capture());
                }
                state.terminal_scroll = None;
                let camera = match (*delta, state.modifiers.alt()) {
                    (_, true) | (mouse::ScrollDelta::Pixels { .. }, false) => {
                        self.camera.pan_by_screen(x, y)
                    }
                    (mouse::ScrollDelta::Lines { .. }, false) => {
                        zoom_camera(self.camera, y, zoom_sensitivity, anchor, bounds)
                    }
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
        cursor: mouse::Cursor,
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
                scene::TerminalOverlay {
                    focused: self.focused_terminal,
                    preedit: &state.preedit,
                },
                theme,
            );
        });
        let mut layers = vec![geometry];
        if let ConnectionMode::PickTarget(source) = self.connection_mode
            && let Some(pointer) = cursor.position_in(bounds).or(state.connection_cursor)
        {
            let mut preview = canvas::Frame::new(renderer, bounds.size());
            scene::draw_connection_preview(
                &mut preview,
                self.camera,
                viewport,
                &self.document,
                scene::ConnectionPreview {
                    source,
                    pointer,
                    target: self.hit_node(pointer, bounds).map(Node::id),
                },
                theme.extended_palette(),
            );
            layers.push(preview.into_geometry());
        }
        layers
    }

    fn mouse_interaction(
        &self,
        state: &State,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        if self.connection_mode != ConnectionMode::Off && cursor.is_over(bounds) {
            return mouse::Interaction::Crosshair;
        }
        if state.drag.is_some() {
            return mouse::Interaction::Grabbing;
        }
        let Some(position) = cursor.position_in(bounds) else {
            return mouse::Interaction::default();
        };
        if self.resize_hit(position, bounds).is_some()
            || self.rename_hit(position, bounds).is_some()
            || self.portal_point_at(position, bounds).is_some()
        {
            mouse::Interaction::Pointer
        } else if self.terminal_cell_at(position, bounds).is_some()
            || self.editable_body_at(position, bounds).is_some()
        {
            mouse::Interaction::Text
        } else if self.hit_node(position, bounds).is_some() {
            mouse::Interaction::Grab
        } else {
            mouse::Interaction::default()
        }
    }
}

impl Surface {
    fn ime_cursor(&self, bounds: Rectangle) -> Rectangle {
        let Some(node_id) = self.focused_terminal else {
            return Rectangle::new(bounds.position(), Size::new(1.0, CELL_HEIGHT));
        };
        let Some(node) = self
            .document
            .layout()
            .nodes()
            .iter()
            .find(|node| node.id() == node_id)
        else {
            return Rectangle::new(bounds.position(), Size::new(1.0, CELL_HEIGHT));
        };
        let Some(terminal) = self.document.terminal(node_id) else {
            return Rectangle::new(bounds.position(), Size::new(1.0, CELL_HEIGHT));
        };
        let top_left = self.camera.world_to_screen(
            WorldPoint::new(
                f64::from(node.position().x()),
                f64::from(node.position().y()),
            ),
            viewport(bounds),
        );
        let zoom = self.camera.zoom() as f32;
        let header_height = (HEADER_HEIGHT * zoom).clamp(28.0, 60.0);
        let cursor = terminal.cursor.unwrap_or(terminal::CursorView {
            row: 0,
            column: 0,
            style: terminal::CursorStyle::Block,
        });
        Rectangle::new(
            Point::new(
                bounds.x
                    + top_left.x as f32
                    + (BODY_PADDING + cursor.column as f32 * CELL_WIDTH) * zoom,
                bounds.y
                    + top_left.y as f32
                    + header_height
                    + (BODY_PADDING + cursor.row as f32 * CELL_HEIGHT) * zoom,
            ),
            Size::new((CELL_WIDTH * zoom).max(1.0), (CELL_HEIGHT * zoom).max(1.0)),
        )
    }

    fn begin_left_drag(
        &self,
        state: &mut State,
        position: Point,
        bounds: Rectangle,
    ) -> Option<Action<Message>> {
        if !state.modifiers.shift()
            && let Some(node_id) = self.rename_hit(position, bounds)
        {
            state.drag = None;
            return Some(Action::publish(Message::RenameRequested(node_id)).and_capture());
        }
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
            return Some(Action::publish(Message::TerminalFocused(None)).and_capture());
        }
        if let Some((node_id, observation_revision, point)) = self.portal_point_at(position, bounds)
        {
            state.geometry.clear();
            return Some(
                Action::publish(Message::PortalClicked {
                    node_id,
                    observation_revision,
                    x: point.x().floor() as u32,
                    y: point.y().floor() as u32,
                })
                .and_capture(),
            );
        }
        if let Some(node) = self.hit_node(position, bounds) {
            if !state.modifiers.shift()
                && let Some(node_id) = self.editable_body_at(position, bounds)
            {
                state.drag = None;
                return Some(Action::publish(Message::EditRequested(node_id)).and_capture());
            }
            if let Some((node_id, row, column, right_side)) =
                self.terminal_cell_at(position, bounds)
            {
                state.drag = Some(Drag::TerminalSelection {
                    button: mouse::Button::Left,
                    node_id,
                });
                state.geometry.clear();
                return Some(
                    Action::publish(Message::TerminalSelectionStarted {
                        node_id,
                        row,
                        column,
                        right_side,
                    })
                    .and_capture(),
                );
            }
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

    fn update_drag(
        &self,
        state: &mut State,
        position: Point,
        bounds: Rectangle,
    ) -> Option<Action<Message>> {
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
            Drag::TerminalSelection { node_id, .. } => {
                let (_, row, column, right_side) =
                    self.terminal_cell_for_node(*node_id, position, bounds)?;
                Message::TerminalSelectionUpdated {
                    node_id: *node_id,
                    row,
                    column,
                    right_side,
                }
            }
        };
        state.geometry.clear();
        Some(Action::publish(message).and_capture())
    }

    fn handle_terminal_key(
        &self,
        state: &State,
        node_id: NodeId,
        key: &Key,
        text: Option<&str>,
        modifiers: Modifiers,
    ) -> Option<Action<Message>> {
        if is_application_shortcut(&self.application_shortcuts, key, modifiers) {
            return Some(Action::capture());
        }
        if is_copy_shortcut(key, modifiers) {
            return Some(Action::publish(Message::TerminalCopyRequested(node_id)).and_capture());
        }
        if is_paste_shortcut(key, modifiers) {
            return Some(Action::publish(Message::TerminalPasteRequested(node_id)).and_capture());
        }
        let mode = self.document.terminal(node_id)?.mode;
        let bytes = terminal::encode_key(key, text, modifiers, mode)?;
        state.geometry.clear();
        Some(Action::publish(Message::TerminalInput { node_id, bytes }).and_capture())
    }

    fn handle_portal_key(
        &self,
        node_id: NodeId,
        key: &Key,
        text: Option<&str>,
        modifiers: Modifiers,
    ) -> Option<Action<Message>> {
        if is_application_shortcut(&self.application_shortcuts, key, modifiers) {
            return Some(Action::capture());
        }
        if modifiers.command() || modifiers.control() || modifiers.alt() {
            return None;
        }
        if let Some(key) = portal_key_input(key) {
            let observation_revision = self.document.portal_frame(node_id)?.revision();
            return Some(
                Action::publish(Message::PortalKey {
                    node_id,
                    observation_revision,
                    key,
                    shift: modifiers.shift(),
                })
                .and_capture(),
            );
        }
        self.portal_text_action(node_id, text?.to_owned())
    }

    fn portal_text_action(&self, node_id: NodeId, text: String) -> Option<Action<Message>> {
        if text.is_empty() {
            return Some(Action::capture());
        }
        let observation_revision = self.document.portal_frame(node_id)?.revision();
        Some(
            Action::publish(Message::PortalText {
                node_id,
                observation_revision,
                text,
            })
            .and_capture(),
        )
    }

    fn portal_point_at(
        &self,
        position: Point,
        bounds: Rectangle,
    ) -> Option<(NodeId, u64, PortalPoint)> {
        if self.camera.zoom() < scene::BODY_MIN_ZOOM {
            return None;
        }
        let node = self.hit_node(position, bounds)?;
        let CanvasNodeContent::Portal(config) = node.content() else {
            return None;
        };
        let portal_frame = self.document.portal_frame(node.id())?;
        let top_left = self.camera.world_to_screen(
            WorldPoint::new(
                f64::from(node.position().x()),
                f64::from(node.position().y()),
            ),
            viewport(bounds),
        );
        let zoom = self.camera.zoom() as f32;
        let size = Size::new(node.size().width() * zoom, node.size().height() * zoom);
        let header_height = (HEADER_HEIGHT * zoom).clamp(28.0, 60.0);
        let body_padding = (12.0 * zoom).clamp(7.0, 16.0);
        let body = PortalRect::new(
            top_left.x + f64::from(body_padding),
            top_left.y + f64::from(header_height + body_padding),
            f64::from((size.width - body_padding * 2.0).max(1.0)),
            f64::from((size.height - header_height - body_padding * 2.0).max(1.0)),
        )?;
        let transform = PortalFrameTransform::new(
            body,
            portal_frame.viewport(),
            config.presentation().preserve_aspect_ratio(),
        );
        let point = transform.canvas_to_viewport(PortalPoint::new(
            f64::from(position.x),
            f64::from(position.y),
        ))?;
        let viewport = portal_frame.viewport();
        Some((
            node.id(),
            portal_frame.revision(),
            PortalPoint::new(
                point.x().min(f64::from(viewport.width() - 1)),
                point.y().min(f64::from(viewport.height() - 1)),
            ),
        ))
    }

    fn rename_hit(&self, position: Point, bounds: Rectangle) -> Option<NodeId> {
        let node = self.hit_node(position, bounds)?;
        (self.selection.contains(&node.id())
            && scene::rename_button_bounds(node, self.camera, viewport(bounds))?.contains(position))
        .then_some(node.id())
    }

    fn editable_body_at(&self, position: Point, bounds: Rectangle) -> Option<NodeId> {
        let node = self.hit_node(position, bounds)?;
        if !matches!(
            node.content(),
            CanvasNodeContent::Note { .. } | CanvasNodeContent::Text { .. }
        ) {
            return None;
        }
        let top_left = self.camera.world_to_screen(
            WorldPoint::new(
                f64::from(node.position().x()),
                f64::from(node.position().y()),
            ),
            viewport(bounds),
        );
        let header = (HEADER_HEIGHT * self.camera.zoom() as f32).clamp(28.0, 60.0);
        (position.y >= top_left.y as f32 + header).then_some(node.id())
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
                    WorldPoint::new(
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

    fn terminal_cell_at(
        &self,
        position: Point,
        bounds: Rectangle,
    ) -> Option<(NodeId, usize, usize, bool)> {
        let node = self.hit_node(position, bounds)?;
        self.terminal_cell_for_node(node.id(), position, bounds)
    }

    fn terminal_cell_for_node(
        &self,
        node_id: NodeId,
        position: Point,
        bounds: Rectangle,
    ) -> Option<(NodeId, usize, usize, bool)> {
        let node = self
            .document
            .layout()
            .nodes()
            .iter()
            .find(|node| node.id() == node_id)?;
        let terminal = self.document.terminal(node_id)?;
        let top_left = self.camera.world_to_screen(
            WorldPoint::new(
                f64::from(node.position().x()),
                f64::from(node.position().y()),
            ),
            viewport(bounds),
        );
        let zoom = self.camera.zoom() as f32;
        let header_height = (HEADER_HEIGHT * zoom).clamp(28.0, 60.0);
        let body_x = top_left.x as f32 + BODY_PADDING * zoom;
        let body_y = top_left.y as f32 + header_height + BODY_PADDING * zoom;
        let local_x = position.x - body_x;
        let local_y = position.y - body_y;
        if local_x < 0.0 || local_y < 0.0 {
            return None;
        }
        let cell_width = CELL_WIDTH * zoom;
        let cell_height = CELL_HEIGHT * zoom;
        let column = (local_x / cell_width).floor() as usize;
        let row = (local_y / cell_height).floor() as usize;
        if column >= usize::from(terminal.size.columns) || row >= usize::from(terminal.size.rows) {
            return None;
        }
        let right_side = local_x % cell_width >= cell_width / 2.0;
        Some((node_id, row, column, right_side))
    }

    fn publish_camera(&self, state: &State, camera: Camera) -> Option<Action<Message>> {
        if camera == self.camera {
            return Some(Action::capture());
        }
        state.geometry.clear();
        Some(Action::publish(Message::CameraChanged(camera)).and_capture())
    }
}

#[cfg(target_os = "macos")]
fn is_copy_shortcut(key: &Key, modifiers: Modifiers) -> bool {
    matches!(key.as_ref(), Key::Character("c")) && modifiers.command()
}

#[cfg(not(target_os = "macos"))]
fn is_copy_shortcut(key: &Key, modifiers: Modifiers) -> bool {
    matches!(key.as_ref(), Key::Character("c")) && modifiers.control() && modifiers.shift()
}

#[cfg(target_os = "macos")]
fn is_paste_shortcut(key: &Key, modifiers: Modifiers) -> bool {
    matches!(key.as_ref(), Key::Character("v")) && modifiers.command()
}

#[cfg(not(target_os = "macos"))]
fn is_paste_shortcut(key: &Key, modifiers: Modifiers) -> bool {
    matches!(key.as_ref(), Key::Character("v")) && modifiers.control() && modifiers.shift()
}

fn viewport(bounds: Rectangle) -> ViewportSize {
    ViewportSize::new(f64::from(bounds.width), f64::from(bounds.height))
}

fn is_application_shortcut(shortcuts: &[Shortcut], key: &Key, modifiers: Modifiers) -> bool {
    crate::navigation_panel::shortcut_from_key(key, modifiers)
        .as_ref()
        .is_some_and(|shortcut| shortcuts.contains(shortcut))
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

fn zoom_camera(
    camera: Camera,
    vertical_delta: f64,
    sensitivity: f64,
    anchor: Point,
    bounds: Rectangle,
) -> Camera {
    camera.zoom_at(
        (vertical_delta * sensitivity).exp(),
        ScreenPoint::new(f64::from(anchor.x), f64::from(anchor.y)),
        viewport(bounds),
    )
}

fn saturating_i32(value: f64) -> i32 {
    value
        .round()
        .clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32
}

fn portal_key_input(key: &Key) -> Option<PortalKeyInput> {
    match key.as_ref() {
        Key::Named(Named::Enter) => Some(PortalKeyInput::Enter),
        Key::Named(Named::Tab) => Some(PortalKeyInput::Tab),
        Key::Named(Named::Backspace) => Some(PortalKeyInput::Backspace),
        Key::Named(Named::Delete) => Some(PortalKeyInput::Delete),
        Key::Named(Named::Escape) => Some(PortalKeyInput::Escape),
        Key::Named(Named::ArrowUp) => Some(PortalKeyInput::ArrowUp),
        Key::Named(Named::ArrowDown) => Some(PortalKeyInput::ArrowDown),
        Key::Named(Named::ArrowLeft) => Some(PortalKeyInput::ArrowLeft),
        Key::Named(Named::ArrowRight) => Some(PortalKeyInput::ArrowRight),
        Key::Named(Named::Home) => Some(PortalKeyInput::Home),
        Key::Named(Named::End) => Some(PortalKeyInput::End),
        Key::Named(Named::PageUp) => Some(PortalKeyInput::PageUp),
        Key::Named(Named::PageDown) => Some(PortalKeyInput::PageDown),
        Key::Named(_) | Key::Character(_) | Key::Unidentified => None,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use openpodium::domain::{
        CanvasNodeContent, CanvasPoint, CanvasSize, Name, Node, Workspace, WorkspaceId,
    };
    use openpodium::localization::{Locale, Localizer};
    use openpodium::portal::{PortalConfig, PortalFrame, PortalFrameEncoding, PortalViewport};

    use super::*;

    #[test]
    fn rename_button_uses_header_hit_bounds_at_every_zoom_without_breaking_drag() {
        use openpodium::domain::{Agent, AgentId, DomainCommand, NodeTarget};
        let node_id = NodeId::new(1);
        let mut workspace = Workspace::new(WorkspaceId::new(1), Name::new("Test").unwrap());
        let node = Node::new(
            node_id,
            NodeTarget::Agent(AgentId::new(1)),
            CanvasPoint::new(-180.0, -130.0).unwrap(),
            CanvasSize::new(360.0, 260.0).unwrap(),
        );
        workspace
            .execute(DomainCommand::AddAgentNode {
                agent: Agent::new(AgentId::new(1), Name::new("Codex 9").unwrap(), None),
                node: node.clone(),
            })
            .unwrap();
        let document = CanvasDocument::new(
            &workspace,
            workspace.canvas_layout(),
            BTreeMap::new(),
            &Localizer::new(Locale::EnUs),
        );
        let bounds = Rectangle::new(Point::ORIGIN, Size::new(1_000.0, 800.0));
        for zoom in [0.5, 1.0, 2.0] {
            let camera = Camera::default().zoom_centered(zoom);
            let mut surface = portal_surface(camera, node_id, document.clone());
            surface.focused_portal = None;
            surface.selection = vec![node_id];
            let button = scene::rename_button_bounds(&node, camera, viewport(bounds)).unwrap();
            let mut state = State::default();
            let action = surface
                .begin_left_drag(&mut state, button.center(), bounds)
                .unwrap();
            assert!(
                matches!(action.into_inner().0, Some(Message::RenameRequested(id)) if id == node_id)
            );
            assert!(state.drag.is_none());
            state.modifiers = Modifiers::SHIFT;
            let action = surface
                .begin_left_drag(&mut state, button.center(), bounds)
                .unwrap();
            assert!(matches!(
                action.into_inner().0,
                Some(Message::SelectionChanged(_))
            ));
            assert!(matches!(state.drag, Some(Drag::Move { .. })));
            state.modifiers = Modifiers::empty();
            let top = camera.world_to_screen(WorldPoint::new(-180.0, -130.0), viewport(bounds));
            let action = surface
                .begin_left_drag(
                    &mut state,
                    Point::new(top.x as f32 + 30.0, top.y as f32 + 10.0),
                    bounds,
                )
                .unwrap();
            assert!(matches!(
                action.into_inner().0,
                Some(Message::SelectionChanged(_))
            ));
            assert!(matches!(state.drag, Some(Drag::Move { .. })));
            surface.selection.clear();
            assert_eq!(surface.rename_hit(button.center(), bounds), None);
        }
    }

    fn portal_document(node_id: NodeId) -> CanvasDocument {
        let node = Node::with_content(
            node_id,
            CanvasNodeContent::Portal(PortalConfig::browser("https://example.test").unwrap()),
            CanvasPoint::new(-320.0, -210.0).unwrap(),
            CanvasSize::new(640.0, 420.0).unwrap(),
        );
        let workspace = Workspace::new(WorkspaceId::new(1), Name::new("Test").unwrap());
        CanvasDocument::new(
            &workspace,
            CanvasLayout::new(vec![node], Vec::new(), Vec::new()),
            BTreeMap::new(),
            &Localizer::new(Locale::EnUs),
        )
        .with_portal_frames(BTreeMap::from([(
            node_id,
            PortalFrame::new(
                7,
                PortalViewport::new(1_280, 720).unwrap(),
                PortalFrameEncoding::Png,
                vec![1],
            )
            .unwrap(),
        )]))
    }

    fn portal_surface(camera: Camera, node_id: NodeId, document: CanvasDocument) -> Surface {
        Surface {
            camera,
            document,
            selection: Vec::new(),
            focused_terminal: None,
            focused_portal: Some(node_id),
            connection_mode: ConnectionMode::Off,
            application_shortcuts: Vec::new(),
            revision: 1,
        }
    }

    #[test]
    fn note_body_edits_while_header_shift_click_and_resize_keep_canvas_gestures() {
        let node_id = NodeId::new(1);
        let workspace = Workspace::new(WorkspaceId::new(1), Name::new("Test").unwrap());
        let node = Node::with_content(
            node_id,
            CanvasNodeContent::Note {
                path: openpodium::domain::ProjectPath::new("note.md").unwrap(),
                title: Name::new("Note").unwrap(),
            },
            CanvasPoint::new(-180.0, -130.0).unwrap(),
            CanvasSize::new(360.0, 260.0).unwrap(),
        );
        let document = CanvasDocument::new(
            &workspace,
            CanvasLayout::new(vec![node], vec![], vec![]),
            BTreeMap::new(),
            &Localizer::new(Locale::EnUs),
        );
        let bounds = Rectangle::new(Point::ORIGIN, Size::new(1_000.0, 800.0));
        for zoom in [0.5, 1.0, 2.0] {
            let camera = Camera::default().zoom_centered(zoom);
            let surface = Surface {
                camera,
                document: document.clone(),
                selection: vec![node_id],
                focused_terminal: None,
                focused_portal: None,
                connection_mode: ConnectionMode::Off,
                application_shortcuts: vec![],
                revision: 1,
            };
            let top = camera.world_to_screen(WorldPoint::new(-180.0, -130.0), viewport(bounds));
            let header = Point::new(top.x as f32 + 30.0, top.y as f32 + 10.0);
            let body = Point::new(500.0, 400.0);
            let mut state = State::default();
            let action = surface.begin_left_drag(&mut state, body, bounds).unwrap();
            assert!(
                matches!(action.into_inner().0, Some(Message::EditRequested(id)) if id == node_id)
            );
            assert!(state.drag.is_none());

            let action = surface.begin_left_drag(&mut state, header, bounds).unwrap();
            assert!(matches!(
                action.into_inner().0,
                Some(Message::SelectionChanged(_))
            ));
            assert!(matches!(state.drag, Some(Drag::Move { .. })));

            state.modifiers = Modifiers::SHIFT;
            let action = surface.begin_left_drag(&mut state, body, bounds).unwrap();
            assert!(matches!(
                action.into_inner().0,
                Some(Message::SelectionChanged(_))
            ));
            assert!(matches!(state.drag, Some(Drag::Move { .. })));

            state.modifiers = Modifiers::empty();
            let corner = camera.world_to_screen(WorldPoint::new(180.0, 130.0), viewport(bounds));
            let _ = surface.begin_left_drag(
                &mut state,
                Point::new(corner.x as f32, corner.y as f32),
                bounds,
            );
            assert!(matches!(state.drag, Some(Drag::Resize { .. })));
        }
    }

    #[test]
    fn connection_gestures_pick_click_or_drag_between_cards_at_any_zoom() {
        use iced::widget::canvas::Program;
        let source = NodeId::new(1);
        let target = NodeId::new(2);
        let workspace = Workspace::new(WorkspaceId::new(1), Name::new("Test").unwrap());
        let nodes = [(source, -350.0), (target, 50.0)]
            .into_iter()
            .map(|(id, x)| {
                Node::with_content(
                    id,
                    CanvasNodeContent::Note {
                        path: openpodium::domain::ProjectPath::new(format!("{id}.md")).unwrap(),
                        title: Name::new("Note").unwrap(),
                    },
                    CanvasPoint::new(x, -100.0).unwrap(),
                    CanvasSize::new(240.0, 180.0).unwrap(),
                )
            })
            .collect();
        let document = CanvasDocument::new(
            &workspace,
            CanvasLayout::new(nodes, vec![], vec![]),
            BTreeMap::new(),
            &Localizer::new(Locale::EnUs),
        );
        let bounds = Rectangle::new(Point::ORIGIN, Size::new(1_200.0, 800.0));
        let press = canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left));
        let release = canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left));
        for zoom in [0.5, 1.0, 2.0] {
            let camera = Camera::default().zoom_centered(zoom);
            let cursor = |x| {
                let point = camera.world_to_screen(WorldPoint::new(x, -10.0), viewport(bounds));
                mouse::Cursor::Available(Point::new(point.x as f32, point.y as f32))
            };
            let mut surface = Surface {
                camera,
                document: document.clone(),
                selection: vec![],
                focused_terminal: None,
                focused_portal: None,
                connection_mode: ConnectionMode::PickSource,
                application_shortcuts: vec![],
                revision: 1,
            };
            let mut state = State::default();
            let action = surface
                .update(&mut state, &press, bounds, cursor(-230.0))
                .unwrap();
            assert!(
                matches!(action.into_inner().0, Some(Message::ConnectionSourceSelected(id)) if id == source)
            );
            surface.connection_mode = ConnectionMode::PickTarget(source);
            let action = surface
                .update(&mut state, &release, bounds, cursor(-230.0))
                .unwrap();
            assert!(action.into_inner().0.is_none());
            let _ = surface.update(&mut state, &press, bounds, cursor(-230.0));
            let action = surface
                .update(&mut state, &release, bounds, cursor(-230.0))
                .unwrap();
            assert!(matches!(action.into_inner().0,
                Some(Message::ConnectNodes { source: from, target: to }) if from == source && to == source
            ));
            let action = surface
                .update(&mut state, &press, bounds, cursor(170.0))
                .unwrap();
            assert!(
                matches!(action.into_inner().0, Some(Message::ConnectNodes { source: from, target: to }) if from == source && to == target)
            );
            assert!(state.drag.is_none());

            let _ = surface.update(&mut state, &press, bounds, cursor(-230.0));
            let action = surface
                .update(&mut state, &release, bounds, cursor(170.0))
                .unwrap();
            assert!(
                matches!(action.into_inner().0, Some(Message::ConnectNodes { source: from, target: to }) if from == source && to == target)
            );
            let action = surface
                .update(
                    &mut state,
                    &press,
                    bounds,
                    mouse::Cursor::Available(Point::new(10.0, 10.0)),
                )
                .unwrap();
            assert!(matches!(
                action.into_inner().0,
                Some(Message::CancelConnection)
            ));
        }
    }

    #[test]
    fn portal_pointer_mapping_tracks_canvas_zoom() {
        let node_id = NodeId::new(1);
        let document = portal_document(node_id);
        let bounds = Rectangle::new(Point::ORIGIN, Size::new(1_000.0, 800.0));

        for camera in [Camera::default(), Camera::default().zoom_centered(2.0)] {
            let surface = portal_surface(camera, node_id, document.clone());
            let top_left =
                camera.world_to_screen(WorldPoint::new(-320.0, -210.0), viewport(bounds));
            let zoom = camera.zoom() as f32;
            let header = (HEADER_HEIGHT * zoom).clamp(28.0, 60.0);
            let padding = (12.0 * zoom).clamp(7.0, 16.0);
            let body_height = 420.0 * zoom - header - padding * 2.0;
            let body_center = Point::new(
                top_left.x as f32 + 320.0 * zoom,
                top_left.y as f32 + header + padding + body_height / 2.0,
            );
            let (_, revision, point) = surface.portal_point_at(body_center, bounds).unwrap();
            assert_eq!(revision, 7);
            assert!((point.x() - 640.0).abs() < 0.001);
            assert!((point.y() - 360.0).abs() < 0.001);
        }
    }

    #[test]
    fn a_portal_hidden_by_zoom_takes_no_pointer_input() {
        let node_id = NodeId::new(1);
        let camera = Camera::default().zoom_centered(0.25);
        assert!(camera.zoom() < scene::BODY_MIN_ZOOM);
        let surface = portal_surface(camera, node_id, portal_document(node_id));
        let bounds = Rectangle::new(Point::ORIGIN, Size::new(1_000.0, 800.0));
        let center = Point::new(500.0, 400.0);

        assert!(surface.hit_node(center, bounds).is_some());
        assert_eq!(surface.portal_point_at(center, bounds), None);
    }

    #[test]
    fn terminal_focus_keeps_escape_and_canvas_shortcuts_for_the_pty() {
        #[cfg(target_os = "macos")]
        let (canvas_command, encoded_undo) = (Modifiers::COMMAND, b"z".to_vec());
        #[cfg(not(target_os = "macos"))]
        let (canvas_command, encoded_undo) = (Modifiers::CTRL, vec![0x1a]);

        let release = Shortcut::parse("Primary+Shift+Escape").unwrap();
        assert_ne!(
            crate::navigation_panel::shortcut_from_key(
                &Key::Named(Named::Escape),
                Modifiers::empty()
            ),
            Some(release.clone())
        );
        assert_eq!(
            terminal::encode_key(
                &Key::Named(Named::Escape),
                None,
                Modifiers::empty(),
                terminal::InputMode::default(),
            ),
            Some(vec![0x1b])
        );
        assert_eq!(
            terminal::encode_key(
                &Key::Character("z".into()),
                Some("z"),
                canvas_command,
                terminal::InputMode::default(),
            ),
            Some(encoded_undo)
        );
        assert_eq!(
            crate::navigation_panel::shortcut_from_key(
                &Key::Named(Named::Escape),
                canvas_command | Modifiers::SHIFT
            ),
            Some(release)
        );
    }

    #[test]
    fn focused_embedded_content_releases_zoom_shortcuts_to_the_application() {
        let shortcuts = [
            Shortcut::parse("plus").unwrap(),
            Shortcut::parse("minus").unwrap(),
            Shortcut::parse("0").unwrap(),
        ];

        assert!(is_application_shortcut(
            &shortcuts,
            &Key::Character("+".into()),
            Modifiers::empty(),
        ));
        assert!(is_application_shortcut(
            &shortcuts,
            &Key::Character("-".into()),
            Modifiers::empty(),
        ));
        assert!(is_application_shortcut(
            &shortcuts,
            &Key::Character("0".into()),
            Modifiers::empty(),
        ));
        assert!(!is_application_shortcut(
            &shortcuts,
            &Key::Character("z".into()),
            Modifiers::empty(),
        ));
    }

    #[test]
    fn mouse_wheel_zooms_while_trackpad_scroll_pans_without_modifiers() {
        let bounds = Rectangle::new(Point::ORIGIN, Size::new(1_000.0, 800.0));
        let anchor = Point::new(760.0, 240.0);
        let camera = Camera::default();
        let world_before = camera.screen_to_world(
            ScreenPoint::new(f64::from(anchor.x), f64::from(anchor.y)),
            viewport(bounds),
        );

        let (_, wheel_y, sensitivity) = scroll_delta(mouse::ScrollDelta::Lines { x: 0.0, y: 1.0 });
        let zoomed = zoom_camera(camera, wheel_y, sensitivity, anchor, bounds);
        let screen_after = zoomed.world_to_screen(world_before, viewport(bounds));
        assert!(zoomed.zoom() > camera.zoom());
        assert!((screen_after.x - f64::from(anchor.x)).abs() < 0.001);
        assert!((screen_after.y - f64::from(anchor.y)).abs() < 0.001);

        let (trackpad_x, trackpad_y, _) =
            scroll_delta(mouse::ScrollDelta::Pixels { x: 18.0, y: 24.0 });
        let panned = camera.pan_by_screen(trackpad_x, trackpad_y);
        assert_eq!(panned.zoom(), camera.zoom());
        assert_ne!(panned.position(), camera.position());
    }

    #[test]
    fn portal_named_keys_map_without_exposing_arbitrary_backend_commands() {
        assert_eq!(
            portal_key_input(&Key::Named(Named::Enter)),
            Some(PortalKeyInput::Enter)
        );
        assert_eq!(
            portal_key_input(&Key::Named(Named::ArrowDown)),
            Some(PortalKeyInput::ArrowDown)
        );
        assert_eq!(portal_key_input(&Key::Character("x".into())), None);
    }
}
