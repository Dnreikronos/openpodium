use iced::event::Status;
use iced::keyboard::{self, Key, key::Named};
use iced::{Event, Subscription, Task, event};
use openpodium::domain::{CanvasNodeContent, NodeId, NodeTarget};
use openpodium::navigation::{
    CommandId, ContentTarget, SearchTarget, Shortcut, index_workspace, traverse_connections,
    traverse_nodes,
};
use openpodium::timeline;

use crate::navigation_panel::{self, Item};

use super::{CanvasAction, Message, OpenPodium, active_workspace_id, now};

const KEYBOARD_ZOOM_FACTOR: f64 = 1.2;
const KEYBOARD_PAN_PIXELS: f64 = 80.0;

#[derive(Debug, Clone, Copy)]
pub(super) enum NavigationKey {
    Up,
    Down,
    Enter,
    Escape,
    Tab { reverse: bool },
    Other,
}

pub(super) fn subscription() -> Subscription<Message> {
    event::listen_with(|event, status, _window| {
        let Event::Keyboard(keyboard::Event::KeyPressed { key, modifiers, .. }) = event else {
            return None;
        };
        let navigation_key = match key.as_ref() {
            Key::Named(Named::ArrowUp) => NavigationKey::Up,
            Key::Named(Named::ArrowDown) => NavigationKey::Down,
            Key::Named(Named::Enter) => NavigationKey::Enter,
            Key::Named(Named::Escape) => NavigationKey::Escape,
            Key::Named(Named::Tab) => NavigationKey::Tab {
                reverse: modifiers.shift(),
            },
            _ => NavigationKey::Other,
        };
        Some(Message::NavigationKey {
            navigation_key,
            shortcut: navigation_panel::shortcut_from_key(&key, modifiers),
            status,
        })
    })
}

pub(super) fn mark_all_stale(state: &mut OpenPodium) {
    let ids = state
        .workspaces
        .as_ref()
        .into_iter()
        .flat_map(|workspaces| {
            workspaces
                .recent_workspaces()
                .map(|workspace| workspace.id())
        })
        .collect::<Vec<_>>();
    state.navigation_ui.stale.extend(ids);
}

pub(super) fn tick(state: &mut OpenPodium) -> Task<Message> {
    state.navigation_ui.refresh_ticks = state.navigation_ui.refresh_ticks.wrapping_add(1);
    for (workspace_id, revision) in &state.timeline_high_watermarks {
        if state.navigation_ui.indexed_revisions.get(workspace_id) != Some(&Some(*revision)) {
            state.navigation_ui.mark_stale(*workspace_id);
        }
    }
    if state.navigation_ui.refresh_ticks.is_multiple_of(50)
        && let Some(workspace_id) = active_workspace_id(state)
    {
        state.navigation_ui.mark_stale(workspace_id);
    }
    if state.navigation_ui.busy.is_some() {
        return Task::none();
    }
    let Some(workspace_id) = state.navigation_ui.stale.pop_first() else {
        return Task::none();
    };
    let Some(workspace) = state
        .workspaces
        .as_ref()
        .and_then(|workspaces| workspaces.workspace(workspace_id))
        .cloned()
    else {
        state.navigation_ui.index.remove_workspace(workspace_id);
        return Task::none();
    };
    let revision = state.timeline_high_watermarks.get(&workspace_id).copied();
    state.navigation_ui.busy = Some(workspace_id);
    Task::perform(
        async move {
            tokio::task::spawn_blocking(move || index_workspace(&workspace))
                .await
                .map_err(|error| format!("Search index worker failed: {error}"))
        },
        move |documents| {
            Message::Navigation(navigation_panel::Message::IndexBuilt {
                workspace_id,
                revision,
                documents,
            })
        },
    )
}

