use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use iced::widget::{button, column, container, row, scrollable, text, text_input};
use iced::{Element, Fill, Task, Theme, clipboard};
use openpodium::domain::{
    Agent, AgentId, AgentProgram, CanvasLayout, CanvasPoint, CanvasSize, ContainerEnvironment,
    CustomEnvironment, DomainCommand, EnvironmentKind, EnvironmentProfile, EnvironmentProfileId,
    Name, Node, NodeId, NodeTarget, SshEnvironment, Timestamp, Workspace, WorkspaceDirectory,
    WorkspaceId,
};
use openpodium::runtime::{
    EnvironmentHealth, LocalProcessRuntime, ProcessEvent, ProcessRuntime, RuntimeError,
    check_environment, prepare_environment_process,
};
use openpodium::workspaces::{WorkspaceManager, WorkspaceSettingsInput};
use tokio::sync::Mutex;

use crate::canvas::{self, Alignment, Camera, History, ZOrder};
use crate::terminal;
use crate::terminal::session::{self, Action as TerminalAction, ProcessStream, Session};

const APP_NAME: &str = "OpenPodium";
const DATABASE_FILE: &str = "openpodium.sqlite";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct TerminalKey {
    workspace_id: WorkspaceId,
    node_id: NodeId,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum EnvironmentDraftKind {
    #[default]
    Ssh,
    Container,
    Custom,
}

struct OpenPodium {
    camera: Camera,
    canvas_selection: Vec<NodeId>,
    canvas_preview: Option<CanvasLayout>,
    canvas_history: History,
    canvas_revision: u64,
    terminals: BTreeMap<TerminalKey, Session>,
    focused_terminal: Option<NodeId>,
    terminal_generation: u64,
    workspaces: Option<WorkspaceManager>,
    create_directory: String,
    name: String,
    icon: String,
    working_directory: String,
    instructions: String,
    selected_environment: Option<EnvironmentProfileId>,
    environment_kind: EnvironmentDraftKind,
    environment_name: String,
    environment_primary: String,
    environment_secondary: String,
    environment_port: String,
    environment_directory: String,
    environment_arguments: String,
    environment_health: BTreeMap<(WorkspaceId, EnvironmentProfileId), EnvironmentHealth>,
    notice: Option<String>,
}

impl Default for OpenPodium {
    fn default() -> Self {
        let (workspaces, notice) = match application_database_path()
            .map_err(|error| error.to_string())
            .and_then(|path| WorkspaceManager::open(path).map_err(|error| error.to_string()))
        {
            Ok(workspaces) => (Some(workspaces), None),
            Err(error) => (None, Some(error)),
        };
        let mut state = Self {
            camera: Camera::default(),
            canvas_selection: Vec::new(),
            canvas_preview: None,
            canvas_history: History::default(),
            canvas_revision: 1,
            terminals: BTreeMap::new(),
            focused_terminal: None,
            terminal_generation: 0,
            workspaces,
            create_directory: String::new(),
            name: String::new(),
            icon: String::new(),
            working_directory: String::new(),
            instructions: String::new(),
            selected_environment: None,
            environment_kind: EnvironmentDraftKind::default(),
            environment_name: String::new(),
            environment_primary: String::new(),
            environment_secondary: String::new(),
            environment_port: String::new(),
            environment_directory: String::new(),
            environment_arguments: String::new(),
            environment_health: BTreeMap::new(),
            notice,
        };
        state.load_active_settings();
        state
    }
}

#[derive(Clone)]
enum Message {
    Canvas(canvas::Message),
    AddAgent(AgentProgram),
    CanvasAction(CanvasAction),
    CreateDirectoryChanged(String),
    CreateWorkspace,
    SwitchWorkspace(WorkspaceId),
    NameChanged(String),
    IconChanged(String),
    WorkingDirectoryChanged(String),
    InstructionsChanged(String),
    SaveSettings,
    SelectEnvironment(Option<EnvironmentProfileId>),
    EnvironmentKindSelected(EnvironmentDraftKind),
    EnvironmentNameChanged(String),
    EnvironmentPrimaryChanged(String),
    EnvironmentSecondaryChanged(String),
    EnvironmentPortChanged(String),
    EnvironmentDirectoryChanged(String),
    EnvironmentArgumentsChanged(String),
    CreateEnvironment,
    RemoveEnvironment(EnvironmentProfileId),
    CheckEnvironment(EnvironmentProfileId),
    EnvironmentChecked {
        workspace_id: WorkspaceId,
        profile_id: EnvironmentProfileId,
        health: EnvironmentHealth,
    },
    StartTerminal(NodeId),
    StopTerminal(NodeId),
    TerminalStarted {
        workspace_id: WorkspaceId,
        node_id: NodeId,
        generation: u64,
        result: Result<ProcessStream, RuntimeError>,
    },
    TerminalEvent {
        workspace_id: WorkspaceId,
        node_id: NodeId,
        generation: u64,
        event: Option<ProcessEvent>,
    },
    ClipboardRead {
        workspace_id: WorkspaceId,
        node_id: NodeId,
        contents: Option<String>,
    },
}

#[derive(Debug, Clone, Copy)]
enum CanvasAction {
    Duplicate,
    Remove,
    Group,
    Ungroup,
    Connect,
    Align(Alignment),
    ZOrder(ZOrder),
    Undo,
    Redo,
}

pub(crate) fn run() -> iced::Result {
    iced::application(OpenPodium::default, update, view)
        .title(APP_NAME)
        .theme(Theme::Dark)
        .centered()
        .run()
}

fn update(state: &mut OpenPodium, message: Message) -> Task<Message> {
    match message {
        Message::Canvas(message) => return handle_canvas_message(state, message),
        Message::AddAgent(program) => return add_agent(state, program),
        Message::CanvasAction(action) => apply_canvas_action(state, action),
        Message::CreateDirectoryChanged(value) => state.create_directory = value,
        Message::NameChanged(value) => state.name = value,
        Message::IconChanged(value) => state.icon = value,
        Message::WorkingDirectoryChanged(value) => state.working_directory = value,
        Message::InstructionsChanged(value) => state.instructions = value,
        Message::SelectEnvironment(environment_id) => {
            state.selected_environment = environment_id;
        }
        Message::EnvironmentKindSelected(kind) => state.environment_kind = kind,
        Message::EnvironmentNameChanged(value) => state.environment_name = value,
        Message::EnvironmentPrimaryChanged(value) => state.environment_primary = value,
        Message::EnvironmentSecondaryChanged(value) => state.environment_secondary = value,
        Message::EnvironmentPortChanged(value) => state.environment_port = value,
        Message::EnvironmentDirectoryChanged(value) => state.environment_directory = value,
        Message::EnvironmentArgumentsChanged(value) => state.environment_arguments = value,
        Message::CreateWorkspace => {
            let result = state
                .workspaces
                .as_mut()
                .ok_or_else(|| "workspace storage is unavailable".to_owned())
                .and_then(|workspaces| {
                    workspaces
                        .create_workspace(&state.create_directory, now())
                        .map_err(|error| error.to_string())
                });
            match result {
                Ok(_) => {
                    state.create_directory.clear();
                    state.notice = Some("Workspace created".to_owned());
                    state.reset_canvas_session();
                    state.load_active_settings();
                }
                Err(error) => state.notice = Some(error),
            }
        }
        Message::SwitchWorkspace(workspace_id) => {
            let result = state
                .workspaces
                .as_mut()
                .ok_or_else(|| "workspace storage is unavailable".to_owned())
                .and_then(|workspaces| {
                    workspaces
                        .switch(workspace_id, now())
                        .map_err(|error| error.to_string())
                });
            match result {
                Ok(()) => {
                    state.notice = None;
                    state.reset_canvas_session();
                    state.load_active_settings();
                }
                Err(error) => state.notice = Some(error),
            }
        }
        Message::SaveSettings => {
            let Some(workspace_id) = state
                .workspaces
                .as_ref()
                .and_then(WorkspaceManager::active_workspace_id)
            else {
                state.notice = Some("Create or select a workspace first".to_owned());
                return Task::none();
            };
            let input = WorkspaceSettingsInput {
                name: state.name.clone(),
                icon: Some(state.icon.clone()),
                working_directory: PathBuf::from(&state.working_directory),
                instructions: Some(state.instructions.clone()),
            };
            let result = state
                .workspaces
                .as_mut()
                .expect("the active workspace came from the manager")
                .update_settings(workspace_id, input, now());
            state.notice = Some(match result {
                Ok(_) => "Workspace settings saved".to_owned(),
                Err(error) => error.to_string(),
            });
        }
        Message::CreateEnvironment => return create_environment(state),
        Message::RemoveEnvironment(profile_id) => remove_environment(state, profile_id),
        Message::CheckEnvironment(profile_id) => {
            return check_environment_task(state, profile_id);
        }
        Message::EnvironmentChecked {
            workspace_id,
            profile_id,
            health,
        } => {
            state
                .environment_health
                .insert((workspace_id, profile_id), health);
            if state
                .workspaces
                .as_ref()
                .and_then(WorkspaceManager::active_workspace_id)
                == Some(workspace_id)
            {
                state.notice = Some(format!("Environment health: {}", health.label()));
            }
        }
        Message::StartTerminal(node_id) => return start_terminal(state, node_id),
        Message::StopTerminal(node_id) => stop_terminal(state, node_id),
        Message::TerminalStarted {
            workspace_id,
            node_id,
            generation,
            result,
        } => {
            return handle_terminal_started(state, workspace_id, node_id, generation, result);
        }
        Message::TerminalEvent {
            workspace_id,
            node_id,
            generation,
            event,
        } => return handle_terminal_event(state, workspace_id, node_id, generation, event),
        Message::ClipboardRead {
            workspace_id,
            node_id,
            contents,
        } => {
            let key = TerminalKey {
                workspace_id,
                node_id,
            };
            if let (Some(session), Some(contents)) = (state.terminals.get(&key), contents) {
                if let Err(error) = session.paste(&contents) {
                    state.notice = Some(error);
                }
            }
        }
    }
    Task::none()
}

fn view(state: &OpenPodium) -> Element<'_, Message> {
    let has_active_workspace = state
        .workspaces
        .as_ref()
        .and_then(WorkspaceManager::active_workspace)
        .is_some();
    let mut workspace_list =
        column![text(APP_NAME).size(24), text("Workspaces").size(14)].spacing(12);
    if let Some(workspaces) = &state.workspaces {
        for workspace in workspaces.recent_workspaces() {
            let icon = workspace.settings().icon().map_or("", |icon| icon.as_str());
            let label = if icon.is_empty() {
                workspace.name().to_owned()
            } else {
                format!("{icon} {}", workspace.name())
            };
            workspace_list = workspace_list.push(
                button(text(label))
                    .on_press(Message::SwitchWorkspace(workspace.id()))
                    .width(Fill),
            );
        }
    }

    let create_form = column![
        text("Open a local directory").size(14),
        text_input("/path/to/project", &state.create_directory)
            .on_input(Message::CreateDirectoryChanged),
        button("Create workspace").on_press(Message::CreateWorkspace),
    ]
    .spacing(8);
    let sidebar = container(
        column![scrollable(workspace_list).height(Fill), create_form]
            .spacing(16)
            .height(Fill),
    )
    .width(280)
    .height(Fill)
    .padding(24);

    let mut settings = column![
        text("Workspace settings").size(28),
        text_input("Name", &state.name).on_input(Message::NameChanged),
        text_input("Icon", &state.icon).on_input(Message::IconChanged),
        text_input("Working directory", &state.working_directory)
            .on_input(Message::WorkingDirectoryChanged),
        text_input("Workspace instructions", &state.instructions)
            .on_input(Message::InstructionsChanged),
        button("Save settings").on_press(Message::SaveSettings),
        text("Add agent").size(18),
        row![
            button("Codex").on_press(Message::AddAgent(AgentProgram::Codex)),
            button("Claude").on_press(Message::AddAgent(AgentProgram::Claude)),
            button("Shell").on_press(Message::AddAgent(AgentProgram::Shell)),
        ]
        .spacing(8),
        text("Runtime environment for new agents").size(18),
        button(if state.selected_environment.is_none() {
            "✓ Local workspace"
        } else {
            "Local workspace"
        })
        .on_press(Message::SelectEnvironment(None)),
        text("Canvas tools").size(18),
        row![
            button("Undo").on_press(Message::CanvasAction(CanvasAction::Undo)),
            button("Redo").on_press(Message::CanvasAction(CanvasAction::Redo)),
            button("Duplicate").on_press(Message::CanvasAction(CanvasAction::Duplicate)),
            button("Delete").on_press(Message::CanvasAction(CanvasAction::Remove)),
        ]
        .spacing(8),
        row![
            button("Group").on_press(Message::CanvasAction(CanvasAction::Group)),
            button("Ungroup").on_press(Message::CanvasAction(CanvasAction::Ungroup)),
            button("Connect").on_press(Message::CanvasAction(CanvasAction::Connect)),
        ]
        .spacing(8),
        row![
            button("Align X").on_press(Message::CanvasAction(CanvasAction::Align(
                Alignment::HorizontalCenters,
            ))),
            button("Align Y").on_press(Message::CanvasAction(CanvasAction::Align(
                Alignment::VerticalCenters,
            ))),
            button("Space X").on_press(Message::CanvasAction(CanvasAction::Align(
                Alignment::DistributeHorizontally,
            ))),
            button("Space Y").on_press(Message::CanvasAction(CanvasAction::Align(
                Alignment::DistributeVertically,
            ))),
        ]
        .spacing(8),
        row![
            button("To front")
                .on_press(Message::CanvasAction(CanvasAction::ZOrder(ZOrder::Front,))),
            button("To back").on_press(Message::CanvasAction(CanvasAction::ZOrder(ZOrder::Back,))),
        ]
        .spacing(8),
        text(format!(
            "Canvas: {}% · x {:.0} · y {:.0} · {} selected · undo {} · redo {}",
            state.camera.zoom_percent(),
            state.camera.position().x,
            state.camera.position().y,
            state.canvas_selection.len(),
            if state.canvas_history.can_undo() {
                "yes"
            } else {
                "no"
            },
            if state.canvas_history.can_redo() {
                "yes"
            } else {
                "no"
            },
        ))
        .size(12),
    ]
    .spacing(12)
    .max_width(720);
    if let Some(workspace) = state
        .workspaces
        .as_ref()
        .and_then(WorkspaceManager::active_workspace)
    {
        for profile in workspace.environment_profiles() {
            let selected = state.selected_environment == Some(profile.id());
            let label = if selected {
                format!("✓ {}", profile.name())
            } else {
                profile.name().to_string()
            };
            let health = state
                .environment_health
                .get(&(workspace.id(), profile.id()))
                .copied()
                .unwrap_or_default();
            settings = settings.push(
                row![
                    button(text(label)).on_press(Message::SelectEnvironment(Some(profile.id()))),
                    text(health.label()).size(12),
                    button("Check").on_press(Message::CheckEnvironment(profile.id())),
                    button("Delete").on_press(Message::RemoveEnvironment(profile.id())),
                ]
                .spacing(8),
            );
        }
        let kind_buttons = row![
            button(if state.environment_kind == EnvironmentDraftKind::Ssh {
                "✓ SSH"
            } else {
                "SSH"
            })
            .on_press(Message::EnvironmentKindSelected(EnvironmentDraftKind::Ssh)),
            button(
                if state.environment_kind == EnvironmentDraftKind::Container {
                    "✓ Container"
                } else {
                    "Container"
                }
            )
            .on_press(Message::EnvironmentKindSelected(
                EnvironmentDraftKind::Container,
            )),
            button(if state.environment_kind == EnvironmentDraftKind::Custom {
                "✓ Custom"
            } else {
                "Custom"
            })
            .on_press(Message::EnvironmentKindSelected(
                EnvironmentDraftKind::Custom,
            )),
        ]
        .spacing(8);
        settings = settings
            .push(text("Add environment profile").size(18))
            .push(kind_buttons)
            .push(
                text_input("Profile name", &state.environment_name)
                    .on_input(Message::EnvironmentNameChanged),
            );
        settings = match state.environment_kind {
            EnvironmentDraftKind::Ssh => settings
                .push(
                    text_input("SSH host", &state.environment_primary)
                        .on_input(Message::EnvironmentPrimaryChanged),
                )
                .push(
                    text_input("SSH user (optional)", &state.environment_secondary)
                        .on_input(Message::EnvironmentSecondaryChanged),
                )
                .push(
                    text_input("SSH port (optional)", &state.environment_port)
                        .on_input(Message::EnvironmentPortChanged),
                )
                .push(
                    text_input("Remote working directory", &state.environment_directory)
                        .on_input(Message::EnvironmentDirectoryChanged),
                ),
            EnvironmentDraftKind::Container => settings
                .push(
                    text_input("Docker-compatible executable", &state.environment_primary)
                        .on_input(Message::EnvironmentPrimaryChanged),
                )
                .push(
                    text_input(
                        "Existing container name or ID",
                        &state.environment_secondary,
                    )
                    .on_input(Message::EnvironmentSecondaryChanged),
                )
                .push(
                    text_input("Container working directory", &state.environment_directory)
                        .on_input(Message::EnvironmentDirectoryChanged),
                ),
            EnvironmentDraftKind::Custom => settings
                .push(
                    text_input("Wrapper executable", &state.environment_primary)
                        .on_input(Message::EnvironmentPrimaryChanged),
                )
                .push(
                    text_input("Arguments as a JSON array", &state.environment_arguments)
                        .on_input(Message::EnvironmentArgumentsChanged),
                )
                .push(
                    text_input("Local working directory", &state.environment_directory)
                        .on_input(Message::EnvironmentDirectoryChanged),
                ),
        };
        settings = settings.push(button("Add environment").on_press(Message::CreateEnvironment));
    }
    if !has_active_workspace {
        settings = column![
            text("Create your first workspace").size(28),
            text("Enter a local project directory in the sidebar to get started."),
        ]
        .spacing(12)
        .max_width(720);
    }
    if let Some(notice) = &state.notice {
        settings = settings.push(text(notice));
    }
    if let Some(node_id) = selected_terminal_node(state) {
        let session = state
            .terminals
            .get(&active_terminal_key(state, node_id).expect("an active workspace exists"));
        let active = session.is_some_and(Session::is_active);
        settings = settings.push(text("Terminal").size(18)).push(if active {
            button("Stop terminal").on_press(Message::StopTerminal(node_id))
        } else if session.is_some() {
            button("Reconnect terminal").on_press(Message::StartTerminal(node_id))
        } else {
            button("Start terminal").on_press(Message::StartTerminal(node_id))
        });
    }

    let stage: Element<'_, Message> = if has_active_workspace {
        let workspace = state
            .workspaces
            .as_ref()
            .and_then(WorkspaceManager::active_workspace)
            .expect("active workspace was checked above");
        let layout = state
            .canvas_preview
            .clone()
            .unwrap_or_else(|| workspace.canvas_layout());
        let terminal_views = terminal_views(state, &layout);
        let document = canvas::CanvasDocument::new(workspace, layout, terminal_views);
        row![
            canvas::view(
                state.camera,
                document,
                state.canvas_selection.clone(),
                state.focused_terminal,
                state.canvas_revision,
            )
            .map(Message::Canvas),
            container(scrollable(settings))
                .width(420)
                .height(Fill)
                .padding(24),
        ]
        .into()
    } else {
        container(settings)
            .width(Fill)
            .height(Fill)
            .center(Fill)
            .padding(32)
            .into()
    };

    row![sidebar, stage].into()
}

