use std::error::Error;
use std::ffi::{OsStr, OsString};
use std::fmt::{self, Display, Formatter};
use std::path::{Path, PathBuf};

mod local;

pub use local::{LocalProcessRuntime, RunningProcess};

const DEFAULT_OUTPUT_CAPACITY: usize = 64;

pub trait ProcessRuntime: Send + Sync {
    fn spawn(&self, spec: ProcessSpec) -> Result<RunningProcess, RuntimeError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessSpec {
    program: OsString,
    arguments: Vec<OsString>,
    working_directory: PathBuf,
    environment: Vec<(OsString, OsString)>,
    terminal_size: TerminalSize,
    output_capacity: usize,
}

impl ProcessSpec {
    pub fn new(program: impl Into<OsString>, working_directory: impl Into<PathBuf>) -> Self {
        Self {
            program: program.into(),
            arguments: Vec::new(),
            working_directory: working_directory.into(),
            environment: Vec::new(),
            terminal_size: TerminalSize::default(),
            output_capacity: DEFAULT_OUTPUT_CAPACITY,
        }
    }

    pub fn arg(mut self, argument: impl Into<OsString>) -> Self {
        self.arguments.push(argument.into());
        self
    }

    pub fn args<I, S>(mut self, arguments: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        self.arguments.extend(arguments.into_iter().map(Into::into));
        self
    }

    pub fn env(mut self, name: impl Into<OsString>, value: impl Into<OsString>) -> Self {
        self.environment.push((name.into(), value.into()));
        self
    }

    pub const fn with_terminal_size(mut self, terminal_size: TerminalSize) -> Self {
        self.terminal_size = terminal_size;
        self
    }

    pub const fn with_output_capacity(mut self, output_capacity: usize) -> Self {
        self.output_capacity = output_capacity;
        self
    }

    pub fn program(&self) -> &OsStr {
        &self.program
    }

    pub fn arguments(&self) -> &[OsString] {
        &self.arguments
    }

    pub fn working_directory(&self) -> &Path {
        &self.working_directory
    }

    pub fn environment(&self) -> &[(OsString, OsString)] {
        &self.environment
    }

    pub const fn terminal_size(&self) -> TerminalSize {
        self.terminal_size
    }

    pub const fn output_capacity(&self) -> usize {
        self.output_capacity
    }

    fn validate(&self) -> Result<(), RuntimeError> {
        if self.program.is_empty() {
            return Err(RuntimeError::new(
                RuntimeOperation::Validate,
                "the process program cannot be empty",
            ));
        }
        if self.output_capacity == 0 {
            return Err(RuntimeError::new(
                RuntimeOperation::Validate,
                "the output capacity must be greater than zero",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerminalSize {
    rows: u16,
    columns: u16,
    pixel_width: u16,
    pixel_height: u16,
}

impl TerminalSize {
    pub fn new(rows: u16, columns: u16) -> Result<Self, RuntimeError> {
        Self::with_pixels(rows, columns, 0, 0)
    }

    pub fn with_pixels(
        rows: u16,
        columns: u16,
        pixel_width: u16,
        pixel_height: u16,
    ) -> Result<Self, RuntimeError> {
        if rows == 0 || columns == 0 {
            return Err(RuntimeError::new(
                RuntimeOperation::Validate,
                "terminal rows and columns must be greater than zero",
            ));
        }
        Ok(Self {
            rows,
            columns,
            pixel_width,
            pixel_height,
        })
    }

    pub const fn rows(self) -> u16 {
        self.rows
    }

    pub const fn columns(self) -> u16 {
        self.columns
    }

    pub const fn pixel_width(self) -> u16 {
        self.pixel_width
    }

    pub const fn pixel_height(self) -> u16 {
        self.pixel_height
    }
}

impl Default for TerminalSize {
    fn default() -> Self {
        Self {
            rows: 24,
            columns: 80,
            pixel_width: 0,
            pixel_height: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcessEvent {
    Output(Vec<u8>),
    Terminated(ProcessTermination),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcessTermination {
    Exited(ProcessExit),
    Cancelled(ProcessExit),
    Failed(RuntimeError),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessExit {
    code: u32,
    signal: Option<String>,
    success: bool,
}

impl ProcessExit {
    pub const fn code(&self) -> u32 {
        self.code
    }

    pub fn signal(&self) -> Option<&str> {
        self.signal.as_deref()
    }

    pub const fn success(&self) -> bool {
        self.success
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeOperation {
    Validate,
    OpenPty,
    CloneReader,
    TakeWriter,
    Spawn,
    SpawnWorker,
    ReadOutput,
    WriteInput,
    CloseInput,
    Resize,
    Cancel,
    Wait,
}

impl Display for RuntimeOperation {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Validate => "validate process specification",
            Self::OpenPty => "open pseudo-terminal",
            Self::CloneReader => "create pseudo-terminal reader",
            Self::TakeWriter => "create pseudo-terminal writer",
            Self::Spawn => "start child process",
            Self::SpawnWorker => "start runtime worker",
            Self::ReadOutput => "read process output",
            Self::WriteInput => "write process input",
            Self::CloseInput => "close process input",
            Self::Resize => "resize pseudo-terminal",
            Self::Cancel => "cancel child process",
            Self::Wait => "observe child process exit",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeError {
    operation: RuntimeOperation,
    message: String,
}

impl RuntimeError {
    pub(crate) fn new(operation: RuntimeOperation, message: impl Into<String>) -> Self {
        Self {
            operation,
            message: message.into(),
        }
    }

    pub const fn operation(&self) -> RuntimeOperation {
        self.operation
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl Display for RuntimeError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        write!(formatter, "could not {}: {}", self.operation, self.message)
    }
}

impl Error for RuntimeError {}

#[cfg(test)]
mod tests;