pub(super) fn update(state: &mut OpenPodium, message: navigation_panel::Message) -> Task<Message> {
    match message {
        navigation_panel::Message::Open => {
            state.navigation_ui.open = true;
            state.navigation_ui.selected = 0;
            return Task::batch([
                iced::widget::operation::focus(navigation_panel::INPUT_ID),
                schedule_search(state),
            ]);
        }
        navigation_panel::Message::Close => close(state),
        navigation_panel::Message::QueryChanged(query) => {
            state.navigation_ui.query = query;
            state.navigation_ui.selected = 0;
            state.navigation_ui.rebinding = None;
            return schedule_search(state);
        }
        navigation_panel::Message::MoveSelection(forward) => {
            let count =
                navigation_panel::items(&state.navigation_ui, &state.command_registry).len();
            state.navigation_ui.move_selection(forward, count);
        }
        navigation_panel::Message::ActivateSelection => {
            let item = navigation_panel::items(&state.navigation_ui, &state.command_registry)
                .get(state.navigation_ui.selected)
                .cloned();
            if let Some(item) = item {
                return activate(state, item);
            }
        }
        navigation_panel::Message::Activate(item) => return activate(state, item),
        navigation_panel::Message::BeginRebind(command) => {
            state.navigation_ui.rebinding = Some(command);
            state.navigation_ui.binding_draft = state
                .command_registry
                .binding(command)
                .map_or_else(String::new, Shortcut::storage_value);
        }
        navigation_panel::Message::BindingChanged(value) => {
            state.navigation_ui.binding_draft = value
        }
        navigation_panel::Message::SaveBinding => return save_binding(state, false),
        navigation_panel::Message::Unbind => return save_binding(state, true),
        navigation_panel::Message::IndexBuilt {
            workspace_id,
            revision,
            documents,
        } => {
            state.navigation_ui.busy = None;
            let documents = match documents {
                Ok(documents) => documents,
                Err(error) => {
                    state.notice = Some(error);
                    return Task::none();
                }
            };
            state
                .navigation_ui
                .index
                .replace_workspace(workspace_id, documents);
            state
                .navigation_ui
                .indexed_revisions
                .insert(workspace_id, revision);
            if state.timeline_high_watermarks.get(&workspace_id).copied() != revision {
                state.navigation_ui.mark_stale(workspace_id);
            }
            if state.navigation_ui.open {
                return schedule_search(state);
            }
        }
        navigation_panel::Message::SearchCompleted {
            generation,
            results,
        } => {
            if generation == state.navigation_ui.search_generation {
                state.navigation_ui.search_results = match results {
                    Ok(results) => results,
                    Err(error) => {
                        state.notice = Some(error);
                        Vec::new()
                    }
                };
                let count =
                    navigation_panel::items(&state.navigation_ui, &state.command_registry).len();
                state.navigation_ui.selected = if count == 0 {
                    0
                } else {
                    state.navigation_ui.selected.min(count - 1)
                };
            }
        }
    }
    Task::none()
}

pub(super) fn handle_key(
    state: &mut OpenPodium,
    navigation_key: NavigationKey,
    shortcut: Option<Shortcut>,
    status: Status,
) -> Task<Message> {
    if state.navigation_ui.open {
        return match navigation_key {
            NavigationKey::Up => update(state, navigation_panel::Message::MoveSelection(false)),
            NavigationKey::Down => update(state, navigation_panel::Message::MoveSelection(true)),
            NavigationKey::Enter => update(state, navigation_panel::Message::ActivateSelection),
            NavigationKey::Escape => update(state, navigation_panel::Message::Close),
            NavigationKey::Tab { reverse } => {
                if reverse {
                    iced::widget::operation::focus_previous()
                } else {
                    iced::widget::operation::focus_next()
                }
            }
            NavigationKey::Other => Task::none(),
        };
    }
    if let NavigationKey::Tab { reverse } = navigation_key {
        if status == Status::Ignored
            && state.focused_terminal.is_none()
            && state.focused_portal.is_none()
        {
            return if reverse {
                iced::widget::operation::focus_previous()
            } else {
                iced::widget::operation::focus_next()
            };
        }
        return Task::none();
    }
    let Some(shortcut) = shortcut else {
        return Task::none();
    };
    let Some(command) = state.command_registry.command_for(&shortcut) else {
        return Task::none();
    };
    if status == Status::Captured
        && !matches!(command, CommandId::OpenPalette | CommandId::FocusCanvas)
    {
        return Task::none();
    }
    execute_command(state, command)
}

fn activate(state: &mut OpenPodium, item: Item) -> Task<Message> {
    close(state);
    match item {
        Item::Command(command) => execute_command(state, command),
        Item::Search(result) => navigate_to_search_target(state, result.document.target),
    }
}

