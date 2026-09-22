mod context_nodes;
mod floors;
mod icons;
mod navigation;
mod portals;
pub(crate) mod shell;
mod trackpad;
pub(crate) mod ui;

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use iced::widget::{
    button as iced_button, column, container, row, scrollable, stack, text, text_editor,
};
use iced::{
    Alignment as IcedAlignment, Color, Element, Fill, Size, Subscription, Task, Theme, clipboard,
    event, theme,
};
use openpodium::domain::{
    Agent, AgentId, AgentProgram, CanvasLayout, CanvasPoint, CanvasSize, ChatAttachmentId,
    ChatDraft, ChatMessageId, ChatThread, ChatThreadId, CommandPreset, CommandPresetId,
    ContainerEnvironment, Content, CustomEnvironment, DomainCommand, EnvironmentKind,
    EnvironmentProfile, EnvironmentProfileId, Name, Node, NodeId, NodeTarget, ProjectPath, Role,
    RoleColor, RoleIcon, RoleId, Routine, RoutineApproval, RoutineCadence, RoutineRetryPolicy,
    RoutineSchedule, RoutineStep, RoutineStepClaims, RoutineStepId, RoutineTrigger,
    RoutineTriggerId, RoutineTriggerKind, RoutineVersion, SshEnvironment, ThreadColor,
    TimelineEventId, Timestamp, Workspace, WorkspaceDirectory, WorkspaceId,
};
use openpodium::ipc::{
    AGENT_ID_ENV, AVAILABLE_ENV, AgentCapabilities, AgentRegistration, CLI_ENV, ENDPOINT_ENV,
    IpcService, SUPPORTED_VERSIONS, TOKEN_ENV, VERSIONS_ENV, WORKSPACE_ID_ENV,
};
use openpodium::localization::{LOCALE_KEY, Locale, Localizer};
use openpodium::navigation::{CommandId, CommandRegistry, Shortcut};
use openpodium::orchestration::{DeliveryRequest, Orchestrator};
use openpodium::persistence::{
    ImportPreview, PointV1, export_canvas_fragment, export_role, import_canvas_fragment,
    import_role,
};
use openpodium::portal::{PortalAction, PortalFrame};
use openpodium::presentation::{
    HIGH_CONTRAST_KEY, PresentationPreferences, REDUCED_MOTION_KEY, TEXT_SCALE_KEY,
};
use openpodium::routines::{
    MissedOccurrences, RoutineDispatch, RoutineScheduler, RunRequest, TriggerEvent, TriggerWatcher,
};
use openpodium::runtime::{
    EnvironmentHealth, LocalProcessRuntime, ProcessEvent, ProcessRuntime, ProcessSpec,
    RuntimeError, check_agent_capability, check_environment, prepare_environment_process,
};
use openpodium::supervisor::{
    self, CollisionObservation, NotificationRateLimiter, WorkspaceActivity,
};
use openpodium::timeline::{self, NavigationTarget, RecoveryAction, TimelineItem};
use openpodium::workspaces::{WorkspaceManager, WorkspaceSettingsInput};
use tokio::sync::Mutex;

use crate::canvas::{self, Alignment, Camera, History, ZOrder};
use crate::chat::{self, AttachmentStore, LinkTarget};
use crate::navigation_panel;
use crate::notifications::NotificationRequest;
use crate::routines_panel;
use crate::supervisor_panel;
use crate::terminal;
use crate::terminal::session::{self, Action as TerminalAction, ProcessStream, Session};
use crate::timeline_panel;

use icons::Icon;
use ui::{
    action_grid, button, count_badge, float_row, labelled, panel_card, pill_divider,
    primary_button, rule, section, section_label, segment, segmented, swatch, text_input,
};

const APP_NAME: &str = "OpenPodium";
const DATABASE_FILE: &str = "openpodium.sqlite";
/// Orchestration ticks between terminal transcript captures. The tick runs at
/// 100ms, so transcripts are written about every three seconds while output is
/// arriving, rather than on every chunk.
const TRANSCRIPT_FLUSH_TICKS: u32 = 30;
/// Preference key for whether the workspace rail is showing.
const SIDEBAR_OPEN_KEY: &str = "sidebar_open";
/// The window the application opens at, and the size assumed until the first
/// resize event arrives.
const DEFAULT_WINDOW_SIZE: Size = Size::new(1_280.0, 820.0);

type TimelineState = (
    BTreeMap<WorkspaceId, Vec<TimelineItem>>,
    BTreeMap<WorkspaceId, TimelineEventId>,
);

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

#[derive(Debug, Clone, Copy)]
enum PortableImportKind {
    Template,
    WorkspaceArchive,
}

#[derive(Debug, Clone)]
struct PortableImportDraft {
    kind: PortableImportKind,
    workspace_id: WorkspaceId,
    destination_floor: Option<u64>,
    payload: String,
    preview: ImportPreview,
    launcher_mappings: BTreeMap<String, CommandPresetId>,
    path_mappings: BTreeMap<String, String>,
}

struct OpenPodium {
    localizer: Localizer,
    presentation: PresentationPreferences,
    controls: Option<Controls>,
    sidebar_open: bool,
    window_size: Size,
    floor_ui: floors::UiState,
    context_ui: context_nodes::UiState,
    portal_ui: portals::UiState,
    camera: Camera,
    canvas_selection: Vec<NodeId>,
    connection_mode: canvas::ConnectionMode,
    connection_error: Option<String>,
    canvas_preview: Option<CanvasLayout>,
    canvas_history: History,
    canvas_revision: u64,
    terminals: BTreeMap<TerminalKey, Session>,
    portal_frames: BTreeMap<NodeId, PortalFrame>,
    focused_terminal: Option<NodeId>,
    focused_portal: Option<NodeId>,
    terminal_generation: u64,
    transcript_ticks: u32,
    chat_ui: chat::UiState,
    timeline_ui: timeline_panel::UiState,
    timeline_items: BTreeMap<WorkspaceId, Vec<TimelineItem>>,
    timeline_high_watermarks: BTreeMap<WorkspaceId, TimelineEventId>,
    supervisor_ui: supervisor_panel::UiState,
    supervisor_snapshot: supervisor::Snapshot,
    notification_limiter: NotificationRateLimiter,
    supervisor_collisions: BTreeMap<WorkspaceId, Vec<CollisionObservation>>,
    navigation_ui: navigation_panel::UiState,
    command_registry: CommandRegistry,
    attachment_store: Option<AttachmentStore>,
    ipc: Option<IpcService>,
    orchestrator: Orchestrator,
    routines: RoutineScheduler,
    trigger_watcher: TriggerWatcher,
    routines_ui: routines_panel::UiState,
    workspaces: Option<WorkspaceManager>,
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
    selected_role: Option<RoleId>,
    editing_preset: Option<CommandPresetId>,
    preset_name: String,
    preset_executable: String,
    preset_arguments: String,
    editing_role: Option<RoleId>,
    role_name: String,
    role_color: String,
    role_icon: String,
    role_instructions: String,
    context_path: String,
    portable_import: Option<PortableImportDraft>,
    notice: Option<String>,
}

impl Default for OpenPodium {
    fn default() -> Self {
        let (mut workspaces, attachment_store, ipc, mut notice) = match application_database_path()
        {
            Ok(path) => {
                let attachment_store = path
                    .parent()
                    .map(|directory| AttachmentStore::new(directory.join("attachments")));
                let workspaces = WorkspaceManager::open(&path).map_err(|error| error.to_string());
                let ipc = path
                    .parent()
                    .ok_or_else(|| "application database has no parent directory".to_owned())
                    .and_then(|directory| {
                        IpcService::start(directory).map_err(|error| error.to_string())
                    });
                let notice = workspaces
                    .as_ref()
                    .err()
                    .cloned()
                    .or_else(|| ipc.as_ref().err().cloned());
                (workspaces.ok(), attachment_store, ipc.ok(), notice)
            }
            Err(error) => (None, None, None, Some(error.to_string())),
        };
        let orchestrator = match workspaces.as_mut() {
            Some(workspaces) => match Orchestrator::recover(workspaces, now()) {
                Ok(orchestrator) => orchestrator,
                Err(error) => {
                    notice = Some(error.to_string());
                    Orchestrator::default()
                }
            },
            None => Orchestrator::default(),
        };
        let routines = match workspaces.as_mut() {
            Some(workspaces) => match RoutineScheduler::recover(workspaces, now()) {
                Ok(scheduler) => scheduler,
                Err(error) => {
                    notice = Some(error.to_string());
                    RoutineScheduler::default()
                }
            },
            None => RoutineScheduler::default(),
        };
        let (timeline_items, timeline_high_watermarks) =
            match workspaces.as_ref().map(load_timeline_state) {
                Some(Ok(timeline)) => timeline,
                Some(Err(error)) => {
                    notice = Some(error);
                    Default::default()
                }
                None => Default::default(),
            };
        let mut command_registry = CommandRegistry::new();
        if let Some(workspaces) = workspaces.as_ref() {
            match workspaces.shortcuts() {
                Ok(shortcuts) => {
                    let warnings = command_registry.apply_stored(shortcuts);
                    if !warnings.is_empty() {
                        notice = Some(warnings.join("; "));
                    }
                }
                Err(error) => notice = Some(error.to_string()),
            }
        }
        let mut state = Self {
            localizer: Localizer::default(),
            presentation: PresentationPreferences::default(),
            controls: None,
            sidebar_open: true,
            window_size: DEFAULT_WINDOW_SIZE,
            floor_ui: floors::UiState::default(),
            context_ui: context_nodes::UiState::default(),
            portal_ui: portals::UiState::default(),
            camera: Camera::default(),
            canvas_selection: Vec::new(),
            connection_mode: canvas::ConnectionMode::Off,
            connection_error: None,
            canvas_preview: None,
            canvas_history: History::default(),
            canvas_revision: 1,
            terminals: BTreeMap::new(),
            portal_frames: BTreeMap::new(),
            focused_terminal: None,
            focused_portal: None,
            terminal_generation: 0,
            transcript_ticks: 0,
            chat_ui: chat::UiState::default(),
            timeline_ui: timeline_panel::UiState::default(),
            timeline_items,
            timeline_high_watermarks,
            supervisor_ui: supervisor_panel::UiState::default(),
            supervisor_snapshot: supervisor::Snapshot::default(),
            notification_limiter: NotificationRateLimiter::default(),
            supervisor_collisions: BTreeMap::new(),
            navigation_ui: navigation_panel::UiState::default(),
            command_registry,
            attachment_store,
            ipc,
            orchestrator,
            routines,
            trigger_watcher: TriggerWatcher::new(now()),
            routines_ui: routines_panel::UiState::default(),
            workspaces,
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
            selected_role: None,
            editing_preset: None,
            preset_name: String::new(),
            preset_executable: String::new(),
            preset_arguments: String::new(),
            editing_role: None,
            role_name: String::new(),
            role_color: RoleColor::DEFAULT.to_owned(),
            role_icon: RoleIcon::DEFAULT.to_owned(),
            role_instructions: String::new(),
            context_path: String::new(),
            portable_import: None,
            notice,
        };
        load_application_preferences(&mut state);
        let scale = state.presentation.text_scale();
        state.window_size = Size::new(
            DEFAULT_WINDOW_SIZE.width / scale,
            DEFAULT_WINDOW_SIZE.height / scale,
        );
        state.load_active_settings();
        state.restore_terminal_transcripts();
        state.sync_ipc_directory();
        refresh_supervisor_snapshot(&mut state);
        navigation::mark_all_stale(&mut state);
        state
    }
}

#[derive(Clone)]
enum Message {
    Floor(floors::Message),
    Portal(portals::Message),
    OrchestrationTick,
    Canvas(canvas::Message),
    CycleTextScale,
    CycleLocale,
    ToggleHighContrast,
    ToggleReducedMotion,
    OpenControls(Controls),
    CloseControls,
    TrackpadMagnified(f64),
    WindowResized(Size),
    ExecuteCommand(CommandId),
    Chat(chat::Message),
    Timeline(timeline_panel::Message),
    Supervisor(supervisor_panel::Message),
    Routines(routines_panel::Message),
    Navigation(navigation_panel::Message),
    NavigationKey {
        navigation_key: navigation::NavigationKey,
        shortcut: Option<Shortcut>,
        status: event::Status,
    },
    NotificationActivated(Option<NavigationTarget>),
    AddAgent(AgentProgram),
    PreviewAgent(AgentProgram),
    CanvasAction(CanvasAction),
    SaveSelectionAsTemplate,
    InstantiateTemplate,
    TemplateImportRead(Option<String>),
    ExportWorkspaceArchive,
    ImportWorkspaceArchive,
    WorkspaceArchiveImportRead(Option<String>),
    MapPortableLauncher {
        launcher_id: String,
        preset_id: CommandPresetId,
    },
    MapPortablePath {
        source: String,
        destination: String,
    },
    ConfirmPortableImport,
    CancelPortableImport,
    ContextPathChanged(String),
    AddContextNode(context_nodes::Kind),
    ContextScanCompleted(Result<context_nodes::ScanResult, String>),
    EditNote(text_editor::Action),
    SaveNote(bool),
    ReloadNote,
    SaveCanvasText,
    OpenProject,
    ProjectDirectorySelected(Option<PathBuf>),
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
    PresetNameChanged(String),
    PresetExecutableChanged(String),
    PresetArgumentsChanged(String),
    SavePreset,
    EditPreset(CommandPresetId),
    RemovePreset(CommandPresetId),
    SelectRole(Option<RoleId>),
    AssignRoleToSelected(Option<RoleId>),
    RoleNameChanged(String),
    RoleColorChanged(String),
    RoleIconChanged(String),
    RoleInstructionsChanged(String),
    SaveRole,
    EditRole(RoleId),
    RemoveRole(RoleId),
    ExportRole(RoleId),
    ImportRole,
    RoleImportRead(Option<String>),
    CanvasFragmentRead(Option<String>),
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Controls {
    Menu,
    Advanced,
    Workspace,
    Canvas,
    Agents,
    Environments,
    Worktrees,
    Portals,
    Routines,
    Tasks,
    Notifications,
    Node,
}

impl Controls {
    fn title(self) -> &'static str {
        match self {
            Self::Menu => "Workspace menu",
            Self::Advanced => "More tools",
            Self::Workspace => "Workspace settings",
            Self::Canvas => "Canvas tools",
            Self::Agents => "Agent setup",
            Self::Environments => "Runtime environments",
            Self::Worktrees => "Worktrees",
            Self::Portals => "Browser portals",
            Self::Routines => "Routines",
            Self::Tasks => "Task actions",
            Self::Notifications => "Notifications",
            Self::Node => "Selected node",
        }
    }
}

pub(crate) fn run() -> iced::Result {
    trackpad::install();
    iced::application(OpenPodium::default, update, view)
        .title(APP_NAME)
        .theme(|state: &OpenPodium| application_theme(state.presentation))
        .scale_factor(|state: &OpenPodium| state.presentation.text_scale())
        .window(iced::window::Settings {
            size: DEFAULT_WINDOW_SIZE,
            min_size: Some(Size::new(900.0, 620.0)),
            ..iced::window::Settings::default()
        })
        .subscription(|_| {
            Subscription::batch([
                iced::time::every(Duration::from_millis(100)).map(|_| Message::OrchestrationTick),
                navigation::subscription(),
                iced::window::resize_events().map(|(_, size)| Message::WindowResized(size)),
                trackpad::subscription().map(Message::TrackpadMagnified),
            ])
        })
        .centered()
        .run()
}

fn application_theme(preferences: PresentationPreferences) -> Theme {
    if preferences.high_contrast() {
        Theme::custom(
            "OpenPodium high contrast",
            theme::Palette {
                background: Color::BLACK,
                text: Color::WHITE,
                primary: Color::from_rgb8(0, 255, 255),
                success: Color::from_rgb8(0, 255, 0),
                warning: Color::from_rgb8(255, 255, 0),
                danger: Color::from_rgb8(255, 96, 96),
            },
        )
    } else {
        shell::theme()
    }
}

fn load_application_preferences(state: &mut OpenPodium) {
    let Some(workspaces) = state.workspaces.as_ref() else {
        return;
    };
    match workspaces.preferences() {
        Ok(values) => {
            if let Some((_, locale)) = values.iter().find(|(key, _)| key == LOCALE_KEY) {
                state.localizer = Localizer::new(Locale::from_tag(locale));
            }
            if let Some((_, open)) = values.iter().find(|(key, _)| key == SIDEBAR_OPEN_KEY) {
                state.sidebar_open = open != "false";
            }
            state.presentation.apply_stored(values);
        }
        Err(error) => state.notice = Some(error.to_string()),
    }
}

/// The area the canvas sheet occupies, derived from the window and the chrome
/// this layout places beside it.
fn canvas_viewport(state: &OpenPodium) -> (f64, f64) {
    let mut width = state.window_size.width - shell::STAGE_PADDING * 2.0;
    if state.sidebar_open {
        width -= shell::SIDEBAR_WIDTH + 1.0;
    }
    (
        f64::from(width.max(1.0)),
        f64::from((state.window_size.height - shell::STAGE_PADDING * 2.0).max(1.0)),
    )
}

/// Shows or hides the workspace rail, remembering the choice for next launch.
fn toggle_sidebar(state: &mut OpenPodium) -> Task<Message> {
    state.sidebar_open = !state.sidebar_open;
    let value = state.sidebar_open.to_string();
    persist_application_preference(state, SIDEBAR_OPEN_KEY, &value);
    Task::none()
}

fn persist_application_preference(state: &mut OpenPodium, key: &str, value: &str) {
    let Some(workspaces) = state.workspaces.as_mut() else {
        return;
    };
    if let Err(error) = workspaces.store_preference(key, value) {
        state.notice = Some(error.to_string());
    }
}

fn update(state: &mut OpenPodium, message: Message) -> Task<Message> {
    match message {
        Message::CycleTextScale => {
            let previous_scale = state.presentation.text_scale();
            state.presentation.cycle_text_scale();
            let ratio = previous_scale / state.presentation.text_scale();
            state.window_size = Size::new(
                state.window_size.width * ratio,
                state.window_size.height * ratio,
            );
            let value = state.presentation.text_scale().to_string();
            persist_application_preference(state, TEXT_SCALE_KEY, &value);
        }
        Message::CycleLocale => {
            state.localizer.cycle_user_locale();
            let value = state.localizer.locale().tag();
            persist_application_preference(state, LOCALE_KEY, value);
        }
        Message::ToggleHighContrast => {
            state.presentation.toggle_high_contrast();
            state.canvas_revision = state.canvas_revision.wrapping_add(1);
            let value = state.presentation.high_contrast().to_string();
            persist_application_preference(state, HIGH_CONTRAST_KEY, &value);
        }
        Message::ToggleReducedMotion => {
            state.presentation.toggle_reduced_motion();
            let value = state.presentation.reduced_motion().to_string();
            persist_application_preference(state, REDUCED_MOTION_KEY, &value);
        }
        Message::OpenControls(controls) => {
            state.cancel_connection();
            state.focused_terminal = None;
            state.focused_portal = None;
            state.controls = Some(controls);
            if controls == Controls::Node {
                return context_nodes::open_editor(state);
            }
        }
        Message::CloseControls => state.controls = None,
        Message::TrackpadMagnified(delta) => {
            if let Some(factor) = trackpad::magnification_factor(delta) {
                state.camera = state.camera.zoom_centered(factor);
                state.canvas_revision = state.canvas_revision.wrapping_add(1);
            }
        }
        Message::ExecuteCommand(command) => return navigation::execute_command(state, command),
        Message::Floor(message) => return floors::update(state, message),
        Message::Portal(message) => return portals::update(state, message),
        Message::WindowResized(size) => state.window_size = size,
        Message::OrchestrationTick => {
            state.transcript_ticks = state.transcript_ticks.wrapping_add(1);
            if state
                .transcript_ticks
                .is_multiple_of(TRANSCRIPT_FLUSH_TICKS)
            {
                state.flush_terminal_transcripts();
            }
            run_orchestration_tick(state);
            run_routine_tick(state);
            let contexts = context_nodes::tick(state);
            let timelines = refresh_timelines(state);
            let floors = floors::tick(state);
            let navigation = navigation::tick(state);
            let portals = portals::tick(state);
            return Task::batch([timelines, floors, contexts, navigation, portals]);
        }
        Message::Canvas(message) => return handle_canvas_message(state, message),
        Message::SaveSelectionAsTemplate => return save_selection_as_template(state),
        Message::InstantiateTemplate => {
            return clipboard::read().map(Message::TemplateImportRead);
        }
        Message::TemplateImportRead(payload) => preview_template_from_clipboard(state, payload),
        Message::ExportWorkspaceArchive => return export_workspace_archive_to_clipboard(state),
        Message::ImportWorkspaceArchive => {
            return clipboard::read().map(Message::WorkspaceArchiveImportRead);
        }
        Message::WorkspaceArchiveImportRead(payload) => {
            preview_workspace_archive_from_clipboard(state, payload);
        }
        Message::MapPortableLauncher {
            launcher_id,
            preset_id,
        } => {
            if let Some(draft) = state.portable_import.as_mut() {
                draft.launcher_mappings.insert(launcher_id, preset_id);
            }
        }
        Message::MapPortablePath {
            source,
            destination,
        } => {
            if let Some(draft) = state.portable_import.as_mut() {
                if destination.trim().is_empty() {
                    draft.path_mappings.remove(&source);
                } else {
                    draft.path_mappings.insert(source, destination);
                }
            }
        }
        Message::ConfirmPortableImport => confirm_portable_import(state),
        Message::CancelPortableImport => state.portable_import = None,
        Message::Chat(message) => return handle_chat_message(state, message),
        Message::Timeline(message) => return handle_timeline_message(state, message),
        Message::Supervisor(message) => state.supervisor_ui.update(message),
        Message::Routines(message) => return handle_routines_message(state, message),
        Message::Navigation(message) => return navigation::update(state, message),
        Message::NavigationKey {
            navigation_key,
            shortcut,
            status,
        } => return navigation::handle_key(state, navigation_key, shortcut, status),
        Message::NotificationActivated(Some(target)) => navigate_to_task(state, target),
        Message::NotificationActivated(None) => {}
        Message::AddAgent(program) => return add_agent(state, program),
        Message::PreviewAgent(program) => preview_agent(state, program),
        Message::CanvasAction(action) => apply_canvas_action(state, action),
        Message::ContextPathChanged(value) => state.context_path = value,
        Message::AddContextNode(kind) => return context_nodes::add(state, kind),
        Message::ContextScanCompleted(result) => context_nodes::scan_completed(state, result),
        Message::EditNote(action) => context_nodes::edit_note(state, action),
        Message::SaveNote(overwrite) => context_nodes::save_note(state, overwrite),
        Message::ReloadNote => context_nodes::reload_note(state),
        Message::SaveCanvasText => context_nodes::save_canvas_text(state),
        Message::OpenProject => {
            let title = state.localizer.text("open-project-dialog-title");
            return Task::perform(
                async move {
                    rfd::AsyncFileDialog::new()
                        .set_title(title)
                        .pick_folder()
                        .await
                        .map(|directory| directory.path().to_path_buf())
                },
                Message::ProjectDirectorySelected,
            );
        }
        Message::ProjectDirectorySelected(Some(directory)) => {
            create_workspace(state, &directory);
        }
        Message::ProjectDirectorySelected(None) => {}
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
        Message::PresetNameChanged(value) => state.preset_name = value,
        Message::PresetExecutableChanged(value) => state.preset_executable = value,
        Message::PresetArgumentsChanged(value) => state.preset_arguments = value,
        Message::RoleNameChanged(value) => state.role_name = value,
        Message::RoleColorChanged(value) => state.role_color = value,
        Message::RoleIconChanged(value) => state.role_icon = value,
        Message::RoleInstructionsChanged(value) => state.role_instructions = value,
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
                    floors::workspace_changed(state);
                    state.reset_canvas_session();
                    state.load_active_settings();
                    state.restore_terminal_transcripts();
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
        Message::SavePreset => save_preset(state),
        Message::EditPreset(preset_id) => edit_preset(state, preset_id),
        Message::RemovePreset(preset_id) => remove_preset(state, preset_id),
        Message::SelectRole(role_id) => state.selected_role = role_id,
        Message::AssignRoleToSelected(role_id) => assign_role_to_selected(state, role_id),
        Message::SaveRole => save_role(state),
        Message::EditRole(role_id) => edit_role(state, role_id),
        Message::RemoveRole(role_id) => remove_role(state, role_id),
        Message::ExportRole(role_id) => return export_role_to_clipboard(state, role_id),
        Message::ImportRole => return clipboard::read().map(Message::RoleImportRead),
        Message::RoleImportRead(payload) => import_role_from_clipboard(state, payload.as_deref()),
        Message::CanvasFragmentRead(payload) => {
            paste_canvas_fragment(state, payload.as_deref());
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
            if let (Some(session), Some(contents)) = (state.terminals.get(&key), contents)
                && let Err(error) = session.paste(&contents)
            {
                state.notice = Some(error);
            }
        }
    }
    Task::none()
}

