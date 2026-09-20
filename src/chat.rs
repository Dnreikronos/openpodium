use std::collections::BTreeMap;
use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use iced::widget::{button, column, container, image, markdown, row, scrollable, text, text_input};
use iced::{Color, ContentFit, Element, Fill, Task};
use openpodium::domain::{
    AgentId, ChatAttachment, ChatAttachmentId, ChatMessageId, ChatThreadId, NodeTarget, Workspace,
    WorkspaceId,
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Surface {
    Chat,
    #[default]
    Terminal,
}

#[derive(Debug, Clone)]
pub enum Message {
    SurfaceChanged(Surface),
    ThreadSelected(ChatThreadId),
    NewThreadNameChanged(String),
    NewThreadColorChanged(String),
    CreateThread,
    DraftChanged(String),
    MentionToggled(NodeTarget),
    AttachmentPathChanged(String),
    Attach,
    Submit,
    Rich(Action),
}

#[derive(Default)]
pub struct UiState {
    surfaces: BTreeMap<(WorkspaceId, AgentId), Surface>,
    selected_threads: BTreeMap<(WorkspaceId, AgentId), ChatThreadId>,
    new_thread_names: BTreeMap<(WorkspaceId, AgentId), String>,
    new_thread_colors: BTreeMap<(WorkspaceId, AgentId), String>,
    attachment_paths: BTreeMap<(WorkspaceId, ChatThreadId), String>,
    rendered_messages: BTreeMap<(WorkspaceId, ChatMessageId), markdown::Content>,
    highlighted_messages: BTreeMap<WorkspaceId, ChatMessageId>,
}

impl UiState {
    pub fn sync(&mut self, workspace: &Workspace) {
        for thread in workspace.chat_threads() {
            for message in thread.messages() {
                self.rendered_messages
                    .entry((workspace.id(), message.id()))
                    .or_insert_with(|| parse(message.content().as_str()));
            }
        }
    }

    pub fn surface(&self, workspace_id: WorkspaceId, agent_id: AgentId) -> Surface {
        self.surfaces
            .get(&(workspace_id, agent_id))
            .copied()
            .unwrap_or_default()
    }

    pub fn set_surface(&mut self, workspace_id: WorkspaceId, agent_id: AgentId, surface: Surface) {
        self.surfaces.insert((workspace_id, agent_id), surface);
    }

    pub fn selected_thread(
        &self,
        workspace: &Workspace,
        agent_id: AgentId,
    ) -> Option<ChatThreadId> {
        self.selected_threads
            .get(&(workspace.id(), agent_id))
            .copied()
            .filter(|thread_id| {
                workspace
                    .chat_thread(*thread_id)
                    .is_some_and(|thread| thread.agent_id() == agent_id)
            })
            .or_else(|| {
                workspace
                    .chat_threads()
                    .find(|thread| thread.agent_id() == agent_id)
                    .map(|thread| thread.id())
            })
    }

    pub fn select_thread(
        &mut self,
        workspace_id: WorkspaceId,
        agent_id: AgentId,
        thread_id: ChatThreadId,
    ) {
        self.selected_threads
            .insert((workspace_id, agent_id), thread_id);
    }

    pub fn new_thread_name(&self, workspace_id: WorkspaceId, agent_id: AgentId) -> &str {
        self.new_thread_names
            .get(&(workspace_id, agent_id))
            .map_or("", String::as_str)
    }

    pub fn set_new_thread_name(
        &mut self,
        workspace_id: WorkspaceId,
        agent_id: AgentId,
        value: String,
    ) {
        self.new_thread_names
            .insert((workspace_id, agent_id), value);
    }

    pub fn new_thread_color(&self, workspace_id: WorkspaceId, agent_id: AgentId) -> &str {
        self.new_thread_colors
            .get(&(workspace_id, agent_id))
            .map_or("#2563EB", String::as_str)
    }

    pub fn set_new_thread_color(
        &mut self,
        workspace_id: WorkspaceId,
        agent_id: AgentId,
        value: String,
    ) {
        self.new_thread_colors
            .insert((workspace_id, agent_id), value);
    }

    pub fn clear_new_thread(&mut self, workspace_id: WorkspaceId, agent_id: AgentId) {
        self.new_thread_names.remove(&(workspace_id, agent_id));
        self.new_thread_colors.remove(&(workspace_id, agent_id));
    }

    pub fn attachment_path(&self, workspace_id: WorkspaceId, thread_id: ChatThreadId) -> &str {
        self.attachment_paths
            .get(&(workspace_id, thread_id))
            .map_or("", String::as_str)
    }

    pub fn set_attachment_path(
        &mut self,
        workspace_id: WorkspaceId,
        thread_id: ChatThreadId,
        value: String,
    ) {
        self.attachment_paths
            .insert((workspace_id, thread_id), value);
    }

    pub fn clear_attachment_path(&mut self, workspace_id: WorkspaceId, thread_id: ChatThreadId) {
        self.attachment_paths.remove(&(workspace_id, thread_id));
    }

    pub fn reveal_message(
        &mut self,
        workspace: &Workspace,
        agent_id: AgentId,
        thread_id: ChatThreadId,
        message_id: ChatMessageId,
    ) -> Option<Task<Message>> {
        let thread = workspace.chat_thread(thread_id)?;
        if thread.agent_id() != agent_id {
            return None;
        }
        let index = thread
            .messages()
            .iter()
            .position(|message| message.id() == message_id)?;
        self.set_surface(workspace.id(), agent_id, Surface::Chat);
        self.select_thread(workspace.id(), agent_id, thread_id);
        self.highlighted_messages.insert(workspace.id(), message_id);
        let denominator = thread.messages().len().saturating_sub(1).max(1) as f32;
        Some(iced::widget::operation::snap_to(
            message_scroll_id(workspace.id(), thread_id),
            iced::widget::operation::RelativeOffset {
                x: 0.0,
                y: index as f32 / denominator,
            },
        ))
    }
}

pub fn conversation_panel<'a>(
    workspace: &'a Workspace,
    agent_id: AgentId,
    state: &'a UiState,
    store: Option<&'a AttachmentStore>,
) -> Element<'a, Message> {
    let workspace_id = workspace.id();
    let mut panel = column![
        text("Agent conversation").size(18),
        row![
            button(if state.surface(workspace_id, agent_id) == Surface::Chat {
                "✓ Chat"
            } else {
                "Chat"
            })
            .on_press(Message::SurfaceChanged(Surface::Chat)),
            button(
                if state.surface(workspace_id, agent_id) == Surface::Terminal {
                    "✓ Terminal"
                } else {
                    "Terminal"
                }
            )
            .on_press(Message::SurfaceChanged(Surface::Terminal)),
        ]
        .spacing(8),
    ]
    .spacing(10);
    if state.surface(workspace_id, agent_id) == Surface::Terminal {
        return panel
            .push(text(
                "The raw terminal remains live on the canvas. Use the controls below to manage it.",
            ))
            .into();
    }

    let selected_thread = state.selected_thread(workspace, agent_id);
    let mut thread_buttons = row![].spacing(6);
    for thread in workspace
        .chat_threads()
        .filter(|thread| thread.agent_id() == agent_id)
    {
        let label = row![
            text(if Some(thread.id()) == selected_thread {
                "✓"
            } else {
                ""
            }),
            text("●").color(parse_thread_color(thread.color().as_str())),
            text(thread.name().to_string()),
        ]
        .spacing(4);
        thread_buttons =
            thread_buttons.push(button(label).on_press(Message::ThreadSelected(thread.id())));
    }
    panel = panel.push(thread_buttons.wrap()).push(
        row![
            text_input(
                "New thread name",
                state.new_thread_name(workspace_id, agent_id),
            )
            .on_input(Message::NewThreadNameChanged),
            text_input("#RRGGBB", state.new_thread_color(workspace_id, agent_id),)
                .on_input(Message::NewThreadColorChanged)
                .width(110),
            button("Create").on_press(Message::CreateThread),
        ]
        .spacing(6),
    );

    let Some(thread_id) = selected_thread else {
        return panel
            .push(text("Create a thread to start a conversation."))
            .into();
    };
    let thread = workspace
        .chat_thread(thread_id)
        .expect("the selected thread was validated");
    let mut messages = column![].spacing(12);
    for message in thread.messages() {
        let author = match message.author() {
            openpodium::domain::ChatAuthor::User => "You".to_owned(),
            openpodium::domain::ChatAuthor::Agent(id) => workspace
                .agent(id)
                .map_or_else(|| format!("Agent {id}"), |agent| agent.name().to_string()),
        };
        let mut body = column![text(author).size(12)].spacing(6);
        if state.highlighted_messages.get(&workspace_id) == Some(&message.id()) {
            body = body.push(text("› Search match").size(12));
        }
        if let Some(rendered) = state.rendered_messages.get(&(workspace_id, message.id())) {
            body = body.push(view(rendered).map(Message::Rich));
        } else {
            body = body.push(text(message.content().as_str()));
        }
        for attachment_id in message.attachments() {
            if let (Some(attachment), Some(store)) =
                (workspace.chat_attachment(*attachment_id), store)
            {
                body =
                    body.push(attachment_view(attachment, workspace_id, store).map(Message::Rich));
            }
        }
        messages = messages.push(container(body).padding(10));
    }
    panel = panel
        .push(
            scrollable(messages)
                .id(message_scroll_id(workspace_id, thread_id))
                .height(260),
        )
        .push(text_input("Write a prompt…", thread.draft().text()).on_input(Message::DraftChanged));

    let connected = connected_targets(workspace, agent_id);
    if !connected.is_empty() {
        let mut mentions = row![text("Mentions:").size(12)].spacing(6);
        for target in connected {
            let selected = thread.draft().mentions().contains(&target);
            mentions = mentions.push(
                button(text(format!(
                    "{}{}",
                    if selected { "✓ " } else { "" },
                    mention_label(workspace, target)
                )))
                .on_press(Message::MentionToggled(target)),
            );
        }
        panel = panel.push(mentions.wrap());
    }
    for attachment_id in thread.draft().attachments() {
        if let Some(attachment) = workspace.chat_attachment(*attachment_id) {
            panel = panel.push(text(format!(
                "Attached: {} ({} bytes)",
                attachment.display_name(),
                attachment.byte_len()
            )));
        }
    }
    panel
        .push(text("Attachments: 10 MiB each, 25 MiB total, up to 8 files").size(12))
        .push(
            row![
                text_input(
                    "/path/to/local/file",
                    state.attachment_path(workspace_id, thread_id),
                )
                .on_input(Message::AttachmentPathChanged),
                button("Attach").on_press(Message::Attach),
            ]
            .spacing(6),
        )
        .push(button("Send prompt").on_press(Message::Submit))
        .into()
}