fn close(state: &mut OpenPodium) {
    state.navigation_ui.open = false;
    state.navigation_ui.rebinding = None;
}

fn save_binding(state: &mut OpenPodium, unbind: bool) -> Task<Message> {
    let Some(command) = state.navigation_ui.rebinding else {
        return Task::none();
    };
    let shortcut = if unbind {
        None
    } else {
        match Shortcut::parse(&state.navigation_ui.binding_draft) {
            Ok(shortcut) => Some(shortcut),
            Err(error) => {
                state.notice = Some(error.to_string());
                return Task::none();
            }
        }
    };
    let previous = state.command_registry.binding(command).cloned();
    if let Err(error) = state.command_registry.rebind(command, shortcut.clone()) {
        state.notice = Some(error.to_string());
        return Task::none();
    }
    let result = state
        .workspaces
        .as_mut()
        .ok_or_else(|| "workspace storage is unavailable".to_owned())
        .and_then(|workspaces| {
            workspaces
                .store_shortcut(
                    command.as_str(),
                    shortcut.as_ref().map(Shortcut::storage_value).as_deref(),
                )
                .map_err(|error| error.to_string())
        });
    match result {
        Ok(()) => {
            state.navigation_ui.rebinding = None;
            state.notice = Some(format!("Shortcut updated for {}", command.label()));
        }
        Err(error) => {
            let _ = state.command_registry.rebind(command, previous);
            state.notice = Some(error);
        }
    }
    Task::none()
}

fn execute_command(state: &mut OpenPodium, command: CommandId) -> Task<Message> {
    match command {
        CommandId::OpenPalette => return update(state, navigation_panel::Message::Open),
        CommandId::NextWorkspace => return cycle_workspace(state, true),
        CommandId::PreviousWorkspace => return cycle_workspace(state, false),
        CommandId::NextAttention => navigate_attention(state, true),
        CommandId::PreviousAttention => navigate_attention(state, false),
        CommandId::NextNode => navigate_node(state, true),
        CommandId::PreviousNode => navigate_node(state, false),
        CommandId::NextConnection => navigate_connection(state, true),
        CommandId::PreviousConnection => navigate_connection(state, false),
        CommandId::PanLeft => state.camera = state.camera.pan_by_screen(KEYBOARD_PAN_PIXELS, 0.0),
        CommandId::PanRight => state.camera = state.camera.pan_by_screen(-KEYBOARD_PAN_PIXELS, 0.0),
        CommandId::PanUp => state.camera = state.camera.pan_by_screen(0.0, KEYBOARD_PAN_PIXELS),
        CommandId::PanDown => state.camera = state.camera.pan_by_screen(0.0, -KEYBOARD_PAN_PIXELS),
        CommandId::ZoomIn => state.camera = state.camera.zoom_centered(KEYBOARD_ZOOM_FACTOR),
        CommandId::ZoomOut => state.camera = state.camera.zoom_centered(1.0 / KEYBOARD_ZOOM_FACTOR),
        CommandId::ResetZoom => state.camera = crate::canvas::Camera::default(),
        CommandId::FocusContent => {
            let task = focus_content(state);
            state.canvas_revision = state.canvas_revision.wrapping_add(1);
            return task;
        }
        CommandId::FocusCanvas => state.focused_terminal = None,
        CommandId::Undo => super::apply_canvas_action(state, CanvasAction::Undo),
        CommandId::Redo => super::apply_canvas_action(state, CanvasAction::Redo),
        CommandId::Duplicate => super::apply_canvas_action(state, CanvasAction::Duplicate),
        CommandId::Delete => super::apply_canvas_action(state, CanvasAction::Remove),
        CommandId::Copy => return super::copy_canvas_fragment(state),
        CommandId::Paste => {
            return iced::clipboard::read().map(Message::CanvasFragmentRead);
        }
    }
    state.canvas_revision = state.canvas_revision.wrapping_add(1);
    Task::none()
}

fn cycle_workspace(state: &OpenPodium, forward: bool) -> Task<Message> {
    let Some(workspaces) = state.workspaces.as_ref() else {
        return Task::none();
    };
    let ids = workspaces
        .recent_workspaces()
        .map(|workspace| workspace.id())
        .collect::<Vec<_>>();
    let Some(next) = cycle(&ids, workspaces.active_workspace_id(), forward) else {
        return Task::none();
    };
    Task::done(Message::SwitchWorkspace(next))
}

