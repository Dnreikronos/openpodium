//! Dependency-free portal state and browser-frame geometry.
//!
//! Live adapters own external processes and protocol handles elsewhere. This
//! module only defines the values that cross that boundary and the invariants
//! needed by the canvas and IPC layers.

use std::error::Error;
use std::fmt::{self, Display, Formatter};

mod browser;
mod device;
mod policy;
pub use browser::{BrowserBackend, BrowserError};
pub use device::{
    DeviceAvailability, DeviceDiscovery, DeviceDiscoveryReport, DeviceKind, DiscoveredDevice,
    ToolProbe,
};
pub use policy::{
    DEFAULT_GRANT_LIFETIME_MS, PendingApproval, PolicyDecision, PolicyRequest, PolicyRule,
    PortalGrant, PortalPolicy, PortalPolicyError,
};

const MAX_SELECTOR_CHARS: usize = 2_048;
const MAX_ELEMENT_ID_CHARS: usize = 256;
const MAX_FRAME_BYTES: usize = 8 * 1024 * 1024;
const MAX_ACCESSIBILITY_CHARS: usize = 2 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PortalTargetKind {
    Browser,
    Android,
    Ios,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortalTarget {
    kind: PortalTargetKind,
    selector: String,
}

impl PortalTarget {
    pub fn new(
        kind: PortalTargetKind,
        selector: impl Into<String>,
    ) -> Result<Self, PortalValidationError> {
        let selector = selector.into();
        validate_text(&selector, MAX_SELECTOR_CHARS, "portal target selector")?;
        Ok(Self { kind, selector })
    }

    pub fn browser(url: impl Into<String>) -> Result<Self, PortalValidationError> {
        Self::new(PortalTargetKind::Browser, url)
    }

    pub const fn kind(&self) -> PortalTargetKind {
        self.kind
    }

    pub fn selector(&self) -> &str {
        &self.selector
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PortalPresentation {
    preserve_aspect_ratio: bool,
    frame_rate_limit: u16,
}

impl Default for PortalPresentation {
    fn default() -> Self {
        Self {
            preserve_aspect_ratio: true,
            frame_rate_limit: 30,
        }
    }
}

impl PortalPresentation {
    pub const fn new(preserve_aspect_ratio: bool, frame_rate_limit: u16) -> Option<Self> {
        if frame_rate_limit == 0 {
            return None;
        }
        Some(Self {
            preserve_aspect_ratio,
            frame_rate_limit,
        })
    }

    pub const fn preserve_aspect_ratio(self) -> bool {
        self.preserve_aspect_ratio
    }

    pub const fn frame_rate_limit(self) -> u16 {
        self.frame_rate_limit
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortalConfig {
    target: PortalTarget,
    presentation: PortalPresentation,
}

impl PortalConfig {
    pub fn new(target: PortalTarget, presentation: PortalPresentation) -> Self {
        Self {
            target,
            presentation,
        }
    }

    pub fn browser(url: impl Into<String>) -> Result<Self, PortalValidationError> {
        Ok(Self::new(
            PortalTarget::browser(url)?,
            PortalPresentation::default(),
        ))
    }

    pub const fn target(&self) -> &PortalTarget {
        &self.target
    }

    pub const fn presentation(&self) -> PortalPresentation {
        self.presentation
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PortalOperation {
    Observe,
    Screenshot,
    Navigate,
    Input,
    CoordinateFallback,
    Upload,
    Download,
    Clipboard,
    SensitivePermission,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CapabilityStatus {
    Supported,
    Unavailable { reason: String },
    PermissionRequired { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortalCapabilities {
    observe: CapabilityStatus,
    screenshot: CapabilityStatus,
    navigate: CapabilityStatus,
    input: CapabilityStatus,
    coordinate_fallback: CapabilityStatus,
    upload: CapabilityStatus,
    download: CapabilityStatus,
    clipboard: CapabilityStatus,
    sensitive_permission: CapabilityStatus,
}

impl PortalCapabilities {
    pub fn browser_defaults() -> Self {
        Self {
            observe: CapabilityStatus::Supported,
            screenshot: CapabilityStatus::Supported,
            navigate: CapabilityStatus::PermissionRequired {
                reason: "navigation can change the target or trigger a side effect".to_owned(),
            },
            input: CapabilityStatus::PermissionRequired {
                reason: "input can submit or otherwise cause a consequential action".to_owned(),
            },
            coordinate_fallback: CapabilityStatus::PermissionRequired {
                reason: "coordinates are less stable than semantic element references".to_owned(),
            },
            upload: CapabilityStatus::PermissionRequired {
                reason: "upload transfers a local file to the target".to_owned(),
            },
            download: CapabilityStatus::PermissionRequired {
                reason: "download writes target data to the local machine".to_owned(),
            },
            clipboard: CapabilityStatus::PermissionRequired {
                reason: "clipboard access crosses the portal boundary".to_owned(),
            },
            sensitive_permission: CapabilityStatus::PermissionRequired {
                reason: "the target may request a sensitive browser permission".to_owned(),
            },
        }
    }

    pub fn status(&self, operation: PortalOperation) -> &CapabilityStatus {
        match operation {
            PortalOperation::Observe => &self.observe,
            PortalOperation::Screenshot => &self.screenshot,
            PortalOperation::Navigate => &self.navigate,
            PortalOperation::Input => &self.input,
            PortalOperation::CoordinateFallback => &self.coordinate_fallback,
            PortalOperation::Upload => &self.upload,
            PortalOperation::Download => &self.download,
            PortalOperation::Clipboard => &self.clipboard,
            PortalOperation::SensitivePermission => &self.sensitive_permission,
        }
    }

    pub fn set_status(&mut self, operation: PortalOperation, status: CapabilityStatus) {
        match operation {
            PortalOperation::Observe => self.observe = status,
            PortalOperation::Screenshot => self.screenshot = status,
            PortalOperation::Navigate => self.navigate = status,
            PortalOperation::Input => self.input = status,
            PortalOperation::CoordinateFallback => self.coordinate_fallback = status,
            PortalOperation::Upload => self.upload = status,
            PortalOperation::Download => self.download = status,
            PortalOperation::Clipboard => self.clipboard = status,
            PortalOperation::SensitivePermission => self.sensitive_permission = status,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortalSessionState {
    Disconnected,
    Connecting,
    Connected,
    Closing,
    Closed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortalElementRef {
    observation_revision: u64,
    backend_id: String,
}

impl PortalElementRef {
    pub fn new(
        observation_revision: u64,
        backend_id: impl Into<String>,
    ) -> Result<Self, PortalValidationError> {
        let backend_id = backend_id.into();
        validate_text(
            &backend_id,
            MAX_ELEMENT_ID_CHARS,
            "portal element reference",
        )?;
        Ok(Self {
            observation_revision,
            backend_id,
        })
    }

    pub const fn observation_revision(&self) -> u64 {
        self.observation_revision
    }

    pub fn backend_id(&self) -> &str {
        &self.backend_id
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PortalAction {
    Click(PortalElementRef),
    ClickCoordinate {
        observation_revision: u64,
        x: u32,
        y: u32,
    },
    TypeText {
        element: PortalElementRef,
        text: String,
    },
    TypeFocused {
        observation_revision: u64,
        text: String,
    },
    Key {
        observation_revision: u64,
        key: PortalKeyInput,
        shift: bool,
    },
    Scroll {
        element: Option<PortalElementRef>,
        delta_x: i32,
        delta_y: i32,
    },
    ScrollCoordinate {
        observation_revision: u64,
        x: u32,
        y: u32,
        delta_x: i32,
        delta_y: i32,
    },
    Navigate(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortalKeyInput {
    Enter,
    Tab,
    Backspace,
    Delete,
    Escape,
    ArrowUp,
    ArrowDown,
    ArrowLeft,
    ArrowRight,
    Home,
    End,
    PageUp,
    PageDown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortalFrameEncoding {
    Png,
    Jpeg,
    Webp,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortalFrame {
    revision: u64,
    viewport: PortalViewport,
    encoding: PortalFrameEncoding,
    bytes: Vec<u8>,
}

impl PortalFrame {
    pub fn new(
        revision: u64,
        viewport: PortalViewport,
        encoding: PortalFrameEncoding,
        bytes: Vec<u8>,
    ) -> Result<Self, PortalValidationError> {
        if bytes.is_empty() {
            return Err(PortalValidationError::EmptyFrame);
        }
        if bytes.len() > MAX_FRAME_BYTES {
            return Err(PortalValidationError::FrameTooLarge {
                max_bytes: MAX_FRAME_BYTES,
            });
        }
        Ok(Self {
            revision,
            viewport,
            encoding,
            bytes,
        })
    }

    pub const fn revision(&self) -> u64 {
        self.revision
    }

    pub const fn viewport(&self) -> PortalViewport {
        self.viewport
    }

    pub const fn encoding(&self) -> PortalFrameEncoding {
        self.encoding
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortalAccessibilitySnapshot {
    revision: u64,
    json: String,
}

impl PortalAccessibilitySnapshot {
    pub fn new(revision: u64, json: impl Into<String>) -> Result<Self, PortalValidationError> {
        let json = json.into();
        if json.chars().count() > MAX_ACCESSIBILITY_CHARS {
            return Err(PortalValidationError::AccessibilityTooLarge {
                max_chars: MAX_ACCESSIBILITY_CHARS,
            });
        }
        if json.contains('\0') {
            return Err(PortalValidationError::ControlCharacter {
                field: "portal accessibility snapshot",
            });
        }
        Ok(Self { revision, json })
    }

    pub const fn revision(&self) -> u64 {
        self.revision
    }

    pub fn json(&self) -> &str {
        &self.json
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PortalViewport {
    width: u32,
    height: u32,
}

impl PortalViewport {
    pub const fn new(width: u32, height: u32) -> Option<Self> {
        if width == 0 || height == 0 {
            return None;
        }
        Some(Self { width, height })
    }

    pub const fn width(self) -> u32 {
        self.width
    }

    pub const fn height(self) -> u32 {
        self.height
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PortalRect {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

impl PortalRect {
    pub fn new(x: f64, y: f64, width: f64, height: f64) -> Option<Self> {
        if [x, y, width, height].iter().all(|value| value.is_finite())
            && width > 0.0
            && height > 0.0
        {
            Some(Self {
                x,
                y,
                width,
                height,
            })
        } else {
            None
        }
    }

    pub const fn x(self) -> f64 {
        self.x
    }

    pub const fn y(self) -> f64 {
        self.y
    }

    pub const fn width(self) -> f64 {
        self.width
    }

    pub const fn height(self) -> f64 {
        self.height
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PortalPoint {
    x: f64,
    y: f64,
}

impl PortalPoint {
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }

    pub const fn x(self) -> f64 {
        self.x
    }

    pub const fn y(self) -> f64 {
        self.y
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PortalFrameTransform {
    source: PortalRect,
    destination: PortalRect,
}

impl PortalFrameTransform {
    pub fn new(
        node_bounds: PortalRect,
        viewport: PortalViewport,
        preserve_aspect_ratio: bool,
    ) -> Self {
        let viewport_width = f64::from(viewport.width);
        let viewport_height = f64::from(viewport.height);
        let scale = if preserve_aspect_ratio {
            (node_bounds.width / viewport_width).min(node_bounds.height / viewport_height)
        } else {
            1.0
        };
        let destination_width = if preserve_aspect_ratio {
            viewport_width * scale
        } else {
            node_bounds.width
        };
        let destination_height = if preserve_aspect_ratio {
            viewport_height * scale
        } else {
            node_bounds.height
        };
        let destination = PortalRect::new(
            node_bounds.x + (node_bounds.width - destination_width) / 2.0,
            node_bounds.y + (node_bounds.height - destination_height) / 2.0,
            destination_width,
            destination_height,
        )
        .expect("portal transform destination is finite and positive");
        Self {
            source: PortalRect::new(0.0, 0.0, viewport_width, viewport_height)
                .expect("portal viewport is non-zero"),
            destination,
        }
    }

    pub const fn destination(self) -> PortalRect {
        self.destination
    }

    pub fn canvas_to_viewport(self, point: PortalPoint) -> Option<PortalPoint> {
        if point.x < self.destination.x
            || point.y < self.destination.y
            || point.x > self.destination.x + self.destination.width
            || point.y > self.destination.y + self.destination.height
        {
            return None;
        }
        Some(PortalPoint::new(
            (point.x - self.destination.x) / self.destination.width * self.source.width,
            (point.y - self.destination.y) / self.destination.height * self.source.height,
        ))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortalObservation {
    revision: u64,
    capabilities: PortalCapabilities,
    frame: Option<PortalFrame>,
    accessibility: Option<PortalAccessibilitySnapshot>,
}

impl PortalObservation {
    pub fn new(revision: u64, capabilities: PortalCapabilities) -> Self {
        Self {
            revision,
            capabilities,
            frame: None,
            accessibility: None,
        }
    }

    pub fn with_frame(mut self, frame: PortalFrame) -> Self {
        self.frame = Some(frame);
        self
    }

    pub fn with_accessibility(mut self, accessibility: PortalAccessibilitySnapshot) -> Self {
        self.accessibility = Some(accessibility);
        self
    }

    pub const fn revision(&self) -> u64 {
        self.revision
    }

    pub const fn capabilities(&self) -> &PortalCapabilities {
        &self.capabilities
    }

    pub const fn frame(&self) -> Option<&PortalFrame> {
        self.frame.as_ref()
    }

    pub const fn accessibility(&self) -> Option<&PortalAccessibilitySnapshot> {
        self.accessibility.as_ref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortalSession {
    generation: u64,
    state: PortalSessionState,
    observation_revision: u64,
    latest_observation: Option<PortalObservation>,
}

impl PortalSession {
    pub const fn new(generation: u64) -> Self {
        Self {
            generation,
            state: PortalSessionState::Disconnected,
            observation_revision: 0,
            latest_observation: None,
        }
    }

    pub const fn generation(&self) -> u64 {
        self.generation
    }

    pub const fn state(&self) -> PortalSessionState {
        self.state
    }

    pub const fn observation_revision(&self) -> u64 {
        self.observation_revision
    }

    pub const fn latest_observation(&self) -> Option<&PortalObservation> {
        self.latest_observation.as_ref()
    }

    pub fn begin_connect(&mut self) -> Result<(), PortalSessionError> {
        if !matches!(self.state, PortalSessionState::Disconnected) {
            return Err(PortalSessionError::InvalidTransition {
                state: self.state,
                action: "connect",
            });
        }
        self.state = PortalSessionState::Connecting;
        Ok(())
    }

    pub fn connected(&mut self) -> Result<(), PortalSessionError> {
        if !matches!(self.state, PortalSessionState::Connecting) {
            return Err(PortalSessionError::InvalidTransition {
                state: self.state,
                action: "finish connecting",
            });
        }
        self.state = PortalSessionState::Connected;
        Ok(())
    }

    pub fn observe(&mut self) -> Result<u64, PortalSessionError> {
        if !matches!(self.state, PortalSessionState::Connected) {
            return Err(PortalSessionError::Unavailable(self.state));
        }
        self.observation_revision = self.observation_revision.wrapping_add(1);
        self.latest_observation = None;
        Ok(self.observation_revision)
    }

    pub fn record_observation(
        &mut self,
        observation: PortalObservation,
    ) -> Result<(), PortalSessionError> {
        if !matches!(self.state, PortalSessionState::Connected) {
            return Err(PortalSessionError::Unavailable(self.state));
        }
        if observation.revision != self.observation_revision {
            return Err(PortalSessionError::StaleObservation {
                expected: self.observation_revision,
                found: observation.revision,
            });
        }
        self.latest_observation = Some(observation);
        Ok(())
    }

    pub fn disconnect(&mut self) {
        self.state = PortalSessionState::Disconnected;
        self.latest_observation = None;
    }

    pub fn accepts(&self, element: &PortalElementRef) -> Result<(), PortalSessionError> {
        self.accepts_revision(element.observation_revision)
    }

    pub fn accepts_revision(&self, observation_revision: u64) -> Result<(), PortalSessionError> {
        if !matches!(self.state, PortalSessionState::Connected) {
            return Err(PortalSessionError::Unavailable(self.state));
        }
        if observation_revision != self.observation_revision {
            return Err(PortalSessionError::StaleObservation {
                expected: self.observation_revision,
                found: observation_revision,
            });
        }
        Ok(())
    }

    pub fn begin_close(&mut self) -> Result<(), PortalSessionError> {
        if matches!(
            self.state,
            PortalSessionState::Closed | PortalSessionState::Disconnected
        ) {
            return Err(PortalSessionError::InvalidTransition {
                state: self.state,
                action: "close",
            });
        }
        self.state = PortalSessionState::Closing;
        Ok(())
    }

    pub fn closed(&mut self) {
        self.state = PortalSessionState::Closed;
    }
}

pub trait PortalBackend {
    type Error: Error + Send + Sync + 'static;

    fn connect(
        &mut self,
        config: &PortalConfig,
        session: &mut PortalSession,
    ) -> Result<(), Self::Error>;
    fn observe(&mut self, session: &mut PortalSession) -> Result<PortalObservation, Self::Error>;
    fn execute(
        &mut self,
        session: &mut PortalSession,
        action: &PortalAction,
    ) -> Result<(), Self::Error>;
    fn close(&mut self, session: &mut PortalSession) -> Result<(), Self::Error>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PortalSessionError {
    Unavailable(PortalSessionState),
    InvalidTransition {
        state: PortalSessionState,
        action: &'static str,
    },
    StaleObservation {
        expected: u64,
        found: u64,
    },
}

impl Display for PortalSessionError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable(state) => write!(formatter, "portal session is {state:?}"),
            Self::InvalidTransition { state, action } => {
                write!(
                    formatter,
                    "cannot {action} while portal session is {state:?}"
                )
            }
            Self::StaleObservation { expected, found } => write!(
                formatter,
                "portal element reference is stale: expected observation {expected}, found {found}"
            ),
        }
    }
}

impl Error for PortalSessionError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PortalValidationError {
    EmptyText {
        field: &'static str,
    },
    TextTooLong {
        field: &'static str,
        max_chars: usize,
    },
    ControlCharacter {
        field: &'static str,
    },
    EmptyFrame,
    FrameTooLarge {
        max_bytes: usize,
    },
    AccessibilityTooLarge {
        max_chars: usize,
    },
}

impl Display for PortalValidationError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyText { field } => write!(formatter, "{field} cannot be empty"),
            Self::TextTooLong { field, max_chars } => {
                write!(formatter, "{field} cannot exceed {max_chars} characters")
            }
            Self::ControlCharacter { field } => {
                write!(
                    formatter,
                    "{field} contains an unsupported control character"
                )
            }
            Self::EmptyFrame => formatter.write_str("portal frame cannot be empty"),
            Self::FrameTooLarge { max_bytes } => {
                write!(formatter, "portal frame cannot exceed {max_bytes} bytes")
            }
            Self::AccessibilityTooLarge { max_chars } => write!(
                formatter,
                "portal accessibility snapshot cannot exceed {max_chars} characters"
            ),
        }
    }
}

impl Error for PortalValidationError {}

fn validate_text(
    value: &str,
    max_chars: usize,
    field: &'static str,
) -> Result<(), PortalValidationError> {
    if value.trim().is_empty() {
        return Err(PortalValidationError::EmptyText { field });
    }
    if value.chars().count() > max_chars {
        return Err(PortalValidationError::TextTooLong { field, max_chars });
    }
    if value.contains('\0') {
        return Err(PortalValidationError::ControlCharacter { field });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browser_config_has_conservative_capabilities() {
        let config = PortalConfig::browser("https://example.test").unwrap();
        assert_eq!(config.target().kind(), PortalTargetKind::Browser);
        assert!(matches!(
            PortalCapabilities::browser_defaults().status(PortalOperation::Observe),
            CapabilityStatus::Supported
        ));
        assert!(matches!(
            PortalCapabilities::browser_defaults().status(PortalOperation::Input),
            CapabilityStatus::PermissionRequired { .. }
        ));
    }

    #[test]
    fn pointer_mapping_survives_canvas_zoom() {
        let viewport = PortalViewport::new(1_000, 500).unwrap();
        let zoom_one = PortalFrameTransform::new(
            PortalRect::new(100.0, 40.0, 500.0, 300.0).unwrap(),
            viewport,
            true,
        );
        let zoom_two = PortalFrameTransform::new(
            PortalRect::new(100.0, 40.0, 1_000.0, 600.0).unwrap(),
            viewport,
            true,
        );
        let first = zoom_one
            .canvas_to_viewport(PortalPoint::new(350.0, 190.0))
            .unwrap();
        let second = zoom_two
            .canvas_to_viewport(PortalPoint::new(600.0, 340.0))
            .unwrap();
        assert_eq!(first, PortalPoint::new(500.0, 250.0));
        assert_eq!(second, first);
    }

    #[test]
    fn letterboxed_pointer_is_rejected() {
        let transform = PortalFrameTransform::new(
            PortalRect::new(0.0, 0.0, 400.0, 400.0).unwrap(),
            PortalViewport::new(1_000, 500).unwrap(),
            true,
        );
        assert_eq!(
            transform.destination(),
            PortalRect::new(0.0, 100.0, 400.0, 200.0).unwrap()
        );
        assert!(
            transform
                .canvas_to_viewport(PortalPoint::new(200.0, 50.0))
                .is_none()
        );
    }

    #[test]
    fn session_rejects_stale_semantic_references() {
        let mut session = PortalSession::new(3);
        session.begin_connect().unwrap();
        session.connected().unwrap();
        assert_eq!(session.observe().unwrap(), 1);
        let reference = PortalElementRef::new(1, "submit").unwrap();
        assert!(session.accepts(&reference).is_ok());
        assert_eq!(session.observe().unwrap(), 2);
        assert_eq!(
            session.accepts(&reference),
            Err(PortalSessionError::StaleObservation {
                expected: 2,
                found: 1
            })
        );
    }

    #[test]
    fn session_rejects_stale_coordinate_actions() {
        let mut session = PortalSession::new(3);
        session.begin_connect().unwrap();
        session.connected().unwrap();
        assert_eq!(session.observe().unwrap(), 1);
        assert!(session.accepts_revision(1).is_ok());
        assert_eq!(session.observe().unwrap(), 2);
        assert_eq!(
            session.accepts_revision(1),
            Err(PortalSessionError::StaleObservation {
                expected: 2,
                found: 1
            })
        );
    }

    #[test]
    fn session_retains_only_bounded_current_observation() {
        let mut session = PortalSession::new(4);
        session.begin_connect().unwrap();
        session.connected().unwrap();
        let revision = session.observe().unwrap();
        let viewport = PortalViewport::new(800, 600).unwrap();
        let frame =
            PortalFrame::new(revision, viewport, PortalFrameEncoding::Png, vec![1, 2, 3]).unwrap();
        let accessibility = PortalAccessibilitySnapshot::new(revision, r#"{"nodes":[]}"#).unwrap();
        let observation = PortalObservation::new(revision, PortalCapabilities::browser_defaults())
            .with_frame(frame)
            .with_accessibility(accessibility);
        session.record_observation(observation).unwrap();
        assert_eq!(session.latest_observation().unwrap().revision(), revision);
        assert!(session.latest_observation().unwrap().frame().is_some());
        assert!(
            PortalFrame::new(revision, viewport, PortalFrameEncoding::Png, Vec::new()).is_err()
        );
    }

    #[test]
    fn session_close_is_terminal() {
        let mut session = PortalSession::new(1);
        session.begin_connect().unwrap();
        session.connected().unwrap();
        session.begin_close().unwrap();
        session.closed();
        assert_eq!(session.state(), PortalSessionState::Closed);
        assert!(session.begin_connect().is_err());
    }
}
