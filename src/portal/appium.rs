use std::collections::BTreeMap;
use std::fmt::{self, Display, Formatter};
use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::time::Duration;

use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, XmlVersion};
use serde_json::{Value, json};

use super::{
    PortalAccessibilitySnapshot, PortalAction, PortalBackend, PortalCapabilities, PortalConfig,
    PortalFrame, PortalFrameEncoding, PortalKeyInput, PortalObservation, PortalSession,
    PortalSessionError, PortalTargetKind, PortalValidationError, PortalViewport,
};

const DEFAULT_ADDRESS: SocketAddr =
    SocketAddr::new(std::net::IpAddr::V4(Ipv4Addr::LOCALHOST), 4723);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

pub struct AppiumBackend {
    address: SocketAddr,
    platform: PortalTargetKind,
    session_id: Option<String>,
    viewport: PortalViewport,
    element_revision: u64,
    element_selectors: BTreeMap<String, String>,
}

impl AppiumBackend {
    pub fn new(platform: PortalTargetKind) -> Result<Self, AppiumError> {
        if platform == PortalTargetKind::Browser {
            return Err(AppiumError::UnsupportedPlatform(platform));
        }
        Ok(Self {
            address: DEFAULT_ADDRESS,
            platform,
            session_id: None,
            viewport: PortalViewport::new(1_080, 1_920).expect("default viewport is non-zero"),
            element_revision: 0,
            element_selectors: BTreeMap::new(),
        })
    }

    pub fn with_address(
        platform: PortalTargetKind,
        address: SocketAddr,
    ) -> Result<Self, AppiumError> {
        let mut backend = Self::new(platform)?;
        backend.address = address;
        Ok(backend)
    }

    fn session_id(&self) -> Result<&str, AppiumError> {
        self.session_id.as_deref().ok_or(AppiumError::NotConnected)
    }

    fn session_path(&self, suffix: &str) -> Result<String, AppiumError> {
        Ok(format!("/session/{}{suffix}", self.session_id()?))
    }

    fn request(&self, method: &str, path: &str, body: Option<Value>) -> Result<Value, AppiumError> {
        let mut stream = TcpStream::connect_timeout(&self.address, REQUEST_TIMEOUT)
            .map_err(AppiumError::Connect)?;
        stream
            .set_read_timeout(Some(REQUEST_TIMEOUT))
            .map_err(AppiumError::Configure)?;
        stream
            .set_write_timeout(Some(REQUEST_TIMEOUT))
            .map_err(AppiumError::Configure)?;
        let body = body.map(|body| body.to_string()).unwrap_or_default();
        write!(
            stream,
            "{method} {path} HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            self.address,
            body.len(),
            body
        )
        .map_err(AppiumError::Write)?;
        let mut response = Vec::new();
        stream
            .read_to_end(&mut response)
            .map_err(AppiumError::Read)?;
        parse_response(&response)
    }

    fn element_request(
        &self,
        element_id: &str,
        suffix: &str,
        body: Value,
    ) -> Result<Value, AppiumError> {
        if element_id.is_empty()
            || !element_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err(AppiumError::InvalidElementId(element_id.to_owned()));
        }
        self.request(
            "POST",
            &self.session_path(&format!("/element/{element_id}{suffix}"))?,
            Some(body),
        )
    }

    fn resolve_element(&self, element_id: &str) -> Result<String, AppiumError> {
        let selector = self
            .element_selectors
            .get(element_id)
            .ok_or_else(|| AppiumError::InvalidElementId(element_id.to_owned()))?;
        let response = self.request(
            "POST",
            &self.session_path("/element")?,
            Some(json!({"using": "xpath", "value": selector})),
        )?;
        response
            .pointer("/value/element-6066-11e4-a52e-4f735466cecf")
            .or_else(|| response.pointer("/value/ELEMENT"))
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| {
                AppiumError::Protocol("element lookup returned no element ID".to_owned())
            })
    }

    fn touch_actions(&self, actions: Vec<Value>) -> Result<(), AppiumError> {
        self.request(
            "POST",
            &self.session_path("/actions")?,
            Some(json!({
                "actions": [{
                    "type": "pointer",
                    "id": "openpodium-touch",
                    "parameters": {"pointerType": "touch"},
                    "actions": actions
                }]
            })),
        )?;
        Ok(())
    }

    fn validate_coordinate(&self, x: u32, y: u32) -> Result<(), AppiumError> {
        if x >= self.viewport.width() || y >= self.viewport.height() {
            return Err(AppiumError::CoordinateOutsideViewport { x, y });
        }
        Ok(())
    }

    fn capture_screenshot(&self, revision: u64) -> Result<PortalFrame, AppiumError> {
        let screenshot = self.request("GET", &self.session_path("/screenshot")?, None)?;
        let encoded = screenshot
            .get("value")
            .and_then(Value::as_str)
            .ok_or_else(|| AppiumError::Protocol("screenshot response has no value".to_owned()))?;
        let bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, encoded)
            .map_err(AppiumError::Base64)?;
        PortalFrame::new(revision, self.viewport, PortalFrameEncoding::Png, bytes)
            .map_err(AppiumError::Validation)
    }
}