fn navigate_attention(state: &mut OpenPodium, forward: bool) {
    let Some(workspaces) = state.workspaces.as_ref() else {
        return;
    };
    let targets = workspaces
        .recent_workspaces()
        .flat_map(|workspace| {
            timeline::task_summaries(workspace)
                .into_iter()
                .filter(|task| task.needs_attention())
                .filter_map(|task| {
                    let agent_id = task.assignee()?;
                    let node_id = workspace
                        .all_canvas_layout()
                        .nodes()
                        .iter()
                        .find(|node| node.reference() == Some(NodeTarget::Agent(agent_id)))
                        .map(|node| node.id())?;
                    Some((
                        workspace.id(),
                        task.id(),
                        SearchTarget {
                            workspace_id: workspace.id(),
                            floor_id: workspace.floors().node_floors.get(&node_id).copied(),
                            node_id: Some(node_id),
                            content: Some(ContentTarget::Task(task.id())),
                        },
                    ))
                })
        })
        .collect::<Vec<_>>();
    if targets.is_empty() {
        state.notice = Some("No agents need attention".to_owned());
        return;
    }
    let current = active_workspace_id(state).and_then(|workspace_id| {
        state
            .timeline_ui
            .selected_task(workspace_id)
            .map(|task_id| (workspace_id, task_id))
    });
    let keys = targets
        .iter()
        .map(|(workspace_id, task_id, _)| (*workspace_id, *task_id))
        .collect::<Vec<_>>();
    let selected = cycle(&keys, current, forward).and_then(|key| {
        targets
            .into_iter()
            .find(|(workspace_id, task_id, _)| (*workspace_id, *task_id) == key)
    });
    if let Some((_, _, target)) = selected {
        let _ = navigate_to_search_target(state, target);
    }
}

fn navigate_node(state: &mut OpenPodium, forward: bool) {
    let Some(layout) = super::current_canvas(state) else {
        return;
    };
    let current = (state.canvas_selection.len() == 1).then(|| state.canvas_selection[0]);
    if let Some(node) = traverse_nodes(&layout, current, forward) {
        select_and_center(state, node);
    }
}

fn navigate_connection(state: &mut OpenPodium, forward: bool) {
    let Some(layout) = super::current_canvas(state) else {
        return;
    };
    let Some(current) = (state.canvas_selection.len() == 1).then(|| state.canvas_selection[0])
    else {
        return;
    };
    if let Some(node) = traverse_connections(&layout, current, None, forward) {
        select_and_center(state, node);
    }
}

fn focus_content(state: &mut OpenPodium) -> Task<Message> {
    let Some(node_id) = (state.canvas_selection.len() == 1).then(|| state.canvas_selection[0])
    else {
        return Task::none();
    };
    let selected = state
        .workspaces
        .as_ref()
        .and_then(|workspaces| workspaces.active_workspace())
        .and_then(|workspace| {
            workspace
                .node(node_id)
                .map(|node| (workspace.id(), node.content().clone()))
        });
    match selected {
        Some((_, CanvasNodeContent::Reference(NodeTarget::Agent(_)))) => {
            state.focused_terminal = Some(node_id);
        }
        Some((workspace_id, CanvasNodeContent::Reference(NodeTarget::Task(task_id)))) => {
            state.timeline_ui.select_task(workspace_id, Some(task_id));
        }
        Some((_, CanvasNodeContent::Note { .. } | CanvasNodeContent::Text { .. })) => {
            crate::app::context_nodes::selection_changed(state);
            return iced::widget::operation::focus(crate::app::context_nodes::EDITOR_ID);
        }
        Some((_, _)) | None => {}
    }
    Task::none()
}

