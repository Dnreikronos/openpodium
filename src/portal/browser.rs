use std::fmt::{self, Display, Formatter};
use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tungstenite::{Message, WebSocket, connect, stream::MaybeTlsStream};

use super::{
    PortalAccessibilitySnapshot, PortalAction, PortalBackend, PortalCapabilities, PortalConfig,
    PortalElementRef, PortalFrame, PortalFrameEncoding, PortalObservation, PortalSession,
    PortalSessionError,
};

const DEFAULT_VIEWPORT_WIDTH: u32 = 1280;
const DEFAULT_VIEWPORT_HEIGHT: u32 = 720;
const STARTUP_TIMEOUT: Duration = Duration::from_secs(5);
const COMMAND_TIMEOUT: Duration = Duration::from_secs(5);

pub struct BrowserBackend {
    executable: PathBuf,
    viewport: super::PortalViewport,
    process: Option<Child>,
    user_data_dir: Option<tempfile::TempDir>,
    socket: Option<WebSocket<MaybeTlsStream<TcpStream>>>,
    next_command_id: u64,
}

impl BrowserBackend {
    pub fn new(executable: impl Into<PathBuf>) -> Self {
        let viewport = super::PortalViewport::new(DEFAULT_VIEWPORT_WIDTH, DEFAULT_VIEWPORT_HEIGHT)
            .expect("default browser viewport is non-zero");
        Self {
            executable: executable.into(),
            viewport,
            process: None,
            user_data_dir: None,
            socket: None,
            next_command_id: 0,
        }
    }

    pub fn discover() -> Result<Self, BrowserError> {
        let mut candidates = Vec::new();
        if let Some(path) = std::env::var_os("OPENPODIUM_CHROMIUM") {
            candidates.push(PathBuf::from(path));
        }
        candidates.extend([
            PathBuf::from("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"),
            PathBuf::from("/Applications/Chromium.app/Contents/MacOS/Chromium"),
        ]);
        if let Some(path) = std::env::var_os("PATH") {
            for directory in std::env::split_paths(&path) {
                for name in [
                    "chromium",
                    "chromium-browser",
                    "google-chrome",
                    "google-chrome-stable",
                ] {
                    candidates.push(directory.join(name));
                }
            }
        }
        candidates
            .into_iter()
            .find(|path| path.is_file())
            .map(Self::new)
            .ok_or(BrowserError::ExecutableUnavailable)
    }

    pub const fn viewport(&self) -> super::PortalViewport {
        self.viewport
    }

    fn command(&mut self, method: &str, params: Value) -> Result<Value, BrowserError> {
        let socket = self.socket.as_mut().ok_or(BrowserError::NotConnected)?;
        self.next_command_id = self.next_command_id.wrapping_add(1);
        let id = self.next_command_id;
        let request = json!({"id": id, "method": method, "params": params});
        socket
            .send(Message::Text(request.to_string().into()))
            .map_err(|error| BrowserError::WebSocket(Box::new(error)))?;
        let deadline = Instant::now() + COMMAND_TIMEOUT;
        loop {
            if Instant::now() >= deadline {
                return Err(BrowserError::Timeout(method.to_owned()));
            }
            let message = socket
                .read()
                .map_err(|error| BrowserError::WebSocket(Box::new(error)))?;
            let Message::Text(text) = message else {
                continue;
            };
            let response: Value =
                serde_json::from_str(&text).map_err(|error| BrowserError::Json(Box::new(error)))?;
            if response.get("id").and_then(Value::as_u64) != Some(id) {
                continue;
            }
            if let Some(error) = response.get("error") {
                return Err(BrowserError::Command {
                    method: method.to_owned(),
                    detail: error.to_string(),
                });
            }
            return Ok(response.get("result").cloned().unwrap_or(Value::Null));
        }
    }