fn message_scroll_id(workspace_id: WorkspaceId, thread_id: ChatThreadId) -> String {
    format!("chat-messages-{}-{}", workspace_id.get(), thread_id.get())
}

fn connected_targets(workspace: &Workspace, agent_id: AgentId) -> Vec<NodeTarget> {
    let layout = workspace.canvas_layout();
    let owner_nodes: Vec<_> = layout
        .nodes()
        .iter()
        .filter(|node| node.reference() == Some(NodeTarget::Agent(agent_id)))
        .map(|node| node.id())
        .collect();
    let mut targets = Vec::new();
    for connection in layout.connections() {
        let other = if owner_nodes.contains(&connection.source()) {
            Some(connection.target())
        } else if owner_nodes.contains(&connection.target()) {
            Some(connection.source())
        } else {
            None
        };
        let Some(target) = other.and_then(|node_id| workspace.node(node_id)?.reference()) else {
            continue;
        };
        if target != NodeTarget::Agent(agent_id) && !targets.contains(&target) {
            targets.push(target);
        }
    }
    targets
}

fn mention_label(workspace: &Workspace, target: NodeTarget) -> String {
    match target {
        NodeTarget::Agent(id) => workspace.agent(id).map_or_else(
            || format!("@agent-{id}"),
            |agent| format!("@{}", agent.name()),
        ),
        NodeTarget::Task(id) => workspace.task(id).map_or_else(
            || format!("@task-{id}"),
            |task| format!("@{}", task.title()),
        ),
        NodeTarget::Handoff(id) => format!("@handoff-{id}"),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    LinkClicked(markdown::Uri),
}

pub fn parse(content: &str) -> markdown::Content {
    markdown::Content::parse(content)
}

pub fn view(content: &markdown::Content) -> Element<'_, Action> {
    markdown::view_with(
        content.items(),
        markdown::Settings::with_style(iced::Theme::Dark),
        &CHAT_VIEWER,
    )
}

