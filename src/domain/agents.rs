use std::error::Error;
use std::fmt::{self, Display, Formatter};

use super::{CommandPresetId, Name};

const MAX_ARGUMENTS: usize = 256;
const MAX_VALUE_CHARS: usize = 32_768;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandPreset {
    id: CommandPresetId,
    name: Name,
    executable: String,
    arguments: Vec<String>,
}

impl CommandPreset {
    pub fn new(
        id: CommandPresetId,
        name: Name,
        executable: impl Into<String>,
        arguments: Vec<String>,
    ) -> Result<Self, AgentConfigurationError> {
        if arguments.len() > MAX_ARGUMENTS {
            return Err(AgentConfigurationError::new(
                "preset arguments",
                AgentConfigurationProblem::TooManyArguments { max: MAX_ARGUMENTS },
            ));
        }
        let arguments = arguments
            .into_iter()
            .map(validate_argument)
            .collect::<Result<_, _>>()?;
        Ok(Self {
            id,
            name,
            executable: validate_text("preset executable", executable.into())?,
            arguments,
        })
    }

    pub const fn id(&self) -> CommandPresetId {
        self.id
    }

    pub const fn name(&self) -> &Name {
        &self.name
    }

    pub fn executable(&self) -> &str {
        &self.executable
    }

    pub fn arguments(&self) -> &[String] {
        &self.arguments
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoleColor(String);

impl RoleColor {
    pub const DEFAULT: &'static str = "#6D7CFF";

    pub fn new(value: impl Into<String>) -> Result<Self, AgentConfigurationError> {
        let value = value.into();
        if value.len() != 7
            || !value.starts_with('#')
            || !value.as_bytes()[1..].iter().all(u8::is_ascii_hexdigit)
        {
            return Err(AgentConfigurationError::new(
                "role color",
                AgentConfigurationProblem::InvalidColor,
            ));
        }
        Ok(Self(value.to_ascii_uppercase()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for RoleColor {
    fn default() -> Self {
        Self(Self::DEFAULT.to_owned())
    }
}

impl Display for RoleColor {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoleIcon(String);

impl RoleIcon {
    pub const DEFAULT: &'static str = "agent";
    pub const MAX_CHARS: usize = 32;

    pub fn new(value: impl Into<String>) -> Result<Self, AgentConfigurationError> {
        let value = value.into();
        let value = value.trim();
        if value.is_empty() {
            return Err(AgentConfigurationError::new(
                "role icon",
                AgentConfigurationProblem::Empty,
            ));
        }
        if value.chars().count() > Self::MAX_CHARS {
            return Err(AgentConfigurationError::new(
                "role icon",
                AgentConfigurationProblem::TooLong {
                    max_chars: Self::MAX_CHARS,
                },
            ));
        }
        if value.contains(['\0', '\n', '\r']) {
            return Err(AgentConfigurationError::new(
                "role icon",
                AgentConfigurationProblem::ControlCharacter,
            ));
        }
        Ok(Self(value.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for RoleIcon {
    fn default() -> Self {
        Self(Self::DEFAULT.to_owned())
    }
}

impl Display for RoleIcon {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

fn validate_text(field: &'static str, value: String) -> Result<String, AgentConfigurationError> {
    if value.is_empty() {
        return Err(AgentConfigurationError::new(
            field,
            AgentConfigurationProblem::Empty,
        ));
    }
    if value.chars().count() > MAX_VALUE_CHARS {
        return Err(AgentConfigurationError::new(
            field,
            AgentConfigurationProblem::TooLong {
                max_chars: MAX_VALUE_CHARS,
            },
        ));
    }
    if value.contains(['\0', '\n', '\r']) {
        return Err(AgentConfigurationError::new(
            field,
            AgentConfigurationProblem::ControlCharacter,
        ));
    }
    Ok(value)
}

fn validate_argument(value: String) -> Result<String, AgentConfigurationError> {
    if value.chars().count() > MAX_VALUE_CHARS {
        return Err(AgentConfigurationError::new(
            "preset argument",
            AgentConfigurationProblem::TooLong {
                max_chars: MAX_VALUE_CHARS,
            },
        ));
    }
    if value.contains('\0') {
        return Err(AgentConfigurationError::new(
            "preset argument",
            AgentConfigurationProblem::ControlCharacter,
        ));
    }
    Ok(value)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentConfigurationProblem {
    Empty,
    TooLong { max_chars: usize },
    TooManyArguments { max: usize },
    ControlCharacter,
    InvalidColor,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentConfigurationError {
    field: &'static str,
    problem: AgentConfigurationProblem,
}

impl AgentConfigurationError {
    const fn new(field: &'static str, problem: AgentConfigurationProblem) -> Self {
        Self { field, problem }
    }

    pub const fn field(&self) -> &'static str {
        self.field
    }

    pub const fn problem(&self) -> AgentConfigurationProblem {
        self.problem
    }
}

impl Display for AgentConfigurationError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self.problem {
            AgentConfigurationProblem::Empty => write!(formatter, "{} cannot be empty", self.field),
            AgentConfigurationProblem::TooLong { max_chars } => write!(
                formatter,
                "{} cannot exceed {max_chars} characters",
                self.field
            ),
            AgentConfigurationProblem::TooManyArguments { max } => write!(
                formatter,
                "{} cannot contain more than {max} values",
                self.field
            ),
            AgentConfigurationProblem::ControlCharacter => write!(
                formatter,
                "{} contains an unsupported control character",
                self.field
            ),
            AgentConfigurationProblem::InvalidColor => {
                write!(formatter, "{} must use #RRGGBB format", self.field)
            }
        }
    }
}

impl Error for AgentConfigurationError {}
