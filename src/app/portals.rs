use std::collections::{BTreeMap, BTreeSet};

use iced::widget::{button, column, row, text, text_input};
use iced::{Element, Task};
use openpodium::domain::{
    CanvasNodeContent, CanvasPoint, CanvasSize, DomainCommand, Node, NodeId, NodeTarget, Workspace,
    WorkspaceId,
};
use openpodium::ipc::{PortalControl, PortalScope};
use openpodium::portal::{BrowserBackend, PortalConfig, PortalFrame};

use super::{Message as AppMessage, OpenPodium, canvas_coordinate, next_node_id, now};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct PortalKey {
    workspace_id: WorkspaceId,
    node_id: NodeId,
}

#[derive(Debug, Clone)]
enum Phase {
    Connecting,
    Connected { portal_id: u64, capture_busy: bool },
    Disconnecting { portal_id: u64 },
}

#[derive(Debug, Clone)]
struct LivePortal {
    config: PortalConfig,
    phase: Phase,
}

pub(super) struct UiState {
    url: String,
    sessions: BTreeMap<PortalKey, LivePortal>,
    tick: u64,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            url: "https://".to_owned(),
            sessions: BTreeMap::new(),
            tick: 0,
        }
    }
}

#[derive(Clone)]
pub(super) enum Message {
    UrlChanged(String),
    Add,
    Connect(NodeId),
    Connected {
        key: PortalKey,
        result: Result<u64, String>,
    },
    Disconnect(NodeId),
    Disconnected {
        key: PortalKey,
        portal_id: u64,
        result: Result<(), String>,
    },
    FrameCaptured {
        key: PortalKey,
        portal_id: u64,
        result: Result<Option<PortalFrame>, String>,
    },
}

pub(super) fn update(state: &mut OpenPodium, message: Message) -> Task<AppMessage> {
    match message {
        Message::UrlChanged(url) => {
            state.portal_ui.url = url;
            Task::none()
        }
        Message::Add => add(state),
        Message::Connect(node_id) => connect(state, node_id),
        Message::Connected { key, result } => connected(state, key, result),
        Message::Disconnect(node_id) => disconnect(state, node_id),
        Message::Disconnected {
            key,
            portal_id,
            result,
        } => disconnected(state, key, portal_id, result),
        Message::FrameCaptured {
            key,
            portal_id,
            result,
        } => frame_captured(state, key, portal_id, result),
    }
}

pub(super) fn tick(state: &mut OpenPodium) -> Task<AppMessage> {
    state.portal_ui.tick = state.portal_ui.tick.wrapping_add(1);
    let active = state
        .workspaces
        .as_ref()
        .and_then(|manager| manager.active_workspace())
        .map(|workspace| {
            let configs = workspace
                .canvas_layout()
                .nodes()
                .iter()
                .filter_map(|node| match node.content() {
                    CanvasNodeContent::Portal(config) => Some((node.id(), config.clone())),
                    _ => None,
                })
                .collect::<BTreeMap<_, _>>();
            (workspace.id(), configs)
        });
    let control = state.ipc.as_ref().map(|ipc| ipc.portal_control());
    let mut tasks = Vec::new();
    let keys = state.portal_ui.sessions.keys().copied().collect::<Vec<_>>();

    for key in keys {
        let stale = active.as_ref().is_none_or(|(workspace_id, configs)| {
            *workspace_id != key.workspace_id
                || configs.get(&key.node_id)
                    != state.portal_ui.sessions.get(&key).map(|live| &live.config)
        });
        if stale {
            if let Some(control) = control.clone()
                && let Some(task) = begin_disconnect(state, key, control)
            {
                tasks.push(task);
            }
            continue;
        }

        let portal_id = match state.portal_ui.sessions.get(&key).map(|live| &live.phase) {
            Some(Phase::Connected {
                portal_id,
                capture_busy: false,
            }) => *portal_id,
            _ => continue,
        };
        let frame_rate = state.portal_ui.sessions[&key]
            .config
            .presentation()
            .frame_rate_limit();
        let capture_every_ticks = 10_u64.div_ceil(u64::from(frame_rate)).max(1);
        if !state.portal_ui.tick.is_multiple_of(capture_every_ticks) {
            continue;
        }
        let Some(control) = control.clone() else {
            continue;
        };
        let agents = state
            .workspaces
            .as_ref()
            .and_then(|manager| manager.workspace(key.workspace_id))
            .map(|workspace| connected_agents(workspace, key.node_id))
            .unwrap_or_default();
        if let Some(live) = state.portal_ui.sessions.get_mut(&key) {
            live.phase = Phase::Connected {
                portal_id,
                capture_busy: true,
            };
        }
        tasks.push(Task::perform(
            async move {
                tokio::task::spawn_blocking(move || capture(&control, portal_id, agents))
                    .await
                    .map_err(|error| error.to_string())
                    .and_then(|result| result)
            },
            move |result| {
                AppMessage::Portal(Message::FrameCaptured {
                    key,
                    portal_id,
                    result,
                })
            },
        ));
    }

    Task::batch(tasks)
}