impl OpenPodium {
    fn reset_canvas_session(&mut self) {
        self.focused_terminal = None;
        self.camera = Camera::default();
        self.canvas_selection.clear();
        self.canvas_preview = None;
        self.canvas_history.clear();
        self.canvas_revision = self.canvas_revision.wrapping_add(1);
    }

    fn stop_all_terminals(&mut self) {
        for session in self.terminals.values_mut() {
            let _ = session.stop();
        }
        self.terminals.clear();
        self.focused_terminal = None;
    }

    fn load_active_settings(&mut self) {
        let Some(workspace) = self
            .workspaces
            .as_ref()
            .and_then(WorkspaceManager::active_workspace)
        else {
            self.name.clear();
            self.icon.clear();
            self.working_directory.clear();
            self.instructions.clear();
            self.selected_environment = None;
            return;
        };

        self.name = workspace.name().to_owned();
        self.icon = workspace
            .settings()
            .icon()
            .map_or_else(String::new, |icon| icon.as_str().to_owned());
        self.working_directory = workspace
            .settings()
            .working_directory()
            .map_or_else(String::new, |directory| directory.as_str().to_owned());
        self.instructions = workspace
            .settings()
            .instructions()
            .map_or_else(String::new, |instructions| instructions.as_str().to_owned());
        self.selected_environment = None;
    }
}