pub fn attachment_view<'a>(
    attachment: &'a ChatAttachment,
    workspace_id: WorkspaceId,
    store: &AttachmentStore,
) -> Element<'a, Action> {
    let details = format!(
        "{} · {} bytes",
        attachment.media_type(),
        attachment.byte_len()
    );
    let card = column![text(attachment.display_name()), text(details).size(12)].spacing(4);
    if is_previewable_image(attachment.media_type()) {
        column![
            image(store.stored_path(workspace_id, attachment))
                .width(Fill)
                .height(220)
                .content_fit(ContentFit::Contain),
            card,
        ]
        .spacing(6)
        .into()
    } else {
        container(card).padding(8).into()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkTarget {
    Web(String),
    WorkspaceFile(PathBuf),
    Attachment(ChatAttachmentId),
    Blocked,
}

pub fn classify_link(uri: &str, workspace_root: &Path) -> LinkTarget {
    if uri.chars().any(char::is_control) || uri.chars().any(char::is_whitespace) {
        return LinkTarget::Blocked;
    }
    if uri.starts_with("https://") || uri.starts_with("http://") {
        return LinkTarget::Web(uri.to_owned());
    }
    if let Some(value) = uri.strip_prefix("attachment:") {
        return value
            .parse::<u64>()
            .ok()
            .map(ChatAttachmentId::new)
            .map_or(LinkTarget::Blocked, LinkTarget::Attachment);
    }
    let Some(reference) = uri.strip_prefix("file:") else {
        return LinkTarget::Blocked;
    };
    resolve_workspace_file(workspace_root, reference)
        .map_or(LinkTarget::Blocked, LinkTarget::WorkspaceFile)
}

fn resolve_workspace_file(workspace_root: &Path, reference: &str) -> Option<PathBuf> {
    let reference = reference.split('#').next().unwrap_or_default();
    let relative = Path::new(reference);
    if relative.as_os_str().is_empty()
        || relative.is_absolute()
        || relative
            .components()
            .any(|component| matches!(component, Component::ParentDir | Component::Prefix(_)))
    {
        return None;
    }
    let root = workspace_root.canonicalize().ok()?;
    let resolved = root.join(relative).canonicalize().ok()?;
    resolved.starts_with(&root).then_some(resolved)
}

#[derive(Debug, Clone)]
pub struct AttachmentStore {
    root: PathBuf,
}

impl AttachmentStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn import(
        &self,
        source: &Path,
        workspace_id: WorkspaceId,
        thread_id: ChatThreadId,
        attachment_id: ChatAttachmentId,
    ) -> Result<ChatAttachment, AttachmentError> {
        let metadata = fs::metadata(source).map_err(|source_error| AttachmentError::Io {
            operation: "inspect attachment",
            path: source.to_owned(),
            source: source_error,
        })?;
        if !metadata.is_file() {
            return Err(AttachmentError::NotAFile(source.to_owned()));
        }
        if metadata.len() > ChatAttachment::MAX_BYTES {
            return Err(AttachmentError::TooLarge {
                max_bytes: ChatAttachment::MAX_BYTES,
                actual_bytes: metadata.len(),
            });
        }
        let display_name = source
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| AttachmentError::InvalidFileName(source.to_owned()))?
            .to_owned();
        let extension = source
            .extension()
            .and_then(|extension| extension.to_str())
            .filter(|extension| {
                !extension.is_empty()
                    && extension.len() <= 10
                    && extension.bytes().all(|byte| byte.is_ascii_alphanumeric())
            });
        let storage_key = extension.map_or_else(
            || format!("attachment-{}", attachment_id.get()),
            |extension| {
                format!(
                    "attachment-{}.{}",
                    attachment_id.get(),
                    extension.to_lowercase()
                )
            },
        );
        let attachment = ChatAttachment::new(
            attachment_id,
            thread_id,
            storage_key,
            display_name,
            media_type(source),
            metadata.len(),
        )
        .map_err(|error| AttachmentError::InvalidMetadata(error.to_string()))?;
        let directory = self.thread_directory(workspace_id, thread_id);
        fs::create_dir_all(&directory).map_err(|source_error| AttachmentError::Io {
            operation: "create attachment directory",
            path: directory.clone(),
            source: source_error,
        })?;
        let destination = directory.join(attachment.storage_key());
        if destination.exists() {
            return Err(AttachmentError::AlreadyExists(destination));
        }
        let staging = directory.join(format!(".{}.staging", attachment.storage_key()));
        if let Err(source_error) = fs::copy(source, &staging) {
            let _ = fs::remove_file(&staging);
            return Err(AttachmentError::Io {
                operation: "stage attachment",
                path: staging,
                source: source_error,
            });
        }
        if let Err(source_error) = fs::rename(&staging, &destination) {
            let _ = fs::remove_file(&staging);
            return Err(AttachmentError::Io {
                operation: "store attachment",
                path: destination,
                source: source_error,
            });
        }
        Ok(attachment)
    }

    pub fn stored_path(&self, workspace_id: WorkspaceId, attachment: &ChatAttachment) -> PathBuf {
        self.thread_directory(workspace_id, attachment.thread_id())
            .join(attachment.storage_key())
    }

    pub fn remove(
        &self,
        workspace_id: WorkspaceId,
        attachment: &ChatAttachment,
    ) -> Result<(), AttachmentError> {
        let path = self
            .thread_directory(workspace_id, attachment.thread_id())
            .join(attachment.storage_key());
        match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(source) => Err(AttachmentError::Io {
                operation: "remove attachment",
                path,
                source,
            }),
        }
    }

    fn thread_directory(&self, workspace_id: WorkspaceId, thread_id: ChatThreadId) -> PathBuf {
        self.root
            .join(workspace_id.get().to_string())
            .join("by-thread")
            .join(thread_id.get().to_string())
    }
}

