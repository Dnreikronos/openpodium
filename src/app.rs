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
    Agent, AgentId, AgentProgram, CanvasLayout, CanvasPoint, CanvasSize, DomainCommand, Name, Node,
    NodeId, NodeTarget, Timestamp, Workspace, WorkspaceId,
};
use openpodium::runtime::{LocalProcessRuntime, ProcessEvent, ProcessRuntime, RuntimeError};
use openpodium::workspaces::{WorkspaceManager, WorkspaceSettingsInput};
use tokio::sync::Mutex;

use crate::canvas::{self, Alignment, Camera, History, ZOrder};
use crate::terminal;
use crate::terminal::session::{self, Action as TerminalAction, ProcessStream, Session};

const APP_NAME: &str = "OpenPodium";
const DATABASE_FILE: &str = "openpodium.sqlite";

struct OpenPodium {
    camera: Camera,
    canvas_selection: Vec<NodeId>,
    canvas_preview: Option<CanvasLayout>,
    canvas_history: History,
    canvas_revision: u64,
    terminals: BTreeMap<NodeId, Session>,
    focused_terminal: Option<NodeId>,
    terminal_generation: u64,
    workspaces: Option<WorkspaceManager>,
    create_directory: String,
    name: String,
    icon: String,
    working_directory: String,
    instructions: String,
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
    StartTerminal(NodeId),
    StopTerminal(NodeId),
    TerminalStarted {
        node_id: NodeId,
        generation: u64,
        result: Result<ProcessStream, RuntimeError>,
    },
    TerminalEvent {
        node_id: NodeId,
        generation: u64,
        event: Option<ProcessEvent>,
    },
    ClipboardRead {
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
        Message::StartTerminal(node_id) => return start_terminal(state, node_id),
        Message::StopTerminal(node_id) => stop_terminal(state, node_id),
        Message::TerminalStarted {
            node_id,
            generation,
            result,
        } => return handle_terminal_started(state, node_id, generation, result),
        Message::TerminalEvent {
            node_id,
            generation,
            event,
        } => return handle_terminal_event(state, node_id, generation, event),
        Message::ClipboardRead { node_id, contents } => {
            if let (Some(session), Some(contents)) = (state.terminals.get(&node_id), contents) {
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
        let active = state
            .terminals
            .get(&node_id)
            .is_some_and(Session::is_active);
        settings = settings.push(text("Terminal").size(18)).push(if active {
            button("Stop terminal").on_press(Message::StopTerminal(node_id))
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
        self.stop_all_terminals();
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
            if let Some(session) = state.terminals.get(&node_id) {
                if let Err(error) = session.write(&bytes) {
                    state.notice = Some(error);
                }
            }
        }
        canvas::Message::TerminalPasteRequested(node_id) => {
            return clipboard::read()
                .map(move |contents| Message::ClipboardRead { node_id, contents });
        }
        canvas::Message::TerminalCopyRequested(node_id) => {
            if let Some(text) = state
                .terminals
                .get(&node_id)
                .and_then(Session::selected_text)
            {
                return clipboard::write(text);
            }
        }
        canvas::Message::TerminalScrolled { node_id, lines } => {
            if let Some(session) = state.terminals.get_mut(&node_id) {
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
            if let Some(session) = state.terminals.get_mut(&node_id) {
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
            if let Some(session) = state.terminals.get_mut(&node_id) {
                session.update_selection(row, column, right_side);
                state.canvas_revision = state.canvas_revision.wrapping_add(1);
            }
        }
    }
    Task::none()
}

fn add_agent(state: &mut OpenPodium, program: AgentProgram) -> Task<Message> {
    let Some((workspace_id, agent_id, node_id, before, node)) = state
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
            Some((workspace.id(), agent_id, node_id, before, node))
        })
    else {
        state.notice = Some("Create or select a workspace first".to_owned());
        return Task::none();
    };
    let label = agent_program_name(program);
    let agent = Agent::with_program(
        agent_id,
        Name::new(format!("{label} {}", agent_id.get())).expect("generated agent name is valid"),
        None,
        program,
    );
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
    let Some((program, working_directory, size)) = state
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
            Some((
                agent.program(),
                PathBuf::from(working_directory),
                terminal::GridSize::for_node(node.size().width(), node.size().height()),
            ))
        })
    else {
        state.notice = Some("The selected agent has no workspace working directory".to_owned());
        return Task::none();
    };

    state.terminal_generation = state.terminal_generation.wrapping_add(1);
    let generation = state.terminal_generation;
    state
        .terminals
        .insert(node_id, Session::starting(size, generation));
    state.focused_terminal = Some(node_id);
    state.canvas_revision = state.canvas_revision.wrapping_add(1);
    state.notice = Some("Terminal starting".to_owned());
    let spec = session::process_spec(program, &working_directory, size);

    Task::perform(
        async move {
            LocalProcessRuntime
                .spawn(spec)
                .map(|process| Arc::new(Mutex::new(process)))
        },
        move |result| Message::TerminalStarted {
            node_id,
            generation,
            result,
        },
    )
}

fn handle_terminal_started(
    state: &mut OpenPodium,
    node_id: NodeId,
    generation: u64,
    result: Result<ProcessStream, RuntimeError>,
) -> Task<Message> {
    let Some(session) = state
        .terminals
        .get_mut(&node_id)
        .filter(|session| session.generation() == generation)
    else {
        return Task::none();
    };
    match result {
        Ok(stream) => {
            if let Err(error) = session.attach(stream) {
                session.fail(error);
                state.notice = Some(error.to_owned());
                return Task::none();
            }
            state.notice = None;
            state.canvas_revision = state.canvas_revision.wrapping_add(1);
            wait_for_terminal_event(
                node_id,
                generation,
                session.stream().expect("just attached"),
            )
        }
        Err(error) => {
            session.fail(&error);
            state.notice = Some(error.to_string());
            state.canvas_revision = state.canvas_revision.wrapping_add(1);
            Task::none()
        }
    }
}

fn wait_for_terminal_event(
    node_id: NodeId,
    generation: u64,
    stream: ProcessStream,
) -> Task<Message> {
    Task::perform(
        async move { stream.lock().await.next_event().await },
        move |event| Message::TerminalEvent {
            node_id,
            generation,
            event,
        },
    )
}

fn handle_terminal_event(
    state: &mut OpenPodium,
    node_id: NodeId,
    generation: u64,
    event: Option<ProcessEvent>,
) -> Task<Message> {
    let Some(session) = state
        .terminals
        .get_mut(&node_id)
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
            tasks.push(wait_for_terminal_event(node_id, generation, stream));
        }
    }
    state.canvas_revision = state.canvas_revision.wrapping_add(1);
    Task::batch(tasks)
}

fn stop_terminal(state: &mut OpenPodium, node_id: NodeId) {
    if let Some(session) = state.terminals.get_mut(&node_id) {
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
    state
        .terminals
        .retain(|node_id, _| layout.nodes().iter().any(|node| node.id() == *node_id));
    if state
        .focused_terminal
        .is_some_and(|node_id| !state.terminals.contains_key(&node_id))
    {
        state.focused_terminal = None;
    }
    for node in layout.nodes() {
        let Some(session) = state.terminals.get_mut(&node.id()) else {
            continue;
        };
        let size = terminal::GridSize::for_node(node.size().width(), node.size().height());
        if let Err(error) = session.resize(size) {
            state.notice = Some(error);
        }
    }
}

fn terminal_views(state: &OpenPodium, layout: &CanvasLayout) -> BTreeMap<NodeId, terminal::View> {
    layout
        .nodes()
        .iter()
        .filter_map(|node| {
            matches!(node.target(), NodeTarget::Agent(_)).then(|| {
                let size = terminal::GridSize::for_node(node.size().width(), node.size().height());
                let view = state
                    .terminals
                    .get(&node.id())
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
}