impl Drop for OpenPodium {
    fn drop(&mut self) {
        self.stop_all_terminals();
    }
}

fn handle_canvas_message(state: &mut OpenPodium, message: canvas::Message) -> Task<Message> {
    match message {
        canvas::Message::CameraChanged(camera) => {
            state.camera = camera;
            state.canvas_revision = state.canvas_revision.wrapping_add(1);
        }
        canvas::Message::SelectionChanged(selection) => {
            state.canvas_selection = selection;
            state.focused_terminal = None;
            state.canvas_revision = state.canvas_revision.wrapping_add(1);
        }
        canvas::Message::PreviewLayout(layout) => {
            state.canvas_preview = Some(layout);
            state.canvas_revision = state.canvas_revision.wrapping_add(1);
        }
        canvas::Message::CommitLayout { before, after } => {
            state.canvas_preview = None;
            if persist_canvas(state, before.clone(), after).is_ok() {
                state.canvas_history.record(before);
                state.notice = None;
            }
        }
        canvas::Message::UndoRequested => apply_canvas_action(state, CanvasAction::Undo),
        canvas::Message::RedoRequested => apply_canvas_action(state, CanvasAction::Redo),
        canvas::Message::DeleteRequested => apply_canvas_action(state, CanvasAction::Remove),
        canvas::Message::DuplicateRequested => {
            apply_canvas_action(state, CanvasAction::Duplicate);
        }
        canvas::Message::TerminalFocused(node_id) => {
            state.focused_terminal = node_id;
            state.canvas_revision = state.canvas_revision.wrapping_add(1);
        }
        canvas::Message::TerminalInput { node_id, bytes } => {
            if let Some(session) =
                active_terminal_key(state, node_id).and_then(|key| state.terminals.get(&key))
            {
                if let Err(error) = session.write(&bytes) {
                    state.notice = Some(error);
                }
            }
        }
        canvas::Message::TerminalPasteRequested(node_id) => {
            let Some(workspace_id) = active_workspace_id(state) else {
                return Task::none();
            };
            return clipboard::read().map(move |contents| Message::ClipboardRead {
                workspace_id,
                node_id,
                contents,
            });
        }
        canvas::Message::TerminalCopyRequested(node_id) => {
            if let Some(text) = state
                .workspaces
                .as_ref()
                .and_then(WorkspaceManager::active_workspace_id)
                .map(|workspace_id| TerminalKey {
                    workspace_id,
                    node_id,
                })
                .and_then(|key| state.terminals.get(&key))
                .and_then(Session::selected_text)
            {
                return clipboard::write(text);
            }
        }
        canvas::Message::TerminalScrolled { node_id, lines } => {
            if let Some(session) =
                active_terminal_key(state, node_id).and_then(|key| state.terminals.get_mut(&key))
            {
                session.scroll(lines);
                state.canvas_revision = state.canvas_revision.wrapping_add(1);
            }
        }
        canvas::Message::TerminalSelectionStarted {
            node_id,
            row,
            column,
            right_side,
        } => {
            state.focused_terminal = Some(node_id);
            state.canvas_selection = vec![node_id];
            if let Some(session) =
                active_terminal_key(state, node_id).and_then(|key| state.terminals.get_mut(&key))
            {
                session.clear_selection();
                session.begin_selection(row, column, right_side);
            }
            state.canvas_revision = state.canvas_revision.wrapping_add(1);
        }
        canvas::Message::TerminalSelectionUpdated {
            node_id,
            row,
            column,
            right_side,
        } => {
            if let Some(session) =
                active_terminal_key(state, node_id).and_then(|key| state.terminals.get_mut(&key))
            {
                session.update_selection(row, column, right_side);
                state.canvas_revision = state.canvas_revision.wrapping_add(1);
            }
        }
    }
    Task::none()
}