impl PortalBackend for AppiumBackend {
    type Error = AppiumError;

    fn capabilities(&self) -> PortalCapabilities {
        PortalCapabilities::appium_defaults()
    }

    fn connect(
        &mut self,
        config: &PortalConfig,
        session: &mut PortalSession,
    ) -> Result<(), Self::Error> {
        if config.target().kind() != self.platform {
            return Err(AppiumError::PlatformMismatch {
                expected: self.platform,
                found: config.target().kind(),
            });
        }
        session.begin_connect().map_err(AppiumError::Session)?;
        let automation = match self.platform {
            PortalTargetKind::Android => "UiAutomator2",
            PortalTargetKind::Ios => "XCUITest",
            PortalTargetKind::Browser => unreachable!("validated by constructor"),
        };
        let platform = match self.platform {
            PortalTargetKind::Android => "Android",
            PortalTargetKind::Ios => "iOS",
            PortalTargetKind::Browser => unreachable!("validated by constructor"),
        };
        let result = (|| {
            let response = self.request(
                "POST",
                "/session",
                Some(json!({
                    "capabilities": {
                        "alwaysMatch": {
                            "platformName": platform,
                            "appium:automationName": automation,
                            "appium:udid": config.target().selector()
                        },
                        "firstMatch": [{}]
                    }
                })),
            )?;
            let session_id = response
                .pointer("/value/sessionId")
                .or_else(|| response.get("sessionId"))
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    AppiumError::Protocol("session response has no session ID".to_owned())
                })?;
            self.session_id = Some(session_id.to_owned());
            if let Ok(rect) = self.request("GET", &self.session_path("/window/rect")?, None)
                && let (Some(width), Some(height)) = (
                    rect.pointer("/value/width").and_then(Value::as_u64),
                    rect.pointer("/value/height").and_then(Value::as_u64),
                )
                && let (Ok(width), Ok(height)) = (u32::try_from(width), u32::try_from(height))
                && let Some(viewport) = PortalViewport::new(width, height)
            {
                self.viewport = viewport;
            }
            session.connected().map_err(AppiumError::Session)
        })();
        if result.is_err() {
            self.session_id = None;
            self.element_selectors.clear();
            session.disconnect();
        }
        result
    }

    fn observe(&mut self, session: &mut PortalSession) -> Result<PortalObservation, Self::Error> {
        let revision = session.observe().map_err(AppiumError::Session)?;
        let frame = self.capture_screenshot(revision)?;
        let source = self.request("GET", &self.session_path("/source")?, None)?;
        let source = source
            .get("value")
            .and_then(Value::as_str)
            .ok_or_else(|| AppiumError::Protocol("source response has no value".to_owned()))?;
        let (element_selectors, elements) = appium_elements(source)?;
        self.element_revision = revision;
        self.element_selectors = element_selectors;
        let accessibility = PortalAccessibilitySnapshot::new(
            revision,
            json!({"format": "appium-elements-v1", "source": source, "elements": elements})
                .to_string(),
        )
        .map_err(AppiumError::Validation)?;
        let observation = PortalObservation::new(revision, self.capabilities())
            .with_frame(frame)
            .with_accessibility(accessibility);
        session
            .record_observation(observation.clone())
            .map_err(AppiumError::Session)?;
        Ok(observation)
    }

    fn capture_frame(
        &mut self,
        session: &PortalSession,
    ) -> Result<Option<PortalFrame>, Self::Error> {
        self.capture_screenshot(session.observation_revision())
            .map(Some)
    }

    fn execute(
        &mut self,
        session: &mut PortalSession,
        action: &PortalAction,
    ) -> Result<(), Self::Error> {
        match action {
            PortalAction::Click(element) => {
                session.accepts(element).map_err(AppiumError::Session)?;
                if element.observation_revision() != self.element_revision {
                    return Err(AppiumError::InvalidElementId(
                        element.backend_id().to_owned(),
                    ));
                }
                let element_id = self.resolve_element(element.backend_id())?;
                self.element_request(&element_id, "/click", json!({}))?;
            }
            PortalAction::TypeText { element, text } => {
                session.accepts(element).map_err(AppiumError::Session)?;
                if element.observation_revision() != self.element_revision {
                    return Err(AppiumError::InvalidElementId(
                        element.backend_id().to_owned(),
                    ));
                }
                let element_id = self.resolve_element(element.backend_id())?;
                self.element_request(
                    &element_id,
                    "/value",
                    json!({"text": text, "value": text.chars().map(String::from).collect::<Vec<_>>() }),
                )?;
            }
            PortalAction::ClickCoordinate {
                observation_revision,
                x,
                y,
            } => {
                session
                    .accepts_revision(*observation_revision)
                    .map_err(AppiumError::Session)?;
                self.validate_coordinate(*x, *y)?;
                self.touch_actions(vec![
                    json!({"type":"pointerMove","duration":0,"x":x,"y":y,"origin":"viewport"}),
                    json!({"type":"pointerDown","button":0}),
                    json!({"type":"pointerUp","button":0}),
                ])?;
            }
            PortalAction::TypeFocused {
                observation_revision,
                text,
            } => {
                session
                    .accepts_revision(*observation_revision)
                    .map_err(AppiumError::Session)?;
                self.request(
                    "POST",
                    &self.session_path("/keys")?,
                    Some(json!({"text": text, "value": text.chars().map(String::from).collect::<Vec<_>>() })),
                )?;
            }
            PortalAction::ScrollCoordinate {
                observation_revision,
                x,
                y,
                delta_x,
                delta_y,
            } => {
                session
                    .accepts_revision(*observation_revision)
                    .map_err(AppiumError::Session)?;
                self.validate_coordinate(*x, *y)?;
                let end_x = i64::from(*x)
                    .saturating_sub(i64::from(*delta_x))
                    .clamp(0, i64::from(self.viewport.width() - 1));
                let end_y = i64::from(*y)
                    .saturating_sub(i64::from(*delta_y))
                    .clamp(0, i64::from(self.viewport.height() - 1));
                self.touch_actions(vec![
                    json!({"type":"pointerMove","duration":0,"x":x,"y":y,"origin":"viewport"}),
                    json!({"type":"pointerDown","button":0}),
                    json!({"type":"pointerMove","duration":250,"x":end_x,"y":end_y,"origin":"viewport"}),
                    json!({"type":"pointerUp","button":0}),
                ])?;
            }
            PortalAction::Key {
                observation_revision,
                key,
                ..
            } => {
                session
                    .accepts_revision(*observation_revision)
                    .map_err(AppiumError::Session)?;
                let PortalTargetKind::Android = self.platform else {
                    return Err(AppiumError::UnsupportedAction("hardware key input on iOS"));
                };
                self.request(
                    "POST",
                    &self.session_path("/appium/device/press_keycode")?,
                    Some(json!({"keycode": android_keycode(*key)?})),
                )?;
            }
            PortalAction::Scroll { .. } => {
                return Err(AppiumError::UnsupportedAction("semantic scrolling"));
            }
            PortalAction::Navigate(_) => {
                return Err(AppiumError::UnsupportedAction("device navigation"));
            }
        }
        Ok(())
    }

    fn close(&mut self, session: &mut PortalSession) -> Result<(), Self::Error> {
        session.begin_close().map_err(AppiumError::Session)?;
        if self.session_id.is_some() {
            self.request("DELETE", &self.session_path("")?, None)?;
            self.session_id = None;
        }
        self.element_selectors.clear();
        session.closed();
        Ok(())
    }
}