    fn launch(&mut self) -> Result<String, BrowserError> {
        let listener =
            TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).map_err(BrowserError::DebugEndpointBind)?;
        let port = listener
            .local_addr()
            .map_err(BrowserError::DebugEndpointBind)?
            .port();
        drop(listener);

        let user_data_dir = tempfile::tempdir().map_err(BrowserError::UserDataDirectory)?;
        let mut child = Command::new(&self.executable)
            .args([
                "--headless=new",
                "--no-first-run",
                "--no-default-browser-check",
                "--disable-gpu",
                "--remote-allow-origins=*",
            ])
            .arg(format!("--remote-debugging-port={port}"))
            .arg(format!(
                "--user-data-dir={}",
                user_data_dir.path().display()
            ))
            .arg("about:blank")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(BrowserError::Launch)?;

        let deadline = Instant::now() + STARTUP_TIMEOUT;
        while Instant::now() < deadline {
            if let Some(endpoint) = debug_websocket_url(port)? {
                self.process = Some(child);
                self.user_data_dir = Some(user_data_dir);
                return Ok(endpoint);
            }
            thread::sleep(Duration::from_millis(25));
        }
        let _ = child.kill();
        let _ = child.wait();
        Err(BrowserError::StartupTimeout)
    }

    fn connect_socket(&mut self, endpoint: &str) -> Result<(), BrowserError> {
        let (socket, _) =
            connect(endpoint).map_err(|error| BrowserError::WebSocket(Box::new(error)))?;
        self.socket = Some(socket);
        Ok(())
    }

    fn element_backend_id(element: &PortalElementRef) -> Result<i64, BrowserError> {
        element
            .backend_id()
            .parse::<i64>()
            .map_err(|_| BrowserError::InvalidElementReference(element.backend_id().to_owned()))
    }

    fn click(&mut self, element: &PortalElementRef) -> Result<(), BrowserError> {
        let backend_node_id = Self::element_backend_id(element)?;
        let model = self.command("DOM.getBoxModel", json!({"backendNodeId": backend_node_id}))?;
        let content = model
            .get("model")
            .and_then(|model| model.get("content"))
            .and_then(Value::as_array)
            .ok_or_else(|| BrowserError::Command {
                method: "DOM.getBoxModel".to_owned(),
                detail: "response did not contain a content quad".to_owned(),
            })?;
        let x = content
            .chunks_exact(2)
            .map(|point| point[0].as_f64().unwrap_or_default())
            .sum::<f64>()
            / 4.0;
        let y = content
            .chunks_exact(2)
            .map(|point| point[1].as_f64().unwrap_or_default())
            .sum::<f64>()
            / 4.0;
        for event_type in ["mousePressed", "mouseReleased"] {
            self.command(
                "Input.dispatchMouseEvent",
                json!({
                    "type": event_type,
                    "x": x,
                    "y": y,
                    "button": "left",
                    "clickCount": 1
                }),
            )?;
        }
        Ok(())
    }
}

impl PortalBackend for BrowserBackend {
    type Error = BrowserError;

    fn connect(
        &mut self,
        config: &PortalConfig,
        session: &mut PortalSession,
    ) -> Result<(), Self::Error> {
        session.begin_connect().map_err(BrowserError::Session)?;
        let result = (|| {
            let endpoint = self.launch()?;
            self.connect_socket(&endpoint)?;
            self.command("Page.enable", Value::Null)?;
            self.command("DOM.enable", Value::Null)?;
            self.command("Accessibility.enable", Value::Null)?;
            self.command(
                "Emulation.setDeviceMetricsOverride",
                json!({
                    "width": self.viewport.width(),
                    "height": self.viewport.height(),
                    "deviceScaleFactor": 1,
                    "mobile": false
                }),
            )?;
            self.command("Page.navigate", json!({"url": config.target().selector()}))?;
            session.connected().map_err(BrowserError::Session)
        })();
        if result.is_err() {
            let _ = self.shutdown();
            session.disconnect();
        }
        result
    }