fn create_environment(state: &mut OpenPodium) -> Task<Message> {
    let Some((workspace_id, profile_id)) = state
        .workspaces
        .as_ref()
        .and_then(WorkspaceManager::active_workspace)
        .and_then(|workspace| {
            next_environment_profile_id(workspace).map(|profile_id| (workspace.id(), profile_id))
        })
    else {
        state.notice = Some("Create or select a workspace first".to_owned());
        return Task::none();
    };
    let profile = match environment_profile_from_draft(state, profile_id) {
        Ok(profile) => profile,
        Err(error) => {
            state.notice = Some(error);
            return Task::none();
        }
    };
    let result = state
        .workspaces
        .as_mut()
        .expect("the active workspace came from the manager")
        .execute(
            workspace_id,
            DomainCommand::AddEnvironmentProfile(profile),
            now(),
        );
    match result {
        Ok(_) => {
            state.selected_environment = Some(profile_id);
            state.environment_name.clear();
            state.environment_primary.clear();
            state.environment_secondary.clear();
            state.environment_port.clear();
            state.environment_directory.clear();
            state.environment_arguments.clear();
            state.notice = Some("Environment profile added".to_owned());
            check_environment_task(state, profile_id)
        }
        Err(error) => {
            state.notice = Some(error.to_string());
            Task::none()
        }
    }
}