pub(super) fn navigate_to_search_target(
    state: &mut OpenPodium,
    target: SearchTarget,
) -> Task<Message> {
    let workspace_changed = active_workspace_id(state) != Some(target.workspace_id);
    if workspace_changed {
        let result = state
            .workspaces
            .as_mut()
            .ok_or_else(|| "workspace storage is unavailable".to_owned())
            .and_then(|workspaces| {
                workspaces
                    .switch(target.workspace_id, now())
                    .map_err(|error| error.to_string())
            });
        if let Err(error) = result {
            state.notice = Some(error);
            return Task::none();
        }
        state.reset_canvas_session();
        state.load_active_settings();
    }
    let floor_changed = state
        .workspaces
        .as_ref()
        .and_then(|workspaces| workspaces.active_workspace())
        .is_some_and(|workspace| workspace.floors().active != target.floor_id);
    if floor_changed {
        let result = state
            .workspaces
            .as_mut()
            .expect("the target workspace is active")
            .switch_floor(target.workspace_id, target.floor_id, now());
        if let Err(error) = result {
            state.notice = Some(error);
            return Task::none();
        }
        state.reset_canvas_session();
    }
    if let Some(node_id) = target.node_id {
        select_and_center(state, node_id);
    }
    match target.content {
        Some(ContentTarget::Task(task_id)) => {
            state
                .timeline_ui
                .select_task(target.workspace_id, Some(task_id));
        }
        Some(ContentTarget::ProjectPath(path)) => {
            state.context_path = path.as_str().to_owned();
            state.notice = Some(format!("Selected project path {path}"));
        }
        Some(ContentTarget::Chat {
            agent_id,
            thread_id,
            message_id,
            character_offset,
        }) => {
            let scroll = state
                .workspaces
                .as_ref()
                .and_then(|workspaces| workspaces.workspace(target.workspace_id))
                .and_then(|workspace| {
                    state
                        .chat_ui
                        .reveal_message(workspace, agent_id, thread_id, message_id)
                })
                .unwrap_or_else(Task::none)
                .map(Message::Chat);
            state.notice = Some(format!(
                "Opened matching message near character {character_offset}"
            ));
            return scroll;
        }
        Some(ContentTarget::Note { character_offset })
        | Some(ContentTarget::Text { character_offset }) => {
            crate::app::context_nodes::selection_changed(state);
            state.notice = Some(format!(
                "Opened matching content near character {character_offset}"
            ));
            return crate::app::context_nodes::reveal_offset(state, character_offset);
        }
        None => state.notice = None,
    }
    Task::none()
}

fn schedule_search(state: &mut OpenPodium) -> Task<Message> {
    state.navigation_ui.search_generation = state.navigation_ui.search_generation.wrapping_add(1);
    let generation = state.navigation_ui.search_generation;
    let query = state.navigation_ui.query.clone();
    if query.is_empty() || query.starts_with('>') {
        state.navigation_ui.search_results.clear();
        return Task::none();
    }
    let index = state.navigation_ui.index.clone();
    Task::perform(
        async move {
            tokio::task::spawn_blocking(move || index.search(&query, 30))
                .await
                .map_err(|error| format!("Search query worker failed: {error}"))
        },
        move |results| {
            Message::Navigation(navigation_panel::Message::SearchCompleted {
                generation,
                results,
            })
        },
    )
}

fn select_and_center(state: &mut OpenPodium, node_id: NodeId) {
    let center = state
        .workspaces
        .as_ref()
        .and_then(|workspaces| workspaces.active_workspace())
        .and_then(|workspace| workspace.node(node_id))
        .map(|node| {
            (
                f64::from(node.position().x() + node.size().width() / 2.0),
                f64::from(node.position().y() + node.size().height() / 2.0),
            )
        });
    state.canvas_selection = vec![node_id];
    state.focused_terminal = None;
    crate::app::context_nodes::selection_changed(state);
    if let Some((x, y)) = center {
        state.camera = state.camera.center_on(x, y);
    }
    state.canvas_revision = state.canvas_revision.wrapping_add(1);
}

fn cycle<T: Copy + PartialEq>(values: &[T], current: Option<T>, forward: bool) -> Option<T> {
    if values.is_empty() {
        return None;
    }
    let index = current.and_then(|current| values.iter().position(|value| *value == current));
    Some(match (index, forward) {
        (Some(index), true) => values[(index + 1) % values.len()],
        (Some(0), false) | (None, false) => *values.last().expect("values is not empty"),
        (Some(index), false) => values[index - 1],
        (None, true) => values[0],
    })
}