#[derive(Default)]
struct XmlPath {
    value: String,
    child_counts: BTreeMap<String, usize>,
}

fn appium_elements(source: &str) -> Result<(BTreeMap<String, String>, Vec<Value>), AppiumError> {
    let mut reader = Reader::from_str(source);
    reader.config_mut().trim_text(true);
    let mut roots = BTreeMap::new();
    let mut stack = Vec::<XmlPath>::new();
    let mut selectors = BTreeMap::new();
    let mut elements = Vec::new();
    loop {
        match reader.read_event() {
            Ok(Event::Start(node)) => {
                let path = record_appium_element(
                    &node,
                    &mut roots,
                    &mut stack,
                    &mut selectors,
                    &mut elements,
                )?;
                stack.push(XmlPath {
                    value: path,
                    child_counts: BTreeMap::new(),
                });
            }
            Ok(Event::Empty(node)) => {
                record_appium_element(
                    &node,
                    &mut roots,
                    &mut stack,
                    &mut selectors,
                    &mut elements,
                )?;
            }
            Ok(Event::End(_)) => {
                stack.pop();
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(error) => {
                return Err(AppiumError::Protocol(format!(
                    "device source is invalid XML: {error}"
                )));
            }
        }
    }
    Ok((selectors, elements))
}

fn record_appium_element(
    node: &BytesStart<'_>,
    roots: &mut BTreeMap<String, usize>,
    stack: &mut [XmlPath],
    selectors: &mut BTreeMap<String, String>,
    elements: &mut Vec<Value>,
) -> Result<String, AppiumError> {
    let role = node.name().as_ref().to_owned();
    let parent = stack
        .last()
        .map_or_else(String::new, |parent| parent.value.clone());
    let index = {
        let counts = stack
            .last_mut()
            .map_or(roots, |parent| &mut parent.child_counts);
        let index = counts.entry(role.clone()).or_default();
        *index += 1;
        *index
    };
    let path = format!("{parent}/{role}[{index}]");
    if role != "hierarchy" {
        let mut attributes = BTreeMap::new();
        for attribute in node.attributes() {
            let attribute = attribute.map_err(|error| {
                AppiumError::Protocol(format!("invalid XML attribute: {error}"))
            })?;
            let key = attribute.key.as_ref().to_owned();
            let value = attribute
                .normalized_value(XmlVersion::Implicit1_0)
                .map_err(|error| {
                    AppiumError::Protocol(format!("invalid XML attribute value: {error}"))
                })?
                .into_owned();
            attributes.insert(key, value);
        }
        let element_id = format!("appium-{}", selectors.len() + 1);
        selectors.insert(element_id.clone(), path.clone());
        elements.push(json!({
            "element_id": element_id,
            "role": role,
            "selector": path,
            "attributes": attributes,
        }));
    }
    Ok(path)
}

impl Drop for AppiumBackend {
    fn drop(&mut self) {
        if self.session_id.is_some()
            && let Ok(path) = self.session_path("")
        {
            let _ = self.request("DELETE", &path, None);
            self.session_id = None;
        }
    }
}

fn android_keycode(key: PortalKeyInput) -> Result<u16, AppiumError> {
    match key {
        PortalKeyInput::Enter => Ok(66),
        PortalKeyInput::Tab => Ok(61),
        PortalKeyInput::Backspace => Ok(67),
        PortalKeyInput::Delete => Ok(112),
        PortalKeyInput::Escape => Ok(111),
        PortalKeyInput::ArrowUp => Ok(19),
        PortalKeyInput::ArrowDown => Ok(20),
        PortalKeyInput::ArrowLeft => Ok(21),
        PortalKeyInput::ArrowRight => Ok(22),
        PortalKeyInput::Home => Ok(122),
        PortalKeyInput::End => Ok(123),
        PortalKeyInput::PageUp | PortalKeyInput::PageDown => {
            Err(AppiumError::UnsupportedAction("this Android key"))
        }
    }
}

fn parse_response(response: &[u8]) -> Result<Value, AppiumError> {
    let response = std::str::from_utf8(response)
        .map_err(|error| AppiumError::Protocol(format!("response is not UTF-8: {error}")))?;
    let (head, body) = response
        .split_once("\r\n\r\n")
        .ok_or_else(|| AppiumError::Protocol("response has no header terminator".to_owned()))?;
    let status = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse::<u16>().ok())
        .ok_or_else(|| AppiumError::Protocol("response has no HTTP status".to_owned()))?;
    let payload: Value = if body.trim().is_empty() {
        Value::Null
    } else {
        serde_json::from_str(body).map_err(AppiumError::Json)?
    };
    if !(200..300).contains(&status) {
        return Err(AppiumError::Command {
            status,
            detail: payload.to_string(),
        });
    }
    Ok(payload)
}