fn environment_profile_from_draft(
    state: &OpenPodium,
    profile_id: EnvironmentProfileId,
) -> Result<EnvironmentProfile, String> {
    let name = Name::new(state.environment_name.clone()).map_err(|error| error.to_string())?;
    let kind = match state.environment_kind {
        EnvironmentDraftKind::Ssh => {
            let port = if state.environment_port.trim().is_empty() {
                None
            } else {
                Some(
                    state
                        .environment_port
                        .parse::<u16>()
                        .map_err(|_| "SSH port must be a number from 1 to 65535".to_owned())?,
                )
            };
            let user = (!state.environment_secondary.trim().is_empty())
                .then(|| state.environment_secondary.clone());
            EnvironmentKind::Ssh(
                SshEnvironment::new(
                    state.environment_primary.clone(),
                    user,
                    port,
                    state.environment_directory.clone(),
                )
                .map_err(|error| error.to_string())?,
            )
        }
        EnvironmentDraftKind::Container => EnvironmentKind::Container(
            ContainerEnvironment::new(
                state.environment_primary.clone(),
                state.environment_secondary.clone(),
                state.environment_directory.clone(),
            )
            .map_err(|error| error.to_string())?,
        ),
        EnvironmentDraftKind::Custom => {
            let arguments = if state.environment_arguments.trim().is_empty() {
                Vec::new()
            } else {
                serde_json::from_str::<Vec<String>>(&state.environment_arguments).map_err(
                    |error| format!("custom arguments must be a JSON string array: {error}"),
                )?
            };
            EnvironmentKind::Custom(
                CustomEnvironment::new(
                    state.environment_primary.clone(),
                    arguments,
                    WorkspaceDirectory::new(state.environment_directory.clone())
                        .map_err(|error| error.to_string())?,
                )
                .map_err(|error| error.to_string())?,
            )
        }
    };
    Ok(EnvironmentProfile::new(profile_id, name, kind))
}

fn remove_environment(state: &mut OpenPodium, profile_id: EnvironmentProfileId) {
    let Some(workspace_id) = active_workspace_id(state) else {
        state.notice = Some("Create or select a workspace first".to_owned());
        return;
    };
    let result = state
        .workspaces
        .as_mut()
        .expect("the active workspace came from the manager")
        .execute(
            workspace_id,
            DomainCommand::RemoveEnvironmentProfile(profile_id),
            now(),
        );
    state.notice = Some(match result {
        Ok(_) => {
            if state.selected_environment == Some(profile_id) {
                state.selected_environment = None;
            }
            state.environment_health.remove(&(workspace_id, profile_id));
            "Environment profile deleted".to_owned()
        }
        Err(error) => error.to_string(),
    });
}

fn check_environment_task(
    state: &mut OpenPodium,
    profile_id: EnvironmentProfileId,
) -> Task<Message> {
    let Some((workspace_id, profile, working_directory)) = state
        .workspaces
        .as_ref()
        .and_then(WorkspaceManager::active_workspace)
        .and_then(|workspace| {
            Some((
                workspace.id(),
                workspace.environment_profile(profile_id)?.clone(),
                PathBuf::from(workspace.settings().working_directory()?.as_str()),
            ))
        })
    else {
        state.notice = Some("The environment or workspace directory is unavailable".to_owned());
        return Task::none();
    };
    state
        .environment_health
        .insert((workspace_id, profile_id), EnvironmentHealth::Checking);
    Task::perform(
        async move { check_environment(Some(&profile), &working_directory) },
        move |health| Message::EnvironmentChecked {
            workspace_id,
            profile_id,
            health,
        },
    )
}

fn add_agent(state: &mut OpenPodium, program: AgentProgram) -> Task<Message> {
    let Some((workspace_id, agent_id, node_id, before, node, environment_id)) = state
        .workspaces
        .as_ref()
        .and_then(WorkspaceManager::active_workspace)
        .and_then(|workspace| {
            let agent_id = next_agent_id(workspace)?;
            let node_id = next_node_id(workspace)?;
            let before = workspace.canvas_layout();
            let offset = (before.nodes().len() % 6) as f64 * 40.0;
            let z_index = before
                .nodes()
                .iter()
                .map(Node::z_index)
                .max()
                .unwrap_or(0)
                .saturating_add(1);
            let node = Node::with_z_index(
                node_id,
                NodeTarget::Agent(agent_id),
                CanvasPoint::new(
                    canvas_coordinate(state.camera.position().x - 180.0 + offset),
                    canvas_coordinate(state.camera.position().y - 130.0 + offset),
                )
                .expect("clamped camera coordinates are finite"),
                CanvasSize::new(360.0, 260.0).expect("default node size is valid"),
                z_index,
            );
            let environment_id = state
                .selected_environment
                .filter(|environment_id| workspace.environment_profile(*environment_id).is_some());
            Some((
                workspace.id(),
                agent_id,
                node_id,
                before,
                node,
                environment_id,
            ))
        })
    else {
        state.notice = Some("Create or select a workspace first".to_owned());
        return Task::none();
    };
    let label = agent_program_name(program);
    let mut agent = Agent::with_program(
        agent_id,
        Name::new(format!("{label} {}", agent_id.get())).expect("generated agent name is valid"),
        None,
        program,
    );
    if let Some(environment_id) = environment_id {
        agent = agent.in_environment(environment_id);
    }
    let result = state
        .workspaces
        .as_mut()
        .expect("active workspace came from the manager")
        .execute(
            workspace_id,
            DomainCommand::AddAgentNode { agent, node },
            now(),
        );
    match result {
        Ok(_) => {
            state.canvas_history.record(before);
            state.canvas_selection = vec![node_id];
            state.canvas_preview = None;
            state.canvas_revision = state.canvas_revision.wrapping_add(1);
            state.notice = Some(format!("{label} agent added; terminal starting"));
            return start_terminal(state, node_id);
        }
        Err(error) => state.notice = Some(error.to_string()),
    }
    Task::none()
}