#[derive(Debug)]
pub enum AttachmentError {
    Io {
        operation: &'static str,
        path: PathBuf,
        source: io::Error,
    },
    NotAFile(PathBuf),
    InvalidFileName(PathBuf),
    TooLarge {
        max_bytes: u64,
        actual_bytes: u64,
    },
    AlreadyExists(PathBuf),
    InvalidMetadata(String),
}

impl Display for AttachmentError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io {
                operation,
                path,
                source,
            } => write!(
                formatter,
                "failed to {operation} {}: {source}",
                path.display()
            ),
            Self::NotAFile(path) => write!(formatter, "{} is not a file", path.display()),
            Self::InvalidFileName(path) => {
                write!(formatter, "{} has an unsupported file name", path.display())
            }
            Self::TooLarge {
                max_bytes,
                actual_bytes,
            } => write!(
                formatter,
                "attachment is {actual_bytes} bytes; the limit is {max_bytes} bytes"
            ),
            Self::AlreadyExists(path) => {
                write!(formatter, "attachment already exists at {}", path.display())
            }
            Self::InvalidMetadata(detail) => formatter.write_str(detail),
        }
    }
}

impl Error for AttachmentError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct ChatViewer;

const CHAT_VIEWER: ChatViewer = ChatViewer;

impl<'a> markdown::Viewer<'a, Action> for ChatViewer {
    fn on_link_click(url: markdown::Uri) -> Action {
        Action::LinkClicked(url)
    }