#[derive(Debug)]
pub enum AppiumError {
    UnsupportedPlatform(PortalTargetKind),
    PlatformMismatch {
        expected: PortalTargetKind,
        found: PortalTargetKind,
    },
    NotConnected,
    Connect(std::io::Error),
    Configure(std::io::Error),
    Write(std::io::Error),
    Read(std::io::Error),
    Json(serde_json::Error),
    Base64(base64::DecodeError),
    Protocol(String),
    Command {
        status: u16,
        detail: String,
    },
    InvalidElementId(String),
    CoordinateOutsideViewport {
        x: u32,
        y: u32,
    },
    UnsupportedAction(&'static str),
    Session(PortalSessionError),
    Validation(PortalValidationError),
}

impl Display for AppiumError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedPlatform(platform) => {
                write!(formatter, "{platform:?} is not an Appium device platform")
            }
            Self::PlatformMismatch { expected, found } => write!(
                formatter,
                "Appium backend expects {expected:?}, found {found:?}"
            ),
            Self::NotConnected => formatter.write_str("Appium backend is not connected"),
            Self::Connect(error) => write!(formatter, "failed to connect to Appium: {error}"),
            Self::Configure(error) => {
                write!(formatter, "failed to configure Appium connection: {error}")
            }
            Self::Write(error) => write!(formatter, "failed to write Appium request: {error}"),
            Self::Read(error) => write!(formatter, "failed to read Appium response: {error}"),
            Self::Json(error) => write!(formatter, "invalid Appium JSON: {error}"),
            Self::Base64(error) => write!(formatter, "invalid Appium screenshot: {error}"),
            Self::Protocol(detail) => write!(formatter, "invalid Appium response: {detail}"),
            Self::Command { status, detail } => write!(
                formatter,
                "Appium command failed with HTTP {status}: {detail}"
            ),
            Self::InvalidElementId(id) => write!(formatter, "invalid Appium element ID {id:?}"),
            Self::CoordinateOutsideViewport { x, y } => write!(
                formatter,
                "device coordinate ({x}, {y}) is outside the viewport"
            ),
            Self::UnsupportedAction(action) => {
                write!(formatter, "Appium adapter does not support {action}")
            }
            Self::Session(error) => Display::fmt(error, formatter),
            Self::Validation(error) => Display::fmt(error, formatter),
        }
    }
}