fn start_terminal(state: &mut OpenPodium, node_id: NodeId) -> Task<Message> {
    let Some((workspace_id, program, profile, working_directory, size)) = state
        .workspaces
        .as_ref()
        .and_then(WorkspaceManager::active_workspace)
        .and_then(|workspace| {
            let node = workspace.node(node_id)?;
            let NodeTarget::Agent(agent_id) = node.target() else {
                return None;
            };
            let agent = workspace.agent(agent_id)?;
            let working_directory = workspace.settings().working_directory()?.as_str();
            let profile = agent
                .environment_id()
                .and_then(|environment_id| workspace.environment_profile(environment_id))
                .cloned();
            Some((
                workspace.id(),
                agent.program(),
                profile,
                PathBuf::from(working_directory),
                terminal::GridSize::for_node(node.size().width(), node.size().height()),
            ))
        })
    else {
        state.notice = Some("The selected agent has no workspace working directory".to_owned());
        return Task::none();
    };
    let key = TerminalKey {
        workspace_id,
        node_id,
    };
    if state.terminals.get(&key).is_some_and(Session::is_active) {
        state.focused_terminal = Some(node_id);
        state.notice = Some("Terminal is already running".to_owned());
        state.canvas_revision = state.canvas_revision.wrapping_add(1);
        return Task::none();
    }

    let spec = match prepare_environment_process(
        profile.as_ref(),
        session::process_spec(program, &working_directory, size),
    ) {
        Ok(spec) => spec,
        Err(error) => {
            state.notice = Some(error.to_string());
            return Task::none();
        }
    };

    state.terminal_generation = state.terminal_generation.wrapping_add(1);
    let generation = state.terminal_generation;
    state
        .terminals
        .insert(key, Session::starting(size, generation));
    state.focused_terminal = Some(node_id);
    state.canvas_revision = state.canvas_revision.wrapping_add(1);
    state.notice = Some("Terminal starting".to_owned());

    Task::perform(
        async move {
            LocalProcessRuntime
                .spawn(spec)
                .map(|process| Arc::new(Mutex::new(process)))
        },
        move |result| Message::TerminalStarted {
            workspace_id,
            node_id,
            generation,
            result,
        },
    )
}

fn handle_terminal_started(
    state: &mut OpenPodium,
    workspace_id: WorkspaceId,
    node_id: NodeId,
    generation: u64,
    result: Result<ProcessStream, RuntimeError>,
) -> Task<Message> {
    let is_active_workspace = active_workspace_id(state) == Some(workspace_id);
    let key = TerminalKey {
        workspace_id,
        node_id,
    };
    let Some(session) = state
        .terminals
        .get_mut(&key)
        .filter(|session| session.generation() == generation)
    else {
        return Task::none();
    };
    match result {
        Ok(stream) => {
            if let Err(error) = session.attach(stream) {
                session.fail(error);
                if is_active_workspace {
                    state.notice = Some(error.to_owned());
                }
                return Task::none();
            }
            if is_active_workspace {
                state.notice = None;
            }
            state.canvas_revision = state.canvas_revision.wrapping_add(1);
            wait_for_terminal_event(
                workspace_id,
                node_id,
                generation,
                session.stream().expect("just attached"),
            )
        }
        Err(error) => {
            session.fail(&error);
            if is_active_workspace {
                state.notice = Some(error.to_string());
            }
            state.canvas_revision = state.canvas_revision.wrapping_add(1);
            Task::none()
        }
    }
}

fn wait_for_terminal_event(
    workspace_id: WorkspaceId,
    node_id: NodeId,
    generation: u64,
    stream: ProcessStream,
) -> Task<Message> {
    Task::perform(
        async move { stream.lock().await.next_event().await },
        move |event| Message::TerminalEvent {
            workspace_id,
            node_id,
            generation,
            event,
        },
    )
}

fn handle_terminal_event(
    state: &mut OpenPodium,
    workspace_id: WorkspaceId,
    node_id: NodeId,
    generation: u64,
    event: Option<ProcessEvent>,
) -> Task<Message> {
    let key = TerminalKey {
        workspace_id,
        node_id,
    };
    let Some(session) = state
        .terminals
        .get_mut(&key)
        .filter(|session| session.generation() == generation)
    else {
        return Task::none();
    };
    let continues = matches!(event, Some(ProcessEvent::Output(_)));
    let actions = match event {
        Some(event) => session.handle_event(event),
        None => {
            session.fail("the terminal event stream closed unexpectedly");
            Vec::new()
        }
    };
    let mut tasks = actions
        .into_iter()
        .filter_map(|action| match action {
            TerminalAction::ClipboardStore(text) => Some(clipboard::write(text)),
            TerminalAction::Bell => None,
        })
        .collect::<Vec<_>>();
    if continues {
        if let Some(stream) = session.stream() {
            tasks.push(wait_for_terminal_event(
                workspace_id,
                node_id,
                generation,
                stream,
            ));
        }
    }
    state.canvas_revision = state.canvas_revision.wrapping_add(1);
    Task::batch(tasks)
}

fn stop_terminal(state: &mut OpenPodium, node_id: NodeId) {
    let Some(key) = active_terminal_key(state, node_id) else {
        return;
    };
    if let Some(session) = state.terminals.get_mut(&key) {
        if let Err(error) = session.stop() {
            state.notice = Some(error);
        }
    }
    if state.focused_terminal == Some(node_id) {
        state.focused_terminal = None;
    }
    state.canvas_revision = state.canvas_revision.wrapping_add(1);
}

fn apply_canvas_action(state: &mut OpenPodium, action: CanvasAction) {
    if matches!(action, CanvasAction::Undo) {
        undo_canvas(state);
        return;
    }
    if matches!(action, CanvasAction::Redo) {
        redo_canvas(state);
        return;
    }
    let Some(before) = current_canvas(state) else {
        state.notice = Some("Create or select a workspace first".to_owned());
        return;
    };
    let mut selection = state.canvas_selection.clone();
    let after = match action {
        CanvasAction::Duplicate => {
            let (after, duplicated) = canvas::editor::duplicate(&before, &selection);
            selection = duplicated;
            after
        }
        CanvasAction::Remove => {
            selection.clear();
            canvas::editor::remove(&before, &state.canvas_selection)
        }
        CanvasAction::Group => canvas::editor::group(&before, &selection),
        CanvasAction::Ungroup => canvas::editor::ungroup(&before, &selection),
        CanvasAction::Connect => match canvas::editor::connect(&before, &selection) {
            Ok(after) => after,
            Err(error) => {
                state.notice = Some(error.to_owned());
                return;
            }
        },
        CanvasAction::Align(alignment) => canvas::editor::align(&before, &selection, alignment),
        CanvasAction::ZOrder(order) => canvas::editor::change_z_order(&before, &selection, order),
        CanvasAction::Undo | CanvasAction::Redo => unreachable!("handled above"),
    };
    if before == after {
        return;
    }
    if persist_canvas(state, before.clone(), after).is_ok() {
        state.canvas_history.record(before);
        state.canvas_selection = selection;
        state.notice = None;
    }
}