pub(super) fn sync_connections(state: &mut OpenPodium) {
    let Some(control) = state.ipc.as_ref().map(|ipc| ipc.portal_control()) else {
        return;
    };
    let updates = state
        .portal_ui
        .sessions
        .iter()
        .filter_map(|(key, live)| match live.phase {
            Phase::Connected { portal_id, .. } => state
                .workspaces
                .as_ref()
                .and_then(|manager| manager.workspace(key.workspace_id))
                .map(|workspace| (portal_id, connected_agents(workspace, key.node_id))),
            Phase::Connecting | Phase::Disconnecting { .. } => None,
        })
        .collect::<Vec<_>>();
    for (portal_id, agents) in updates {
        if let Err(error) = control.replace_agents(portal_id, agents) {
            state.notice = Some(error.to_string());
        }
    }
}

pub(super) fn shutdown(state: &mut OpenPodium) {
    let Some(control) = state.ipc.as_ref().map(|ipc| ipc.portal_control()) else {
        return;
    };
    let portal_ids = state
        .portal_ui
        .sessions
        .values()
        .filter_map(|live| match live.phase {
            Phase::Connecting => None,
            Phase::Connected { portal_id, .. } | Phase::Disconnecting { portal_id } => {
                Some(portal_id)
            }
        })
        .collect::<Vec<_>>();
    for portal_id in portal_ids {
        let _ = control.unregister(portal_id);
    }
    state.portal_ui.sessions.clear();
}

pub(super) fn creation_view(state: &OpenPodium) -> Element<'_, AppMessage> {
    column![
        text("Browser portals").size(18),
        row![
            text_input("https://example.com", &state.portal_ui.url)
                .on_input(|url| AppMessage::Portal(Message::UrlChanged(url))),
            button("Add portal").on_press(AppMessage::Portal(Message::Add)),
        ]
        .spacing(8),
    ]
    .spacing(8)
    .into()
}

pub(super) fn selected_view(state: &OpenPodium) -> Option<Element<'_, AppMessage>> {
    let (key, config) = selected_portal(state)?;
    let content = column![
        text("Portal").size(18),
        text(config.target().selector()).size(12),
    ]
    .spacing(8);
    let content = match state.portal_ui.sessions.get(&key).map(|live| &live.phase) {
        None => content.push(
            button("Connect browser").on_press(AppMessage::Portal(Message::Connect(key.node_id))),
        ),
        Some(Phase::Connecting) => content.push(text("Connecting isolated Chromium…")),
        Some(Phase::Connected { portal_id, .. }) => content
            .push(text(format!("Connected as portal {portal_id}")))
            .push(
                button("Disconnect browser")
                    .on_press(AppMessage::Portal(Message::Disconnect(key.node_id))),
            ),
        Some(Phase::Disconnecting { .. }) => content.push(text("Disconnecting browser…")),
    };
    Some(content.into())
}