impl std::error::Error for AppiumError {}

#[cfg(test)]
mod tests {
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};
    use std::thread;

    use crate::portal::{PortalBackend, PortalElementRef, PortalPresentation, PortalTarget};

    use super::*;

    fn read_http_request(stream: &mut TcpStream) -> String {
        let mut request = Vec::new();
        loop {
            let mut chunk = [0_u8; 512];
            let size = stream.read(&mut chunk).unwrap();
            assert!(size > 0, "client closed before completing HTTP request");
            request.extend_from_slice(&chunk[..size]);
            let Some(header_end) = request.windows(4).position(|bytes| bytes == b"\r\n\r\n") else {
                continue;
            };
            let headers = String::from_utf8_lossy(&request[..header_end]);
            let content_length = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().unwrap())
                })
                .unwrap_or(0);
            if request.len() >= header_end + 4 + content_length {
                return String::from_utf8(request).unwrap();
            }
        }
    }

    #[test]
    fn parses_success_and_error_responses() {
        let success = parse_response(
            b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n{\"value\":{\"sessionId\":\"abc\"}}",
        )
        .unwrap();
        assert_eq!(
            success.pointer("/value/sessionId").and_then(Value::as_str),
            Some("abc")
        );

        assert!(matches!(
            parse_response(b"HTTP/1.1 500 Error\r\n\r\n{\"value\":{\"error\":\"failed\"}}"),
            Err(AppiumError::Command { status: 500, .. })
        ));
    }

    #[test]
    fn rejects_browser_targets_and_maps_android_keys() {
        assert!(matches!(
            AppiumBackend::new(PortalTargetKind::Browser),
            Err(AppiumError::UnsupportedPlatform(PortalTargetKind::Browser))
        ));
        assert_eq!(android_keycode(PortalKeyInput::Enter).unwrap(), 66);
        // Text navigation, not KEYCODE_HOME, which would background the app.
        assert_eq!(android_keycode(PortalKeyInput::Home).unwrap(), 122);
        assert_eq!(android_keycode(PortalKeyInput::End).unwrap(), 123);
        assert!(android_keycode(PortalKeyInput::PageUp).is_err());
    }

    #[test]
    fn observes_and_operates_without_stopping_the_device() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let worker = thread::spawn(move || {
            for response in [
                r#"{"value":{"sessionId":"session-1"}}"#,
                r#"{"value":{"width":800,"height":600}}"#,
                r#"{"value":"AQ=="}"#,
                r#"{"value":"<hierarchy/>"}"#,
                r#"{"value":null}"#,
                r#"{"value":null}"#,
            ] {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = [0_u8; 2048];
                let _ = stream.read(&mut request).unwrap();
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    response.len(),
                    response
                )
                .unwrap();
                stream.flush().unwrap();
                stream.shutdown(std::net::Shutdown::Write).unwrap();
                let mut remaining = Vec::new();
                let _ = stream.read_to_end(&mut remaining);
            }
        });
        let mut backend = AppiumBackend::with_address(PortalTargetKind::Android, address).unwrap();
        let config = PortalConfig::new(
            PortalTarget::new(PortalTargetKind::Android, "emulator-5554").unwrap(),
            PortalPresentation::default(),
        );
        let mut session = PortalSession::new(1);

        backend.connect(&config, &mut session).unwrap();
        assert_eq!(backend.viewport, PortalViewport::new(800, 600).unwrap());
        let observation = backend.observe(&mut session).unwrap();
        assert_eq!(observation.revision(), 1);
        assert_eq!(observation.frame().unwrap().bytes(), &[1]);
        assert!(
            observation
                .accessibility()
                .unwrap()
                .json()
                .contains("<hierarchy/>")
        );
        backend
            .execute(
                &mut session,
                &PortalAction::ClickCoordinate {
                    observation_revision: 1,
                    x: 100,
                    y: 200,
                },
            )
            .unwrap();
        backend.close(&mut session).unwrap();

        worker.join().unwrap();
    }

    #[test]
    fn semantic_observation_resolves_scoped_element_tokens() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured_requests = Arc::clone(&requests);
        let worker = thread::spawn(move || {
            for response in [
                r#"{"value":{"sessionId":"session-1"}}"#,
                r#"{"value":{"width":800,"height":600}}"#,
                r#"{"value":"AQ=="}"#,
                r#"{"value":"<hierarchy><android.widget.Button text=\"Submit\" resource-id=\"submit\"/></hierarchy>"}"#,
                r#"{"value":{"element-6066-11e4-a52e-4f735466cecf":"element-42"}}"#,
                r#"{"value":null}"#,
                r#"{"value":null}"#,
            ] {
                let (mut stream, _) = listener.accept().unwrap();
                let request = read_http_request(&mut stream);
                captured_requests.lock().unwrap().push(request);
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    response.len(),
                    response
                )
                .unwrap();
                stream.flush().unwrap();
                stream.shutdown(std::net::Shutdown::Write).unwrap();
            }
        });
        let mut backend = AppiumBackend::with_address(PortalTargetKind::Android, address).unwrap();
        let config = PortalConfig::new(
            PortalTarget::new(PortalTargetKind::Android, "emulator-5554").unwrap(),
            PortalPresentation::default(),
        );
        let mut session = PortalSession::new(1);

        backend.connect(&config, &mut session).unwrap();
        let observation = backend.observe(&mut session).unwrap();
        let accessibility: Value =
            serde_json::from_str(observation.accessibility().unwrap().json()).unwrap();
        let element_id = accessibility
            .pointer("/elements/0/element_id")
            .and_then(Value::as_str)
            .unwrap();
        assert_eq!(element_id, "appium-1");
        assert_eq!(
            accessibility
                .pointer("/elements/0/attributes/text")
                .and_then(Value::as_str),
            Some("Submit")
        );

        backend
            .execute(
                &mut session,
                &PortalAction::Click(
                    PortalElementRef::new(observation.revision(), element_id).unwrap(),
                ),
            )
            .unwrap();
        backend.close(&mut session).unwrap();
        worker.join().unwrap();

        let requests = requests.lock().unwrap();
        let lookup = requests
            .iter()
            .find(|request| request.contains("POST /session/session-1/element HTTP/1.1"))
            .expect("semantic action must resolve its observation-scoped token");
        assert!(lookup.contains("\"using\":\"xpath\""));
        assert!(lookup.contains("/hierarchy[1]/android.widget.Button[1]"));
        assert!(
            requests
                .iter()
                .any(|request| request.contains("/element/element-42/click"))
        );
    }
}
