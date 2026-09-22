use std::collections::BTreeMap;

use iced::widget::canvas::Program;
use openpodium::domain::{
    AgentId, CanvasPoint, CanvasSize, Name, NodeTarget, Workspace, WorkspaceId,
};
use openpodium::localization::{Locale, Localizer};

use super::*;

fn surface(mouse_reporting: bool, zoom: f64) -> Surface {
    let node_id = NodeId::new(1);
    let node = Node::new(
        node_id,
        NodeTarget::Agent(AgentId::new(1)),
        CanvasPoint::new(-180.0, -130.0).unwrap(),
        CanvasSize::new(360.0, 260.0).unwrap(),
    );
    let workspace = Workspace::new(WorkspaceId::new(1), Name::new("Scroll test").unwrap());
    let mut terminal = terminal::View::offline(terminal::GridSize::for_node(360.0, 260.0));
    terminal.mode.mouse_reporting = mouse_reporting;
    Surface {
        camera: Camera::default().zoom_centered(zoom),
        document: CanvasDocument::new(
            &workspace,
            CanvasLayout::new(vec![node], vec![], vec![]),
            BTreeMap::from([(node_id, terminal)]),
            &Localizer::new(Locale::EnUs),
        ),
        selection: vec![],
        focused_terminal: None,
        focused_portal: None,
        connection_mode: ConnectionMode::Off,
        application_shortcuts: vec![],
        revision: 0,
    }
}

fn scroll(
    surface: &Surface,
    state: &mut State,
    position: Point,
    delta: mouse::ScrollDelta,
) -> Option<Message> {
    let (message, _, status) = surface
        .update(
            state,
            &canvas::Event::Mouse(mouse::Event::WheelScrolled { delta }),
            Rectangle::new(Point::ORIGIN, Size::new(1_000.0, 800.0)),
            mouse::Cursor::Available(position),
        )
        .unwrap()
        .into_inner();
    assert_eq!(status, iced::event::Status::Captured);
    message
}

#[test]
fn trackpad_scrolls_unfocused_terminal_at_every_zoom_without_moving_board() {
    for zoom in [0.5, 1.0, 2.0] {
        let surface = surface(false, zoom);
        let mut state = State::default();
        for (y, expected) in [(32.0, 2), (-48.0, -3)] {
            assert!(matches!(
                scroll(&surface, &mut state, Point::new(500.0, 400.0),
                    mouse::ScrollDelta::Pixels { x: 8.0, y }),
                Some(Message::TerminalScrolled { node_id, lines })
                    if node_id == NodeId::new(1) && lines == expected
            ));
        }
    }
}

#[test]
fn small_and_horizontal_trackpad_deltas_are_captured_and_accumulated() {
    let surface = surface(false, 1.0);
    let mut state = State::default();
    let position = Point::new(500.0, 400.0);
    for _ in 0..7 {
        assert!(
            scroll(
                &surface,
                &mut state,
                position,
                mouse::ScrollDelta::Pixels { x: 0.0, y: 2.0 }
            )
            .is_none()
        );
    }
    assert!(
        scroll(
            &surface,
            &mut state,
            position,
            mouse::ScrollDelta::Pixels { x: 24.0, y: 0.0 }
        )
        .is_none()
    );
    assert!(matches!(
        scroll(
            &surface,
            &mut state,
            position,
            mouse::ScrollDelta::Pixels { x: 0.0, y: 2.0 }
        ),
        Some(Message::TerminalScrolled { lines: 1, .. })
    ));
}

#[test]
fn mouse_reporting_receives_each_wheel_step_in_both_directions() {
    let surface = surface(true, 1.0);
    let position = Point::new(500.0, 400.0);
    let bounds = Rectangle::new(Point::ORIGIN, Size::new(1_000.0, 800.0));
    let (_, row, column, _) = surface.terminal_cell_at(position, bounds).unwrap();
    for (y, upward) in [(1.0, true), (-1.0, false)] {
        let message = scroll(
            &surface,
            &mut State::default(),
            position,
            mouse::ScrollDelta::Lines { x: 0.0, y },
        );
        assert!(
            matches!(message, Some(Message::TerminalInput { node_id, bytes })
            if node_id == NodeId::new(1)
                && bytes == terminal::encode_mouse_wheel(row, column, upward).repeat(3))
        );
    }
}

#[test]
fn terminal_padding_and_header_do_not_leak_scroll_to_board() {
    let surface = surface(false, 1.0);
    for position in [Point::new(322.0, 400.0), Point::new(500.0, 275.0)] {
        assert!(matches!(
            scroll(
                &surface,
                &mut State::default(),
                position,
                mouse::ScrollDelta::Pixels { x: 0.0, y: 16.0 }
            ),
            Some(Message::TerminalScrolled { lines: 1, .. })
        ));
    }
}

#[test]
fn background_keeps_pan_and_zoom_and_clears_terminal_remainder() {
    let surface = surface(false, 1.0);
    let mut state = State {
        terminal_scroll: Some((NodeId::new(1), 0.75)),
        ..State::default()
    };
    let position = Point::new(20.0, 20.0);
    let message = scroll(
        &surface,
        &mut state,
        position,
        mouse::ScrollDelta::Pixels { x: 18.0, y: 24.0 },
    );
    assert!(matches!(message, Some(Message::CameraChanged(camera))
        if camera == surface.camera.pan_by_screen(18.0, 24.0)));
    assert!(state.terminal_scroll.is_none());
    let message = scroll(
        &surface,
        &mut state,
        position,
        mouse::ScrollDelta::Lines { x: 0.0, y: 1.0 },
    );
    assert!(matches!(message, Some(Message::CameraChanged(camera))
        if camera.zoom() > surface.camera.zoom()));
}
