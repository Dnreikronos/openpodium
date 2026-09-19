use std::error::Error;
use std::fmt::{self, Display, Formatter};

use super::{EnvironmentProfileId, Name, WorkspaceDirectory};

const MAX_ARGUMENTS: usize = 256;
const MAX_VALUE_CHARS: usize = 32_768;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvironmentProfile {
    id: EnvironmentProfileId,
    name: Name,
    kind: EnvironmentKind,
}

impl EnvironmentProfile {
    pub const fn new(id: EnvironmentProfileId, name: Name, kind: EnvironmentKind) -> Self {
        Self { id, name, kind }
    }

    pub const fn id(&self) -> EnvironmentProfileId {
        self.id
    }

    pub const fn name(&self) -> &Name {
        &self.name
    }

    pub const fn kind(&self) -> &EnvironmentKind {
        &self.kind
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnvironmentKind {
    Ssh(SshEnvironment),
    Container(ContainerEnvironment),
    Custom(CustomEnvironment),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SshEnvironment {
    host: String,
    user: Option<String>,
    port: Option<u16>,
    working_directory: String,
}

impl SshEnvironment {
    pub fn new(
        host: impl Into<String>,
        user: Option<String>,
        port: Option<u16>,
        working_directory: impl Into<String>,
    ) -> Result<Self, EnvironmentValidationError> {
        let host = validate_identifier("SSH host", host.into())?;
        let user = user
            .map(|value| validate_identifier("SSH user", value))
            .transpose()?;
        if port == Some(0) {
            return Err(EnvironmentValidationError::new(
                "SSH port",
                EnvironmentValidationProblem::Zero,
            ));
        }
        let working_directory =
            validate_text("remote working directory", working_directory.into())?;
        Ok(Self {
            host,
            user,
            port,
            working_directory,
        })
    }

    pub fn host(&self) -> &str {
        &self.host
    }

    pub fn user(&self) -> Option<&str> {
        self.user.as_deref()
    }

    pub const fn port(&self) -> Option<u16> {
        self.port
    }

    pub fn working_directory(&self) -> &str {
        &self.working_directory
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContainerEnvironment {
    engine: String,
    container: String,
    working_directory: String,
}

impl ContainerEnvironment {
    pub fn new(
        engine: impl Into<String>,
        container: impl Into<String>,
        working_directory: impl Into<String>,
    ) -> Result<Self, EnvironmentValidationError> {
        Ok(Self {
            engine: validate_text("container engine", engine.into())?,
            container: validate_identifier("container", container.into())?,
            working_directory: validate_text(
                "container working directory",
                working_directory.into(),
            )?,
        })
    }

    pub fn engine(&self) -> &str {
        &self.engine
    }

    pub fn container(&self) -> &str {
        &self.container
    }

    pub fn working_directory(&self) -> &str {
        &self.working_directory
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomEnvironment {
    executable: String,
    arguments: Vec<String>,
    working_directory: WorkspaceDirectory,
}

impl CustomEnvironment {
    pub fn new(
        executable: impl Into<String>,
        arguments: Vec<String>,
        working_directory: WorkspaceDirectory,
    ) -> Result<Self, EnvironmentValidationError> {
        if arguments.len() > MAX_ARGUMENTS {
            return Err(EnvironmentValidationError::new(
                "custom arguments",
                EnvironmentValidationProblem::TooManyArguments { max: MAX_ARGUMENTS },
            ));
        }
        let arguments = arguments
            .into_iter()
            .map(validate_argument)
            .collect::<Result<_, _>>()?;
        Ok(Self {
            executable: validate_text("custom executable", executable.into())?,
            arguments,
            working_directory,
        })
    }

    pub fn executable(&self) -> &str {
        &self.executable
    }

    pub fn arguments(&self) -> &[String] {
        &self.arguments
    }

    pub const fn working_directory(&self) -> &WorkspaceDirectory {
        &self.working_directory
    }
}

fn validate_identifier(
    field: &'static str,
    value: String,
) -> Result<String, EnvironmentValidationError> {
    let value = validate_text(field, value)?;
    if value.starts_with('-') || value.chars().any(char::is_whitespace) {
        return Err(EnvironmentValidationError::new(
            field,
            EnvironmentValidationProblem::InvalidIdentifier,
        ));
    }
    Ok(value)
}

fn validate_text(field: &'static str, value: String) -> Result<String, EnvironmentValidationError> {
    if value.is_empty() {
        return Err(EnvironmentValidationError::new(
            field,
            EnvironmentValidationProblem::Empty,
        ));
    }
    if value.chars().count() > MAX_VALUE_CHARS {
        return Err(EnvironmentValidationError::new(
            field,
            EnvironmentValidationProblem::TooLong {
                max_chars: MAX_VALUE_CHARS,
            },
        ));
    }
    if value.contains('\0') || value.contains('\n') || value.contains('\r') {
        return Err(EnvironmentValidationError::new(
            field,
            EnvironmentValidationProblem::ControlCharacter,
        ));
    }
    Ok(value)
}

fn validate_argument(value: String) -> Result<String, EnvironmentValidationError> {
    if value.chars().count() > MAX_VALUE_CHARS {
        return Err(EnvironmentValidationError::new(
            "custom argument",
            EnvironmentValidationProblem::TooLong {
                max_chars: MAX_VALUE_CHARS,
            },
        ));
    }
    if value.contains('\0') {
        return Err(EnvironmentValidationError::new(
            "custom argument",
            EnvironmentValidationProblem::ControlCharacter,
        ));
    }
    Ok(value)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnvironmentValidationProblem {
    Empty,
    TooLong { max_chars: usize },
    TooManyArguments { max: usize },
    Zero,
    ControlCharacter,
    InvalidIdentifier,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvironmentValidationError {
    field: &'static str,
    problem: EnvironmentValidationProblem,
}

impl EnvironmentValidationError {
    const fn new(field: &'static str, problem: EnvironmentValidationProblem) -> Self {
        Self { field, problem }
    }

    pub const fn field(&self) -> &'static str {
        self.field
    }

    pub const fn problem(&self) -> EnvironmentValidationProblem {
        self.problem
    }
}

impl Display for EnvironmentValidationError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self.problem {
            EnvironmentValidationProblem::Empty => {
                write!(formatter, "{} cannot be empty", self.field)
            }
            EnvironmentValidationProblem::TooLong { max_chars } => {
                write!(
                    formatter,
                    "{} cannot exceed {max_chars} characters",
                    self.field
                )
            }
            EnvironmentValidationProblem::TooManyArguments { max } => {
                write!(
                    formatter,
                    "{} cannot contain more than {max} values",
                    self.field
                )
            }
            EnvironmentValidationProblem::Zero => {
                write!(formatter, "{} must be greater than zero", self.field)
            }
            EnvironmentValidationProblem::ControlCharacter => {
                write!(
                    formatter,
                    "{} contains an unsupported control character",
                    self.field
                )
            }
            EnvironmentValidationProblem::InvalidIdentifier => {
                write!(
                    formatter,
                    "{} is not a valid command-line identifier",
                    self.field
                )
            }
        }
    }
}

impl Error for EnvironmentValidationError {}
