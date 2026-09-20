use std::error::Error;
use std::fmt::{self, Display, Formatter};

use super::{Name, NodeTarget, PortalConfig};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProjectPath(String);

impl ProjectPath {
    pub const MAX_CHARS: usize = 32_768;

    pub fn new(value: impl Into<String>) -> Result<Self, ProjectPathError> {
        let value = value.into();
        if value.is_empty() {
            return Err(ProjectPathError::Empty);
        }
        if value.chars().count() > Self::MAX_CHARS {
            return Err(ProjectPathError::TooLong);
        }
        if value == "." {
            return Ok(Self(value));
        }
        if value.starts_with('/') || value.starts_with('\\') || value.contains('\\') {
            return Err(ProjectPathError::NotNormalized);
        }
        if value.as_bytes().contains(&0) {
            return Err(ProjectPathError::InvalidCharacter);
        }

        let mut components = value.split('/');
        let Some(first) = components.next() else {
            return Err(ProjectPathError::Empty);
        };
        if is_invalid_component(first) || looks_like_windows_drive(first) {
            return Err(ProjectPathError::NotNormalized);
        }
        if components.any(is_invalid_component) {
            return Err(ProjectPathError::NotNormalized);
        }

        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Display for ProjectPath {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

fn is_invalid_component(component: &str) -> bool {
    component.is_empty() || matches!(component, "." | "..")
}

fn looks_like_windows_drive(component: &str) -> bool {
    component.as_bytes().get(1) == Some(&b':')
        && component
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphabetic)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectPathError {
    Empty,
    TooLong,
    NotNormalized,
    InvalidCharacter,
}

impl Display for ProjectPathError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Empty => "project path cannot be empty",
            Self::TooLong => "project path is too long",
            Self::NotNormalized => {
                "project path must be relative and use normalized forward-slash components"
            }
            Self::InvalidCharacter => "project path contains an invalid character",
        })
    }
}

impl Error for ProjectPathError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CanvasNodeContent {
    Reference(NodeTarget),
    Note {
        path: ProjectPath,
        title: Name,
    },
    FileTree {
        root: ProjectPath,
    },
    Artifact {
        path: ProjectPath,
    },
    Diff {
        path: ProjectPath,
        comparison: DiffComparison,
    },
    Text {
        markdown: CanvasText,
    },
    Portal(PortalConfig),
    Shape(Shape),
    Arrow(Arrow),
    Freehand(Freehand),
}

impl From<NodeTarget> for CanvasNodeContent {
    fn from(target: NodeTarget) -> Self {
        Self::Reference(target)
    }
}