fn undo_canvas(state: &mut OpenPodium) {
    let Some(target) = state.canvas_history.undo_target().cloned() else {
        return;
    };
    let Some(current) = current_canvas(state) else {
        return;
    };
    if persist_canvas(state, current.clone(), target.clone()).is_ok() {
        state.canvas_history.complete_undo(current);
        retain_existing_selection(state, &target);
        state.notice = None;
    }
}

fn redo_canvas(state: &mut OpenPodium) {
    let Some(target) = state.canvas_history.redo_target().cloned() else {
        return;
    };
    let Some(current) = current_canvas(state) else {
        return;
    };
    if persist_canvas(state, current.clone(), target.clone()).is_ok() {
        state.canvas_history.complete_redo(current);
        retain_existing_selection(state, &target);
        state.notice = None;
    }
}

fn persist_canvas(
    state: &mut OpenPodium,
    before: CanvasLayout,
    after: CanvasLayout,
) -> Result<(), ()> {
    let Some(workspace_id) = state
        .workspaces
        .as_ref()
        .and_then(WorkspaceManager::active_workspace_id)
    else {
        state.notice = Some("Create or select a workspace first".to_owned());
        return Err(());
    };
    state.canvas_preview = None;
    let runtime_layout = after.clone();
    match state
        .workspaces
        .as_mut()
        .expect("active workspace came from the manager")
        .execute(
            workspace_id,
            DomainCommand::ReplaceCanvas { before, after },
            now(),
        ) {
        Ok(_) => {
            synchronize_terminals(state, &runtime_layout);
            state.canvas_revision = state.canvas_revision.wrapping_add(1);
            Ok(())
        }
        Err(error) => {
            state.notice = Some(error.to_string());
            state.canvas_revision = state.canvas_revision.wrapping_add(1);
            Err(())
        }
    }
}

fn synchronize_terminals(state: &mut OpenPodium, layout: &CanvasLayout) {
    let Some(workspace_id) = active_workspace_id(state) else {
        return;
    };
    state.terminals.retain(|key, _| {
        key.workspace_id != workspace_id
            || layout.nodes().iter().any(|node| node.id() == key.node_id)
    });
    if state.focused_terminal.is_some_and(|node_id| {
        !state.terminals.contains_key(&TerminalKey {
            workspace_id,
            node_id,
        })
    }) {
        state.focused_terminal = None;
    }
    for node in layout.nodes() {
        let Some(session) = state.terminals.get_mut(&TerminalKey {
            workspace_id,
            node_id: node.id(),
        }) else {
            continue;
        };
        let size = terminal::GridSize::for_node(node.size().width(), node.size().height());
        if let Err(error) = session.resize(size) {
            state.notice = Some(error);
        }
    }
}

fn terminal_views(state: &OpenPodium, layout: &CanvasLayout) -> BTreeMap<NodeId, terminal::View> {
    let Some(workspace_id) = active_workspace_id(state) else {
        return BTreeMap::new();
    };
    layout
        .nodes()
        .iter()
        .filter_map(|node| {
            matches!(node.target(), NodeTarget::Agent(_)).then(|| {
                let size = terminal::GridSize::for_node(node.size().width(), node.size().height());
                let view = state
                    .terminals
                    .get(&TerminalKey {
                        workspace_id,
                        node_id: node.id(),
                    })
                    .map_or_else(|| terminal::View::offline(size), Session::view);
                (node.id(), view)
            })
        })
        .collect()
}

fn selected_terminal_node(state: &OpenPodium) -> Option<NodeId> {
    let workspace = state.workspaces.as_ref()?.active_workspace()?;
    state.canvas_selection.iter().copied().find(|node_id| {
        workspace
            .node(*node_id)
            .is_some_and(|node| matches!(node.target(), NodeTarget::Agent(_)))
    })
}

fn active_workspace_id(state: &OpenPodium) -> Option<WorkspaceId> {
    state
        .workspaces
        .as_ref()
        .and_then(WorkspaceManager::active_workspace_id)
}

fn active_terminal_key(state: &OpenPodium, node_id: NodeId) -> Option<TerminalKey> {
    Some(TerminalKey {
        workspace_id: active_workspace_id(state)?,
        node_id,
    })
}

fn current_canvas(state: &OpenPodium) -> Option<CanvasLayout> {
    state
        .workspaces
        .as_ref()?
        .active_workspace()
        .map(Workspace::canvas_layout)
}

fn retain_existing_selection(state: &mut OpenPodium, layout: &CanvasLayout) {
    state
        .canvas_selection
        .retain(|node_id| layout.nodes().iter().any(|node| node.id() == *node_id));
}

fn next_agent_id(workspace: &Workspace) -> Option<AgentId> {
    let mut value = 1_u64;
    loop {
        let id = AgentId::new(value);
        if workspace.agent(id).is_none() {
            return Some(id);
        }
        value = value.checked_add(1)?;
    }
}

fn next_node_id(workspace: &Workspace) -> Option<NodeId> {
    let mut value = 1_u64;
    loop {
        let id = NodeId::new(value);
        if workspace.node(id).is_none() {
            return Some(id);
        }
        value = value.checked_add(1)?;
    }
}

fn next_environment_profile_id(workspace: &Workspace) -> Option<EnvironmentProfileId> {
    let mut value = 1_u64;
    loop {
        let id = EnvironmentProfileId::new(value);
        if workspace.environment_profile(id).is_none() {
            return Some(id);
        }
        value = value.checked_add(1)?;
    }
}

fn canvas_coordinate(value: f64) -> f32 {
    value.clamp(-1_000_000_000.0, 1_000_000_000.0) as f32
}

fn agent_program_name(program: AgentProgram) -> &'static str {
    match program {
        AgentProgram::Codex => "Codex",
        AgentProgram::Claude => "Claude",
        AgentProgram::Shell => "Shell",
    }
}

fn application_database_path() -> Result<PathBuf, std::io::Error> {
    let directory = if let Some(path) = env::var_os("OPENPODIUM_DATA_DIR") {
        PathBuf::from(path)
    } else {
        platform_data_directory()?
    };
    fs::create_dir_all(&directory)?;
    Ok(directory.join(DATABASE_FILE))
}

#[cfg(target_os = "macos")]
fn platform_data_directory() -> Result<PathBuf, io::Error> {
    let home = env::var_os("HOME").ok_or_else(missing_data_directory)?;
    Ok(PathBuf::from(home)
        .join("Library")
        .join("Application Support")
        .join(APP_NAME))
}