fn add(state: &mut OpenPodium) -> Task<AppMessage> {
    let config = match PortalConfig::browser(state.portal_ui.url.trim()) {
        Ok(config) => config,
        Err(error) => {
            state.notice = Some(error.to_string());
            return Task::none();
        }
    };
    let Some(workspace) = state
        .workspaces
        .as_ref()
        .and_then(|manager| manager.active_workspace())
    else {
        state.notice = Some("Create or select a workspace first".to_owned());
        return Task::none();
    };
    let Some(node_id) = next_node_id(workspace) else {
        state.notice = Some("No more canvas node identifiers are available".to_owned());
        return Task::none();
    };
    let before = workspace.canvas_layout();
    let offset = (before.nodes().len() % 6) as f64 * 40.0;
    let z_index = before
        .nodes()
        .iter()
        .map(Node::z_index)
        .max()
        .unwrap_or(0)
        .saturating_add(1);
    let node = Node::with_content_and_z_index(
        node_id,
        CanvasNodeContent::Portal(config),
        CanvasPoint::new(
            canvas_coordinate(state.camera.position().x - 320.0 + offset),
            canvas_coordinate(state.camera.position().y - 210.0 + offset),
        )
        .expect("clamped camera coordinates are finite"),
        CanvasSize::new(640.0, 420.0).expect("default portal size is valid"),
        z_index,
    );
    let workspace_id = workspace.id();
    match state
        .workspaces
        .as_mut()
        .expect("active workspace came from the manager")
        .execute(workspace_id, DomainCommand::AddNode(node), now())
    {
        Ok(_) => {
            state.canvas_history.record(before);
            state.canvas_selection = vec![node_id];
            state.canvas_preview = None;
            state.canvas_revision = state.canvas_revision.wrapping_add(1);
            state.notice = Some("Browser portal added".to_owned());
        }
        Err(error) => state.notice = Some(error.to_string()),
    }
    Task::none()
}

fn connect(state: &mut OpenPodium, node_id: NodeId) -> Task<AppMessage> {
    let Some(control) = state.ipc.as_ref().map(|ipc| ipc.portal_control()) else {
        state.notice = Some("IPC service is unavailable".to_owned());
        return Task::none();
    };
    let Some(workspace) = state
        .workspaces
        .as_ref()
        .and_then(|manager| manager.active_workspace())
    else {
        return Task::none();
    };
    let Some(CanvasNodeContent::Portal(config)) = workspace.node(node_id).map(Node::content) else {
        return Task::none();
    };
    let key = PortalKey {
        workspace_id: workspace.id(),
        node_id,
    };
    if state.portal_ui.sessions.contains_key(&key) {
        return Task::none();
    }
    let scope = PortalScope::new(
        workspace.id().get(),
        workspace.floors().node_floors.get(&node_id).copied(),
        node_id.get(),
    )
    .expect("workspace and node IDs are positive");
    let config = config.clone();
    let agents = connected_agents(workspace, node_id);
    state.portal_ui.sessions.insert(
        key,
        LivePortal {
            config: config.clone(),
            phase: Phase::Connecting,
        },
    );
    state.notice = Some("Connecting isolated Chromium".to_owned());

    Task::perform(
        async move {
            tokio::task::spawn_blocking(move || connect_backend(&control, scope, config, agents))
                .await
                .map_err(|error| error.to_string())
                .and_then(|result| result)
        },
        move |result| AppMessage::Portal(Message::Connected { key, result }),
    )
}

fn connected(
    state: &mut OpenPodium,
    key: PortalKey,
    result: Result<u64, String>,
) -> Task<AppMessage> {
    match result {
        Ok(portal_id) => {
            if !session_config_is_active(state, key) {
                state.portal_ui.sessions.remove(&key);
                return cleanup_task(state, key, portal_id);
            }
            let Some(live) = state.portal_ui.sessions.get_mut(&key) else {
                return cleanup_task(state, key, portal_id);
            };
            if !matches!(live.phase, Phase::Connecting) {
                return cleanup_task(state, key, portal_id);
            }
            live.phase = Phase::Connected {
                portal_id,
                capture_busy: false,
            };
            state.notice = Some("Browser portal connected".to_owned());
        }
        Err(error) => {
            state.portal_ui.sessions.remove(&key);
            state.notice = Some(error);
        }
    }
    Task::none()
}

fn disconnect(state: &mut OpenPodium, node_id: NodeId) -> Task<AppMessage> {
    let Some(workspace_id) = state
        .workspaces
        .as_ref()
        .and_then(|manager| manager.active_workspace_id())
    else {
        return Task::none();
    };
    let key = PortalKey {
        workspace_id,
        node_id,
    };
    let Some(control) = state.ipc.as_ref().map(|ipc| ipc.portal_control()) else {
        return Task::none();
    };
    begin_disconnect(state, key, control).unwrap_or_else(Task::none)
}

