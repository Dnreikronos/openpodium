use std::env;
use std::path::Path;
use std::sync::Arc;

use openpodium::domain::AgentProgram;
use openpodium::runtime::{
    ProcessController, ProcessEvent, ProcessSpec, ProcessTermination, RunningProcess, TerminalSize,
};
use tokio::sync::Mutex;

use super::{GridSize, InputMode, Model, Status, Update, View, encode_paste};

pub(crate) type ProcessStream = Arc<Mutex<RunningProcess>>;

pub(crate) enum Action {
    ClipboardStore(String),
    Bell,
}

pub(crate) struct Session {
    model: Model,
    status: Status,
    controller: Option<ProcessController>,
    stream: Option<ProcessStream>,
    generation: u64,
}

impl Session {
    pub(crate) fn starting(size: GridSize, generation: u64) -> Self {
        Self {
            model: Model::new(size),
            status: Status::Starting,
            controller: None,
            stream: None,
            generation,
        }
    }

    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }

    pub(crate) fn attach(&mut self, stream: ProcessStream) -> Result<(), &'static str> {
        let controller = stream
            .try_lock()
            .map_err(|_| "the process stream was busy before event polling started")?
            .controller();
        self.controller = Some(controller);
        self.stream = Some(stream);
        self.status = Status::Running;
        Ok(())
    }

    pub(crate) fn fail(&mut self, error: impl ToString) {
        self.controller = None;
        self.stream = None;
        self.status = Status::Failed(error.to_string());
    }

    pub(crate) fn stream(&self) -> Option<ProcessStream> {
        self.stream.clone()
    }

    pub(crate) fn is_active(&self) -> bool {
        matches!(self.status, Status::Starting | Status::Running)
    }

    pub(crate) fn view(&self) -> View {
        self.model.view(self.status.clone())
    }

    pub(crate) fn input_mode(&self) -> InputMode {
        self.model.input_mode()
    }

    pub(crate) fn write(&self, bytes: &[u8]) -> Result<(), String> {
        self.controller
            .as_ref()
            .ok_or_else(|| "the terminal process is not running".to_owned())?
            .write_input(bytes)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn paste(&self, text: &str) -> Result<(), String> {
        self.write(&encode_paste(text, self.input_mode()))
    }

    pub(crate) fn resize(&mut self, size: GridSize) -> Result<(), String> {
        self.model.resize(size);
        let Some(controller) = &self.controller else {
            return Ok(());
        };
        controller
            .resize(
                TerminalSize::new(size.rows, size.columns)
                    .expect("terminal grid dimensions are always non-zero"),
            )
            .map_err(|error| error.to_string())
    }

    pub(crate) fn scroll(&mut self, lines: i32) {
        self.model.scroll(lines);
    }

    pub(crate) fn begin_selection(&mut self, row: usize, column: usize, right_side: bool) {
        self.model.begin_selection(row, column, right_side);
    }

    pub(crate) fn update_selection(&mut self, row: usize, column: usize, right_side: bool) {
        self.model.update_selection(row, column, right_side);
    }

    pub(crate) fn clear_selection(&mut self) {
        self.model.clear_selection();
    }

    pub(crate) fn selected_text(&self) -> Option<String> {
        self.model.selected_text()
    }

    pub(crate) fn handle_event(&mut self, event: ProcessEvent) -> Vec<Action> {
        match event {
            ProcessEvent::Output(bytes) => self
                .model
                .feed(&bytes)
                .into_iter()
                .filter_map(|update| match update {
                    Update::PtyWrite(bytes) => {
                        if let Err(error) = self.write(&bytes) {
                            self.status = Status::Failed(error.to_string());
                        }
                        None
                    }
                    Update::ClipboardStore(text) => Some(Action::ClipboardStore(text)),
                    Update::Bell => Some(Action::Bell),
                    Update::TitleChanged(title) => {
                        let _ = title;
                        None
                    }
                })
                .collect(),
            ProcessEvent::Terminated(termination) => {
                self.controller = None;
                self.stream = None;
                self.status = match termination {
                    ProcessTermination::Exited(exit) => {
                        let detail = exit
                            .signal()
                            .map_or_else(|| exit.code().to_string(), |signal| signal.to_owned());
                        Status::Exited(detail)
                    }
                    ProcessTermination::Cancelled(_) => Status::Stopped,
                    ProcessTermination::Failed(error) => Status::Failed(error.to_string()),
                };
                Vec::new()
            }
        }
    }

    pub(crate) fn stop(&mut self) -> Result<(), String> {
        if let Some(controller) = self.controller.take() {
            controller.cancel().map_err(|error| error.to_string())?;
        }
        self.stream = None;
        if self.is_active() {
            self.status = Status::Stopped;
        }
        Ok(())
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

pub(crate) fn process_spec(
    program: AgentProgram,
    working_directory: &Path,
    size: GridSize,
) -> ProcessSpec {
    let spec = match program {
        AgentProgram::Codex => ProcessSpec::new("codex", working_directory),
        AgentProgram::Claude => ProcessSpec::new("claude", working_directory),
        AgentProgram::Shell => shell_spec(working_directory),
    };
    spec.with_terminal_size(
        TerminalSize::new(size.rows, size.columns)
            .expect("terminal grid dimensions are always non-zero"),
    )
    .env("TERM", "xterm-256color")
    .env("COLORTERM", "truecolor")
    .env("TERM_PROGRAM", "OpenPodium")
}

#[cfg(unix)]
fn shell_spec(working_directory: &Path) -> ProcessSpec {
    let shell = env::var_os("SHELL").unwrap_or_else(|| "/bin/sh".into());
    ProcessSpec::new(shell, working_directory).arg("-l")
}

#[cfg(windows)]
fn shell_spec(working_directory: &Path) -> ProcessSpec {
    let shell = env::var_os("COMSPEC").unwrap_or_else(|| "cmd.exe".into());
    ProcessSpec::new(shell, working_directory)
}