fn create_workspace(state: &mut OpenPodium, directory: &Path) {
    let result = state
        .workspaces
        .as_mut()
        .ok_or_else(|| "workspace storage is unavailable".to_owned())
        .and_then(|workspaces| {
            workspaces
                .create_workspace(directory, now())
                .map_err(|error| error.to_string())
        });
    match result {
        Ok(workspace_id) => {
            state.notice = Some(state.localizer.text("workspace-created"));
            state.navigation_ui.mark_stale(workspace_id);
            floors::workspace_changed(state);
            state.reset_canvas_session();
            state.load_active_settings();
            state.restore_terminal_transcripts();
            state.sync_ipc_directory();
        }
        Err(error) => state.notice = Some(error),
    }
}

/// The control that shows and hides the workspace rail.
fn sidebar_toggle(state: &OpenPodium) -> Element<'_, Message> {
    let shortcut = state
        .command_registry
        .binding(CommandId::ToggleSidebar)
        .map_or_else(String::new, |shortcut| {
            format!(
                " ({})",
                shortcut.display(openpodium::navigation::Platform::current())
            )
        });
    labelled(
        icons::control(Icon::Sidebar)
            .style(shell::navigation_button(!state.sidebar_open))
            .on_press(Message::ExecuteCommand(CommandId::ToggleSidebar)),
        format!(
            "{}{shortcut}",
            state.localizer.text(if state.sidebar_open {
                "hide-sidebar"
            } else {
                "show-sidebar"
            })
        ),
    )
}

fn view(state: &OpenPodium) -> Element<'_, Message> {
    let active_workspace_id = state
        .workspaces
        .as_ref()
        .and_then(WorkspaceManager::active_workspace)
        .map(Workspace::id);
    let has_active_workspace = active_workspace_id.is_some();
    let palette_shortcut = state
        .command_registry
        .binding(openpodium::navigation::CommandId::OpenPalette)
        .map_or_else(
            || state.localizer.text("unbound"),
            |shortcut| shortcut.display(openpodium::navigation::Platform::current()),
        );
    let brand = row![
        container(text("O").size(14))
            .width(26)
            .height(26)
            .align_x(IcedAlignment::Center)
            .align_y(IcedAlignment::Center)
            .style(shell::app_mark),
        text(state.localizer.text("app-name"))
            .size(13)
            .width(Fill)
            .wrapping(iced::widget::text::Wrapping::None),
        labelled(
            icons::control(Icon::Plus).on_press(Message::OpenProject),
            state.localizer.text(if has_active_workspace {
                "open-another-project"
            } else {
                "open-project"
            }),
        ),
        sidebar_toggle(state),
    ]
    .spacing(4)
    .align_y(IcedAlignment::Center);
    // Styled as a recessed field rather than a button: it is where you go to
    // type, even though typing happens in the palette it opens.
    let search = button(
        container(
            row![
                icons::view(Icon::Search),
                text(state.localizer.text("search-short"))
                    .size(11)
                    .style(shell::muted_text)
                    .width(Fill)
                    .wrapping(iced::widget::text::Wrapping::None),
                text(palette_shortcut).size(10).style(shell::subtle_text),
            ]
            .spacing(8)
            .align_y(IcedAlignment::Center),
        )
        .style(shell::field)
        .padding([7, 8])
        .width(Fill)
        .clip(true),
    )
    .style(shell::utility_button)
    .padding(0)
    .on_press(Message::Navigation(navigation_panel::Message::Open))
    .width(Fill);
    let mut workspace_list = column![section_label(state.localizer.text("workspaces"))]
        .spacing(2)
        .width(Fill);
    if let Some(workspaces) = &state.workspaces {
        for workspace in workspaces.ordered_workspaces() {
            let icon = workspace.settings().icon().map_or("", |icon| icon.as_str());
            let workspace_icon: Element<'_, Message> = if icon.is_empty() {
                icons::view(Icon::Files)
            } else {
                text(icon.to_owned()).size(13).into()
            };
            let attention = timeline::attention_counts(workspace).total();
            let agents = workspace.agents().count();
            let selected = active_workspace_id == Some(workspace.id());
            // Agent count on the right tells you how loaded a workspace is;
            // the red badge only shows up when something wants you.
            let mut entry = row![
                workspace_icon,
                text(workspace.name().to_owned())
                    .size(13)
                    .width(Fill)
                    .wrapping(iced::widget::text::Wrapping::None),
            ]
            .spacing(8)
            .align_y(IcedAlignment::Center);
            if attention > 0 {
                entry = entry.push(count_badge(attention, true));
            }
            if agents > 0 {
                entry = entry.push(
                    row![
                        icons::view(Icon::Terminal),
                        text(agents.to_string()).size(11).style(shell::subtle_text),
                    ]
                    .spacing(3)
                    .align_y(IcedAlignment::Center),
                );
            }
            workspace_list = workspace_list.push(
                button(entry)
                    .style(shell::navigation_button(selected))
                    .padding([7, 8])
                    .on_press(Message::SwitchWorkspace(workspace.id()))
                    .width(Fill),
            );
        }
    }

    let enabled = |enabled| {
        state
            .localizer
            .text(if enabled { "state-on" } else { "state-off" })
    };
    // Display preferences stay reachable but sit at footnote weight, so the
    // rail never competes with the canvas for attention.
    let preference = |label: String, message: Message| {
        button(text(label).size(12))
            .style(shell::utility_button)
            .padding([5, 8])
            .on_press(message)
            .width(Fill)
    };
    let accessibility_controls = column![
        rule(),
        section_label(state.localizer.text("display-settings")),
        row![
            preference(
                state
                    .localizer
                    .text(match state.localizer.locale() {
                        Locale::EnUs | Locale::PseudoRtl => "locale-short-en",
                        Locale::PtBr => "locale-short-pt",
                    })
                    .to_string(),
                Message::CycleLocale,
            ),
            preference(
                format!("{}%", state.presentation.text_scale_percent()),
                Message::CycleTextScale,
            ),
        ]
        .spacing(4),
        row![
            preference(
                state
                    .localizer
                    .with_str(
                        "contrast-short",
                        "state",
                        enabled(state.presentation.high_contrast()),
                    )
                    .to_string(),
                Message::ToggleHighContrast,
            ),
            preference(
                state
                    .localizer
                    .with_str(
                        "motion-short",
                        "state",
                        enabled(state.presentation.reduced_motion()),
                    )
                    .to_string(),
                Message::ToggleReducedMotion,
            ),
        ]
        .spacing(4),
    ]
    .spacing(6);
    let sidebar = container(
        column![
            brand,
            search,
            scrollable(workspace_list).height(Fill),
            accessibility_controls,
        ]
        .spacing(10)
        .height(Fill),
    )
    .style(shell::sidebar)
    .width(shell::SIDEBAR_WIDTH)
    .height(Fill)
    .padding(10);

    let small = |label: &'static str, message: Message| {
        button(text(label).size(12))
            .padding([6, 10])
            .on_press(message)
            .into()
    };
    let mut settings = match state.controls {
        Some(Controls::Workspace) => column![section(
            "Details",
            column![
                text_input("Name", &state.name).on_input(Message::NameChanged),
                text_input("Icon", &state.icon).on_input(Message::IconChanged),
                text_input("Working directory", &state.working_directory)
                    .on_input(Message::WorkingDirectoryChanged),
                text_input("Workspace instructions", &state.instructions)
                    .on_input(Message::InstructionsChanged),
                primary_button(text(state.localizer.text("save-workspace")).size(13))
                    .on_press(Message::SaveSettings),
            ]
            .spacing(8),
        ),],
        Some(Controls::Agents) => column![
            section(
                "Add agent",
                column![
                    action_grid([
                        small("Codex", Message::AddAgent(AgentProgram::Codex)),
                        small("Claude", Message::AddAgent(AgentProgram::Claude)),
                        small("OpenCode", Message::AddAgent(AgentProgram::OpenCode)),
                        small("Shell", Message::AddAgent(AgentProgram::Shell)),
                    ]),
                    section_label("Preview without adding"),
                    action_grid([
                        small("Codex", Message::PreviewAgent(AgentProgram::Codex)),
                        small("Claude", Message::PreviewAgent(AgentProgram::Claude)),
                        small("OpenCode", Message::PreviewAgent(AgentProgram::OpenCode)),
                        small("Shell", Message::PreviewAgent(AgentProgram::Shell)),
                    ]),
                    rule(),
                    text(format!(
                        "Codex {} · Claude {} · OpenCode {} · shell {}",
                        capability_label(AgentProgram::Codex, None),
                        capability_label(AgentProgram::Claude, None),
                        capability_label(AgentProgram::OpenCode, None),
                        capability_label(AgentProgram::Shell, None),
                    ))
                    .size(11)
                    .style(shell::muted_text),
                ]
                .spacing(8),
            ),
            section(
                "Runtime environment for new agents",
                button(text("Local workspace").size(13))
                    .style(shell::navigation_button(
                        state.selected_environment.is_none()
                    ))
                    .width(Fill)
                    .on_press(Message::SelectEnvironment(None)),
            ),
        ],
        Some(Controls::Canvas) => column![
            section(
                "Canvas tools",
                column![
                    action_grid([
                        button(text("Undo").size(12))
                            .on_press_maybe(
                                state
                                    .canvas_history
                                    .can_undo()
                                    .then_some(Message::CanvasAction(CanvasAction::Undo))
                            )
                            .into(),
                        button(text("Redo").size(12))
                            .on_press_maybe(
                                state
                                    .canvas_history
                                    .can_redo()
                                    .then_some(Message::CanvasAction(CanvasAction::Redo))
                            )
                            .into(),
                        small("Duplicate", Message::CanvasAction(CanvasAction::Duplicate)),
                        small("Delete", Message::CanvasAction(CanvasAction::Remove)),
                        small("Copy", Message::Canvas(canvas::Message::CopyRequested)),
                        small("Paste", Message::Canvas(canvas::Message::PasteRequested)),
                        small("Group", Message::CanvasAction(CanvasAction::Group)),
                        small("Ungroup", Message::CanvasAction(CanvasAction::Ungroup)),
                        small("Connect", Message::CanvasAction(CanvasAction::Connect)),
                        small(
                            "To front",
                            Message::CanvasAction(CanvasAction::ZOrder(ZOrder::Front))
                        ),
                        small(
                            "To back",
                            Message::CanvasAction(CanvasAction::ZOrder(ZOrder::Back))
                        ),
                    ]),
                    section_label("Align and distribute"),
                    action_grid([
                        small(
                            "Align X",
                            Message::CanvasAction(CanvasAction::Align(
                                Alignment::HorizontalCenters
                            )),
                        ),
                        small(
                            "Align Y",
                            Message::CanvasAction(CanvasAction::Align(Alignment::VerticalCenters)),
                        ),
                        small(
                            "Space X",
                            Message::CanvasAction(CanvasAction::Align(
                                Alignment::DistributeHorizontally
                            )),
                        ),
                        small(
                            "Space Y",
                            Message::CanvasAction(CanvasAction::Align(
                                Alignment::DistributeVertically
                            )),
                        ),
                    ]),
                    section_label("Templates and archives"),
                    action_grid([
                        small("Save template", Message::SaveSelectionAsTemplate),
                        small("Use template", Message::InstantiateTemplate),
                        small("Export archive", Message::ExportWorkspaceArchive),
                        small("Import archive", Message::ImportWorkspaceArchive),
                    ]),
                ]
                .spacing(8),
            ),
            section(
                "Context and drawing nodes",
                column![
                    action_grid([
                        small("Note", Message::AddContextNode(context_nodes::Kind::Note)),
                        small(
                            "Files",
                            Message::AddContextNode(context_nodes::Kind::FileTree)
                        ),
                        small("Text", Message::AddContextNode(context_nodes::Kind::Text)),
                        small(
                            "Rectangle",
                            Message::AddContextNode(context_nodes::Kind::Rectangle)
                        ),
                        small(
                            "Ellipse",
                            Message::AddContextNode(context_nodes::Kind::Ellipse)
                        ),
                        small("Arrow", Message::AddContextNode(context_nodes::Kind::Arrow)),
                        small(
                            "Freehand",
                            Message::AddContextNode(context_nodes::Kind::Freehand)
                        ),
                    ]),
                    text_input("Project-relative file path", &state.context_path)
                        .on_input(Message::ContextPathChanged),
                    action_grid([
                        small(
                            "Artifact",
                            Message::AddContextNode(context_nodes::Kind::Artifact)
                        ),
                        small("Diff", Message::AddContextNode(context_nodes::Kind::Diff)),
                    ]),
                ]
                .spacing(8),
            ),
        ],
        Some(Controls::Worktrees) => column![panel_card(floors::view(state))],
        Some(Controls::Portals) => column![panel_card(portals::creation_view(state))],
        _ => column![],
    }
    .spacing(12)
    .max_width(720);
    if state.controls == Some(Controls::Canvas)
        && let Some(draft) = &state.portable_import
    {
        let kind = match draft.kind {
            PortableImportKind::Template => "template",
            PortableImportKind::WorkspaceArchive => "workspace archive",
        };
        settings = settings
            .push(text(format!(
                "Import preview: {kind} · {} roles · {} agents · {} tasks · {} handoffs · {} nodes · {} groups · {} connections",
                draft.preview.counts.roles,
                draft.preview.counts.agents,
                draft.preview.counts.tasks,
                draft.preview.counts.handoffs,
                draft.preview.counts.nodes,
                draft.preview.counts.groups,
                draft.preview.counts.connections,
            )))
            .push(text(if draft.preview.unresolved_launchers.is_empty() {
                "All launchers resolved"
            } else {
                "Custom launchers require an explicit mapping before import"
            }));
        let destination = state
            .workspaces
            .as_ref()
            .and_then(|manager| manager.workspace(draft.workspace_id));
        if draft.preview.referenced_paths.is_empty() {
            settings = settings.push(text("No project paths referenced"));
        } else {
            settings = settings.push(text("Referenced project paths:"));
            for source in &draft.preview.referenced_paths {
                let mapped = draft
                    .path_mappings
                    .get(source)
                    .map(String::as_str)
                    .unwrap_or(source.as_str());
                let status = destination
                    .map(|workspace| portable_path_status(workspace, mapped))
                    .unwrap_or_else(|| "destination unavailable".to_owned());
                let source_for_message = source.clone();
                settings = settings.push(
                    row![
                        text(format!("{source} → {mapped} · {status}")),
                        text_input("Destination project-relative path", mapped).on_input(
                            move |destination| Message::MapPortablePath {
                                source: source_for_message.clone(),
                                destination,
                            },
                        ),
                    ]
                    .spacing(8),
                );
            }
        }
        for conflict in &draft.preview.role_conflicts {
            settings = settings.push(text(format!("Role conflict: {conflict}")));
        }
        for warning in &draft.preview.warnings {
            settings = settings.push(text(format!("Warning: {warning}")));
        }
        let presets = state
            .workspaces
            .as_ref()
            .and_then(WorkspaceManager::active_workspace)
            .map(|workspace| {
                workspace
                    .command_presets()
                    .map(|preset| (preset.id(), preset.name().as_str().to_owned()))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for launcher_id in &draft.preview.unresolved_launchers {
            settings = settings.push(text(format!("Resolve {launcher_id} with a custom preset:")));
            for (preset_id, preset_name) in &presets {
                let selected = draft.launcher_mappings.get(launcher_id) == Some(preset_id);
                settings = settings.push(
                    button(text(if selected {
                        format!("✓ {preset_name}")
                    } else {
                        preset_name.clone()
                    }))
                    .on_press(Message::MapPortableLauncher {
                        launcher_id: launcher_id.clone(),
                        preset_id: *preset_id,
                    }),
                );
            }
        }
        settings = settings.push(
            row![
                button("Confirm import").on_press(Message::ConfirmPortableImport),
                button("Cancel").on_press(Message::CancelPortableImport),
            ]
            .spacing(8),
        );
    }
    if let Some(workspace) = state
        .workspaces
        .as_ref()
        .and_then(WorkspaceManager::active_workspace)
    {
        if state.controls == Some(Controls::Tasks) {
            settings = settings
                .push(timeline_panel::panel(workspace, &state.timeline_ui).map(Message::Timeline));
        }
        if state.controls == Some(Controls::Routines) {
            settings = settings
                .push(routines_panel::panel(workspace, &state.routines_ui).map(Message::Routines));
        }
    }
    if let Some(workspace) = state
        .workspaces
        .as_ref()
        .and_then(WorkspaceManager::active_workspace)
    {
        // Roles. Each role collapses to a single selectable row; its actions
        // only unfold for the selected one, so a long list stays scannable.
        let mut roles = column![action_grid([
            small("No role", Message::SelectRole(None)),
            small("Clear on agent", Message::AssignRoleToSelected(None)),
            small("Import role", Message::ImportRole),
        ])]
        .spacing(6);
        for role in workspace.roles() {
            let selected = state.selected_role == Some(role.id());
            roles = roles.push(
                button(
                    row![
                        swatch(role.color().as_str()),
                        text(role.icon().to_string()).size(13),
                        text(role.name().to_string()).size(13).width(Fill),
                    ]
                    .spacing(8)
                    .align_y(IcedAlignment::Center),
                )
                .style(shell::navigation_button(selected))
                .padding([7, 9])
                .width(Fill)
                .on_press(Message::SelectRole(Some(role.id()))),
            );
            if selected {
                roles = roles.push(action_grid([
                    small("Assign", Message::AssignRoleToSelected(Some(role.id()))),
                    small("Edit", Message::EditRole(role.id())),
                    small("Export", Message::ExportRole(role.id())),
                    button(text("Delete").size(12))
                        .style(shell::danger_button)
                        .padding([6, 10])
                        .on_press(Message::RemoveRole(role.id()))
                        .into(),
                ]));
            }
        }
        roles = roles
            .push(rule())
            .push(section_label(if state.editing_role.is_some() {
                "Edit role"
            } else {
                "New role"
            }))
            .push(text_input("Role name", &state.role_name).on_input(Message::RoleNameChanged))
            .push(
                row![
                    text_input("#RRGGBB", &state.role_color).on_input(Message::RoleColorChanged),
                    text_input("Icon", &state.role_icon).on_input(Message::RoleIconChanged),
                ]
                .spacing(6),
            )
            .push(
                text_input("Role instructions", &state.role_instructions)
                    .on_input(Message::RoleInstructionsChanged),
            )
            .push(
                primary_button(
                    text(if state.editing_role.is_some() {
                        "Update role"
                    } else {
                        "Create role"
                    })
                    .size(13),
                )
                .on_press(Message::SaveRole),
            );
        if state.controls == Some(Controls::Agents) {
            settings = settings.push(section("Reusable roles", roles));
        }

        let mut presets = column![].spacing(6);
        for preset in workspace.command_presets() {
            let program = AgentProgram::Custom(preset.id());
            presets = presets
                .push(
                    row![
                        text(preset.name().to_string()).size(13).width(Fill),
                        text(capability_label(program, Some(preset)))
                            .size(11)
                            .style(shell::muted_text),
                    ]
                    .spacing(8)
                    .align_y(IcedAlignment::Center),
                )
                .push(action_grid([
                    small("Add", Message::AddAgent(program)),
                    small("Preview", Message::PreviewAgent(program)),
                    small("Edit", Message::EditPreset(preset.id())),
                    button(text("Delete").size(12))
                        .style(shell::danger_button)
                        .padding([6, 10])
                        .on_press(Message::RemovePreset(preset.id()))
                        .into(),
                ]));
        }
        presets = presets
            .push(rule())
            .push(section_label(if state.editing_preset.is_some() {
                "Edit preset"
            } else {
                "New preset"
            }))
            .push(
                text_input("Preset name", &state.preset_name).on_input(Message::PresetNameChanged),
            )
            .push(
                text_input("Executable", &state.preset_executable)
                    .on_input(Message::PresetExecutableChanged),
            )
            .push(
                text_input("Arguments as a JSON array", &state.preset_arguments)
                    .on_input(Message::PresetArgumentsChanged),
            )
            .push(
                primary_button(
                    text(if state.editing_preset.is_some() {
                        "Update preset"
                    } else {
                        "Create preset"
                    })
                    .size(13),
                )
                .on_press(Message::SavePreset),
            );
        if state.controls == Some(Controls::Agents) {
            settings = settings.push(section("Custom command presets", presets));
        }

        let mut environments = column![].spacing(6);
        for profile in workspace.environment_profiles() {
            let selected = state.selected_environment == Some(profile.id());
            let health = state
                .environment_health
                .get(&(workspace.id(), profile.id()))
                .copied()
                .unwrap_or_default();
            environments = environments.push(
                button(
                    row![
                        text(profile.name().to_string()).size(13).width(Fill),
                        text(health.label()).size(11).style(shell::muted_text),
                    ]
                    .spacing(8)
                    .align_y(IcedAlignment::Center),
                )
                .style(shell::navigation_button(selected))
                .padding([7, 9])
                .width(Fill)
                .on_press(Message::SelectEnvironment(Some(profile.id()))),
            );
            if selected {
                environments = environments.push(action_grid([
                    small("Check", Message::CheckEnvironment(profile.id())),
                    button(text("Delete").size(12))
                        .style(shell::danger_button)
                        .padding([6, 10])
                        .on_press(Message::RemoveEnvironment(profile.id()))
                        .into(),
                ]));
            }
        }
        // A segmented picker makes the three profile kinds mutually exclusive
        // at a glance, instead of three lookalike buttons with check marks.
        let kind_picker = segmented([
            segment("SSH", state.environment_kind == EnvironmentDraftKind::Ssh)
                .on_press(Message::EnvironmentKindSelected(EnvironmentDraftKind::Ssh))
                .into(),
            segment(
                "Container",
                state.environment_kind == EnvironmentDraftKind::Container,
            )
            .on_press(Message::EnvironmentKindSelected(
                EnvironmentDraftKind::Container,
            ))
            .into(),
            segment(
                "Custom",
                state.environment_kind == EnvironmentDraftKind::Custom,
            )
            .on_press(Message::EnvironmentKindSelected(
                EnvironmentDraftKind::Custom,
            ))
            .into(),
        ]);
        let mut draft = column![
            rule(),
            section_label("Add environment profile"),
            kind_picker,
            text_input("Profile name", &state.environment_name)
                .on_input(Message::EnvironmentNameChanged),
        ]
        .spacing(6);
        draft = match state.environment_kind {
            EnvironmentDraftKind::Ssh => draft
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
            EnvironmentDraftKind::Container => draft
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
            EnvironmentDraftKind::Custom => draft
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
        environments = environments.push(draft).push(
            primary_button(text("Add environment").size(13)).on_press(Message::CreateEnvironment),
        );
        if state.controls == Some(Controls::Environments) {
            settings = settings.push(section("Environments", environments));
        }
    }
    if !has_active_workspace {
        settings = column![
            text(state.localizer.text("create-first-workspace")).size(28),
            text(state.localizer.text("create-first-workspace-detail")),
        ]
        .spacing(12)
        .max_width(720);
    }
    if state.controls == Some(Controls::Node)
        && let Some(panel) = context_nodes::note_panel(&state.context_ui)
    {
        settings = settings.push(panel);
    }
    let editing_note = state.controls == Some(Controls::Node)
        && context_nodes::editor_title(&state.context_ui).is_some();
    if !editing_note && let Some(notice) = &state.notice {
        settings = settings.push(
            container(text(notice).size(12))
                .style(shell::section_card)
                .padding(12)
                .width(Fill),
        );
    }
    if state.controls == Some(Controls::Node)
        && let Some((workspace_id, agent_id)) = selected_agent(state)
        && let Some(workspace) = state
            .workspaces
            .as_ref()
            .and_then(|workspaces| workspaces.workspace(workspace_id))
    {
        settings = settings.push(panel_card(
            chat::conversation_panel(
                workspace,
                agent_id,
                &state.chat_ui,
                state.attachment_store.as_ref(),
            )
            .map(Message::Chat),
        ));
    }
    if state.controls == Some(Controls::Node)
        && let Some(node_id) = selected_terminal_node(state)
    {
        let session = state
            .terminals
            .get(&active_terminal_key(state, node_id).expect("an active workspace exists"));
        let active = session.is_some_and(Session::is_active);
        settings = settings.push(section(
            "Terminal",
            if active {
                primary_button(text("Stop terminal").size(13))
                    .on_press(Message::StopTerminal(node_id))
            } else if session.is_some() {
                primary_button(text("Reconnect terminal").size(13))
                    .on_press(Message::StartTerminal(node_id))
            } else {
                primary_button(text("Start terminal").size(13))
                    .on_press(Message::StartTerminal(node_id))
            },
        ));
    }
    if state.controls == Some(Controls::Node)
        && let Some(portal) = portals::selected_view(state)
    {
        settings = settings.push(panel_card(portal));
    }
    if has_active_workspace && state.controls == Some(Controls::Notifications) {
        settings =
            settings.push(supervisor_panel::panel(&state.supervisor_ui).map(Message::Supervisor));
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
        let document =
            canvas::CanvasDocument::new(workspace, layout, terminal_views, &state.localizer)
                .with_git_severity(floors::node_severities(state))
                .with_context_bodies(context_nodes::bodies(&state.context_ui))
                .with_portal_frames(state.portal_frames.clone());
        let canvas = canvas::view(
            state.camera,
            document,
            canvas::Interaction {
                selection: state.canvas_selection.clone(),
                focused_terminal: state.focused_terminal,
                focused_portal: state.focused_portal,
                connection_mode: state.connection_mode,
            },
            [
                CommandId::OpenPalette,
                CommandId::FocusCanvas,
                CommandId::ZoomIn,
                CommandId::ZoomOut,
                CommandId::ResetZoom,
            ]
            .into_iter()
            .filter_map(|command| state.command_registry.binding(command).cloned())
            .collect(),
            state.canvas_revision,
        )
        .map(Message::Canvas);
        let workspace_title = workspace.name().to_owned();

        let workspace_chip = labelled(
            iced_button(
                container(
                    row![
                        icons::view(Icon::Files),
                        text(workspace_title)
                            .size(11)
                            .style(shell::muted_text)
                            .wrapping(iced::widget::text::Wrapping::None),
                    ]
                    .spacing(6)
                    .align_y(IcedAlignment::Center),
                )
                .style(shell::floating_chip)
                .padding([7, 10])
                .clip(true),
            )
            .style(shell::utility_button)
            .padding(0)
            .on_press(Message::OpenControls(Controls::Menu)),
            format!("Workspace menu · {}", state.working_directory),
        );

        let add_agent = |icon, label: &'static str, program: AgentProgram| {
            labelled(
                icons::control(icon).on_press(Message::AddAgent(program)),
                label,
            )
        };
        let add_node = |icon, label: &'static str, kind: context_nodes::Kind| {
            labelled(
                icons::control(icon).on_press(Message::AddContextNode(kind)),
                label,
            )
        };
        let agent_tools = row![
            add_agent(Icon::Code, "Add Codex", AgentProgram::Codex),
            add_agent(Icon::Spark, "Add Claude", AgentProgram::Claude),
            add_agent(Icon::Agent, "Add OpenCode", AgentProgram::OpenCode),
            add_agent(Icon::Terminal, "Add shell", AgentProgram::Shell),
        ]
        .spacing(1)
        .align_y(IcedAlignment::Center);
        let canvas_tools = row![
            add_node(Icon::Note, "Add note", context_nodes::Kind::Note),
            add_node(Icon::Files, "Add file tree", context_nodes::Kind::FileTree),
            add_node(Icon::Text, "Add text", context_nodes::Kind::Text),
            labelled(
                icons::control(Icon::Link)
                    .style(shell::navigation_button(
                        state.connection_mode != canvas::ConnectionMode::Off
                    ))
                    .on_press(Message::CanvasAction(CanvasAction::Connect)),
                "Connect cards · choose a source and destination",
            ),
        ]
        .spacing(1)
        .align_y(IcedAlignment::Center);
        let tools: Element<'_, Message> = if canvas_viewport(state).0 < 560.0 {
            column![agent_tools, canvas_tools].spacing(2).into()
        } else {
            row![agent_tools, pill_divider(), canvas_tools]
                .spacing(5)
                .align_y(IcedAlignment::Center)
                .into()
        };
        let tool_pill = container(tools).style(shell::floating_pill).padding(3);

        let zoom_pill = container(
            row![
                labelled(
                    icons::control(Icon::Minus)
                        .on_press(Message::ExecuteCommand(CommandId::ZoomOut)),
                    CommandId::ZoomOut.label(),
                ),
                iced_button(text(format!("{}%", state.camera.zoom_percent())).size(12))
                    .style(shell::utility_button)
                    .padding([7, 6])
                    .on_press(Message::ExecuteCommand(CommandId::ResetZoom)),
                labelled(
                    icons::control(Icon::Plus).on_press(Message::ExecuteCommand(CommandId::ZoomIn)),
                    CommandId::ZoomIn.label(),
                ),
            ]
            .spacing(1)
            .align_y(IcedAlignment::Center),
        )
        .style(shell::floating_pill)
        .padding(3);

        let rail_toggle = if state.sidebar_open {
            iced::widget::Space::new().into()
        } else {
            container(sidebar_toggle(state))
                .style(shell::floating_pill)
                .padding(3)
                .into()
        };
        let mut node_actions = row![].spacing(4).align_y(IcedAlignment::Center);
        if let Some(node_id) = selected_terminal_node(state) {
            let running = active_terminal_key(state, node_id)
                .and_then(|key| state.terminals.get(&key))
                .is_some_and(Session::is_active);
            node_actions = node_actions
                .push(
                    button(
                        text(if running {
                            "Stop terminal"
                        } else {
                            "Start terminal"
                        })
                        .size(12),
                    )
                    .on_press(if running {
                        Message::StopTerminal(node_id)
                    } else {
                        Message::StartTerminal(node_id)
                    }),
                )
                .push(
                    button(text("Chat").size(12)).on_press(Message::OpenControls(Controls::Node)),
                );
        } else if state.canvas_selection.len() == 1
            && state
                .canvas_selection
                .first()
                .and_then(|id| workspace.node(*id))
                .is_some_and(|node| {
                    matches!(
                        node.content(),
                        openpodium::domain::CanvasNodeContent::Note { .. }
                            | openpodium::domain::CanvasNodeContent::Text { .. }
                            | openpodium::domain::CanvasNodeContent::Portal(_)
                    )
                })
        {
            node_actions = node_actions.push(
                button(text("Edit selected node").size(12))
                    .on_press(Message::OpenControls(Controls::Node)),
            );
        }
        let connection_hint = (state.connection_mode != canvas::ConnectionMode::Off).then(|| {
            container(
                row![
                    text(state.connection_error.as_deref().unwrap_or(
                        match state.connection_mode {
                            canvas::ConnectionMode::PickSource =>
                                "Click or drag from a card to connect",
                            _ => "Choose the destination card",
                        }
                    ))
                    .size(12),
                    button(text("Cancel · Esc").size(12))
                        .on_press(Message::Canvas(canvas::Message::CancelConnection)),
                ]
                .spacing(10)
                .align_y(IcedAlignment::Center),
            )
            .style(shell::floating_pill)
            .padding([6, 10])
        });
        let overlay = column![
            float_row(
                rail_toggle,
                tool_pill.into(),
                iced::widget::Space::new().into()
            ),
            container(column![connection_hint]).center_x(Fill),
            iced::widget::Space::new().width(Fill).height(Fill),
            container(node_actions).center_x(Fill),
            row![container(workspace_chip).width(Fill).clip(true), zoom_pill,]
                .spacing(10)
                .align_y(IcedAlignment::Center),
        ]
        .spacing(10)
        .height(Fill)
        .padding(10);

        let canvas_sheet = container(stack![container(canvas).width(Fill).height(Fill), overlay,])
            .width(Fill)
            .height(Fill)
            .style(shell::canvas_surface)
            .clip(true);

        container(canvas_sheet)
            .style(shell::canvas)
            .width(Fill)
            .height(Fill)
            .padding(shell::STAGE_PADDING)
            .into()
    } else {
        let mut empty_content = column![
            container(text("⌘").size(24))
                .width(56)
                .height(56)
                .align_x(IcedAlignment::Center)
                .align_y(IcedAlignment::Center)
                .style(shell::app_mark),
            text(state.localizer.text("empty-eyebrow"))
                .size(11)
                .style(shell::muted_text),
            text(state.localizer.text("create-first-workspace")).size(32),
            text(state.localizer.text("create-first-workspace-detail"))
                .size(15)
                .style(shell::muted_text),
            primary_button(
                container(text(state.localizer.text("open-project")).size(14))
                    .width(Fill)
                    .align_x(IcedAlignment::Center),
            )
            .padding([12, 18])
            .on_press(Message::OpenProject)
            .width(Fill),
        ]
        .spacing(14)
        .align_x(IcedAlignment::Center);
        if let Some(notice) = &state.notice {
            empty_content = empty_content.push(text(notice).size(12));
        }
        let empty_card = container(empty_content)
            .style(shell::card)
            .width(Fill)
            .max_width(620)
            .padding(44);
        let empty_stage = container(empty_card)
            .style(shell::canvas)
            .width(Fill)
            .height(Fill)
            .center(Fill)
            .padding(40);
        if state.sidebar_open {
            empty_stage.into()
        } else {
            // The first-run view has no canvas chip to carry the toggle, so it
            // gets its own rather than stranding the rail behind a shortcut.
            stack![
                empty_stage,
                container(sidebar_toggle(state))
                    .width(Fill)
                    .height(Fill)
                    .align_x(IcedAlignment::Start)
                    .align_y(IcedAlignment::Start)
                    .padding(14),
            ]
            .into()
        }
    };

    // A hairline is the only thing separating the rail from the stage; both
    // share the same chrome grey so neither reads as a heavier slab.
    let application: Element<'_, Message> = if state.sidebar_open {
        row![
            sidebar,
            container(iced::widget::Space::new().width(1).height(Fill))
                .style(shell::rule)
                .width(1)
                .height(Fill),
            stage,
        ]
        .height(Fill)
        .into()
    } else {
        stage
    };
    let application = if let Some(controls) = state.controls {
        if matches!(controls, Controls::Menu | Controls::Advanced) {
            let choices: &[Controls] = if controls == Controls::Menu {
                &[Controls::Workspace, Controls::Advanced]
            } else {
                &[
                    Controls::Canvas,
                    Controls::Agents,
                    Controls::Environments,
                    Controls::Worktrees,
                    Controls::Portals,
                    Controls::Routines,
                    Controls::Tasks,
                    Controls::Notifications,
                ]
            };
            settings = column(choices.iter().map(|choice| {
                button(text(choice.title()).size(13))
                    .style(shell::utility_button)
                    .width(Fill)
                    .on_press(Message::OpenControls(*choice))
                    .into()
            }))
            .spacing(4);
        }
        let scrim = iced::widget::mouse_area(
            container(iced::widget::Space::new().width(Fill).height(Fill))
                .width(Fill)
                .height(Fill)
                .style(shell::scrim),
        )
        .on_press(Message::CloseControls);
        let height = (state.window_size.height - 100.0).clamp(180.0, 600.0);
        let body: Element<'_, Message> = if matches!(controls, Controls::Menu | Controls::Advanced)
        {
            settings.into()
        } else {
            container(scrollable(settings).height(iced::Shrink))
                .max_height(height)
                .into()
        };
        let dialog = container(
            column![
                row![
                    text(if editing_note {
                        context_nodes::editor_title(&state.context_ui).unwrap_or(controls.title())
                    } else {
                        controls.title()
                    })
                    .size(18)
                    .width(Fill),
                    button(text("Close").size(12)).on_press(Message::CloseControls),
                ]
                .align_y(IcedAlignment::Center)
                .spacing(12),
                body,
            ]
            .spacing(14),
        )
        .style(shell::card)
        .padding(18)
        .width(Fill)
        .max_width(if matches!(controls, Controls::Menu | Controls::Advanced) {
            340.0
        } else {
            620.0
        });
        stack![
            application,
            scrim,
            container(iced::widget::opaque(dialog))
                .center(Fill)
                .padding(20)
        ]
        .into()
    } else {
        application
    };
    if state.navigation_ui.open {
        stack![
            application,
            navigation_panel::view(&state.navigation_ui, &state.command_registry)
                .map(Message::Navigation),
        ]
        .into()
    } else {
        application
    }
}