fn begin_disconnect(
    state: &mut OpenPodium,
    key: PortalKey,
    control: PortalControl,
) -> Option<Task<AppMessage>> {
    let live = state.portal_ui.sessions.get_mut(&key)?;
    let portal_id = match live.phase {
        Phase::Connected { portal_id, .. } => portal_id,
        Phase::Connecting | Phase::Disconnecting { .. } => return None,
    };
    live.phase = Phase::Disconnecting { portal_id };
    Some(Task::perform(
        async move {
            tokio::task::spawn_blocking(move || {
                control
                    .unregister(portal_id)
                    .map_err(|error| error.to_string())
            })
            .await
            .map_err(|error| error.to_string())
            .and_then(|result| result)
        },
        move |result| {
            AppMessage::Portal(Message::Disconnected {
                key,
                portal_id,
                result,
            })
        },
    ))
}

fn disconnected(
    state: &mut OpenPodium,
    key: PortalKey,
    portal_id: u64,
    result: Result<(), String>,
) -> Task<AppMessage> {
    if state.portal_ui.sessions.get(&key).is_some_and(|live| {
        matches!(live.phase, Phase::Disconnecting { portal_id: current } if current == portal_id)
    }) {
        state.portal_ui.sessions.remove(&key);
        if state
            .workspaces
            .as_ref()
            .and_then(|manager| manager.active_workspace_id())
            == Some(key.workspace_id)
        {
            state.portal_frames.remove(&key.node_id);
        }
        state.canvas_revision = state.canvas_revision.wrapping_add(1);
    }
    state.notice = Some(match result {
        Ok(()) => "Browser portal disconnected".to_owned(),
        Err(error) => error,
    });
    Task::none()
}

fn frame_captured(
    state: &mut OpenPodium,
    key: PortalKey,
    portal_id: u64,
    result: Result<Option<PortalFrame>, String>,
) -> Task<AppMessage> {
    let Some(live) = state.portal_ui.sessions.get_mut(&key) else {
        return Task::none();
    };
    if !matches!(live.phase, Phase::Connected { portal_id: current, .. } if current == portal_id) {
        return Task::none();
    }
    if state
        .workspaces
        .as_ref()
        .and_then(|manager| manager.active_workspace_id())
        != Some(key.workspace_id)
    {
        let Some(control) = state.ipc.as_ref().map(|ipc| ipc.portal_control()) else {
            state.portal_ui.sessions.remove(&key);
            return Task::none();
        };
        return begin_disconnect(state, key, control).unwrap_or_else(Task::none);
    }
    match result {
        Ok(frame) => {
            live.phase = Phase::Connected {
                portal_id,
                capture_busy: false,
            };
            if let Some(frame) = frame
                && state.portal_frames.get(&key.node_id) != Some(&frame)
            {
                state.portal_frames.insert(key.node_id, frame);
                state.canvas_revision = state.canvas_revision.wrapping_add(1);
            }
            Task::none()
        }
        Err(error) => {
            state.notice = Some(error);
            let Some(control) = state.ipc.as_ref().map(|ipc| ipc.portal_control()) else {
                state.portal_ui.sessions.remove(&key);
                return Task::none();
            };
            begin_disconnect(state, key, control).unwrap_or_else(Task::none)
        }
    }
}

fn cleanup_task(state: &OpenPodium, key: PortalKey, portal_id: u64) -> Task<AppMessage> {
    let Some(control) = state.ipc.as_ref().map(|ipc| ipc.portal_control()) else {
        return Task::none();
    };
    Task::perform(
        async move {
            tokio::task::spawn_blocking(move || {
                control
                    .unregister(portal_id)
                    .map_err(|error| error.to_string())
            })
            .await
            .map_err(|error| error.to_string())
            .and_then(|result| result)
        },
        move |result| {
            AppMessage::Portal(Message::Disconnected {
                key,
                portal_id,
                result,
            })
        },
    )
}