#[cfg(target_os = "windows")]
fn platform_data_directory() -> Result<PathBuf, io::Error> {
    let local_app_data = env::var_os("LOCALAPPDATA").ok_or_else(missing_data_directory)?;
    Ok(PathBuf::from(local_app_data).join(APP_NAME))
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn platform_data_directory() -> Result<PathBuf, io::Error> {
    if let Some(path) = env::var_os("XDG_DATA_HOME") {
        return Ok(PathBuf::from(path).join("openpodium"));
    }
    let home = env::var_os("HOME").ok_or_else(missing_data_directory)?;
    Ok(PathBuf::from(home).join(".local/share/openpodium"))
}

fn missing_data_directory() -> io::Error {
    io::Error::new(
        io::ErrorKind::NotFound,
        "cannot locate the platform application-data directory; set OPENPODIUM_DATA_DIR",
    )
}

fn now() -> Timestamp {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    Timestamp::from_unix_millis(u64::try_from(millis).unwrap_or(u64::MAX))
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn adding_an_agent_is_persisted_and_participates_in_undo_redo() {
        let temp = TempDir::new().unwrap();
        let project = temp.path().join("project");
        fs::create_dir(&project).unwrap();
        let database = temp.path().join("state.sqlite");
        let mut workspaces = WorkspaceManager::open(&database).unwrap();
        let workspace_id = workspaces
            .create_workspace(&project, Timestamp::from_unix_millis(1))
            .unwrap();
        let mut state = OpenPodium {
            camera: Camera::default(),
            canvas_selection: Vec::new(),
            canvas_preview: None,
            canvas_history: History::default(),
            canvas_revision: 1,
            terminals: BTreeMap::new(),
            focused_terminal: None,
            terminal_generation: 0,
            workspaces: Some(workspaces),
            create_directory: String::new(),
            name: String::new(),
            icon: String::new(),
            working_directory: String::new(),
            instructions: String::new(),
            selected_environment: None,
            environment_kind: EnvironmentDraftKind::default(),
            environment_name: String::new(),
            environment_primary: String::new(),
            environment_secondary: String::new(),
            environment_port: String::new(),
            environment_directory: String::new(),
            environment_arguments: String::new(),
            environment_health: BTreeMap::new(),
            notice: None,
        };

        let _task = add_agent(&mut state, AgentProgram::Codex);
        let workspace = state
            .workspaces
            .as_ref()
            .unwrap()
            .active_workspace()
            .unwrap();
        assert_eq!(workspace.canvas_layout().nodes().len(), 1);
        assert_eq!(
            workspace.agent(AgentId::new(1)).unwrap().program(),
            AgentProgram::Codex
        );

        undo_canvas(&mut state);
        assert!(
            state
                .workspaces
                .as_ref()
                .unwrap()
                .active_workspace()
                .unwrap()
                .canvas_layout()
                .nodes()
                .is_empty()
        );
        redo_canvas(&mut state);
        assert_eq!(
            state
                .workspaces
                .as_ref()
                .unwrap()
                .active_workspace()
                .unwrap()
                .canvas_layout()
                .nodes()
                .len(),
            1
        );

        drop(state);
        let restored = WorkspaceManager::open(database).unwrap();
        assert_eq!(
            restored
                .workspace(workspace_id)
                .unwrap()
                .canvas_layout()
                .nodes()
                .len(),
            1
        );
    }

    #[test]
    fn resetting_the_canvas_view_preserves_background_terminal_sessions() {
        let temp = TempDir::new().unwrap();
        let first = temp.path().join("first");
        let second = temp.path().join("second");
        fs::create_dir(&first).unwrap();
        fs::create_dir(&second).unwrap();
        let mut workspaces = WorkspaceManager::open(temp.path().join("state.sqlite")).unwrap();
        let first_id = workspaces
            .create_workspace(&first, Timestamp::from_unix_millis(1))
            .unwrap();
        let second_id = workspaces
            .create_workspace(&second, Timestamp::from_unix_millis(2))
            .unwrap();
        workspaces
            .switch(first_id, Timestamp::from_unix_millis(3))
            .unwrap();
        let key = TerminalKey {
            workspace_id: first_id,
            node_id: NodeId::new(1),
        };
        let mut terminals = BTreeMap::new();
        terminals.insert(
            key,
            Session::starting(terminal::GridSize::for_node(640.0, 480.0), 1),
        );
        let mut state = test_state(workspaces, terminals);

        state
            .workspaces
            .as_mut()
            .unwrap()
            .switch(second_id, Timestamp::from_unix_millis(4))
            .unwrap();
        state.reset_canvas_session();

        assert!(state.terminals.contains_key(&key));
    }

    #[test]
    fn starting_an_active_terminal_does_not_duplicate_its_process() {
        let temp = TempDir::new().unwrap();
        let project = temp.path().join("project");
        fs::create_dir(&project).unwrap();
        let mut workspaces = WorkspaceManager::open(temp.path().join("state.sqlite")).unwrap();
        let workspace_id = workspaces
            .create_workspace(&project, Timestamp::from_unix_millis(1))
            .unwrap();
        let mut state = test_state(workspaces, BTreeMap::new());

        let _first_start = add_agent(&mut state, AgentProgram::Shell);
        let generation = state.terminal_generation;
        let key = TerminalKey {
            workspace_id,
            node_id: NodeId::new(1),
        };
        assert!(state.terminals.get(&key).is_some_and(Session::is_active));

        let _duplicate_start = start_terminal(&mut state, NodeId::new(1));

        assert_eq!(state.terminal_generation, generation);
        assert_eq!(state.terminals.len(), 1);
        assert_eq!(state.notice.as_deref(), Some("Terminal is already running"));
    }

    fn test_state(
        workspaces: WorkspaceManager,
        terminals: BTreeMap<TerminalKey, Session>,
    ) -> OpenPodium {
        OpenPodium {
            camera: Camera::default(),
            canvas_selection: Vec::new(),
            canvas_preview: None,
            canvas_history: History::default(),
            canvas_revision: 1,
            terminals,
            focused_terminal: None,
            terminal_generation: 0,
            workspaces: Some(workspaces),
            create_directory: String::new(),
            name: String::new(),
            icon: String::new(),
            working_directory: String::new(),
            instructions: String::new(),
            selected_environment: None,
            environment_kind: EnvironmentDraftKind::default(),
            environment_name: String::new(),
            environment_primary: String::new(),
            environment_secondary: String::new(),
            environment_port: String::new(),
            environment_directory: String::new(),
            environment_arguments: String::new(),
            environment_health: BTreeMap::new(),
            notice: None,
        }
    }
}