    fn observe(&mut self, session: &mut PortalSession) -> Result<PortalObservation, Self::Error> {
        let revision = session.observe().map_err(BrowserError::Session)?;
        let screenshot = self.command(
            "Page.captureScreenshot",
            json!({"format": "png", "fromSurface": true}),
        )?;
        let encoded = screenshot
            .get("data")
            .and_then(Value::as_str)
            .ok_or_else(|| BrowserError::Command {
                method: "Page.captureScreenshot".to_owned(),
                detail: "response did not contain base64 screenshot data".to_owned(),
            })?;
        let bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, encoded)
            .map_err(|error| BrowserError::Base64(Box::new(error)))?;
        let frame = PortalFrame::new(revision, self.viewport, PortalFrameEncoding::Png, bytes)
            .map_err(BrowserError::Validation)?;
        let accessibility = self.command("Accessibility.getFullAXTree", Value::Null)?;
        let accessibility = PortalAccessibilitySnapshot::new(revision, accessibility.to_string())
            .map_err(BrowserError::Validation)?;
        let observation = PortalObservation::new(revision, PortalCapabilities::browser_defaults())
            .with_frame(frame)
            .with_accessibility(accessibility);
        session
            .record_observation(observation.clone())
            .map_err(BrowserError::Session)?;
        Ok(observation)
    }

    fn execute(
        &mut self,
        session: &mut PortalSession,
        action: &PortalAction,
    ) -> Result<(), Self::Error> {
        match action {
            PortalAction::Click(element) => {
                session.accepts(element).map_err(BrowserError::Session)?;
                self.click(element)
            }
            PortalAction::TypeText { element, text } => {
                session.accepts(element).map_err(BrowserError::Session)?;
                let backend_node_id = Self::element_backend_id(element)?;
                self.command("DOM.focus", json!({"backendNodeId": backend_node_id}))?;
                self.command("Input.insertText", json!({"text": text}))?;
                Ok(())
            }
            PortalAction::Scroll {
                element,
                delta_x,
                delta_y,
            } => {
                if let Some(element) = element {
                    session.accepts(element).map_err(BrowserError::Session)?;
                }
                self.command(
                    "Input.dispatchMouseEvent",
                    json!({
                        "type": "mouseWheel",
                        "x": f64::from(self.viewport.width()) / 2.0,
                        "y": f64::from(self.viewport.height()) / 2.0,
                        "deltaX": delta_x,
                        "deltaY": delta_y
                    }),
                )?;
                Ok(())
            }
            PortalAction::Navigate(url) => {
                self.command("Page.navigate", json!({"url": url}))?;
                Ok(())
            }
        }
    }

    fn close(&mut self, session: &mut PortalSession) -> Result<(), Self::Error> {
        session.begin_close().map_err(BrowserError::Session)?;
        self.shutdown()?;
        session.closed();
        Ok(())
    }
}

impl BrowserBackend {
    fn shutdown(&mut self) -> Result<(), BrowserError> {
        if let Some(mut socket) = self.socket.take() {
            let _ = socket.close(None);
        }
        if let Some(mut process) = self.process.take() {
            let _ = process.kill();
            let _ = process.wait();
        }
        let _ = self.user_data_dir.take();
        Ok(())
    }
}