fn connect_backend(
    control: &PortalControl,
    scope: PortalScope,
    config: PortalConfig,
    agents: Vec<u64>,
) -> Result<u64, String> {
    let backend = BrowserBackend::discover().map_err(|error| error.to_string())?;
    let portal_id = control
        .register(scope, config, backend)
        .map_err(|error| error.to_string())?;
    let result = control
        .replace_agents(portal_id, agents)
        .and_then(|()| control.connect(portal_id));
    if let Err(error) = result {
        let _ = control.unregister(portal_id);
        return Err(error.to_string());
    }
    Ok(portal_id)
}

fn capture(
    control: &PortalControl,
    portal_id: u64,
    agents: Vec<u64>,
) -> Result<Option<PortalFrame>, String> {
    control
        .replace_agents(portal_id, agents)
        .map_err(|error| error.to_string())?;
    let observation = control
        .observe(portal_id)
        .map_err(|error| error.to_string())?;
    Ok(observation.core.frame().cloned())
}

fn selected_portal(state: &OpenPodium) -> Option<(PortalKey, &PortalConfig)> {
    let workspace = state.workspaces.as_ref()?.active_workspace()?;
    let node_id = (state.canvas_selection.len() == 1).then_some(state.canvas_selection[0])?;
    let CanvasNodeContent::Portal(config) = workspace.node(node_id)?.content() else {
        return None;
    };
    Some((
        PortalKey {
            workspace_id: workspace.id(),
            node_id,
        },
        config,
    ))
}

fn session_config_is_active(state: &OpenPodium, key: PortalKey) -> bool {
    let Some(live) = state.portal_ui.sessions.get(&key) else {
        return false;
    };
    state
        .workspaces
        .as_ref()
        .and_then(|manager| manager.active_workspace())
        .filter(|workspace| workspace.id() == key.workspace_id)
        .and_then(|workspace| workspace.node(key.node_id))
        .is_some_and(|node| {
            matches!(node.content(), CanvasNodeContent::Portal(config) if config == &live.config)
        })
}

fn connected_agents(workspace: &Workspace, portal_node_id: NodeId) -> Vec<u64> {
    let layout = workspace.canvas_layout();
    layout
        .connections()
        .iter()
        .filter_map(|connection| {
            let other = if connection.source() == portal_node_id {
                connection.target()
            } else if connection.target() == portal_node_id {
                connection.source()
            } else {
                return None;
            };
            match layout
                .nodes()
                .iter()
                .find(|node| node.id() == other)
                .and_then(Node::reference)
            {
                Some(NodeTarget::Agent(agent_id)) => Some(agent_id.get()),
                _ => None,
            }
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

#[cfg(test)]
mod tests {
    use openpodium::domain::{
        Agent, AgentId, CanvasLayout, Connection, ConnectionId, ConnectionKind, Name,
    };

    use super::*;

    #[test]
    fn only_directly_connected_agents_can_address_a_portal() {
        let mut workspace = Workspace::new(WorkspaceId::new(1), Name::new("Test").unwrap());
        let agent_id = AgentId::new(1);
        let agent_node = Node::new(
            NodeId::new(1),
            NodeTarget::Agent(agent_id),
            CanvasPoint::new(0.0, 0.0).unwrap(),
            CanvasSize::new(320.0, 240.0).unwrap(),
        );
        workspace
            .execute(DomainCommand::AddAgentNode {
                agent: Agent::new(agent_id, Name::new("Agent").unwrap(), None),
                node: agent_node,
            })
            .unwrap();
        let portal_id = NodeId::new(2);
        workspace
            .execute(DomainCommand::AddNode(Node::with_content(
                portal_id,
                CanvasNodeContent::Portal(PortalConfig::browser("https://example.test").unwrap()),
                CanvasPoint::new(400.0, 0.0).unwrap(),
                CanvasSize::new(640.0, 420.0).unwrap(),
            )))
            .unwrap();
        let before = workspace.canvas_layout();
        let after = CanvasLayout::new(
            before.nodes().to_vec(),
            Vec::new(),
            vec![Connection::new(
                ConnectionId::new(1),
                portal_id,
                NodeId::new(1),
                ConnectionKind::Reference,
            )],
        );
        workspace
            .execute(DomainCommand::ReplaceCanvas { before, after })
            .unwrap();

        assert_eq!(
            connected_agents(&workspace, portal_id),
            vec![agent_id.get()]
        );
        assert!(connected_agents(&workspace, NodeId::new(1)).is_empty());
    }
}