    fn code_block(
        &self,
        settings: markdown::Settings,
        language: Option<&'a str>,
        code: &'a str,
        lines: &'a [markdown::Text],
    ) -> Element<'a, Action> {
        if language.is_some_and(|language| language.eq_ignore_ascii_case("mermaid"))
            && let Ok(diagram) = Diagram::parse(code)
        {
            return diagram.into_view();
        }
        markdown::code_block(settings, lines, Self::on_link_click)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Direction {
    TopDown,
    LeftRight,
}

#[derive(Debug, PartialEq, Eq)]
struct Diagram {
    direction: Direction,
    nodes: BTreeMap<String, String>,
    edges: Vec<(String, String)>,
}

impl Diagram {
    const MAX_NODES: usize = 64;
    const MAX_EDGES: usize = 128;

    fn parse(source: &str) -> Result<Self, ()> {
        let mut lines = source
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty());
        let mut header = lines.next().ok_or(())?.split_whitespace();
        if !matches!(header.next(), Some("graph" | "flowchart")) {
            return Err(());
        }
        let direction = match header.next() {
            Some("TD" | "TB") => Direction::TopDown,
            Some("LR") => Direction::LeftRight,
            _ => return Err(()),
        };
        if header.next().is_some() {
            return Err(());
        }

