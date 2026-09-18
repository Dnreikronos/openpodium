use std::error::Error;
use std::fmt::{self, Display, Formatter};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Name(String);

impl Name {
    pub const MAX_CHARS: usize = 100;

    pub fn new(value: impl Into<String>) -> Result<Self, ValidationError> {
        let value = value.into();
        let value = value.trim();

        if value.is_empty() {
            return Err(ValidationError::new("name", ValidationProblem::Empty));
        }
        if value.chars().count() > Self::MAX_CHARS {
            return Err(ValidationError::new(
                "name",
                ValidationProblem::TooLong {
                    max_chars: Self::MAX_CHARS,
                },
            ));
        }

        Ok(Self(value.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Display for Name {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceIcon(String);

impl WorkspaceIcon {
    pub const MAX_CHARS: usize = 32;

    pub fn new(value: impl Into<String>) -> Result<Self, ValidationError> {
        let value = value.into();
        let value = value.trim();

        if value.is_empty() {
            return Err(ValidationError::new(
                "workspace icon",
                ValidationProblem::Empty,
            ));
        }
        if value.chars().count() > Self::MAX_CHARS {
            return Err(ValidationError::new(
                "workspace icon",
                ValidationProblem::TooLong {
                    max_chars: Self::MAX_CHARS,
                },
            ));
        }

        Ok(Self(value.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceDirectory(String);

impl WorkspaceDirectory {
    pub const MAX_CHARS: usize = 32_768;

    pub fn new(value: impl Into<String>) -> Result<Self, ValidationError> {
        let value = value.into();

        if value.is_empty() {
            return Err(ValidationError::new(
                "working directory",
                ValidationProblem::Empty,
            ));
        }
        if value.chars().count() > Self::MAX_CHARS {
            return Err(ValidationError::new(
                "working directory",
                ValidationProblem::TooLong {
                    max_chars: Self::MAX_CHARS,
                },
            ));
        }

        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Content(String);

impl Content {
    pub const MAX_CHARS: usize = 65_536;

    pub fn new(value: impl Into<String>) -> Result<Self, ValidationError> {
        let value = value.into();

        if value.trim().is_empty() {
            return Err(ValidationError::new("content", ValidationProblem::Empty));
        }
        if value.chars().count() > Self::MAX_CHARS {
            return Err(ValidationError::new(
                "content",
                ValidationProblem::TooLong {
                    max_chars: Self::MAX_CHARS,
                },
            ));
        }

        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CanvasPoint {
    x: f32,
    y: f32,
}

impl CanvasPoint {
    pub fn new(x: f32, y: f32) -> Result<Self, ValidationError> {
        if !x.is_finite() || !y.is_finite() {
            return Err(ValidationError::new(
                "canvas position",
                ValidationProblem::NotFinite,
            ));
        }

        Ok(Self { x, y })
    }

    pub const fn x(self) -> f32 {
        self.x
    }

    pub const fn y(self) -> f32 {
        self.y
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CanvasSize {
    width: f32,
    height: f32,
}

impl CanvasSize {
    pub fn new(width: f32, height: f32) -> Result<Self, ValidationError> {
        if !width.is_finite() || !height.is_finite() {
            return Err(ValidationError::new(
                "canvas size",
                ValidationProblem::NotFinite,
            ));
        }
        if width <= 0.0 || height <= 0.0 {
            return Err(ValidationError::new(
                "canvas size",
                ValidationProblem::NotPositive,
            ));
        }

        Ok(Self { width, height })
    }

    pub const fn width(self) -> f32 {
        self.width
    }

    pub const fn height(self) -> f32 {
        self.height
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Timestamp(u64);

impl Timestamp {
    pub const fn from_unix_millis(value: u64) -> Self {
        Self(value)
    }

    pub const fn as_unix_millis(self) -> u64 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationProblem {
    Empty,
    TooLong { max_chars: usize },
    NotFinite,
    NotPositive,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationError {
    field: &'static str,
    problem: ValidationProblem,
}

impl ValidationError {
    const fn new(field: &'static str, problem: ValidationProblem) -> Self {
        Self { field, problem }
    }

    pub const fn field(&self) -> &'static str {
        self.field
    }

    pub const fn problem(&self) -> ValidationProblem {
        self.problem
    }
}

impl Display for ValidationError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self.problem {
            ValidationProblem::Empty => write!(formatter, "{} cannot be empty", self.field),
            ValidationProblem::TooLong { max_chars } => write!(
                formatter,
                "{} cannot exceed {max_chars} characters",
                self.field
            ),
            ValidationProblem::NotFinite => {
                write!(formatter, "{} must contain finite values", self.field)
            }
            ValidationProblem::NotPositive => {
                write!(formatter, "{} values must be greater than zero", self.field)
            }
        }
    }
}

impl Error for ValidationError {}