impl Drop for BrowserBackend {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

fn debug_websocket_url(port: u16) -> Result<Option<String>, BrowserError> {
    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let mut stream = match TcpStream::connect_timeout(&address, Duration::from_millis(100)) {
        Ok(stream) => stream,
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::TimedOut
            ) =>
        {
            return Ok(None);
        }
        Err(error) => return Err(BrowserError::DebugEndpoint(error)),
    };
    stream
        .set_read_timeout(Some(Duration::from_millis(100)))
        .map_err(BrowserError::DebugEndpoint)?;
    stream
        .write_all(b"GET /json/list HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .map_err(BrowserError::DebugEndpoint)?;
    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .map_err(BrowserError::DebugEndpoint)?;
    let body = response.split("\r\n\r\n").nth(1).unwrap_or_default();
    if body.trim().is_empty() {
        return Ok(None);
    }
    let body: Value =
        serde_json::from_str(body).map_err(|error| BrowserError::Json(Box::new(error)))?;
    Ok(body.as_array().and_then(|targets| {
        targets.iter().find_map(|target| {
            target
                .get("webSocketDebuggerUrl")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
    }))
}

#[derive(Debug)]
pub enum BrowserError {
    NotConnected,
    ExecutableUnavailable,
    Launch(std::io::Error),
    DebugEndpointBind(std::io::Error),
    DebugEndpoint(std::io::Error),
    UserDataDirectory(std::io::Error),
    WebSocket(Box<tungstenite::Error>),
    Json(Box<serde_json::Error>),
    Base64(Box<base64::DecodeError>),
    Command { method: String, detail: String },
    Timeout(String),
    InvalidElementReference(String),
    Session(PortalSessionError),
    Validation(super::PortalValidationError),
    StartupTimeout,
}

impl Display for BrowserError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotConnected => formatter.write_str("browser backend is not connected"),
            Self::ExecutableUnavailable => formatter.write_str("Chromium executable was not found"),
            Self::Launch(error) => write!(formatter, "failed to launch Chromium: {error}"),
            Self::DebugEndpointBind(error) => {
                write!(formatter, "failed to reserve a CDP port: {error}")
            }
            Self::DebugEndpoint(error) => {
                write!(formatter, "failed to query CDP endpoint: {error}")
            }
            Self::UserDataDirectory(error) => {
                write!(
                    formatter,
                    "failed to create isolated browser profile: {error}"
                )
            }
            Self::WebSocket(error) => write!(formatter, "CDP websocket failed: {error}"),
            Self::Json(error) => write!(formatter, "invalid CDP JSON: {error}"),
            Self::Base64(error) => write!(formatter, "invalid CDP screenshot data: {error}"),
            Self::Command { method, detail } => write!(formatter, "CDP {method} failed: {detail}"),
            Self::Timeout(method) => write!(formatter, "CDP {method} timed out"),
            Self::InvalidElementReference(id) => {
                write!(formatter, "invalid portal element reference {id:?}")
            }
            Self::Session(error) => Display::fmt(error, formatter),
            Self::Validation(error) => Display::fmt(error, formatter),
            Self::StartupTimeout => formatter.write_str("Chromium did not expose CDP in time"),
        }
    }
}

impl std::error::Error for BrowserError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portal::{PortalBackend, PortalSessionState};

    #[test]
    fn failed_launch_returns_session_to_disconnected() {
        let mut backend = BrowserBackend::new("/missing/chromium");
        let config = PortalConfig::browser("https://example.test").unwrap();
        let mut session = PortalSession::new(1);

        assert!(backend.connect(&config, &mut session).is_err());
        assert_eq!(session.state(), PortalSessionState::Disconnected);
    }

    #[test]
    fn close_during_connect_releases_an_empty_backend() {
        let mut backend = BrowserBackend::new("/missing/chromium");
        let mut session = PortalSession::new(1);
        session.begin_connect().unwrap();

        backend.close(&mut session).unwrap();
        assert_eq!(session.state(), PortalSessionState::Closed);
    }

    #[test]
    fn discovers_a_page_target_websocket() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let worker = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 512];
            let _ = stream.read(&mut request).unwrap();
            let body =
                r#"[{"type":"page","webSocketDebuggerUrl":"ws://127.0.0.1:1234/devtools/page/1"}]"#;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            )
            .unwrap();
        });

        assert_eq!(
            debug_websocket_url(port).unwrap().as_deref(),
            Some("ws://127.0.0.1:1234/devtools/page/1")
        );
        worker.join().unwrap();
    }
}