        let mut nodes = BTreeMap::new();
        let mut edges = Vec::new();
        for line in lines {
            if is_unsafe_mermaid(line) {
                return Err(());
            }
            let line = line.trim_end_matches(';').trim();
            if let Some((source, target)) = line.split_once("-->") {
                if edges.len() == Self::MAX_EDGES {
                    return Err(());
                }
                let (source_id, source_label) = parse_mermaid_node(source.trim())?;
                let (target_id, target_label) = parse_mermaid_node(target.trim())?;
                nodes.entry(source_id.clone()).or_insert(source_label);
                nodes.entry(target_id.clone()).or_insert(target_label);
                edges.push((source_id, target_id));
            } else {
                let (id, label) = parse_mermaid_node(line)?;
                nodes.entry(id).or_insert(label);
            }
            if nodes.len() > Self::MAX_NODES {
                return Err(());
            }
        }
        if nodes.is_empty() {
            return Err(());
        }
        Ok(Self {
            direction,
            nodes,
            edges,
        })
    }

    fn into_view<'a>(self) -> Element<'a, Action> {
        let node = |label: String| container(text(label)).padding(8);
        if self.edges.is_empty() {
            return column(self.nodes.into_values().map(|label| node(label).into()))
                .spacing(8)
                .into();
        }
        let direction = self.direction;
        column(self.edges.into_iter().map(move |(source, target)| {
            let source = self.nodes.get(&source).cloned().unwrap_or(source);
            let target = self.nodes.get(&target).cloned().unwrap_or(target);
            match direction {
                Direction::LeftRight => row![node(source), text("→"), node(target)]
                    .spacing(8)
                    .into(),
                Direction::TopDown => column![node(source), text("↓"), node(target)]
                    .spacing(4)
                    .into(),
            }
        }))
        .spacing(10)
        .into()
    }
}

fn parse_mermaid_node(value: &str) -> Result<(String, String), ()> {
    let id_end = value
        .find(|character: char| {
            !character.is_ascii_alphanumeric() && character != '_' && character != '-'
        })
        .unwrap_or(value.len());
    let id = &value[..id_end];
    if id.is_empty() || id.len() > 32 {
        return Err(());
    }
    let remainder = value[id_end..].trim();
    let label = if remainder.is_empty() {
        id
    } else {
        let (opening, closing) = match (remainder.as_bytes().first(), remainder.as_bytes().last()) {
            (Some(b'['), Some(b']')) => ('[', ']'),
            (Some(b'('), Some(b')')) => ('(', ')'),
            (Some(b'{'), Some(b'}')) => ('{', '}'),
            _ => return Err(()),
        };
        remainder
            .strip_prefix(opening)
            .and_then(|label| label.strip_suffix(closing))
            .ok_or(())?
            .trim()
    };
    if label.is_empty()
        || label.chars().count() > 80
        || label.contains(['[', ']', '{', '}', '<', '>'])
    {
        return Err(());
    }
    Ok((id.to_owned(), label.to_owned()))
}