impl OpenPodium {
    fn cancel_connection(&mut self) {
        self.connection_mode = canvas::ConnectionMode::Off;
        self.connection_error = None;
        self.canvas_revision = self.canvas_revision.wrapping_add(1);
    }

    fn reset_canvas_session(&mut self) {
        self.cancel_connection();
        self.controls = None;
        self.focused_terminal = None;
        self.focused_portal = None;
        self.portal_frames.clear();
        self.camera = Camera::default();
        self.canvas_selection.clear();
        self.canvas_preview = None;
        self.canvas_history.clear();
        self.canvas_revision = self.canvas_revision.wrapping_add(1);
    }

    /// Rebuilds offline terminal views for the active workspace from stored
    /// transcripts, so a reopened node shows the output it had. Nodes with a
    /// live session keep it; nothing is started here.
    fn restore_terminal_transcripts(&mut self) {
        let Some(workspaces) = self.workspaces.as_ref() else {
            return;
        };
        let Some(workspace) = workspaces.active_workspace() else {
            return;
        };
        let workspace_id = workspace.id();
        let sizes = workspace
            .canvas_layout()
            .nodes()
            .iter()
            .filter(|node| matches!(node.reference(), Some(NodeTarget::Agent(_))))
            .map(|node| {
                (
                    node.id(),
                    terminal::GridSize::for_node(node.size().width(), node.size().height()),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let Ok(transcripts) = workspaces.terminal_transcripts(workspace_id) else {
            return;
        };

        let mut orphans = Vec::new();
        for (node_id, payload) in transcripts {
            let raw_node_id = node_id;
            let node_id = NodeId::new(node_id);
            let Some(size) = sizes.get(&node_id).copied() else {
                // The node is gone and undo history does not survive a reload,
                // so nothing can bring it back to claim this transcript.
                orphans.push(raw_node_id);
                continue;
            };
            let key = TerminalKey {
                workspace_id,
                node_id,
            };
            if self.terminals.contains_key(&key) {
                continue;
            }
            self.terminal_generation = self.terminal_generation.wrapping_add(1);
            self.terminals.insert(
                key,
                Session::restored(size, self.terminal_generation, payload),
            );
        }

        if let Some(workspaces) = self.workspaces.as_mut() {
            for node_id in orphans {
                let _ = workspaces.clear_terminal_transcript(workspace_id, node_id);
            }
        }
        self.canvas_revision = self.canvas_revision.wrapping_add(1);
    }

    /// Writes every transcript that changed since the last capture.
    fn flush_terminal_transcripts(&mut self) {
        let pending = self
            .terminals
            .iter_mut()
            .filter_map(|(key, session)| {
                session
                    .take_transcript()
                    .map(|transcript| (*key, transcript.to_vec()))
            })
            .collect::<Vec<_>>();
        if pending.is_empty() {
            return;
        }
        let Some(workspaces) = self.workspaces.as_mut() else {
            return;
        };
        let captured_at = now();
        for (key, payload) in pending {
            let _ = workspaces.store_terminal_transcript(
                key.workspace_id,
                key.node_id.get(),
                captured_at,
                &payload,
            );
        }
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
            self.selected_role = None;
            clear_preset_draft(self);
            clear_role_draft(self);
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
        self.chat_ui.sync(workspace);
        self.selected_environment = None;
        self.selected_role = None;
        clear_preset_draft(self);
        clear_role_draft(self);
    }

    fn sync_ipc_directory(&self) {
        let (Some(ipc), Some(workspaces)) = (&self.ipc, &self.workspaces) else {
            return;
        };
        for workspace in workspaces.recent_workspaces() {
            ipc.replace_workspace_agents(
                workspace.id().get(),
                workspace.agents().map(|agent| AgentRegistration {
                    id: agent.id().get(),
                    name: agent.name().as_str().to_owned(),
                    program: agent.program().label().to_owned(),
                    state: agent.state().to_string(),
                    capabilities: if agent.environment_id().is_none()
                        && supports_automatic_delivery(agent.program())
                    {
                        AgentCapabilities::CONNECTED
                    } else {
                        AgentCapabilities::UNAVAILABLE
                    },
                }),
            );
        }
    }
}

impl Drop for OpenPodium {
    fn drop(&mut self) {
        portals::shutdown(self);
        // Capture before stopping: stopping drops the sessions holding them.
        self.flush_terminal_transcripts();
        self.stop_all_terminals();
    }
}

fn load_timeline_state(workspaces: &WorkspaceManager) -> Result<TimelineState, String> {
    let mut items = BTreeMap::new();
    let mut high_watermarks = BTreeMap::new();
    for workspace in workspaces.recent_workspaces() {
        let events = workspaces
            .timeline(workspace.id())
            .map_err(|error| error.to_string())?;
        if let Some(event) = events.last() {
            high_watermarks.insert(workspace.id(), event.id());
        }
        items.insert(workspace.id(), timeline::project(workspace, &events));
    }
    Ok((items, high_watermarks))
}

fn refresh_supervisor_snapshot(state: &mut OpenPodium) {
    let Some(workspaces) = state.workspaces.as_ref() else {
        state.supervisor_snapshot = supervisor::Snapshot::default();
        return;
    };
    state.supervisor_snapshot = supervisor::aggregate(
        workspaces
            .recent_workspaces()
            .map(|workspace| WorkspaceActivity {
                workspace,
                timeline: state
                    .timeline_items
                    .get(&workspace.id())
                    .map_or(&[][..], Vec::as_slice),
            }),
        state.supervisor_collisions.values().flatten().cloned(),
    )
}

fn refresh_timelines(state: &mut OpenPodium) -> Task<Message> {
    let workspace_ids = state
        .workspaces
        .as_ref()
        .map(|workspaces| {
            workspaces
                .recent_workspaces()
                .map(Workspace::id)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let mut notifications = Vec::new();
    let mut supervisor_changed = false;
    for workspace_id in workspace_ids {
        let update = (|| -> Result<_, String> {
            let workspaces = state
                .workspaces
                .as_ref()
                .ok_or_else(|| "workspace storage is unavailable".to_owned())?;
            let events = workspaces
                .timeline_after(
                    workspace_id,
                    state.timeline_high_watermarks.get(&workspace_id).copied(),
                )
                .map_err(|error| error.to_string())?;
            let workspace = workspaces
                .workspace(workspace_id)
                .ok_or_else(|| format!("workspace {workspace_id} is unavailable"))?;
            let items = timeline::project(workspace, &events);
            let requests = items
                .iter()
                .filter_map(|item| {
                    let class = supervisor::notification_class(item)?;
                    Some((
                        NotificationRequest {
                            target: item.task_id().and_then(|task_id| {
                                timeline::navigation_target(workspace, task_id)
                            }),
                            title: item.title().to_owned(),
                            body: item.detail().to_owned(),
                        },
                        class,
                        item.occurred_at(),
                    ))
                })
                .collect::<Vec<_>>();
            Ok((events.last().map(|event| event.id()), items, requests))
        })();
        let (high_watermark, items, requests) = match update {
            Ok(update) => update,
            Err(error) => {
                state.notice = Some(format!("Timeline refresh failed: {error}"));
                continue;
            }
        };
        if let Some(high_watermark) = high_watermark {
            state
                .timeline_high_watermarks
                .insert(workspace_id, high_watermark);
        }
        state
            .timeline_items
            .entry(workspace_id)
            .or_default()
            .extend(items);
        supervisor_changed |= high_watermark.is_some();
        notifications.extend(
            requests
                .into_iter()
                .filter_map(|(request, class, occurred_at)| {
                    state
                        .notification_limiter
                        .should_send(
                            workspace_id,
                            class,
                            occurred_at,
                            state.supervisor_ui.notification_settings(),
                        )
                        .then_some(request)
                }),
        );
    }
    if supervisor_changed {
        refresh_supervisor_snapshot(state);
    }
    Task::batch(notifications.into_iter().map(|request| {
        Task::perform(
            crate::notifications::show(request),
            Message::NotificationActivated,
        )
    }))
}

fn handle_timeline_message(
    state: &mut OpenPodium,
    message: timeline_panel::Message,
) -> Task<Message> {
    let Some(workspace_id) = active_workspace_id(state) else {
        state.notice = Some("Create or select a workspace first".to_owned());
        return Task::none();
    };
    match message {
        timeline_panel::Message::Filter(task_id) => {
            state.timeline_ui.select_task(workspace_id, task_id);
        }
        timeline_panel::Message::Inspect(task_id) => {
            let target = state
                .workspaces
                .as_ref()
                .and_then(|workspaces| workspaces.workspace(workspace_id))
                .and_then(|workspace| timeline::navigation_target(workspace, task_id));
            if let Some(target) = target {
                navigate_to_task(state, target);
            } else {
                state.notice = Some(format!("Task {task_id} is unavailable"));
            }
        }
        timeline_panel::Message::Recover { task_id, action } => {
            let result = state
                .workspaces
                .as_mut()
                .ok_or_else(|| "workspace storage is unavailable".to_owned())
                .and_then(|workspaces| {
                    match action {
                        RecoveryAction::Retry => state
                            .orchestrator
                            .retry_task(workspaces, workspace_id, task_id, now())
                            .map(|retry_id| format!("Task retried as {retry_id}")),
                        RecoveryAction::Cancel => state
                            .orchestrator
                            .cancel_task(workspaces, workspace_id, task_id, now())
                            .map(|()| "Task cancelled".to_owned()),
                        RecoveryAction::Resume => state
                            .orchestrator
                            .resume_task(workspaces, workspace_id, task_id, now())
                            .map(|()| "Task resumed".to_owned()),
                        RecoveryAction::Inspect => unreachable!("inspect has a dedicated message"),
                    }
                    .map_err(|error| error.to_string())
                });
            state.notice = Some(result.unwrap_or_else(|error| error));
            return refresh_timelines(state);
        }
    }
    Task::none()
}

fn navigate_to_task(state: &mut OpenPodium, target: NavigationTarget) {
    if active_workspace_id(state) != Some(target.workspace_id) {
        let result = state
            .workspaces
            .as_mut()
            .expect("a notification target requires workspace storage")
            .switch(target.workspace_id, now());
        if let Err(error) = result {
            state.notice = Some(error.to_string());
            return;
        }
        state.reset_canvas_session();
        state.load_active_settings();
        state.restore_terminal_transcripts();
    }
    state
        .timeline_ui
        .select_task(target.workspace_id, Some(target.task_id));
    let current_node = state
        .workspaces
        .as_ref()
        .and_then(|workspaces| workspaces.workspace(target.workspace_id))
        .and_then(|workspace| timeline::navigation_target(workspace, target.task_id))
        .and_then(|target| target.node_id);
    if let Some(node) = current_node {
        let floor = state
            .workspaces
            .as_ref()
            .and_then(|m| m.workspace(target.workspace_id))
            .and_then(|w| w.floors().node_floors.get(&node).copied());
        if let Some(manager) = state.workspaces.as_mut()
            && let Err(error) = manager.switch_floor(target.workspace_id, floor, now())
        {
            state.notice = Some(error);
            return;
        }
        state.reset_canvas_session();
    }
    state.canvas_selection = current_node.into_iter().collect();
    state.focused_terminal = None;
    state.canvas_revision = state.canvas_revision.wrapping_add(1);
    state.notice = Some(format!("Inspecting task {}", target.task_id));
}

fn handle_routines_message(
    state: &mut OpenPodium,
    message: routines_panel::Message,
) -> Task<Message> {
    let Some(workspace_id) = active_workspace_id(state) else {
        state.notice = Some("Create or select a workspace first".to_owned());
        return Task::none();
    };
    match message {
        routines_panel::Message::SelectRoutine(routine_id) => {
            if let Some(workspace) = state
                .workspaces
                .as_ref()
                .and_then(|manager| manager.workspace(workspace_id))
            {
                state.routines_ui.select(workspace, routine_id);
            }
        }
        routines_panel::Message::EditName(value) => state.routines_ui.name = value,
        routines_panel::Message::EditStepName(value) => state.routines_ui.step_name = value,
        routines_panel::Message::EditStepPrompt(value) => state.routines_ui.step_prompt = value,
        routines_panel::Message::EditStepAgent(value) => state.routines_ui.step_agent = value,
        routines_panel::Message::EditInput { key, value } => {
            state.routines_ui.inputs.insert(key, value);
        }
        routines_panel::Message::SelectRun(run_id) => state.routines_ui.selected_run = run_id,
        routines_panel::Message::AddStep => {
            let result = state
                .routines_ui
                .step_agent
                .trim()
                .parse::<u64>()
                .map_err(|_| "Agent ID must be a number".to_owned())
                .and_then(|agent| {
                    if state.routines_ui.step_name.trim().is_empty()
                        || state.routines_ui.step_prompt.trim().is_empty()
                    {
                        return Err("A step needs a name and a prompt".to_owned());
                    }
                    Ok(routines_panel::PendingStep {
                        name: state.routines_ui.step_name.trim().to_owned(),
                        prompt: state.routines_ui.step_prompt.trim().to_owned(),
                        agent_id: AgentId::new(agent),
                    })
                });
            match result {
                Ok(step) => {
                    state.routines_ui.pending_steps.push(step);
                    state.routines_ui.step_name.clear();
                    state.routines_ui.step_prompt.clear();
                }
                Err(error) => state.notice = Some(error),
            }
        }
        routines_panel::Message::ToggleCanvasTemplate => {
            state.routines_ui.capture_canvas = !state.routines_ui.capture_canvas;
            if state.routines_ui.capture_canvas && state.canvas_selection.is_empty() {
                state.notice = Some("Select the canvas nodes each run should rebuild".to_owned());
            }
        }
        routines_panel::Message::SaveRoutine => {
            state.notice = Some(save_routine(state, workspace_id).unwrap_or_else(|error| error));
            return refresh_timelines(state);
        }
        routines_panel::Message::StartRun => {
            let Some(routine_id) = state.routines_ui.selected_routine else {
                state.notice = Some("Select a routine to run".to_owned());
                return Task::none();
            };
            let request = RunRequest {
                inputs: state.routines_ui.input_values(),
                ..RunRequest::default()
            };
            let result = state
                .workspaces
                .as_mut()
                .ok_or_else(|| "workspace storage is unavailable".to_owned())
                .and_then(|workspaces| {
                    state
                        .routines
                        .start_run(workspaces, workspace_id, routine_id, request, now())
                        .map_err(|error| error.to_string())
                });
            state.notice = Some(match result {
                Ok(run_id) => format!("Routine started as run {run_id}"),
                Err(error) => error,
            });
            return refresh_timelines(state);
        }
        routines_panel::Message::ToggleTrigger {
            trigger_id,
            enabled,
        } => {
            state.notice = Some(
                toggle_trigger(state, workspace_id, trigger_id, enabled).unwrap_or_else(|e| e),
            );
            return refresh_timelines(state);
        }
        routines_panel::Message::AddScheduleTrigger => {
            state.notice = Some(
                add_trigger(state, workspace_id, ScheduleOrWatch::Schedule).unwrap_or_else(|e| e),
            );
            return refresh_timelines(state);
        }
        routines_panel::Message::AddFilesystemTrigger => {
            state.notice = Some(
                add_trigger(state, workspace_id, ScheduleOrWatch::Filesystem).unwrap_or_else(|e| e),
            );
            return refresh_timelines(state);
        }
        routines_panel::Message::Approve {
            run_id,
            step_id,
            approved,
        } => {
            let result = state
                .workspaces
                .as_mut()
                .ok_or_else(|| "workspace storage is unavailable".to_owned())
                .and_then(|workspaces| {
                    state
                        .routines
                        .decide_approval(
                            workspaces,
                            workspace_id,
                            run_id,
                            step_id,
                            routines_panel::decision(approved),
                            None,
                            now(),
                        )
                        .map_err(|error| error.to_string())
                });
            state.notice = Some(result.map_or_else(
                |error| error,
                |()| {
                    if approved {
                        "Step approved".to_owned()
                    } else {
                        "Step rejected".to_owned()
                    }
                },
            ));
            return refresh_timelines(state);
        }
        routines_panel::Message::ResolveInterruption { run_id, step_id } => {
            let result = state
                .workspaces
                .as_mut()
                .ok_or_else(|| "workspace storage is unavailable".to_owned())
                .and_then(|workspaces| {
                    state
                        .routines
                        .resolve_interruption(workspaces, workspace_id, run_id, step_id, now())
                        .map_err(|error| error.to_string())
                });
            state.notice = Some(result.map_or_else(
                |error| error,
                |()| "Interruption resolved; the step can run again".to_owned(),
            ));
            return refresh_timelines(state);
        }
        routines_panel::Message::CancelRun(run_id) => {
            let result = match state.workspaces.as_mut() {
                Some(workspaces) => state
                    .routines
                    .cancel_run(
                        workspaces,
                        &mut state.orchestrator,
                        workspace_id,
                        run_id,
                        now(),
                    )
                    .map_err(|error| error.to_string()),
                None => Err("workspace storage is unavailable".to_owned()),
            };
            state.notice = Some(result.map_or_else(
                |error| error,
                |()| "Cancellation requested; steps stay reserved until confirmed".to_owned(),
            ));
            return refresh_timelines(state);
        }
    }
    Task::none()
}

enum ScheduleOrWatch {
    Schedule,
    Filesystem,
}

/// Saves the drafted routine. A routine that already exists gains a version;
/// the stored one keeps running unchanged for any run that pinned it.
fn save_routine(state: &mut OpenPodium, workspace_id: WorkspaceId) -> Result<String, String> {
    let name =
        Name::new(state.routines_ui.name.trim().to_owned()).map_err(|error| error.to_string())?;
    if state.routines_ui.capture_canvas && state.canvas_selection.is_empty() {
        return Err("Select the canvas nodes each run should rebuild".to_owned());
    }
    let selection = state.canvas_selection.clone();
    let workspaces = state
        .workspaces
        .as_mut()
        .ok_or_else(|| "workspace storage is unavailable".to_owned())?;
    let workspace = workspaces
        .workspace(workspace_id)
        .ok_or_else(|| "workspace is not loaded".to_owned())?;
    let existing = state
        .routines_ui
        .selected_routine
        .and_then(|id| workspace.routine(id))
        .cloned();
    if existing.is_none() && state.routines_ui.pending_steps.is_empty() {
        return Err("Add at least one step before saving".to_owned());
    }
    let (next_routine_id, next_version_id, _) = openpodium::routines::next_routine_ids(workspace);

    // An edit appends to the latest immutable version. Preserve every saved
    // step verbatim so the basic panel cannot erase bindings, outputs,
    // approvals, retry policies, resource claims, or an existing DAG.
    let mut steps = existing.as_ref().map_or_else(Vec::new, |routine| {
        routine.latest_version().steps().to_vec()
    });
    let mut previous = steps.last().map(RoutineStep::id);
    let mut next_step_id = steps.iter().map(|step| step.id().get()).max().unwrap_or(0);
    for pending in &state.routines_ui.pending_steps {
        next_step_id = next_step_id
            .checked_add(1)
            .ok_or_else(|| "Routine step IDs are exhausted".to_owned())?;
        let step_id = RoutineStepId::new(next_step_id);
        steps.push(
            RoutineStep::new(
                step_id,
                Name::new(pending.name.clone()).map_err(|error| error.to_string())?,
                pending.agent_id,
                Content::new(pending.prompt.clone()).map_err(|error| error.to_string())?,
                previous,
                [],
                [],
                RoutineApproval::NotRequired,
                RoutineRetryPolicy::default(),
                RoutineStepClaims::default(),
            )
            .map_err(|error| error.to_string())?,
        );
        previous = Some(step_id);
    }

    let routine_id = existing.as_ref().map_or(next_routine_id, Routine::id);
    let number = existing
        .as_ref()
        .map_or(1, |routine| routine.latest_version().number() + 1);
    // Capturing the selection now freezes the arrangement into the version, so
    // later canvas edits cannot change what an already-saved version rebuilds.
    let template = if state.routines_ui.capture_canvas {
        let document = workspaces
            .export_template(workspace_id, &selection)
            .map_err(|error| error.to_string())?;
        Some(Content::new(document).map_err(|error| error.to_string())?)
    } else {
        existing
            .as_ref()
            .and_then(|routine| routine.latest_version().template().cloned())
    };
    let inputs = existing.as_ref().map_or_else(Vec::new, |routine| {
        routine.latest_version().inputs().to_vec()
    });
    let version = RoutineVersion::new(
        next_version_id,
        routine_id,
        number,
        inputs,
        steps,
        template,
        now(),
    )
    .map_err(|error| error.to_string())?;

    let commands = if let Some(routine) = existing {
        let mut commands = Vec::with_capacity(2);
        if routine.name() != &name {
            commands.push(DomainCommand::UpdateRoutine {
                routine_id,
                name,
                description: routine.description().cloned(),
            });
        }
        commands.push(DomainCommand::AddRoutineVersion {
            routine_id,
            version,
        });
        commands
    } else {
        vec![DomainCommand::AddRoutine(
            Routine::new(routine_id, name, None, version).map_err(|error| error.to_string())?,
        )]
    };
    workspaces
        .execute_batch(workspace_id, commands, now())
        .map_err(|error| error.to_string())?;
    state.routines_ui.pending_steps.clear();
    if let Some(workspace) = state
        .workspaces
        .as_ref()
        .and_then(|manager| manager.workspace(workspace_id))
    {
        state.routines_ui.select(workspace, Some(routine_id));
    }
    Ok(format!("Saved routine {routine_id} version {number}"))
}

fn toggle_trigger(
    state: &mut OpenPodium,
    workspace_id: WorkspaceId,
    trigger_id: RoutineTriggerId,
    enabled: bool,
) -> Result<String, String> {
    let workspaces = state
        .workspaces
        .as_mut()
        .ok_or_else(|| "workspace storage is unavailable".to_owned())?;
    let workspace = workspaces
        .workspace(workspace_id)
        .ok_or_else(|| "workspace is not loaded".to_owned())?;
    let routine = openpodium::routines::routine_of_trigger(workspace, trigger_id)
        .ok_or_else(|| format!("trigger {trigger_id} does not exist"))?;
    let routine_id = routine.id();
    let trigger = routine
        .trigger(trigger_id)
        .expect("the trigger was just located on this routine")
        .clone()
        .with_enabled(enabled);
    workspaces
        .execute(
            workspace_id,
            DomainCommand::PutRoutineTrigger {
                routine_id,
                trigger,
            },
            now(),
        )
        .map_err(|error| error.to_string())?;
    Ok(if enabled {
        "Trigger enabled".to_owned()
    } else {
        "Trigger disabled".to_owned()
    })
}

fn add_trigger(
    state: &mut OpenPodium,
    workspace_id: WorkspaceId,
    kind: ScheduleOrWatch,
) -> Result<String, String> {
    let routine_id = state
        .routines_ui
        .selected_routine
        .ok_or_else(|| "Select a routine first".to_owned())?;
    let workspaces = state
        .workspaces
        .as_mut()
        .ok_or_else(|| "workspace storage is unavailable".to_owned())?;
    let workspace = workspaces
        .workspace(workspace_id)
        .ok_or_else(|| "workspace is not loaded".to_owned())?;
    let (_, _, trigger_id) = openpodium::routines::next_routine_ids(workspace);
    let (name, kind) = match kind {
        ScheduleOrWatch::Schedule => (
            "Hourly",
            RoutineTriggerKind::Schedule(
                RoutineSchedule::new(
                    RoutineCadence::Hourly { minute: 0 },
                    0,
                    None,
                    RoutineCadence::Hourly { minute: 0 }.next_occurrence(now(), 0),
                )
                .map_err(|error| error.to_string())?,
            ),
        ),
        ScheduleOrWatch::Filesystem => (
            "Watched files",
            RoutineTriggerKind::Filesystem {
                patterns: vec!["**/*".to_owned()],
                debounce_ms: 2_000,
            },
        ),
    };
    let trigger = RoutineTrigger::new(
        trigger_id,
        routine_id,
        Name::new(name).map_err(|error| error.to_string())?,
        kind,
        true,
        BTreeMap::new(),
    )
    .map_err(|error| error.to_string())?;
    workspaces
        .execute(
            workspace_id,
            DomainCommand::PutRoutineTrigger {
                routine_id,
                trigger,
            },
            now(),
        )
        .map_err(|error| error.to_string())?;
    Ok(format!("Added trigger {trigger_id}"))
}

/// Observes triggers, starts the runs they justify, and advances every run.
///
/// Trigger events are advisory: consuming the occurrence and creating the run
/// happen together inside `start_run`, so a duplicate observation is refused
/// there rather than producing a second run here.
fn run_routine_tick(state: &mut OpenPodium) {
    let current_time = now();
    let Some(workspaces) = state.workspaces.as_mut() else {
        return;
    };
    let poll = state.trigger_watcher.poll(workspaces, current_time);
    for warning in poll.warnings {
        state.notice = Some(format!(
            "Routine {} trigger {} was not scanned: {}",
            warning.routine_id, warning.trigger_id, warning.detail
        ));
    }
    for missed in poll.missed {
        record_missed_occurrences(workspaces, &missed, current_time, &mut state.notice);
    }
    for event in poll.events {
        start_triggered_run(state, &event, current_time);
    }

    let Some(workspaces) = state.workspaces.as_mut() else {
        return;
    };
    let dispatches = match state
        .routines
        .tick(workspaces, &mut state.orchestrator, current_time)
    {
        Ok(dispatches) => dispatches,
        Err(error) => {
            state.notice = Some(format!("Routine scheduling failed: {error}"));
            return;
        }
    };
    for dispatch in dispatches {
        register_routine_dispatch(state, &dispatch);
    }
}

fn record_missed_occurrences(
    workspaces: &mut WorkspaceManager,
    missed: &MissedOccurrences,
    at: Timestamp,
    notice: &mut Option<String>,
) {
    if let Err(error) = workspaces.execute(
        missed.workspace_id,
        DomainCommand::SkipRoutineOccurrences {
            routine_id: missed.routine_id,
            trigger_id: missed.trigger_id,
            skipped: missed.skipped,
            next_occurrence: missed.next_occurrence,
        },
        at,
    ) {
        *notice = Some(format!("Could not record missed schedule runs: {error}"));
    } else {
        *notice = Some(format!(
            "Skipped {} scheduled run(s) that came due while OpenPodium was closed",
            missed.skipped
        ));
    }
}

fn start_triggered_run(state: &mut OpenPodium, event: &TriggerEvent, at: Timestamp) {
    let Some(workspaces) = state.workspaces.as_mut() else {
        return;
    };
    let request = RunRequest {
        observed: event.observed.clone(),
        trigger: Some(event.firing.clone()),
        ..RunRequest::default()
    };
    match state.routines.start_run(
        workspaces,
        event.workspace_id,
        event.routine_id,
        request,
        at,
    ) {
        Ok(run_id) => {
            state.trigger_watcher.mark_consumed(event);
            state.notice = Some(format!("Trigger started routine run {run_id}"));
        }
        Err(error) if !error.is_retryable() => {
            // A repeated trigger for a routine that is already running, or an
            // occurrence that was already consumed, is expected: it coalesces
            // onto the run in flight rather than starting a second one.
            state.trigger_watcher.mark_consumed(event);
            state.notice = Some(format!("Trigger did not start a run: {error}"));
        }
        Err(error) => {
            state.notice = Some(format!("Trigger will retry: {error}"));
        }
    }
}

/// Publishes a dispatched routine handoff to the IPC service so the assigned
/// agent can return structured outputs against it.
fn register_routine_dispatch(state: &mut OpenPodium, dispatch: &RoutineDispatch) {
    let Some(ipc) = state.ipc.as_ref() else {
        return;
    };
    let Ok(message_id) = openpodium::ipc::MessageId::new(dispatch.message_id().as_str()) else {
        state.notice = Some("Routine handoff has an unusable message ID".to_owned());
        return;
    };
    let (title, body) = state
        .workspaces
        .as_ref()
        .and_then(|workspaces| workspaces.workspace(dispatch.workspace_id()))
        .and_then(|workspace| {
            let run = workspace.routine_run(dispatch.run_id())?;
            let task_id = run.step(dispatch.step_id())?.attempts().last()?.task_id();
            let task = workspace.task(task_id)?;
            Some((
                task.title().as_str().to_owned(),
                task.prompt().as_str().to_owned(),
            ))
        })
        .unwrap_or_else(|| ("Routine step".to_owned(), "Routine step".to_owned()));
    if let Err(error) = ipc.register_routine_handoff(
        dispatch.workspace_id().get(),
        &message_id,
        dispatch.agent_id().get(),
        &title,
        &body,
    ) {
        state.notice = Some(format!("Routine handoff registration failed: {error}"));
    }
}

fn run_orchestration_tick(state: &mut OpenPodium) {
    const MAX_MESSAGES_PER_TICK: usize = 64;
    const MAX_DELIVERIES_PER_TICK: usize = 64;

    let current_time = now();
    let Some(workspaces) = state.workspaces.as_mut() else {
        return;
    };
    if let Err(error) = state.orchestrator.expire_due(workspaces, current_time) {
        state.notice = Some(format!("Handoff deadline processing failed: {error}"));
        return;
    }

    for _ in 0..MAX_MESSAGES_PER_TICK {
        let accepted = match state.ipc.as_ref().map(IpcService::next_message) {
            Some(Ok(Some(accepted))) => accepted,
            Some(Ok(None)) | None => break,
            Some(Err(error)) => {
                state.notice = Some(format!("IPC queue read failed: {error}"));
                break;
            }
        };
        match state
            .orchestrator
            .accept(workspaces, &accepted, current_time)
        {
            Ok(()) => {
                if let Some(ipc) = &state.ipc
                    && let Err(error) = ipc.mark_processed(&accepted)
                {
                    ipc.release(&accepted);
                    state.notice = Some(format!("IPC acknowledgement failed: {error}"));
                    break;
                }
            }
            Err(error) if error.is_retryable() => {
                if let Some(ipc) = &state.ipc {
                    ipc.release(&accepted);
                }
                state.notice = Some(format!("Handoff processing will retry: {error}"));
                break;
            }
            Err(error) => {
                if let Some(ipc) = &state.ipc
                    && let Err(acknowledgement_error) = ipc.mark_processed(&accepted)
                {
                    ipc.release(&accepted);
                    state.notice = Some(format!(
                        "Handoff rejected ({error}); IPC acknowledgement failed: {acknowledgement_error}"
                    ));
                    break;
                }
                state.notice = Some(format!("Handoff rejected: {error}"));
            }
        }
    }

    for _ in 0..MAX_DELIVERIES_PER_TICK {
        let request = match state.orchestrator.prepare_next(workspaces, current_time) {
            Ok(Some(request)) => request,
            Ok(None) => break,
            Err(error) => {
                state.notice = Some(format!("Handoff delivery preparation failed: {error}"));
                break;
            }
        };
        let result = deliver_handoff(&state.terminals, workspaces, &request);
        if let Err(error) =
            state
                .orchestrator
                .finish_delivery(workspaces, &request, result, current_time)
        {
            state.notice = Some(format!("Handoff delivery recording failed: {error}"));
            break;
        }
    }
}

fn deliver_handoff(
    terminals: &BTreeMap<TerminalKey, Session>,
    workspaces: &WorkspaceManager,
    request: &DeliveryRequest,
) -> Result<(), String> {
    let key = delivery_target(workspaces, request)?;
    let session = terminals
        .get(&key)
        .ok_or_else(|| "recipient terminal is not running".to_owned())?;
    session.paste(request.prompt())?;
    session.write(b"\r")
}

fn delivery_target(
    workspaces: &WorkspaceManager,
    request: &DeliveryRequest,
) -> Result<TerminalKey, String> {
    let workspace = workspaces
        .workspace(request.workspace_id())
        .ok_or_else(|| format!("workspace {} is not loaded", request.workspace_id()))?;
    let node_id = workspace
        .all_canvas_layout()
        .nodes()
        .iter()
        .find(|node| node.reference() == Some(NodeTarget::Agent(request.recipient_agent_id())))
        .map(Node::id)
        .ok_or_else(|| {
            format!(
                "agent {} has no terminal node",
                request.recipient_agent_id()
            )
        })?;
    Ok(TerminalKey {
        workspace_id: request.workspace_id(),
        node_id,
    })
}

fn handle_canvas_message(state: &mut OpenPodium, message: canvas::Message) -> Task<Message> {
    match message {
        canvas::Message::ConnectionSourceSelected(source) => {
            state.connection_mode = canvas::ConnectionMode::PickTarget(source);
            state.connection_error = None;
            state.canvas_selection = vec![source];
            state.canvas_revision = state.canvas_revision.wrapping_add(1);
        }
        canvas::Message::ConnectNodes { source, target } => connect_nodes(state, source, target),
        canvas::Message::CancelConnection => state.cancel_connection(),
        canvas::Message::CameraChanged(camera) => {
            state.camera = camera;
            state.canvas_revision = state.canvas_revision.wrapping_add(1);
        }
        canvas::Message::SelectionChanged(selection) => {
            state.canvas_selection = selection;
            state.focused_terminal = None;
            state.focused_portal = None;
            context_nodes::selection_changed(state);
            state.canvas_revision = state.canvas_revision.wrapping_add(1);
        }
        canvas::Message::EditRequested(node_id) => {
            state.canvas_selection = vec![node_id];
            state.canvas_revision = state.canvas_revision.wrapping_add(1);
            return context_nodes::open_editor(state);
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
        canvas::Message::CopyRequested => return copy_canvas_fragment(state),
        canvas::Message::PasteRequested => {
            return clipboard::read().map(Message::CanvasFragmentRead);
        }
        canvas::Message::TerminalFocused(node_id) => {
            state.focused_terminal = node_id;
            state.focused_portal = None;
            state.canvas_revision = state.canvas_revision.wrapping_add(1);
        }
        canvas::Message::TerminalInput { node_id, bytes } => {
            if let Some(session) =
                active_terminal_key(state, node_id).and_then(|key| state.terminals.get(&key))
                && let Err(error) = session.write(&bytes)
            {
                state.notice = Some(error);
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
            state.focused_portal = None;
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
        canvas::Message::PortalClicked {
            node_id,
            observation_revision,
            x,
            y,
        } => {
            state.focused_terminal = None;
            state.focused_portal = Some(node_id);
            state.canvas_selection = vec![node_id];
            state.canvas_revision = state.canvas_revision.wrapping_add(1);
            return portals::execute(
                state,
                node_id,
                PortalAction::ClickCoordinate {
                    observation_revision,
                    x,
                    y,
                },
            );
        }
        canvas::Message::PortalScrolled {
            node_id,
            observation_revision,
            x,
            y,
            delta_x,
            delta_y,
        } => {
            return portals::execute(
                state,
                node_id,
                PortalAction::ScrollCoordinate {
                    observation_revision,
                    x,
                    y,
                    delta_x,
                    delta_y,
                },
            );
        }
        canvas::Message::PortalText {
            node_id,
            observation_revision,
            text,
        } => {
            return portals::execute(
                state,
                node_id,
                PortalAction::TypeFocused {
                    observation_revision,
                    text,
                },
            );
        }
        canvas::Message::PortalKey {
            node_id,
            observation_revision,
            key,
            shift,
        } => {
            return portals::execute(
                state,
                node_id,
                PortalAction::Key {
                    observation_revision,
                    key,
                    shift,
                },
            );
        }
    }
    Task::none()
}

fn handle_chat_message(state: &mut OpenPodium, message: chat::Message) -> Task<Message> {
    match message {
        chat::Message::SurfaceChanged(surface) => {
            let Some((workspace_id, agent_id)) = selected_agent(state) else {
                state.notice = Some("Select an agent node first".to_owned());
                return Task::none();
            };
            state.chat_ui.set_surface(workspace_id, agent_id, surface);
        }
        chat::Message::ThreadSelected(thread_id) => {
            let Some((workspace_id, agent_id)) = selected_agent(state) else {
                state.notice = Some("Select an agent node first".to_owned());
                return Task::none();
            };
            let valid = state
                .workspaces
                .as_ref()
                .and_then(|workspaces| workspaces.workspace(workspace_id))
                .and_then(|workspace| workspace.chat_thread(thread_id))
                .is_some_and(|thread| thread.agent_id() == agent_id);
            if valid {
                state
                    .chat_ui
                    .select_thread(workspace_id, agent_id, thread_id);
            }
        }
        chat::Message::NewThreadNameChanged(value) => {
            if let Some((workspace_id, agent_id)) = selected_agent(state) {
                state
                    .chat_ui
                    .set_new_thread_name(workspace_id, agent_id, value);
            }
        }
        chat::Message::NewThreadColorChanged(value) => {
            if let Some((workspace_id, agent_id)) = selected_agent(state) {
                state
                    .chat_ui
                    .set_new_thread_color(workspace_id, agent_id, value);
            }
        }
        chat::Message::CreateThread => create_chat_thread(state),
        chat::Message::DraftChanged(text) => update_chat_draft_text(state, text),
        chat::Message::MentionToggled(target) => toggle_chat_mention(state, target),
        chat::Message::AttachmentPathChanged(value) => {
            if let Some((workspace_id, _, thread_id)) = selected_chat_context(state) {
                state
                    .chat_ui
                    .set_attachment_path(workspace_id, thread_id, value);
            }
        }
        chat::Message::Attach => attach_chat_file(state),
        chat::Message::Submit => submit_chat_draft(state),
        chat::Message::Rich(chat::Action::LinkClicked(uri)) => {
            return handle_chat_link(state, &uri);
        }
    }
    Task::none()
}

fn create_chat_thread(state: &mut OpenPodium) {
    let Some((workspace_id, agent_id)) = selected_agent(state) else {
        state.notice = Some("Select an agent node first".to_owned());
        return;
    };
    let Some(workspace) = state
        .workspaces
        .as_ref()
        .and_then(|workspaces| workspaces.workspace(workspace_id))
    else {
        state.notice = Some("The selected workspace is unavailable".to_owned());
        return;
    };
    let Some(thread_id) = next_chat_thread_id(workspace) else {
        state.notice = Some("No chat thread identifiers remain".to_owned());
        return;
    };
    let thread = Name::new(state.chat_ui.new_thread_name(workspace_id, agent_id))
        .map_err(|error| error.to_string())
        .and_then(|name| {
            Ok(ChatThread::with_color(
                thread_id,
                agent_id,
                name,
                ThreadColor::new(state.chat_ui.new_thread_color(workspace_id, agent_id))
                    .map_err(|error| error.to_string())?,
            ))
        })
        .and_then(|thread| {
            state
                .workspaces
                .as_mut()
                .expect("the workspace was checked")
                .execute(workspace_id, DomainCommand::AddChatThread(thread), now())
                .map_err(|error| error.to_string())
        });
    state.notice = Some(match thread {
        Ok(_) => {
            state
                .chat_ui
                .select_thread(workspace_id, agent_id, thread_id);
            state.chat_ui.clear_new_thread(workspace_id, agent_id);
            "Chat thread created".to_owned()
        }
        Err(error) => error,
    });
}

fn update_chat_draft_text(state: &mut OpenPodium, text: String) {
    let Some((workspace_id, _, thread_id)) = selected_chat_context(state) else {
        return;
    };
    let Some(thread) = state
        .workspaces
        .as_ref()
        .and_then(|workspaces| workspaces.workspace(workspace_id))
        .and_then(|workspace| workspace.chat_thread(thread_id))
    else {
        return;
    };
    let draft = ChatDraft::new(
        text,
        thread.draft().attachments().to_vec(),
        thread.draft().mentions().to_vec(),
    );
    persist_chat_draft(state, workspace_id, thread_id, draft);
}

fn toggle_chat_mention(state: &mut OpenPodium, target: NodeTarget) {
    let Some((workspace_id, _, thread_id)) = selected_chat_context(state) else {
        return;
    };
    let Some(thread) = state
        .workspaces
        .as_ref()
        .and_then(|workspaces| workspaces.workspace(workspace_id))
        .and_then(|workspace| workspace.chat_thread(thread_id))
    else {
        return;
    };
    let mut mentions = thread.draft().mentions().to_vec();
    if let Some(index) = mentions.iter().position(|mention| *mention == target) {
        mentions.remove(index);
    } else {
        mentions.push(target);
    }
    let draft = ChatDraft::new(
        thread.draft().text(),
        thread.draft().attachments().to_vec(),
        mentions,
    );
    persist_chat_draft(state, workspace_id, thread_id, draft);
}

fn persist_chat_draft(
    state: &mut OpenPodium,
    workspace_id: WorkspaceId,
    thread_id: ChatThreadId,
    draft: Result<ChatDraft, openpodium::domain::ChatValidationError>,
) {
    let result = draft.map_err(|error| error.to_string()).and_then(|draft| {
        let unchanged = state
            .workspaces
            .as_ref()
            .and_then(|workspaces| workspaces.workspace(workspace_id))
            .and_then(|workspace| workspace.chat_thread(thread_id))
            .is_some_and(|thread| thread.draft() == &draft);
        if unchanged {
            return Ok(());
        }
        state
            .workspaces
            .as_mut()
            .expect("the workspace was checked")
            .execute(
                workspace_id,
                DomainCommand::UpdateChatDraft { thread_id, draft },
                now(),
            )
            .map(|_| ())
            .map_err(|error| error.to_string())
    });
    if let Err(error) = result {
        state.notice = Some(error);
    }
}

fn attach_chat_file(state: &mut OpenPodium) {
    let Some((workspace_id, _, thread_id)) = selected_chat_context(state) else {
        state.notice = Some("Select a chat thread first".to_owned());
        return;
    };
    let Some(store) = state.attachment_store.clone() else {
        state.notice = Some("Attachment storage is unavailable".to_owned());
        return;
    };
    let Some((attachment_id, path, current_draft, current_bytes)) = state
        .workspaces
        .as_ref()
        .and_then(|workspaces| workspaces.workspace(workspace_id))
        .and_then(|workspace| {
            let thread = workspace.chat_thread(thread_id)?;
            Some((
                next_chat_attachment_id(workspace)?,
                PathBuf::from(state.chat_ui.attachment_path(workspace_id, thread_id)),
                thread.draft().clone(),
                thread
                    .draft()
                    .attachments()
                    .iter()
                    .filter_map(|id| workspace.chat_attachment(*id))
                    .map(|attachment| attachment.byte_len())
                    .sum::<u64>(),
            ))
        })
    else {
        state.notice = Some("The selected chat thread is unavailable".to_owned());
        return;
    };
    if current_draft.attachments().len() == ChatDraft::MAX_ATTACHMENTS {
        state.notice = Some(format!(
            "A draft can contain at most {} attachments",
            ChatDraft::MAX_ATTACHMENTS
        ));
        return;
    }
    let result = store
        .import(&path, workspace_id, thread_id, attachment_id)
        .map_err(|error| error.to_string())
        .and_then(|attachment| {
            let total = current_bytes.saturating_add(attachment.byte_len());
            if total > ChatDraft::MAX_TOTAL_ATTACHMENT_BYTES {
                let _ = store.remove(workspace_id, &attachment);
                return Err(format!(
                    "Draft attachments total {total} bytes; the limit is {} bytes",
                    ChatDraft::MAX_TOTAL_ATTACHMENT_BYTES
                ));
            }
            let add_result = state
                .workspaces
                .as_mut()
                .expect("the workspace was checked")
                .execute(
                    workspace_id,
                    DomainCommand::AddChatAttachment(attachment.clone()),
                    now(),
                );
            if let Err(error) = add_result {
                let _ = store.remove(workspace_id, &attachment);
                return Err(error.to_string());
            }
            let mut attachments = current_draft.attachments().to_vec();
            attachments.push(attachment.id());
            let draft = ChatDraft::new(
                current_draft.text(),
                attachments,
                current_draft.mentions().to_vec(),
            )
            .map_err(|error| error.to_string())?;
            state
                .workspaces
                .as_mut()
                .expect("the workspace was checked")
                .execute(
                    workspace_id,
                    DomainCommand::UpdateChatDraft { thread_id, draft },
                    now(),
                )
                .map_err(|error| error.to_string())?;
            Ok(attachment.display_name().to_owned())
        });
    state.notice = Some(match result {
        Ok(name) => {
            state.chat_ui.clear_attachment_path(workspace_id, thread_id);
            format!("Attached {name}")
        }
        Err(error) => error,
    });
}

fn submit_chat_draft(state: &mut OpenPodium) {
    let Some((workspace_id, _, thread_id)) = selected_chat_context(state) else {
        state.notice = Some("Select a chat thread first".to_owned());
        return;
    };
    let Some((message_id, prompt)) = state
        .workspaces
        .as_ref()
        .and_then(|workspaces| workspaces.workspace(workspace_id))
        .and_then(|workspace| {
            Some((
                next_chat_message_id(workspace)?,
                workspace.chat_thread(thread_id)?.draft().text().to_owned(),
            ))
        })
    else {
        state.notice = Some("The selected chat thread is unavailable".to_owned());
        return;
    };
    let result = state
        .workspaces
        .as_mut()
        .expect("the workspace was checked")
        .execute(
            workspace_id,
            DomainCommand::SubmitChatDraft {
                thread_id,
                message_id,
                sent_at: now(),
            },
            now(),
        );
    if let Err(error) = result {
        state.notice = Some(error.to_string());
        return;
    }
    if let Some(workspace) = state
        .workspaces
        .as_ref()
        .and_then(|workspaces| workspaces.workspace(workspace_id))
    {
        state.chat_ui.sync(workspace);
    }

    let terminal = selected_terminal_node(state).and_then(|node_id| {
        state.terminals.get(&TerminalKey {
            workspace_id,
            node_id,
        })
    });
    let mut input = prompt.into_bytes();
    input.push(b'\r');
    state.notice = Some(match terminal {
        Some(session) => match session.write(&input) {
            Ok(()) => "Prompt saved and sent to the terminal".to_owned(),
            Err(error) => format!("Prompt saved, but terminal delivery failed: {error}"),
        },
        None => "Prompt saved; start the terminal to deliver it manually".to_owned(),
    });
}

fn handle_chat_link(state: &mut OpenPodium, uri: &str) -> Task<Message> {
    let Some(workspace) = state
        .workspaces
        .as_ref()
        .and_then(WorkspaceManager::active_workspace)
    else {
        state.notice = Some("No workspace is active".to_owned());
        return Task::none();
    };
    let Some(root) = workspace.settings().working_directory() else {
        state.notice = Some("The workspace has no working directory".to_owned());
        return Task::none();
    };
    match chat::classify_link(uri, PathBuf::from(root.as_str()).as_path()) {
        LinkTarget::Web(url) => {
            state.notice = Some("Web link copied to the clipboard".to_owned());
            clipboard::write(url)
        }
        LinkTarget::WorkspaceFile(path) => {
            state.notice = Some("Workspace file path copied to the clipboard".to_owned());
            clipboard::write(path.display().to_string())
        }
        LinkTarget::Attachment(id) => {
            state.notice = Some(format!("Local attachment {id}"));
            Task::none()
        }
        LinkTarget::Blocked => {
            state.notice = Some("Blocked an unsafe or unsupported link".to_owned());
            Task::none()
        }
    }
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

fn preview_agent(state: &mut OpenPodium, program: AgentProgram) {
    let Some((preset, role, profile, working_directory)) = state
        .workspaces
        .as_ref()
        .and_then(WorkspaceManager::active_workspace)
        .and_then(|workspace| {
            let preset = match program {
                AgentProgram::Custom(preset_id) => workspace.command_preset(preset_id).cloned(),
                _ => None,
            };
            let role = state
                .selected_role
                .and_then(|role_id| workspace.role(role_id))
                .cloned();
            let profile = state
                .selected_environment
                .and_then(|environment_id| workspace.environment_profile(environment_id))
                .cloned();
            Some((
                preset,
                role,
                profile,
                PathBuf::from(workspace.active_directory()?.as_str()),
            ))
        })
    else {
        state.notice = Some("Create or select a workspace first".to_owned());
        return;
    };

    let capability = check_agent_capability(program, preset.as_ref())
        .map(|capability| {
            if capability.installed() {
                "installed locally".to_owned()
            } else {
                capability
                    .setup_guidance()
                    .unwrap_or("missing locally")
                    .to_owned()
            }
        })
        .unwrap_or_else(|error| error.to_string());
    let result = session::process_spec(
        program,
        preset.as_ref(),
        role.as_ref(),
        &working_directory,
        terminal::GridSize::for_node(640.0, 480.0),
    )
    .map_err(|error| error.to_string())
    .and_then(|spec| {
        prepare_environment_process(profile.as_ref(), spec).map_err(|error| error.to_string())
    });
    state.notice = Some(match result {
        Ok(spec) => format!("{capability}\n{}", format_process_preview(&spec)),
        Err(error) => error,
    });
}

fn format_process_preview(spec: &openpodium::runtime::ProcessSpec) -> String {
    let arguments = spec
        .arguments()
        .iter()
        .map(|argument| {
            format!(
                "{:?}",
                openpodium::security::redact_secrets(&argument.to_string_lossy())
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    let environment = spec
        .environment()
        .iter()
        .map(|(name, value)| {
            if openpodium::security::is_sensitive_name(&name.to_string_lossy()) {
                format!("{name:?}=\"[redacted]\"")
            } else {
                format!(
                    "{name:?}={:?}",
                    openpodium::security::redact_secrets(&value.to_string_lossy())
                )
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "program: {:?}\narguments: [{arguments}]\nenvironment: [{environment}]",
        spec.program()
    )
}

fn capability_label(program: AgentProgram, preset: Option<&CommandPreset>) -> &'static str {
    match check_agent_capability(program, preset) {
        Ok(capability) if capability.installed() => "installed",
        Ok(_) => "missing",
        Err(_) => "invalid",
    }
}

fn save_preset(state: &mut OpenPodium) {
    let Some((workspace_id, preset_id)) = state
        .workspaces
        .as_ref()
        .and_then(WorkspaceManager::active_workspace)
        .and_then(|workspace| {
            Some((
                workspace.id(),
                state
                    .editing_preset
                    .or_else(|| next_command_preset_id(workspace))?,
            ))
        })
    else {
        state.notice = Some("Create or select a workspace first".to_owned());
        return;
    };
    let result = parse_argument_array(&state.preset_arguments, "preset arguments")
        .and_then(|arguments| {
            CommandPreset::new(
                preset_id,
                Name::new(state.preset_name.clone()).map_err(|error| error.to_string())?,
                state.preset_executable.clone(),
                arguments,
            )
            .map_err(|error| error.to_string())
        })
        .and_then(|preset| {
            state
                .workspaces
                .as_mut()
                .expect("active workspace was checked")
                .execute(
                    workspace_id,
                    if state.editing_preset.is_some() {
                        DomainCommand::UpdateCommandPreset(preset)
                    } else {
                        DomainCommand::AddCommandPreset(preset)
                    },
                    now(),
                )
                .map_err(|error| error.to_string())
        });
    state.notice = Some(match result {
        Ok(_) => {
            clear_preset_draft(state);
            "Command preset saved; reconnect custom agents to apply changes".to_owned()
        }
        Err(error) => error,
    });
}

fn edit_preset(state: &mut OpenPodium, preset_id: CommandPresetId) {
    let preset = state
        .workspaces
        .as_ref()
        .and_then(WorkspaceManager::active_workspace)
        .and_then(|workspace| workspace.command_preset(preset_id))
        .cloned();
    let Some(preset) = preset else {
        state.notice = Some(format!("Command preset {preset_id} does not exist"));
        return;
    };
    state.editing_preset = Some(preset_id);
    state.preset_name = preset.name().as_str().to_owned();
    state.preset_executable = preset.executable().to_owned();
    state.preset_arguments = serde_json::to_string(preset.arguments())
        .expect("serializing preset arguments cannot fail");
}

fn remove_preset(state: &mut OpenPodium, preset_id: CommandPresetId) {
    let Some(workspace_id) = active_workspace_id(state) else {
        state.notice = Some("Create or select a workspace first".to_owned());
        return;
    };
    let result = state
        .workspaces
        .as_mut()
        .expect("active workspace was checked")
        .execute(
            workspace_id,
            DomainCommand::RemoveCommandPreset(preset_id),
            now(),
        );
    state.notice = Some(match result {
        Ok(_) => {
            if state.editing_preset == Some(preset_id) {
                clear_preset_draft(state);
            }
            "Command preset deleted".to_owned()
        }
        Err(error) => error.to_string(),
    });
}

fn save_role(state: &mut OpenPodium) {
    let Some((workspace_id, role_id)) = state
        .workspaces
        .as_ref()
        .and_then(WorkspaceManager::active_workspace)
        .and_then(|workspace| {
            Some((
                workspace.id(),
                state.editing_role.or_else(|| next_role_id(workspace))?,
            ))
        })
    else {
        state.notice = Some("Create or select a workspace first".to_owned());
        return;
    };
    let role = Name::new(state.role_name.clone())
        .map_err(|error| error.to_string())
        .and_then(|name| {
            Ok(Role::with_appearance(
                role_id,
                name,
                RoleColor::new(state.role_color.clone()).map_err(|error| error.to_string())?,
                RoleIcon::new(state.role_icon.clone()).map_err(|error| error.to_string())?,
                Content::new(state.role_instructions.clone()).map_err(|error| error.to_string())?,
            ))
        })
        .and_then(|role| {
            state
                .workspaces
                .as_mut()
                .expect("active workspace was checked")
                .execute(
                    workspace_id,
                    if state.editing_role.is_some() {
                        DomainCommand::UpdateRole(role)
                    } else {
                        DomainCommand::AddRole(role)
                    },
                    now(),
                )
                .map_err(|error| error.to_string())
        });
    state.notice = Some(match role {
        Ok(_) => {
            state.selected_role = Some(role_id);
            clear_role_draft(state);
            "Role saved; reconnect assigned agents to apply changes".to_owned()
        }
        Err(error) => error,
    });
}

fn edit_role(state: &mut OpenPodium, role_id: RoleId) {
    let role = state
        .workspaces
        .as_ref()
        .and_then(WorkspaceManager::active_workspace)
        .and_then(|workspace| workspace.role(role_id))
        .cloned();
    let Some(role) = role else {
        state.notice = Some(format!("Role {role_id} does not exist"));
        return;
    };
    state.editing_role = Some(role_id);
    state.role_name = role.name().as_str().to_owned();
    state.role_color = role.color().as_str().to_owned();
    state.role_icon = role.icon().as_str().to_owned();
    state.role_instructions = role.instructions().as_str().to_owned();
}

fn remove_role(state: &mut OpenPodium, role_id: RoleId) {
    let Some(workspace_id) = active_workspace_id(state) else {
        state.notice = Some("Create or select a workspace first".to_owned());
        return;
    };
    let result = state
        .workspaces
        .as_mut()
        .expect("active workspace was checked")
        .execute(workspace_id, DomainCommand::RemoveRole(role_id), now());
    state.notice = Some(match result {
        Ok(_) => {
            if state.selected_role == Some(role_id) {
                state.selected_role = None;
            }
            if state.editing_role == Some(role_id) {
                clear_role_draft(state);
            }
            "Role deleted".to_owned()
        }
        Err(error) => error.to_string(),
    });
}

fn assign_role_to_selected(state: &mut OpenPodium, role_id: Option<RoleId>) {
    let Some((workspace_id, agent_id)) = selected_agent(state) else {
        state.notice = Some("Select an agent node first".to_owned());
        return;
    };
    let result = state
        .workspaces
        .as_mut()
        .expect("selected agent belongs to a workspace")
        .execute(
            workspace_id,
            DomainCommand::AssignAgentRole { agent_id, role_id },
            now(),
        );
    state.notice = Some(match result {
        Ok(_) => "Agent role updated; reconnect to apply it".to_owned(),
        Err(error) => error.to_string(),
    });
}

fn export_role_to_clipboard(state: &mut OpenPodium, role_id: RoleId) -> Task<Message> {
    let result = state
        .workspaces
        .as_ref()
        .and_then(WorkspaceManager::active_workspace)
        .and_then(|workspace| workspace.role(role_id))
        .ok_or_else(|| format!("Role {role_id} does not exist"))
        .and_then(|role| export_role(role).map_err(|error| error.to_string()));
    match result {
        Ok(payload) => {
            state.notice = Some("Role JSON copied to the clipboard".to_owned());
            clipboard::write(payload)
        }
        Err(error) => {
            state.notice = Some(error);
            Task::none()
        }
    }
}

fn save_selection_as_template(state: &mut OpenPodium) -> Task<Message> {
    let Some(workspace_id) = state
        .workspaces
        .as_ref()
        .and_then(WorkspaceManager::active_workspace_id)
    else {
        state.notice = Some("Create or select a workspace first".to_owned());
        return Task::none();
    };
    let result = state
        .workspaces
        .as_ref()
        .expect("active workspace came from the manager")
        .export_template(workspace_id, &state.canvas_selection);
    match result {
        Ok(payload) => {
            state.notice = Some("Template JSON copied to the clipboard".to_owned());
            clipboard::write(payload)
        }
        Err(error) => {
            state.notice = Some(error.to_string());
            Task::none()
        }
    }
}

fn export_workspace_archive_to_clipboard(state: &mut OpenPodium) -> Task<Message> {
    let Some(workspace_id) = state
        .workspaces
        .as_ref()
        .and_then(WorkspaceManager::active_workspace_id)
    else {
        state.notice = Some("Create or select a workspace first".to_owned());
        return Task::none();
    };
    let result = state
        .workspaces
        .as_ref()
        .expect("active workspace came from the manager")
        .export_workspace_archive(workspace_id);
    match result {
        Ok(payload) => {
            state.notice = Some("Workspace archive JSON copied to the clipboard".to_owned());
            clipboard::write(payload)
        }
        Err(error) => {
            state.notice = Some(error.to_string());
            Task::none()
        }
    }
}

fn preview_template_from_clipboard(state: &mut OpenPodium, payload: Option<String>) {
    let Some(payload) = payload else {
        state.notice = Some("The clipboard does not contain text".to_owned());
        return;
    };
    let Some(workspace_id) = state
        .workspaces
        .as_ref()
        .and_then(WorkspaceManager::active_workspace_id)
    else {
        state.notice = Some("Create or select a workspace first".to_owned());
        return;
    };
    let result = state
        .workspaces
        .as_ref()
        .expect("active workspace came from the manager")
        .preview_template_import(workspace_id, &payload);
    state.portable_import = match result {
        Ok(preview) => Some(PortableImportDraft {
            kind: PortableImportKind::Template,
            workspace_id,
            destination_floor: state
                .workspaces
                .as_ref()
                .and_then(|manager| manager.workspace(workspace_id))
                .and_then(|workspace| workspace.floors().active),
            payload,
            preview,
            launcher_mappings: BTreeMap::new(),
            path_mappings: BTreeMap::new(),
        }),
        Err(error) => {
            state.notice = Some(error.to_string());
            None
        }
    };
}

fn preview_workspace_archive_from_clipboard(state: &mut OpenPodium, payload: Option<String>) {
    let Some(payload) = payload else {
        state.notice = Some("The clipboard does not contain text".to_owned());
        return;
    };
    let Some(workspace_id) = state
        .workspaces
        .as_ref()
        .and_then(WorkspaceManager::active_workspace_id)
    else {
        state.notice = Some("Create or select a workspace first".to_owned());
        return;
    };
    let result = state
        .workspaces
        .as_ref()
        .expect("active workspace came from the manager")
        .preview_workspace_archive_import(workspace_id, &payload);
    state.portable_import = match result {
        Ok(preview) => Some(PortableImportDraft {
            kind: PortableImportKind::WorkspaceArchive,
            workspace_id,
            destination_floor: state
                .workspaces
                .as_ref()
                .and_then(|manager| manager.workspace(workspace_id))
                .and_then(|workspace| workspace.floors().active),
            payload,
            preview,
            launcher_mappings: BTreeMap::new(),
            path_mappings: BTreeMap::new(),
        }),
        Err(error) => {
            state.notice = Some(error.to_string());
            None
        }
    };
}

fn confirm_portable_import(state: &mut OpenPodium) {
    let Some(draft) = state.portable_import.take() else {
        return;
    };
    let Some(workspace_id) = state
        .workspaces
        .as_ref()
        .and_then(WorkspaceManager::active_workspace_id)
    else {
        state.notice = Some("Create or select a workspace first".to_owned());
        state.portable_import = Some(draft);
        return;
    };
    if workspace_id != draft.workspace_id {
        state.notice = Some("The import destination changed; preview it again".to_owned());
        return;
    }
    let camera = state.camera.position();
    let origin = PointV1 {
        x: canvas_coordinate(camera.x),
        y: canvas_coordinate(camera.y),
    };
    let workspaces = state
        .workspaces
        .as_mut()
        .expect("active workspace came from the manager");
    let result = match draft.kind {
        PortableImportKind::Template => workspaces.import_template(
            workspace_id,
            &draft.payload,
            draft.destination_floor,
            origin,
            &draft.launcher_mappings,
            &draft.path_mappings,
            now(),
        ),
        PortableImportKind::WorkspaceArchive => workspaces.import_workspace_archive(
            workspace_id,
            &draft.payload,
            draft.destination_floor,
            &draft.launcher_mappings,
            &draft.path_mappings,
            now(),
        ),
    };
    match result {
        Ok(_) => {
            state.reset_canvas_session();
            state.navigation_ui.mark_stale(workspace_id);
            state.sync_ipc_directory();
            state.notice = Some("Portable import committed".to_owned());
        }
        Err(error) => {
            state.notice = Some(error.to_string());
            state.portable_import = Some(draft);
        }
    }
}

fn portable_path_status(workspace: &Workspace, path: &str) -> String {
    let Some(checkout) = workspace.active_directory() else {
        return "no destination checkout".to_owned();
    };
    let Ok(project_path) = ProjectPath::new(path.to_owned()) else {
        return "invalid project-relative path".to_owned();
    };
    match openpodium::context::resolve_project_path(Path::new(checkout.as_str()), &project_path) {
        Ok(resolved) if resolved.is_file() => "present file".to_owned(),
        Ok(resolved) if resolved.is_dir() => "present directory".to_owned(),
        Ok(_) => "missing".to_owned(),
        Err(error) => format!("unsafe: {error}"),
    }
}

fn import_role_from_clipboard(state: &mut OpenPodium, payload: Option<&str>) {
    let Some(payload) = payload else {
        state.notice = Some("The clipboard does not contain text".to_owned());
        return;
    };
    let Some((workspace_id, role_id)) = state
        .workspaces
        .as_ref()
        .and_then(WorkspaceManager::active_workspace)
        .and_then(|workspace| Some((workspace.id(), next_role_id(workspace)?)))
    else {
        state.notice = Some("Create or select a workspace first".to_owned());
        return;
    };
    let result = import_role(payload, role_id)
        .map_err(|error| error.to_string())
        .and_then(|role| {
            state
                .workspaces
                .as_mut()
                .expect("active workspace was checked")
                .execute(workspace_id, DomainCommand::AddRole(role), now())
                .map_err(|error| error.to_string())
        });
    state.notice = Some(match result {
        Ok(_) => {
            state.selected_role = Some(role_id);
            "Role imported".to_owned()
        }
        Err(error) => error,
    });
}

fn parse_argument_array(value: &str, field: &str) -> Result<Vec<String>, String> {
    if value.trim().is_empty() {
        return Ok(Vec::new());
    }
    serde_json::from_str(value)
        .map_err(|error| format!("{field} must be a JSON string array: {error}"))
}

fn clear_preset_draft(state: &mut OpenPodium) {
    state.editing_preset = None;
    state.preset_name.clear();
    state.preset_executable.clear();
    state.preset_arguments.clear();
}

fn clear_role_draft(state: &mut OpenPodium) {
    state.editing_role = None;
    state.role_name.clear();
    state.role_color = RoleColor::DEFAULT.to_owned();
    state.role_icon = RoleIcon::DEFAULT.to_owned();
    state.role_instructions.clear();
}

fn add_agent(state: &mut OpenPodium, program: AgentProgram) -> Task<Message> {
    if floors::is_busy(state) {
        state.notice =
            Some("Wait for the Git operation to finish before adding an agent".to_owned());
        return Task::none();
    }
    if let Some(workspace) = state
        .workspaces
        .as_ref()
        .and_then(WorkspaceManager::active_workspace)
        && workspace.floors().active.is_some()
        && (workspace.active_directory().is_none() || state.selected_environment.is_some())
    {
        state.notice = Some(
            "Choose an available floor and the local environment to run an isolated agent"
                .to_owned(),
        );
        return Task::none();
    }
    let Some((workspace_id, agent_id, node_id, before, node, environment_id, role_id)) = state
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
            let role_id = state
                .selected_role
                .filter(|role_id| workspace.role(*role_id).is_some());
            Some((
                workspace.id(),
                agent_id,
                node_id,
                before,
                node,
                environment_id,
                role_id,
            ))
        })
    else {
        state.notice = Some("Create or select a workspace first".to_owned());
        return Task::none();
    };
    let label = program.label();
    let mut agent = Agent::with_program(
        agent_id,
        Name::new(format!("{label} {}", agent_id.get())).expect("generated agent name is valid"),
        role_id,
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
            state.sync_ipc_directory();
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
    if floors::is_busy(state) {
        state.notice =
            Some("Wait for the Git operation to finish before starting a terminal".to_owned());
        return Task::none();
    }
    let Some((
        workspace_id,
        agent_id,
        program,
        preset,
        role,
        profile,
        working_directory,
        connected_notes,
        size,
    )) = state
        .workspaces
        .as_ref()
        .and_then(WorkspaceManager::active_workspace)
        .and_then(|workspace| {
            let node = workspace.node(node_id)?;
            let Some(NodeTarget::Agent(agent_id)) = node.reference() else {
                return None;
            };
            let agent = workspace.agent(agent_id)?;
            let working_directory = PathBuf::from(workspace.node_directory(node_id)?.as_str());
            let connected_notes =
                context_nodes::connected_note_paths(workspace, node_id, &working_directory);
            let profile = agent
                .environment_id()
                .and_then(|environment_id| workspace.environment_profile(environment_id))
                .cloned();
            let preset = match agent.program() {
                AgentProgram::Custom(preset_id) => workspace.command_preset(preset_id).cloned(),
                _ => None,
            };
            let role = agent
                .role_id()
                .and_then(|role_id| workspace.role(role_id))
                .cloned();
            Some((
                workspace.id(),
                agent.id(),
                agent.program(),
                preset,
                role,
                profile,
                working_directory,
                connected_notes,
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

    if profile.is_none() {
        match check_agent_capability(program, preset.as_ref()) {
            Ok(capability) if !capability.installed() => {
                state.notice = capability.setup_guidance().map(str::to_owned);
                return Task::none();
            }
            Ok(_) => {}
            Err(error) => {
                state.notice = Some(error.to_string());
                return Task::none();
            }
        }
    }

    let spec = match session::process_spec(
        program,
        preset.as_ref(),
        role.as_ref(),
        &working_directory,
        size,
    )
    .map_err(|error| error.to_string())
    .and_then(|spec| {
        let spec = apply_ipc_environment(
            spec,
            state.ipc.as_ref(),
            workspace_id,
            agent_id,
            profile.is_none() && supports_automatic_delivery(program),
        );
        let spec = if profile.is_none() && !connected_notes.is_empty() {
            let paths = connected_notes
                .iter()
                .map(|path| path.to_string_lossy())
                .collect::<Vec<_>>();
            spec.env(
                openpodium::context::CONNECTED_NOTES_ENV,
                serde_json::to_string(&paths).expect("note paths are JSON serializable"),
            )
        } else {
            spec
        };
        prepare_environment_process(profile.as_ref(), spec).map_err(|error| error.to_string())
    }) {
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

fn apply_ipc_environment(
    mut spec: ProcessSpec,
    ipc: Option<&IpcService>,
    workspace_id: WorkspaceId,
    agent_id: AgentId,
    is_local: bool,
) -> ProcessSpec {
    spec = spec
        .env(WORKSPACE_ID_ENV, workspace_id.get().to_string())
        .env(AGENT_ID_ENV, agent_id.get().to_string());
    let Some(connection) = is_local
        .then(|| ipc.and_then(|ipc| ipc.connection_info(workspace_id.get(), agent_id.get())))
        .flatten()
    else {
        return spec.env(AVAILABLE_ENV, "0");
    };
    let spec = spec
        .env(AVAILABLE_ENV, "1")
        .env(ENDPOINT_ENV, connection.endpoint().to_string())
        .env(TOKEN_ENV, connection.token())
        .env(
            VERSIONS_ENV,
            SUPPORTED_VERSIONS
                .iter()
                .map(u16::to_string)
                .collect::<Vec<_>>()
                .join(","),
        );
    match env::current_exe() {
        Ok(executable) => spec.env(CLI_ENV, executable),
        Err(_) => spec,
    }
}

fn supports_automatic_delivery(program: AgentProgram) -> bool {
    matches!(
        program,
        AgentProgram::Codex | AgentProgram::Claude | AgentProgram::OpenCode
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
    if continues && let Some(stream) = session.stream() {
        tasks.push(wait_for_terminal_event(
            workspace_id,
            node_id,
            generation,
            stream,
        ));
    }
    state.canvas_revision = state.canvas_revision.wrapping_add(1);
    Task::batch(tasks)
}

fn stop_terminal(state: &mut OpenPodium, node_id: NodeId) {
    let Some(key) = active_terminal_key(state, node_id) else {
        return;
    };
    if let Some(session) = state.terminals.get_mut(&key)
        && let Err(error) = session.stop()
    {
        state.notice = Some(error);
    }
    if state.focused_terminal == Some(node_id) {
        state.focused_terminal = None;
    }
    state.canvas_revision = state.canvas_revision.wrapping_add(1);
}

fn copy_canvas_fragment(state: &mut OpenPodium) -> Task<Message> {
    let Some(layout) = current_canvas(state) else {
        state.notice = Some("Create or select a workspace first".to_owned());
        return Task::none();
    };
    let fragment = canvas::editor::selection_fragment(&layout, &state.canvas_selection);
    if fragment.nodes().is_empty() {
        state.notice = Some("Select at least one canvas node to copy".to_owned());
        return Task::none();
    }
    match export_canvas_fragment(&fragment) {
        Ok(payload) => {
            state.notice = Some("Canvas fragment copied and exported".to_owned());
            clipboard::write(payload)
        }
        Err(error) => {
            state.notice = Some(error.to_string());
            Task::none()
        }
    }
}

fn paste_canvas_fragment(state: &mut OpenPodium, payload: Option<&str>) {
    let Some(payload) = payload else {
        state.notice = Some("The clipboard does not contain text".to_owned());
        return;
    };
    let fragment = match import_canvas_fragment(payload) {
        Ok(fragment) => fragment,
        Err(error) => {
            state.notice = Some(error.to_string());
            return;
        }
    };
    let Some(before) = current_canvas(state) else {
        state.notice = Some("Create or select a workspace first".to_owned());
        return;
    };
    let all = state
        .workspaces
        .as_ref()
        .and_then(WorkspaceManager::active_workspace)
        .map(Workspace::all_canvas_layout)
        .unwrap_or_else(|| before.clone());
    let (after, selection) = canvas::editor::paste_fragment(&before, &fragment, &all);
    if before == after {
        state.notice = Some("The canvas fragment contains no nodes".to_owned());
        return;
    }
    if persist_canvas(state, before.clone(), after).is_ok() {
        state.canvas_history.record(before);
        state.canvas_selection = selection;
        state.notice = Some("Canvas fragment pasted and imported".to_owned());
    }
}

fn apply_canvas_action(state: &mut OpenPodium, action: CanvasAction) {
    if matches!(action, CanvasAction::Connect) {
        if state.connection_mode != canvas::ConnectionMode::Off {
            state.cancel_connection();
            return;
        }
        state.controls = None;
        state.focused_terminal = None;
        state.focused_portal = None;
        state.connection_error = None;
        state.connection_mode = match state.canvas_selection.as_slice() {
            [source] => canvas::ConnectionMode::PickTarget(*source),
            [source, target] => {
                let (source, target) = (*source, *target);
                connect_nodes(state, source, target);
                return;
            }
            _ => canvas::ConnectionMode::PickSource,
        };
        state.canvas_revision = state.canvas_revision.wrapping_add(1);
        return;
    }
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
    let all = state
        .workspaces
        .as_ref()
        .and_then(WorkspaceManager::active_workspace)
        .map(Workspace::all_canvas_layout)
        .unwrap_or_else(|| before.clone());
    let mut selection = state.canvas_selection.clone();
    let after = match action {
        CanvasAction::Duplicate => {
            let (after, duplicated) = canvas::editor::duplicate(&before, &selection, &all);
            selection = duplicated;
            after
        }
        CanvasAction::Remove => {
            selection.clear();
            canvas::editor::remove(&before, &state.canvas_selection)
        }
        CanvasAction::Group => canvas::editor::group(&before, &selection, &all),
        CanvasAction::Ungroup => canvas::editor::ungroup(&before, &selection),
        CanvasAction::Align(alignment) => canvas::editor::align(&before, &selection, alignment),
        CanvasAction::ZOrder(order) => canvas::editor::change_z_order(&before, &selection, order),
        CanvasAction::Undo | CanvasAction::Redo | CanvasAction::Connect => {
            unreachable!("handled above")
        }
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

fn connect_nodes(state: &mut OpenPodium, source: NodeId, target: NodeId) {
    let Some(before) = current_canvas(state) else {
        return;
    };
    state.connection_mode = canvas::ConnectionMode::PickTarget(source);
    if source == target {
        state.connection_error = Some("Choose a different card".to_owned());
        return;
    }
    let all = state
        .workspaces
        .as_ref()
        .and_then(WorkspaceManager::active_workspace)
        .map(Workspace::all_canvas_layout)
        .unwrap_or_else(|| before.clone());
    match canvas::editor::connect(&before, &[source, target], &all) {
        Ok(after) => {
            if persist_canvas(state, before.clone(), after).is_ok() {
                state.canvas_history.record(before);
                state.canvas_selection = vec![source, target];
                state.cancel_connection();
            } else {
                state.connection_error = state.notice.take();
            }
        }
        Err(error) => state.connection_error = Some(error.to_owned()),
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
            let runtime_layout = state
                .workspaces
                .as_ref()
                .and_then(|m| m.workspace(workspace_id))
                .expect("workspace was just updated")
                .all_canvas_layout();
            synchronize_terminals(state, &runtime_layout);
            state.sync_ipc_directory();
            portals::sync_connections(state);
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
            matches!(node.reference(), Some(NodeTarget::Agent(_))).then(|| {
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
            .is_some_and(|node| matches!(node.reference(), Some(NodeTarget::Agent(_))))
    })
}

fn selected_agent(state: &OpenPodium) -> Option<(WorkspaceId, AgentId)> {
    let workspace = state.workspaces.as_ref()?.active_workspace()?;
    let node = workspace.node(selected_terminal_node(state)?)?;
    let Some(NodeTarget::Agent(agent_id)) = node.reference() else {
        return None;
    };
    Some((workspace.id(), agent_id))
}

fn selected_chat_context(state: &OpenPodium) -> Option<(WorkspaceId, AgentId, ChatThreadId)> {
    let (workspace_id, agent_id) = selected_agent(state)?;
    let workspace = state.workspaces.as_ref()?.workspace(workspace_id)?;
    let thread_id = state.chat_ui.selected_thread(workspace, agent_id)?;
    Some((workspace_id, agent_id, thread_id))
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
        if workspace.recorded_agent(id).is_none() {
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

fn next_command_preset_id(workspace: &Workspace) -> Option<CommandPresetId> {
    let mut value = 1_u64;
    loop {
        let id = CommandPresetId::new(value);
        if workspace.command_preset(id).is_none() {
            return Some(id);
        }
        value = value.checked_add(1)?;
    }
}

fn next_role_id(workspace: &Workspace) -> Option<RoleId> {
    let mut value = 1_u64;
    loop {
        let id = RoleId::new(value);
        if workspace.role(id).is_none() {
            return Some(id);
        }
        value = value.checked_add(1)?;
    }
}

fn next_chat_thread_id(workspace: &Workspace) -> Option<ChatThreadId> {
    let mut value = 1_u64;
    loop {
        let id = ChatThreadId::new(value);
        if workspace.chat_thread(id).is_none() {
            return Some(id);
        }
        value = value.checked_add(1)?;
    }
}

fn next_chat_message_id(workspace: &Workspace) -> Option<ChatMessageId> {
    let mut value = 1_u64;
    loop {
        let id = ChatMessageId::new(value);
        if workspace
            .chat_threads()
            .all(|thread| thread.messages().iter().all(|message| message.id() != id))
        {
            return Some(id);
        }
        value = value.checked_add(1)?;
    }
}

fn next_chat_attachment_id(workspace: &Workspace) -> Option<ChatAttachmentId> {
    let mut value = 1_u64;
    loop {
        let id = ChatAttachmentId::new(value);
        if workspace.chat_attachment(id).is_none() {
            return Some(id);
        }
        value = value.checked_add(1)?;
    }
}

fn canvas_coordinate(value: f64) -> f32 {
    value.clamp(-1_000_000_000.0, 1_000_000_000.0) as f32
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
    use std::path::Path;

    use tempfile::TempDir;

    use super::*;

    #[test]
    fn ipc_launch_environment_is_scoped_and_secret_preview_is_redacted() {
        let temp = TempDir::new().unwrap();
        let service = IpcService::start(temp.path()).unwrap();
        service.replace_workspace_agents(
            4,
            [AgentRegistration {
                id: 7,
                name: "Builder".to_owned(),
                program: "Codex".to_owned(),
                state: "running".to_owned(),
                capabilities: AgentCapabilities::CONNECTED,
            }],
        );
        let local = apply_ipc_environment(
            ProcessSpec::new("agent", temp.path()),
            Some(&service),
            WorkspaceId::new(4),
            AgentId::new(7),
            true,
        );
        let token = local
            .environment()
            .iter()
            .find(|(name, _)| name == TOKEN_ENV)
            .map(|(_, value)| value.to_string_lossy().into_owned())
            .unwrap();
        assert!(!token.is_empty());
        assert!(
            local
                .environment()
                .iter()
                .any(|(name, value)| name == AVAILABLE_ENV && value == "1")
        );
        assert!(local.environment().iter().any(|(name, _)| name == CLI_ENV));
        let preview = format_process_preview(&local);
        assert!(preview.contains("[redacted]"));
        assert!(!preview.contains(&token));

        let remote = apply_ipc_environment(
            ProcessSpec::new("agent", temp.path()),
            Some(&service),
            WorkspaceId::new(4),
            AgentId::new(7),
            false,
        );
        assert!(
            remote
                .environment()
                .iter()
                .any(|(name, value)| name == AVAILABLE_ENV && value == "0")
        );
        assert!(
            remote
                .environment()
                .iter()
                .all(|(name, _)| name != TOKEN_ENV && name != ENDPOINT_ENV && name != CLI_ENV)
        );
    }

    #[test]
    fn deleting_agent_window_updates_registry_ipc_and_undo_without_reusing_ids() {
        let temp = TempDir::new().unwrap();
        let database = temp.path().join("state.sqlite");
        let (mut state, workspace_id, thread_id) = state_with_chat(&temp, &database);
        state.ipc = Some(IpcService::start(temp.path()).unwrap());
        state.sync_ipc_directory();
        let connected = |state: &OpenPodium| {
            state
                .ipc
                .as_ref()
                .unwrap()
                .connection_info(workspace_id.get(), 1)
                .is_some()
        };
        assert!(connected(&state));
        apply_canvas_action(&mut state, CanvasAction::Remove);
        let workspace = state
            .workspaces
            .as_ref()
            .unwrap()
            .workspace(workspace_id)
            .unwrap();
        assert_eq!(workspace.agent_count(), 0);
        assert_eq!(next_agent_id(workspace), Some(AgentId::new(2)));
        assert!(workspace.chat_thread(thread_id).is_some());
        assert!(!connected(&state));
        undo_canvas(&mut state);
        assert!(connected(&state));
        assert_eq!(
            state
                .workspaces
                .as_ref()
                .unwrap()
                .workspace(workspace_id)
                .unwrap()
                .agent_count(),
            1
        );
        redo_canvas(&mut state);
        assert!(!connected(&state));
        assert_eq!(
            WorkspaceManager::open(&database)
                .unwrap()
                .workspace(workspace_id)
                .unwrap()
                .agent_count(),
            0
        );
    }

    #[test]
    fn handoff_delivery_target_is_independent_of_the_active_workspace() {
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
        workspaces
            .execute(
                first_id,
                DomainCommand::AddAgent(Agent::with_program(
                    AgentId::new(1),
                    Name::new("Lead").unwrap(),
                    None,
                    AgentProgram::Codex,
                )),
                Timestamp::from_unix_millis(4),
            )
            .unwrap();
        workspaces
            .execute(
                first_id,
                DomainCommand::AddAgentNode {
                    agent: Agent::with_program(
                        AgentId::new(2),
                        Name::new("Builder").unwrap(),
                        None,
                        AgentProgram::Claude,
                    ),
                    node: Node::new(
                        NodeId::new(7),
                        NodeTarget::Agent(AgentId::new(2)),
                        CanvasPoint::new(0.0, 0.0).unwrap(),
                        CanvasSize::new(400.0, 300.0).unwrap(),
                    ),
                },
                Timestamp::from_unix_millis(5),
            )
            .unwrap();
        let before = workspaces.workspace(first_id).unwrap().floors().clone();
        let mut after = before.clone();
        after.entries.insert(
            1,
            openpodium::domain::Floor {
                name: Name::new("Background floor").unwrap(),
                directory: WorkspaceDirectory::new(first.to_str().unwrap()).unwrap(),
                repository: WorkspaceDirectory::new(first.to_str().unwrap()).unwrap(),
                branch: Some("background".to_owned()),
                base_revision: "base".to_owned(),
                base_branch: Some("main".to_owned()),
                managed: false,
                ownership_token: None,
                owner: None,
                dirty: false,
                lifecycle: openpodium::domain::FloorLifecycle::Available,
            },
        );
        after.node_floors.insert(NodeId::new(7), 1);
        workspaces
            .execute(
                first_id,
                DomainCommand::ReplaceFloors { before, after },
                Timestamp::from_unix_millis(6),
            )
            .unwrap();
        assert!(
            workspaces
                .workspace(first_id)
                .unwrap()
                .canvas_layout()
                .nodes()
                .is_empty()
        );
        workspaces
            .switch(second_id, Timestamp::from_unix_millis(6))
            .unwrap();
        assert_eq!(workspaces.active_workspace_id(), Some(second_id));

        let mut orchestrator =
            Orchestrator::recover(&mut workspaces, Timestamp::from_unix_millis(7)).unwrap();
        orchestrator
            .accept(
                &mut workspaces,
                &openpodium::ipc::AcceptedMessage {
                    workspace_id: first_id.get(),
                    sender_agent_id: 1,
                    recipient: openpodium::ipc::MessagePeer::Agent(2),
                    command: openpodium::ipc::ProtocolCommand::SendHandoff {
                        message_id: openpodium::ipc::MessageId::new("task-1").unwrap(),
                        recipient_agent_id: 2,
                        kind: openpodium::ipc::HandoffKind::Task,
                        title: Some("Background delivery".to_owned()),
                        body: "Keep the destination stable".to_owned(),
                        parent_message_id: None,
                        response_timeout_ms: None,
                    },
                },
                Timestamp::from_unix_millis(8),
            )
            .unwrap();
        let request = orchestrator
            .prepare_next(&mut workspaces, Timestamp::from_unix_millis(9))
            .unwrap()
            .unwrap();

        assert_eq!(
            delivery_target(&workspaces, &request).unwrap(),
            TerminalKey {
                workspace_id: first_id,
                node_id: NodeId::new(7),
            }
        );
        workspaces
            .switch(first_id, Timestamp::from_unix_millis(10))
            .unwrap();
        let key = TerminalKey {
            workspace_id: first_id,
            node_id: NodeId::new(7),
        };
        let mut terminals = BTreeMap::new();
        terminals.insert(
            key,
            Session::starting(terminal::GridSize::for_node(400.0, 300.0), 1),
        );
        let mut state = test_state(workspaces, terminals);
        persist_canvas(&mut state, CanvasLayout::default(), CanvasLayout::default()).unwrap();
        assert!(
            state.terminals.contains_key(&key),
            "saving the main canvas must preserve hidden floor terminals"
        );
    }

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
            transcript_ticks: 0,
            sidebar_open: true,
            window_size: DEFAULT_WINDOW_SIZE,
            localizer: Localizer::new(openpodium::localization::Locale::EnUs),
            presentation: PresentationPreferences::default(),
            controls: None,
            floor_ui: floors::UiState::default(),
            context_ui: context_nodes::UiState::default(),
            portal_ui: portals::UiState::default(),
            camera: Camera::default(),
            canvas_selection: Vec::new(),
            canvas_preview: None,
            connection_mode: canvas::ConnectionMode::Off,
            connection_error: None,
            canvas_history: History::default(),
            canvas_revision: 1,
            terminals: BTreeMap::new(),
            portal_frames: BTreeMap::new(),
            focused_terminal: None,
            focused_portal: None,
            terminal_generation: 0,
            chat_ui: chat::UiState::default(),
            timeline_ui: timeline_panel::UiState::default(),
            timeline_items: BTreeMap::new(),
            timeline_high_watermarks: BTreeMap::new(),
            supervisor_ui: supervisor_panel::UiState::default(),
            supervisor_snapshot: supervisor::Snapshot::default(),
            notification_limiter: NotificationRateLimiter::default(),
            supervisor_collisions: BTreeMap::new(),
            navigation_ui: navigation_panel::UiState::default(),
            command_registry: CommandRegistry::new(),
            attachment_store: None,
            ipc: None,
            orchestrator: Orchestrator::default(),
            routines: RoutineScheduler::default(),
            trigger_watcher: TriggerWatcher::default(),
            routines_ui: routines_panel::UiState::default(),
            workspaces: Some(workspaces),
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
            selected_role: None,
            editing_preset: None,
            preset_name: String::new(),
            preset_executable: String::new(),
            preset_arguments: String::new(),
            editing_role: None,
            role_name: String::new(),
            role_color: RoleColor::DEFAULT.to_owned(),
            role_icon: RoleIcon::DEFAULT.to_owned(),
            role_instructions: String::new(),
            context_path: String::new(),
            portable_import: None,
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
    fn preset_and_role_controls_persist_safe_agent_configuration() {
        let temp = TempDir::new().unwrap();
        let project = temp.path().join("project");
        fs::create_dir(&project).unwrap();
        let mut workspaces = WorkspaceManager::open(temp.path().join("state.sqlite")).unwrap();
        workspaces
            .create_workspace(&project, Timestamp::from_unix_millis(1))
            .unwrap();
        let mut state = test_state(workspaces, BTreeMap::new());

        state.preset_name = "Safe custom".to_owned();
        state.preset_executable = "/openpodium/does/not/exist".to_owned();
        state.preset_arguments = r#"["--prompt","$(touch /tmp/unsafe)"]"#.to_owned();
        save_preset(&mut state);
        state.role_name = "Reviewer".to_owned();
        state.role_color = "#8B5CF6".to_owned();
        state.role_icon = "review".to_owned();
        state.role_instructions = "Review for regressions".to_owned();
        save_role(&mut state);

        let _task = add_agent(&mut state, AgentProgram::Custom(CommandPresetId::new(1)));
        preview_agent(&mut state, AgentProgram::Custom(CommandPresetId::new(1)));

        let workspace = state
            .workspaces
            .as_ref()
            .unwrap()
            .active_workspace()
            .unwrap();
        let agent = workspace.agent(AgentId::new(1)).unwrap();
        assert_eq!(agent.role_id(), Some(RoleId::new(1)));
        assert_eq!(
            agent.program(),
            AgentProgram::Custom(CommandPresetId::new(1))
        );
        assert_eq!(
            workspace
                .command_preset(CommandPresetId::new(1))
                .unwrap()
                .arguments(),
            ["--prompt", "$(touch /tmp/unsafe)"]
        );
        let preview = state.notice.as_deref().unwrap();
        assert!(preview.contains("arguments: [\"--prompt\", \"$(touch /tmp/unsafe)\"]"));
        assert!(preview.contains("OPENPODIUM_ROLE_INSTRUCTIONS"));
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
    fn toolbar_zoom_commands_update_and_reset_the_canvas_camera() {
        let temp = TempDir::new().unwrap();
        let workspaces = WorkspaceManager::open(temp.path().join("state.sqlite")).unwrap();
        let mut state = test_state(workspaces, BTreeMap::new());

        let _ = update(&mut state, Message::ExecuteCommand(CommandId::ZoomOut));
        assert_eq!(state.camera.zoom_percent(), 83);

        let _ = update(&mut state, Message::ExecuteCommand(CommandId::ZoomIn));
        assert_eq!(state.camera.zoom_percent(), 100);

        let _ = update(&mut state, Message::ExecuteCommand(CommandId::ZoomOut));
        let _ = update(&mut state, Message::ExecuteCommand(CommandId::ResetZoom));
        assert_eq!(state.camera, Camera::default());

        let zoom_out = state
            .command_registry
            .binding(CommandId::ZoomOut)
            .cloned()
            .unwrap();
        let _ = navigation::handle_key(
            &mut state,
            navigation::NavigationKey::Other,
            Some(zoom_out.clone()),
            event::Status::Captured,
        );
        assert_eq!(state.camera.zoom_percent(), 100);

        state.focused_terminal = Some(NodeId::new(1));
        let _ = navigation::handle_key(
            &mut state,
            navigation::NavigationKey::Other,
            Some(zoom_out),
            event::Status::Captured,
        );
        assert_eq!(state.camera.zoom_percent(), 83);
    }

    #[test]
    fn workspace_controls_isolate_canvas_shortcuts_and_release_embedded_focus() {
        let temp = TempDir::new().unwrap();
        let workspaces = WorkspaceManager::open(temp.path().join("state.sqlite")).unwrap();
        let mut state = test_state(workspaces, BTreeMap::new());
        state.focused_terminal = Some(NodeId::new(1));
        state.focused_portal = Some(NodeId::new(2));

        let _ = update(&mut state, Message::OpenControls(Controls::Workspace));
        assert!(state.focused_terminal.is_none());
        assert!(state.focused_portal.is_none());
        let zoom_out = state.command_registry.binding(CommandId::ZoomOut).cloned();
        let _ = navigation::handle_key(
            &mut state,
            navigation::NavigationKey::Other,
            zoom_out.clone(),
            event::Status::Ignored,
        );
        assert_eq!(state.camera.zoom_percent(), 100);

        let _ = navigation::handle_key(
            &mut state,
            navigation::NavigationKey::Escape,
            None,
            event::Status::Ignored,
        );
        assert!(state.controls.is_none());
        let _ = navigation::handle_key(
            &mut state,
            navigation::NavigationKey::Other,
            zoom_out,
            event::Status::Ignored,
        );
        assert_eq!(state.camera.zoom_percent(), 83);
    }

    #[test]
    fn note_editing_opens_on_create_click_and_enter_and_saves_to_disk() {
        let temp = TempDir::new().unwrap();
        let mut workspaces = WorkspaceManager::open(temp.path().join("state.sqlite")).unwrap();
        workspaces.create_workspace(temp.path(), now()).unwrap();
        let mut state = test_state(workspaces, BTreeMap::new());

        let _ = update(
            &mut state,
            Message::AddContextNode(context_nodes::Kind::Note),
        );
        let node_id = state.canvas_selection[0];
        assert_eq!(state.controls, Some(Controls::Node));
        assert!(context_nodes::note_panel(&state.context_ui).is_some());
        let _ = update(
            &mut state,
            Message::EditNote(text_editor::Action::Edit(text_editor::Edit::Paste(
                std::sync::Arc::new("A writable note".to_owned()),
            ))),
        );
        let _ = update(&mut state, Message::SaveNote(false));
        let note_path = temp
            .path()
            .join(format!(".openpodium/notes/{}.md", node_id.get()));
        assert_eq!(
            fs::read_to_string(&note_path).unwrap().trim_end(),
            "A writable note"
        );

        let _ = update(&mut state, Message::CloseControls);
        let _ = handle_canvas_message(&mut state, canvas::Message::SelectionChanged(Vec::new()));
        let _ = handle_canvas_message(&mut state, canvas::Message::EditRequested(node_id));
        assert_eq!(state.controls, Some(Controls::Node));
        assert_eq!(state.canvas_selection, vec![node_id]);
        assert_eq!(
            context_nodes::bodies(&state.context_ui)[&node_id].trim_end(),
            "A writable note"
        );

        let _ = update(&mut state, Message::CloseControls);
        let _ = update(&mut state, Message::ExecuteCommand(CommandId::FocusContent));
        assert_eq!(state.controls, Some(Controls::Node));
        assert!(context_nodes::note_panel(&state.context_ui).is_some());
    }

    #[test]
    fn text_editing_opens_on_create_and_saves_to_the_canvas() {
        let temp = TempDir::new().unwrap();
        let mut workspaces = WorkspaceManager::open(temp.path().join("state.sqlite")).unwrap();
        workspaces.create_workspace(temp.path(), now()).unwrap();
        let mut state = test_state(workspaces, BTreeMap::new());
        let _ = update(
            &mut state,
            Message::AddContextNode(context_nodes::Kind::Text),
        );
        let node_id = state.canvas_selection[0];
        assert_eq!(state.controls, Some(Controls::Node));
        assert!(context_nodes::note_panel(&state.context_ui).is_some());
        let _ = update(
            &mut state,
            Message::EditNote(text_editor::Action::SelectAll),
        );
        let _ = update(
            &mut state,
            Message::EditNote(text_editor::Action::Edit(text_editor::Edit::Paste(
                std::sync::Arc::new("Edited text".to_owned()),
            ))),
        );
        let _ = update(&mut state, Message::SaveCanvasText);
        let node = state
            .workspaces
            .as_ref()
            .unwrap()
            .active_workspace()
            .unwrap()
            .node(node_id)
            .unwrap();
        assert!(
            matches!(node.content(), openpodium::domain::CanvasNodeContent::Text { markdown } if markdown.as_str().trim_end() == "Edited text")
        );
    }

    #[test]
    fn connection_tool_creates_a_durable_link_with_undo_redo_and_duplicate_protection() {
        let temp = TempDir::new().unwrap();
        let database = temp.path().join("state.sqlite");
        let mut workspaces = WorkspaceManager::open(&database).unwrap();
        workspaces.create_workspace(temp.path(), now()).unwrap();
        let mut state = test_state(workspaces, BTreeMap::new());
        let _ = context_nodes::add(&mut state, context_nodes::Kind::Note);
        let source = state.canvas_selection[0];
        let _ = context_nodes::add(&mut state, context_nodes::Kind::Note);
        let target = state.canvas_selection[0];
        state.canvas_selection = vec![source];
        state.focused_terminal = Some(source);
        apply_canvas_action(&mut state, CanvasAction::Connect);
        assert_eq!(
            state.connection_mode,
            canvas::ConnectionMode::PickTarget(source)
        );
        assert!(state.controls.is_none());
        assert!(state.focused_terminal.is_none());

        let _ = handle_canvas_message(&mut state, canvas::Message::ConnectNodes { source, target });
        assert_eq!(state.connection_mode, canvas::ConnectionMode::Off);
        let layout = current_canvas(&state).unwrap();
        assert_eq!(layout.connections().len(), 1);
        assert_eq!(layout.connections()[0].source(), source);
        assert_eq!(layout.connections()[0].target(), target);

        undo_canvas(&mut state);
        assert!(current_canvas(&state).unwrap().connections().is_empty());
        redo_canvas(&mut state);
        assert_eq!(current_canvas(&state).unwrap().connections().len(), 1);
        connect_nodes(&mut state, source, target);
        assert!(
            state
                .connection_error
                .as_deref()
                .unwrap()
                .contains("already connected")
        );
        assert_eq!(current_canvas(&state).unwrap().connections().len(), 1);
        connect_nodes(&mut state, source, source);
        assert_eq!(
            state.connection_error.as_deref(),
            Some("Choose a different card")
        );
        assert_eq!(current_canvas(&state).unwrap().connections().len(), 1);
        let restored = WorkspaceManager::open(database).unwrap();
        assert_eq!(
            restored
                .active_workspace()
                .unwrap()
                .canvas_layout()
                .connections(),
            layout.connections()
        );
    }

    #[test]
    fn connection_tool_can_pick_a_source_and_cancel_without_changing_the_canvas() {
        let temp = TempDir::new().unwrap();
        let mut workspaces = WorkspaceManager::open(temp.path().join("state.sqlite")).unwrap();
        workspaces.create_workspace(temp.path(), now()).unwrap();
        let mut state = test_state(workspaces, BTreeMap::new());
        let _ = context_nodes::add(&mut state, context_nodes::Kind::Note);
        let source = state.canvas_selection[0];
        state.canvas_selection.clear();
        let before = current_canvas(&state).unwrap();
        apply_canvas_action(&mut state, CanvasAction::Connect);
        assert_eq!(state.connection_mode, canvas::ConnectionMode::PickSource);
        let _ = handle_canvas_message(
            &mut state,
            canvas::Message::ConnectionSourceSelected(source),
        );
        assert_eq!(
            state.connection_mode,
            canvas::ConnectionMode::PickTarget(source)
        );
        let delete = state.command_registry.binding(CommandId::Delete).cloned();
        let _ = navigation::handle_key(
            &mut state,
            navigation::NavigationKey::Other,
            delete,
            event::Status::Ignored,
        );
        assert_eq!(current_canvas(&state).unwrap(), before);
        let _ = navigation::handle_key(
            &mut state,
            navigation::NavigationKey::Escape,
            None,
            event::Status::Ignored,
        );
        assert_eq!(state.connection_mode, canvas::ConnectionMode::Off);
        assert_eq!(current_canvas(&state).unwrap(), before);
        apply_canvas_action(&mut state, CanvasAction::Connect);
        let _ = handle_canvas_message(&mut state, canvas::Message::CancelConnection);
        assert_eq!(state.connection_mode, canvas::ConnectionMode::Off);
        apply_canvas_action(&mut state, CanvasAction::Connect);
        apply_canvas_action(&mut state, CanvasAction::Connect);
        assert_eq!(state.connection_mode, canvas::ConnectionMode::Off);
        assert_eq!(current_canvas(&state).unwrap(), before);
    }

    #[test]
    fn chat_draft_is_journaled_and_restored() {
        let temp = TempDir::new().unwrap();
        let database = temp.path().join("state.sqlite");
        let (mut state, workspace_id, thread_id) = state_with_chat(&temp, &database);

        update_chat_draft_text(&mut state, "Unsent **draft**".to_owned());
        assert_eq!(
            state
                .workspaces
                .as_ref()
                .unwrap()
                .workspace(workspace_id)
                .unwrap()
                .chat_thread(thread_id)
                .unwrap()
                .draft()
                .text(),
            "Unsent **draft**"
        );

        drop(state);
        let restored = WorkspaceManager::open(database).unwrap();
        assert_eq!(
            restored
                .workspace(workspace_id)
                .unwrap()
                .chat_thread(thread_id)
                .unwrap()
                .draft()
                .text(),
            "Unsent **draft**"
        );
    }

    #[test]
    fn switching_chat_and_terminal_preserves_the_live_session() {
        let temp = TempDir::new().unwrap();
        let database = temp.path().join("state.sqlite");
        let (mut state, workspace_id, _) = state_with_chat(&temp, &database);
        let key = TerminalKey {
            workspace_id,
            node_id: NodeId::new(1),
        };
        state.terminals.insert(
            key,
            Session::starting(terminal::GridSize::for_node(400.0, 300.0), 17),
        );

        let _task = handle_chat_message(
            &mut state,
            chat::Message::SurfaceChanged(chat::Surface::Chat),
        );
        let session = state.terminals.get(&key).expect("terminal is preserved");
        assert_eq!(session.generation(), 17);
        assert!(session.is_active());
        assert_eq!(
            state.chat_ui.surface(workspace_id, AgentId::new(1)),
            chat::Surface::Chat
        );
    }

    #[test]
    fn submitting_without_a_terminal_keeps_the_durable_message() {
        let temp = TempDir::new().unwrap();
        let database = temp.path().join("state.sqlite");
        let (mut state, workspace_id, thread_id) = state_with_chat(&temp, &database);
        update_chat_draft_text(&mut state, "Run the targeted tests".to_owned());

        submit_chat_draft(&mut state);

        let thread = state
            .workspaces
            .as_ref()
            .unwrap()
            .workspace(workspace_id)
            .unwrap()
            .chat_thread(thread_id)
            .unwrap();
        assert_eq!(thread.messages().len(), 1);
        assert_eq!(
            thread.messages()[0].content().as_str(),
            "Run the targeted tests"
        );
        assert!(thread.draft().is_empty());
        assert_eq!(
            state.notice.as_deref(),
            Some("Prompt saved; start the terminal to deliver it manually")
        );
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

    #[test]
    fn notification_navigation_selects_the_workspace_node_and_task() {
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
            .execute(
                second_id,
                DomainCommand::AddAgent(Agent::new(
                    AgentId::new(1),
                    Name::new("Lead").unwrap(),
                    None,
                )),
                Timestamp::from_unix_millis(3),
            )
            .unwrap();
        workspaces
            .execute(
                second_id,
                DomainCommand::AddAgentNode {
                    agent: Agent::new(AgentId::new(2), Name::new("Builder").unwrap(), None),
                    node: Node::new(
                        NodeId::new(9),
                        NodeTarget::Agent(AgentId::new(2)),
                        CanvasPoint::new(0.0, 0.0).unwrap(),
                        CanvasSize::new(400.0, 300.0).unwrap(),
                    ),
                },
                Timestamp::from_unix_millis(4),
            )
            .unwrap();
        let task_id = openpodium::domain::TaskId::new(1);
        let task = openpodium::domain::Task::new(
            task_id,
            Name::new("Inspect me").unwrap(),
            Content::new("Open the right target").unwrap(),
            Some(AgentId::new(2)),
            None,
        );
        let handoff = openpodium::domain::Handoff::tracked(
            openpodium::domain::HandoffId::new(1),
            openpodium::domain::HandoffMessageId::new("inspect-1").unwrap(),
            openpodium::domain::HandoffOrigin::Agent(AgentId::new(1)),
            AgentId::new(2),
            openpodium::domain::HandoffPayload::Task(task_id),
            None,
            Timestamp::from_unix_millis(5),
            None,
        )
        .unwrap();
        workspaces
            .execute(
                second_id,
                DomainCommand::AddTaskHandoff { task, handoff },
                Timestamp::from_unix_millis(5),
            )
            .unwrap();
        let target =
            timeline::navigation_target(workspaces.workspace(second_id).unwrap(), task_id).unwrap();
        workspaces
            .switch(first_id, Timestamp::from_unix_millis(6))
            .unwrap();
        let mut state = test_state(workspaces, BTreeMap::new());

        navigate_to_task(&mut state, target);

        assert_eq!(active_workspace_id(&state), Some(second_id));
        assert_eq!(state.canvas_selection, vec![NodeId::new(9)]);
        assert_eq!(state.timeline_ui.selected_task(second_id), Some(task_id));

        let before = state
            .workspaces
            .as_ref()
            .unwrap()
            .workspace(second_id)
            .unwrap()
            .canvas_layout();
        let reused_node = Node::new(
            NodeId::new(9),
            NodeTarget::Agent(AgentId::new(1)),
            CanvasPoint::new(0.0, 0.0).unwrap(),
            CanvasSize::new(400.0, 300.0).unwrap(),
        );
        state
            .workspaces
            .as_mut()
            .unwrap()
            .execute(
                second_id,
                DomainCommand::ReplaceCanvas {
                    before,
                    after: CanvasLayout::new(vec![reused_node], vec![], vec![]),
                },
                Timestamp::from_unix_millis(7),
            )
            .unwrap();

        navigate_to_task(&mut state, target);

        assert!(state.canvas_selection.is_empty());
    }

    #[test]
    fn search_navigation_restores_workspace_floor_node_and_chat_message() {
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
        let agent_id = AgentId::new(1);
        let node_id = NodeId::new(7);
        let thread_id = ChatThreadId::new(3);
        let message_id = ChatMessageId::new(4);
        workspaces
            .execute(
                first_id,
                DomainCommand::AddAgentNode {
                    agent: Agent::new(agent_id, Name::new("Builder").unwrap(), None),
                    node: Node::new(
                        node_id,
                        NodeTarget::Agent(agent_id),
                        CanvasPoint::new(100.0, 200.0).unwrap(),
                        CanvasSize::new(400.0, 300.0).unwrap(),
                    ),
                },
                Timestamp::from_unix_millis(3),
            )
            .unwrap();
        workspaces
            .execute(
                first_id,
                DomainCommand::AddChatThread(ChatThread::new(
                    thread_id,
                    agent_id,
                    Name::new("Search results").unwrap(),
                )),
                Timestamp::from_unix_millis(4),
            )
            .unwrap();
        workspaces
            .execute(
                first_id,
                DomainCommand::AppendAgentChatMessage(
                    openpodium::domain::ChatMessage::new(
                        message_id,
                        thread_id,
                        openpodium::domain::ChatAuthor::Agent(agent_id),
                        Content::new("The matching message").unwrap(),
                        vec![],
                        vec![],
                        Timestamp::from_unix_millis(5),
                    )
                    .unwrap(),
                ),
                Timestamp::from_unix_millis(5),
            )
            .unwrap();
        let before = workspaces.workspace(first_id).unwrap().floors().clone();
        let mut after = before.clone();
        after.entries.insert(
            1,
            openpodium::domain::Floor {
                name: Name::new("Search floor").unwrap(),
                directory: WorkspaceDirectory::new(first.to_str().unwrap()).unwrap(),
                repository: WorkspaceDirectory::new(first.to_str().unwrap()).unwrap(),
                branch: Some("search-floor".to_owned()),
                base_revision: "base".to_owned(),
                base_branch: Some("main".to_owned()),
                managed: false,
                ownership_token: None,
                owner: None,
                dirty: false,
                lifecycle: openpodium::domain::FloorLifecycle::Available,
            },
        );
        after.node_floors.insert(node_id, 1);
        workspaces
            .execute(
                first_id,
                DomainCommand::ReplaceFloors { before, after },
                Timestamp::from_unix_millis(6),
            )
            .unwrap();
        assert_eq!(workspaces.active_workspace_id(), Some(second_id));
        let mut state = test_state(workspaces, BTreeMap::new());

        let _task = navigation::navigate_to_search_target(
            &mut state,
            openpodium::navigation::SearchTarget {
                workspace_id: first_id,
                floor_id: Some(1),
                node_id: Some(node_id),
                content: Some(openpodium::navigation::ContentTarget::Chat {
                    agent_id,
                    thread_id,
                    message_id,
                    character_offset: 4,
                }),
            },
        );

        assert_eq!(active_workspace_id(&state), Some(first_id));
        assert_eq!(
            state
                .workspaces
                .as_ref()
                .unwrap()
                .active_workspace()
                .unwrap()
                .floors()
                .active,
            Some(1)
        );
        assert_eq!(state.canvas_selection, vec![node_id]);
        assert_eq!(state.camera.position().x, 300.0);
        assert_eq!(state.camera.position().y, 350.0);
        assert_eq!(
            state.chat_ui.surface(first_id, agent_id),
            chat::Surface::Chat
        );
        assert_eq!(
            state.chat_ui.selected_thread(
                state
                    .workspaces
                    .as_ref()
                    .unwrap()
                    .active_workspace()
                    .unwrap(),
                agent_id,
            ),
            Some(thread_id)
        );
        assert_eq!(
            state.notice.as_deref(),
            Some("Opened matching message near character 4")
        );
    }

    #[test]
    fn saving_an_existing_routine_preserves_its_contract_and_rename() {
        let temp = TempDir::new().unwrap();
        let project = temp.path().join("project");
        fs::create_dir(&project).unwrap();
        let mut workspaces = WorkspaceManager::open(temp.path().join("openpodium.sqlite")).unwrap();
        let workspace_id = workspaces
            .create_workspace(&project, Timestamp::from_unix_millis(1))
            .unwrap();
        workspaces
            .execute(
                workspace_id,
                DomainCommand::AddAgent(Agent::with_program(
                    AgentId::new(1),
                    Name::new("Builder").unwrap(),
                    None,
                    AgentProgram::Codex,
                )),
                Timestamp::from_unix_millis(2),
            )
            .unwrap();

        let routine_id = openpodium::domain::RoutineId::new(1);
        let input = openpodium::domain::RoutineInputDeclaration::new(
            openpodium::domain::RoutineInputKey::new("scope").unwrap(),
            Name::new("Scope").unwrap(),
            true,
            None,
        );
        let original_step = RoutineStep::new(
            RoutineStepId::new(7),
            Name::new("Plan").unwrap(),
            AgentId::new(1),
            Content::new("Plan {{scope}}").unwrap(),
            [],
            [(
                openpodium::domain::RoutineInputKey::new("scope").unwrap(),
                openpodium::domain::RoutineBindingSource::Input(
                    openpodium::domain::RoutineInputKey::new("scope").unwrap(),
                ),
            )],
            [openpodium::domain::RoutineOutputKey::new("plan").unwrap()],
            RoutineApproval::Required,
            RoutineRetryPolicy::new(3).unwrap(),
            RoutineStepClaims::default(),
        )
        .unwrap();
        let version = RoutineVersion::new(
            openpodium::domain::RoutineVersionId::new(1),
            routine_id,
            1,
            vec![input.clone()],
            vec![original_step.clone()],
            None,
            Timestamp::from_unix_millis(3),
        )
        .unwrap();
        workspaces
            .execute(
                workspace_id,
                DomainCommand::AddRoutine(
                    Routine::new(routine_id, Name::new("Draft").unwrap(), None, version).unwrap(),
                ),
                Timestamp::from_unix_millis(4),
            )
            .unwrap();

        let mut state = test_state(workspaces, BTreeMap::new());
        state.routines_ui.select(
            state
                .workspaces
                .as_ref()
                .unwrap()
                .workspace(workspace_id)
                .unwrap(),
            Some(routine_id),
        );
        state.routines_ui.name = "Renamed".to_owned();
        state
            .routines_ui
            .pending_steps
            .push(routines_panel::PendingStep {
                name: "Build".to_owned(),
                prompt: "Build the plan".to_owned(),
                agent_id: AgentId::new(1),
            });

        save_routine(&mut state, workspace_id).unwrap();

        let routine = state
            .workspaces
            .as_ref()
            .unwrap()
            .workspace(workspace_id)
            .unwrap()
            .routine(routine_id)
            .unwrap();
        assert_eq!(routine.name().as_str(), "Renamed");
        assert_eq!(routine.latest_version().number(), 2);
        assert_eq!(routine.latest_version().inputs(), &[input]);
        assert_eq!(routine.latest_version().steps().len(), 2);
        assert_eq!(routine.latest_version().steps()[0], original_step);
        assert_eq!(
            routine.latest_version().steps()[1]
                .depends_on()
                .collect::<Vec<_>>(),
            vec![RoutineStepId::new(7)]
        );
    }

    #[test]
    fn changing_one_application_preference_does_not_freeze_system_defaults() {
        let temp = TempDir::new().unwrap();
        let workspaces = WorkspaceManager::open(temp.path().join("state.sqlite")).unwrap();
        let mut state = test_state(workspaces, BTreeMap::new());

        let _ = update(&mut state, Message::CycleLocale);
        assert_eq!(state.localizer.locale(), Locale::PtBr);
        assert_eq!(
            state.workspaces.as_ref().unwrap().preferences().unwrap(),
            vec![(LOCALE_KEY.to_owned(), "pt-BR".to_owned())]
        );

        let previous_contrast = state.presentation.high_contrast();
        let previous_canvas_revision = state.canvas_revision;
        let _ = update(&mut state, Message::ToggleHighContrast);
        assert_eq!(state.presentation.high_contrast(), !previous_contrast);
        assert_ne!(state.canvas_revision, previous_canvas_revision);
        assert_eq!(
            state.workspaces.as_ref().unwrap().preferences().unwrap(),
            vec![
                (
                    HIGH_CONTRAST_KEY.to_owned(),
                    (!previous_contrast).to_string()
                ),
                (LOCALE_KEY.to_owned(), "pt-BR".to_owned()),
            ]
        );
    }

    #[test]
    fn interface_scale_and_workspace_controls_preserve_canvas_geometry() {
        let temp = TempDir::new().unwrap();
        let workspaces = WorkspaceManager::open(temp.path().join("state.sqlite")).unwrap();
        let mut state = test_state(workspaces, BTreeMap::new());
        state
            .presentation
            .apply_stored([(TEXT_SCALE_KEY.to_owned(), "1".to_owned())]);
        let initial_viewport = canvas_viewport(&state);
        let _ = update(&mut state, Message::OpenControls(Controls::Workspace));
        assert_eq!(canvas_viewport(&state), initial_viewport);

        for _ in 0..4 {
            let _ = update(&mut state, Message::CycleTextScale);
        }
        assert_eq!(state.presentation.text_scale_percent(), 200);
        assert!((state.window_size.width - 640.0).abs() < 0.01);
        let with_controls = canvas_viewport(&state);
        assert!(with_controls.0 > 400.0);
        let _ = update(&mut state, Message::CloseControls);
        assert_eq!(canvas_viewport(&state), with_controls);

        let _ = update(&mut state, Message::WindowResized(Size::new(900.0, 620.0)));
        assert!(canvas_viewport(&state).0 > 670.0);
    }

    #[test]
    fn selecting_a_project_directory_creates_and_activates_its_workspace() {
        let temp = TempDir::new().unwrap();
        let project = temp.path().join("selected-project");
        fs::create_dir(&project).unwrap();
        let workspaces = WorkspaceManager::open(temp.path().join("state.sqlite")).unwrap();
        let mut state = test_state(workspaces, BTreeMap::new());

        let _ = update(&mut state, Message::ProjectDirectorySelected(Some(project)));

        let workspace = state
            .workspaces
            .as_ref()
            .and_then(WorkspaceManager::active_workspace)
            .expect("the selected directory should become the active workspace");
        assert_eq!(workspace.name(), "selected-project");
        assert_eq!(state.notice.as_deref(), Some("Workspace created"));
    }

    pub(super) fn test_state(
        workspaces: WorkspaceManager,
        terminals: BTreeMap<TerminalKey, Session>,
    ) -> OpenPodium {
        OpenPodium {
            transcript_ticks: 0,
            sidebar_open: true,
            window_size: DEFAULT_WINDOW_SIZE,
            localizer: Localizer::new(openpodium::localization::Locale::EnUs),
            presentation: PresentationPreferences::default(),
            controls: None,
            floor_ui: floors::UiState::default(),
            context_ui: context_nodes::UiState::default(),
            portal_ui: portals::UiState::default(),
            camera: Camera::default(),
            canvas_selection: Vec::new(),
            canvas_preview: None,
            connection_mode: canvas::ConnectionMode::Off,
            connection_error: None,
            canvas_history: History::default(),
            canvas_revision: 1,
            terminals,
            portal_frames: BTreeMap::new(),
            focused_terminal: None,
            focused_portal: None,
            terminal_generation: 0,
            chat_ui: chat::UiState::default(),
            timeline_ui: timeline_panel::UiState::default(),
            timeline_items: BTreeMap::new(),
            timeline_high_watermarks: BTreeMap::new(),
            supervisor_ui: supervisor_panel::UiState::default(),
            supervisor_snapshot: supervisor::Snapshot::default(),
            notification_limiter: NotificationRateLimiter::default(),
            supervisor_collisions: BTreeMap::new(),
            navigation_ui: navigation_panel::UiState::default(),
            command_registry: CommandRegistry::new(),
            attachment_store: None,
            ipc: None,
            orchestrator: Orchestrator::default(),
            routines: RoutineScheduler::default(),
            trigger_watcher: TriggerWatcher::default(),
            routines_ui: routines_panel::UiState::default(),
            workspaces: Some(workspaces),
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
            selected_role: None,
            editing_preset: None,
            preset_name: String::new(),
            preset_executable: String::new(),
            preset_arguments: String::new(),
            editing_role: None,
            role_name: String::new(),
            role_color: RoleColor::DEFAULT.to_owned(),
            role_icon: RoleIcon::DEFAULT.to_owned(),
            role_instructions: String::new(),
            context_path: String::new(),
            portable_import: None,
            notice: None,
        }
    }

    fn state_with_chat(temp: &TempDir, database: &Path) -> (OpenPodium, WorkspaceId, ChatThreadId) {
        let project = temp.path().join("project");
        fs::create_dir_all(&project).unwrap();
        let mut workspaces = WorkspaceManager::open(database).unwrap();
        let workspace_id = workspaces
            .create_workspace(&project, Timestamp::from_unix_millis(1))
            .unwrap();
        let node = Node::new(
            NodeId::new(1),
            NodeTarget::Agent(AgentId::new(1)),
            CanvasPoint::new(0.0, 0.0).unwrap(),
            CanvasSize::new(400.0, 300.0).unwrap(),
        );
        workspaces
            .execute(
                workspace_id,
                DomainCommand::AddAgentNode {
                    agent: Agent::new(AgentId::new(1), Name::new("Builder").unwrap(), None),
                    node,
                },
                Timestamp::from_unix_millis(2),
            )
            .unwrap();
        let thread_id = ChatThreadId::new(1);
        workspaces
            .execute(
                workspace_id,
                DomainCommand::AddChatThread(ChatThread::new(
                    thread_id,
                    AgentId::new(1),
                    Name::new("General").unwrap(),
                )),
                Timestamp::from_unix_millis(3),
            )
            .unwrap();
        let mut state = test_state(workspaces, BTreeMap::new());
        state.canvas_selection = vec![NodeId::new(1)];
        state
            .chat_ui
            .select_thread(workspace_id, AgentId::new(1), thread_id);
        (state, workspace_id, thread_id)
    }
}
