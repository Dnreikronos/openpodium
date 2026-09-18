use std::env;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use iced::widget::{button, column, container, row, scrollable, text, text_input};
use iced::{Element, Fill, Theme};
use openpodium::domain::{Timestamp, WorkspaceId};
use openpodium::workspaces::{WorkspaceManager, WorkspaceSettingsInput};

use crate::canvas::Camera;

const APP_NAME: &str = "OpenPodium";
const DATABASE_FILE: &str = "openpodium.sqlite";

struct OpenPodium {
    camera: Camera,
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

#[derive(Debug, Clone)]
enum Message {
    CreateDirectoryChanged(String),
    CreateWorkspace,
    SwitchWorkspace(WorkspaceId),
    NameChanged(String),
    IconChanged(String),
    WorkingDirectoryChanged(String),
    InstructionsChanged(String),
    SaveSettings,
}

pub(crate) fn run() -> iced::Result {
    iced::application(OpenPodium::default, update, view)
        .title(APP_NAME)
        .theme(Theme::Dark)
        .centered()
        .run()
}

fn update(state: &mut OpenPodium, message: Message) {
    match message {
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
                return;
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
    }
}

fn view(state: &OpenPodium) -> Element<'_, Message> {
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
        text(format!("Zoom: {}%", state.camera.zoom_percent())).size(12),
    ]
    .spacing(12)
    .max_width(720);
    if state
        .workspaces
        .as_ref()
        .and_then(WorkspaceManager::active_workspace)
        .is_none()
    {
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

    let stage = container(settings)
        .width(Fill)
        .height(Fill)
        .center(Fill)
        .padding(32);

    row![sidebar, stage].into()
}

impl OpenPodium {
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