fn is_unsafe_mermaid(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    line.contains('<')
        || lower.starts_with("click ")
        || lower.starts_with("style ")
        || lower.starts_with("classdef ")
        || lower.contains("http:")
        || lower.contains("https:")
        || lower.contains("%%{")
        || lower.contains(":::")
}

fn media_type(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("md") => "text/markdown",
        Some("txt" | "rs" | "toml" | "json" | "yaml" | "yml") => "text/plain",
        _ => "application/octet-stream",
    }
}

fn is_previewable_image(media_type: &str) -> bool {
    matches!(
        media_type,
        "image/png" | "image/jpeg" | "image/gif" | "image/webp"
    )
}

fn parse_thread_color(value: &str) -> Color {
    let component =
        |range: std::ops::Range<usize>| u8::from_str_radix(&value[range], 16).unwrap_or_default();
    Color::from_rgb8(component(1..3), component(3..5), component(5..7))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn blocks_unsafe_links_and_workspace_file_escapes() {
        let temp = TempDir::new().unwrap();
        let root = temp.path().join("workspace");
        fs::create_dir(&root).unwrap();
        fs::write(root.join("safe.rs"), "fn main() {}").unwrap();

        assert!(matches!(
            classify_link("javascript:alert(1)", &root),
            LinkTarget::Blocked
        ));
        assert!(matches!(
            classify_link("file:../outside", &root),
            LinkTarget::Blocked
        ));
        assert!(matches!(
            classify_link("file:safe.rs#L1", &root),
            LinkTarget::WorkspaceFile(_)
        ));
    }

    #[test]
    fn parses_only_the_bounded_safe_mermaid_subset() {
        let diagram = Diagram::parse("flowchart LR\nA[Draft] --> B[Sent]").unwrap();
        assert_eq!(diagram.direction, Direction::LeftRight);
        assert_eq!(diagram.nodes.len(), 2);
        assert_eq!(diagram.edges, [("A".to_owned(), "B".to_owned())]);
        assert!(Diagram::parse("flowchart TD\nclick A https://example.com").is_err());
        assert!(Diagram::parse("sequenceDiagram\nA --> B").is_err());
    }

    #[test]
    fn native_markdown_drops_raw_html_content() {
        let content = parse("<script>alert('unsafe')</script>\n\n**Safe text**");
        let parsed = format!("{:?}", content.items());
        assert!(!parsed.contains("unsafe"));
        assert!(parsed.contains("Safe text"));
    }

    #[test]
    fn imports_files_locally_and_rejects_oversized_attachments() {
        let temp = TempDir::new().unwrap();
        let source = temp.path().join("notes.md");
        fs::write(&source, "local only").unwrap();
        let store = AttachmentStore::new(temp.path().join("attachments"));
        let attachment = store
            .import(
                &source,
                WorkspaceId::new(1),
                ChatThreadId::new(2),
                ChatAttachmentId::new(3),
            )
            .unwrap();
        let stored = temp
            .path()
            .join("attachments/1/by-thread/2")
            .join(attachment.storage_key());
        assert_eq!(fs::read_to_string(stored).unwrap(), "local only");
        assert_eq!(fs::read_to_string(source).unwrap(), "local only");

        let oversized = temp.path().join("large.bin");
        fs::File::create(&oversized)
            .unwrap()
            .set_len(ChatAttachment::MAX_BYTES + 1)
            .unwrap();
        assert!(matches!(
            store.import(
                &oversized,
                WorkspaceId::new(1),
                ChatThreadId::new(2),
                ChatAttachmentId::new(4),
            ),
            Err(AttachmentError::TooLarge { .. })
        ));
    }
}