impl CanvasNodeContent {
    pub const fn reference(&self) -> Option<NodeTarget> {
        match self {
            Self::Reference(target) => Some(*target),
            Self::Note { .. }
            | Self::FileTree { .. }
            | Self::Artifact { .. }
            | Self::Diff { .. }
            | Self::Text { .. }
            | Self::Portal(_)
            | Self::Shape(_)
            | Self::Arrow(_)
            | Self::Freehand(_) => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffComparison {
    WorkingTreeAgainstHead,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanvasText(String);

impl CanvasText {
    pub const MAX_CHARS: usize = 65_536;

    pub fn new(value: impl Into<String>) -> Result<Self, CanvasTextError> {
        let value = value.into();
        if value.chars().count() > Self::MAX_CHARS {
            return Err(CanvasTextError);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CanvasTextError;

impl Display for CanvasTextError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str("canvas text is too long")
    }
}

impl Error for CanvasTextError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShapeKind {
    Rectangle,
    Ellipse,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shape {
    kind: ShapeKind,
    fill: CanvasColor,
    stroke: CanvasColor,
    stroke_width: StrokeWidth,
}

impl Shape {
    pub const fn new(
        kind: ShapeKind,
        fill: CanvasColor,
        stroke: CanvasColor,
        stroke_width: StrokeWidth,
    ) -> Self {
        Self {
            kind,
            fill,
            stroke,
            stroke_width,
        }
    }

    pub const fn kind(&self) -> ShapeKind {
        self.kind
    }

    pub const fn fill(&self) -> CanvasColor {
        self.fill
    }

    pub const fn stroke(&self) -> CanvasColor {
        self.stroke
    }

    pub const fn stroke_width(&self) -> StrokeWidth {
        self.stroke_width
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Arrow {
    start: NormalizedPoint,
    end: NormalizedPoint,
    stroke: CanvasColor,
    stroke_width: StrokeWidth,
    label: Option<Name>,
}

impl Arrow {
    pub const fn new(
        start: NormalizedPoint,
        end: NormalizedPoint,
        stroke: CanvasColor,
        stroke_width: StrokeWidth,
        label: Option<Name>,
    ) -> Self {
        Self {
            start,
            end,
            stroke,
            stroke_width,
            label,
        }
    }

    pub const fn start(&self) -> NormalizedPoint {
        self.start
    }

    pub const fn end(&self) -> NormalizedPoint {
        self.end
    }

    pub const fn stroke(&self) -> CanvasColor {
        self.stroke
    }

    pub const fn stroke_width(&self) -> StrokeWidth {
        self.stroke_width
    }

    pub const fn label(&self) -> Option<&Name> {
        self.label.as_ref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Freehand {
    points: Vec<NormalizedPoint>,
    stroke: CanvasColor,
    stroke_width: StrokeWidth,
}

impl Freehand {
    pub const MAX_POINTS: usize = 65_536;

    pub fn new(
        points: Vec<NormalizedPoint>,
        stroke: CanvasColor,
        stroke_width: StrokeWidth,
    ) -> Result<Self, DrawingValidationError> {
        if points.len() < 2 {
            return Err(DrawingValidationError::TooFewPoints);
        }
        if points.len() > Self::MAX_POINTS {
            return Err(DrawingValidationError::TooManyPoints);
        }
        Ok(Self {
            points,
            stroke,
            stroke_width,
        })
    }

    pub fn points(&self) -> &[NormalizedPoint] {
        &self.points
    }

    pub const fn stroke(&self) -> CanvasColor {
        self.stroke
    }

    pub const fn stroke_width(&self) -> StrokeWidth {
        self.stroke_width
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NormalizedPoint {
    x: u16,
    y: u16,
}

impl NormalizedPoint {
    pub const MAX: u16 = 10_000;

    pub fn new(x: f32, y: f32) -> Result<Self, DrawingValidationError> {
        if !x.is_finite() || !y.is_finite() {
            return Err(DrawingValidationError::NotFinite);
        }
        if !(0.0..=1.0).contains(&x) || !(0.0..=1.0).contains(&y) {
            return Err(DrawingValidationError::OutsideNode);
        }
        Ok(Self {
            x: (x * f32::from(Self::MAX)).round() as u16,
            y: (y * f32::from(Self::MAX)).round() as u16,
        })
    }

    pub fn x(self) -> f32 {
        f32::from(self.x) / f32::from(Self::MAX)
    }

    pub fn y(self) -> f32 {
        f32::from(self.y) / f32::from(Self::MAX)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CanvasColor([u8; 4]);

impl CanvasColor {
    pub const fn rgba(red: u8, green: u8, blue: u8, alpha: u8) -> Self {
        Self([red, green, blue, alpha])
    }

    pub const fn channels(self) -> [u8; 4] {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StrokeWidth(u16);

impl StrokeWidth {
    pub const MAX_TENTHS: u16 = 640;

    pub fn new(value: f32) -> Result<Self, DrawingValidationError> {
        if !value.is_finite() {
            return Err(DrawingValidationError::NotFinite);
        }
        if value <= 0.0 || value > f32::from(Self::MAX_TENTHS) / 10.0 {
            return Err(DrawingValidationError::InvalidStrokeWidth);
        }
        Ok(Self((value * 10.0).round() as u16))
    }

    pub fn get(self) -> f32 {
        f32::from(self.0) / 10.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrawingValidationError {
    NotFinite,
    OutsideNode,
    InvalidStrokeWidth,
    TooFewPoints,
    TooManyPoints,
}

impl Display for DrawingValidationError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NotFinite => "drawing coordinates must be finite",
            Self::OutsideNode => "drawing coordinates must be between zero and one",
            Self::InvalidStrokeWidth => "stroke width must be greater than zero and at most 64",
            Self::TooFewPoints => "freehand drawings require at least two points",
            Self::TooManyPoints => "freehand drawing has too many points",
        })
    }
}

impl Error for DrawingValidationError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_paths_are_portable_and_normalized() {
        assert_eq!(
            ProjectPath::new(".openpodium/notes/7.md").unwrap().as_str(),
            ".openpodium/notes/7.md"
        );
        assert_eq!(ProjectPath::new(".").unwrap().as_str(), ".");
        for invalid in [
            "",
            "/tmp/note.md",
            "C:/note.md",
            "notes//note.md",
            "notes/./note.md",
            "notes/../note.md",
            "notes\\note.md",
        ] {
            assert!(ProjectPath::new(invalid).is_err(), "accepted {invalid:?}");
        }
    }

    #[test]
    fn drawing_values_reject_invalid_geometry() {
        assert_eq!(
            NormalizedPoint::new(-0.1, 0.5),
            Err(DrawingValidationError::OutsideNode)
        );
        assert_eq!(
            StrokeWidth::new(0.0),
            Err(DrawingValidationError::InvalidStrokeWidth)
        );
        assert_eq!(
            Freehand::new(
                vec![NormalizedPoint::new(0.0, 0.0).unwrap()],
                CanvasColor::rgba(0, 0, 0, 255),
                StrokeWidth::new(1.0).unwrap(),
            ),
            Err(DrawingValidationError::TooFewPoints)
        );
    }
}
