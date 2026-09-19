use std::error::Error;
use std::fmt::{self, Display, Formatter};

use super::{
    AgentId, ChatAttachmentId, ChatMessageId, ChatThreadId, Content, Name, NodeTarget, Timestamp,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadColor(String);

impl ThreadColor {
    pub const DEFAULT: &'static str = "#2563EB";

    pub fn new(value: impl Into<String>) -> Result<Self, ChatValidationError> {
        let value = value.into();
        let valid = value.len() == 7
            && value.starts_with('#')
            && value[1..].bytes().all(|byte| byte.is_ascii_hexdigit());
        if !valid {
            return Err(ChatValidationError::InvalidColor);
        }
        Ok(Self(value.to_ascii_uppercase()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for ThreadColor {
    fn default() -> Self {
        Self(Self::DEFAULT.to_owned())
    }
}

impl Display for ThreadColor {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatAuthor {
    User,
    Agent(AgentId),
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ChatDraft {
    text: String,
    attachments: Vec<ChatAttachmentId>,
    mentions: Vec<NodeTarget>,
}

impl ChatDraft {
    pub const MAX_ATTACHMENTS: usize = 8;
    pub const MAX_TOTAL_ATTACHMENT_BYTES: u64 = 25 * 1024 * 1024;

    pub fn new(
        text: impl Into<String>,
        attachments: Vec<ChatAttachmentId>,
        mentions: Vec<NodeTarget>,
    ) -> Result<Self, ChatValidationError> {
        let text = text.into();
        if text.chars().count() > Content::MAX_CHARS {
            return Err(ChatValidationError::DraftTooLong {
                max_chars: Content::MAX_CHARS,
            });
        }
        if attachments.len() > Self::MAX_ATTACHMENTS {
            return Err(ChatValidationError::TooManyAttachments {
                max: Self::MAX_ATTACHMENTS,
            });
        }
        if has_duplicates(&attachments) {
            return Err(ChatValidationError::DuplicateAttachment);
        }
        if has_duplicates(&mentions) {
            return Err(ChatValidationError::DuplicateMention);
        }
        Ok(Self {
            text,
            attachments,
            mentions,
        })
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn attachments(&self) -> &[ChatAttachmentId] {
        &self.attachments
    }

    pub fn mentions(&self) -> &[NodeTarget] {
        &self.mentions
    }

    pub fn is_empty(&self) -> bool {
        self.text.trim().is_empty() && self.attachments.is_empty() && self.mentions.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatAttachment {
    id: ChatAttachmentId,
    thread_id: ChatThreadId,
    storage_key: String,
    display_name: String,
    media_type: String,
    byte_len: u64,
}

impl ChatAttachment {
    pub const MAX_BYTES: u64 = 10 * 1024 * 1024;
    pub const MAX_DISPLAY_NAME_CHARS: usize = 255;
    pub const MAX_MEDIA_TYPE_CHARS: usize = 127;

    pub fn new(
        id: ChatAttachmentId,
        thread_id: ChatThreadId,
        storage_key: impl Into<String>,
        display_name: impl Into<String>,
        media_type: impl Into<String>,
        byte_len: u64,
    ) -> Result<Self, ChatValidationError> {
        let storage_key = storage_key.into();
        let display_name = display_name.into();
        let media_type = media_type.into();
        if storage_key.is_empty()
            || matches!(storage_key.as_str(), "." | "..")
            || !storage_key
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        {
            return Err(ChatValidationError::InvalidStorageKey);
        }
        if display_name.trim().is_empty()
            || display_name.chars().count() > Self::MAX_DISPLAY_NAME_CHARS
            || display_name.contains('/')
            || display_name.contains('\\')
        {
            return Err(ChatValidationError::InvalidDisplayName);
        }
        if media_type.trim().is_empty()
            || media_type.chars().count() > Self::MAX_MEDIA_TYPE_CHARS
            || !media_type.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'-' | b'+' | b'.')
            })
        {
            return Err(ChatValidationError::InvalidMediaType);
        }
        if byte_len > Self::MAX_BYTES {
            return Err(ChatValidationError::AttachmentTooLarge {
                max_bytes: Self::MAX_BYTES,
                actual_bytes: byte_len,
            });
        }
        Ok(Self {
            id,
            thread_id,
            storage_key,
            display_name,
            media_type,
            byte_len,
        })
    }

    pub const fn id(&self) -> ChatAttachmentId {
        self.id
    }

    pub const fn thread_id(&self) -> ChatThreadId {
        self.thread_id
    }

    pub fn storage_key(&self) -> &str {
        &self.storage_key
    }

    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    pub fn media_type(&self) -> &str {
        &self.media_type
    }

    pub const fn byte_len(&self) -> u64 {
        self.byte_len
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatMessage {
    id: ChatMessageId,
    thread_id: ChatThreadId,
    author: ChatAuthor,
    content: Content,
    attachments: Vec<ChatAttachmentId>,
    mentions: Vec<NodeTarget>,
    sent_at: Timestamp,
}

impl ChatMessage {
    pub fn new(
        id: ChatMessageId,
        thread_id: ChatThreadId,
        author: ChatAuthor,
        content: Content,
        attachments: Vec<ChatAttachmentId>,
        mentions: Vec<NodeTarget>,
        sent_at: Timestamp,
    ) -> Result<Self, ChatValidationError> {
        if attachments.len() > ChatDraft::MAX_ATTACHMENTS {
            return Err(ChatValidationError::TooManyAttachments {
                max: ChatDraft::MAX_ATTACHMENTS,
            });
        }
        if has_duplicates(&attachments) {
            return Err(ChatValidationError::DuplicateAttachment);
        }
        if has_duplicates(&mentions) {
            return Err(ChatValidationError::DuplicateMention);
        }
        Ok(Self {
            id,
            thread_id,
            author,
            content,
            attachments,
            mentions,
            sent_at,
        })
    }

    pub(super) fn from_draft(
        id: ChatMessageId,
        thread_id: ChatThreadId,
        draft: &ChatDraft,
        sent_at: Timestamp,
    ) -> Result<Self, ChatValidationError> {
        Self::new(
            id,
            thread_id,
            ChatAuthor::User,
            Content::new(draft.text.clone()).map_err(|_| ChatValidationError::EmptyMessage)?,
            draft.attachments.clone(),
            draft.mentions.clone(),
            sent_at,
        )
    }

    pub const fn id(&self) -> ChatMessageId {
        self.id
    }

    pub const fn thread_id(&self) -> ChatThreadId {
        self.thread_id
    }

    pub const fn author(&self) -> ChatAuthor {
        self.author
    }

    pub const fn content(&self) -> &Content {
        &self.content
    }

    pub fn attachments(&self) -> &[ChatAttachmentId] {
        &self.attachments
    }

    pub fn mentions(&self) -> &[NodeTarget] {
        &self.mentions
    }

    pub const fn sent_at(&self) -> Timestamp {
        self.sent_at
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatThread {
    id: ChatThreadId,
    agent_id: AgentId,
    name: Name,
    color: ThreadColor,
    draft: ChatDraft,
    messages: Vec<ChatMessage>,
}

impl ChatThread {
    pub fn new(id: ChatThreadId, agent_id: AgentId, name: Name) -> Self {
        Self {
            id,
            agent_id,
            name,
            color: ThreadColor::default(),
            draft: ChatDraft::default(),
            messages: Vec::new(),
        }
    }

    pub fn with_color(id: ChatThreadId, agent_id: AgentId, name: Name, color: ThreadColor) -> Self {
        Self {
            color,
            ..Self::new(id, agent_id, name)
        }
    }

    pub const fn id(&self) -> ChatThreadId {
        self.id
    }

    pub const fn agent_id(&self) -> AgentId {
        self.agent_id
    }

    pub const fn name(&self) -> &Name {
        &self.name
    }

    pub const fn color(&self) -> &ThreadColor {
        &self.color
    }

    pub const fn draft(&self) -> &ChatDraft {
        &self.draft
    }

    pub fn messages(&self) -> &[ChatMessage] {
        &self.messages
    }

    pub(super) fn set_appearance(&mut self, name: Name, color: ThreadColor) {
        self.name = name;
        self.color = color;
    }

    pub(super) fn set_draft(&mut self, draft: ChatDraft) {
        self.draft = draft;
    }

    pub(super) fn push_message(&mut self, message: ChatMessage) {
        self.messages.push(message);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChatValidationError {
    InvalidColor,
    DraftTooLong { max_chars: usize },
    TooManyAttachments { max: usize },
    DuplicateAttachment,
    DuplicateMention,
    InvalidStorageKey,
    InvalidDisplayName,
    InvalidMediaType,
    AttachmentTooLarge { max_bytes: u64, actual_bytes: u64 },
    EmptyMessage,
}

impl Display for ChatValidationError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidColor => formatter.write_str("thread color must use #RRGGBB format"),
            Self::DraftTooLong { max_chars } => {
                write!(formatter, "draft cannot exceed {max_chars} characters")
            }
            Self::TooManyAttachments { max } => {
                write!(
                    formatter,
                    "a draft cannot contain more than {max} attachments"
                )
            }
            Self::DuplicateAttachment => formatter.write_str("an attachment can appear only once"),
            Self::DuplicateMention => formatter.write_str("a mention can appear only once"),
            Self::InvalidStorageKey => formatter.write_str("attachment storage key is invalid"),
            Self::InvalidDisplayName => formatter.write_str("attachment display name is invalid"),
            Self::InvalidMediaType => formatter.write_str("attachment media type is invalid"),
            Self::AttachmentTooLarge {
                max_bytes,
                actual_bytes,
            } => write!(
                formatter,
                "attachment is {actual_bytes} bytes; the limit is {max_bytes} bytes"
            ),
            Self::EmptyMessage => formatter.write_str("a message cannot be empty"),
        }
    }
}

impl Error for ChatValidationError {}

fn has_duplicates<T: PartialEq>(values: &[T]) -> bool {
    values
        .iter()
        .enumerate()
        .any(|(index, value)| values[..index].contains(value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{
        Agent, CanvasPoint, CanvasSize, Connection, ConnectionId, ConnectionKind, DomainCommand,
        Name, Node, NodeId, Workspace, WorkspaceId,
    };

    #[test]
    fn validates_colors_and_attachment_metadata() {
        assert_eq!(
            ThreadColor::new("#a1b2c3").expect("valid color").as_str(),
            "#A1B2C3"
        );
        assert!(ThreadColor::new("red").is_err());
        assert!(
            ChatAttachment::new(
                ChatAttachmentId::new(1),
                ChatThreadId::new(1),
                "safe-key.png",
                "../unsafe.png",
                "image/png",
                10,
            )
            .is_err()
        );
        assert!(
            ChatAttachment::new(
                ChatAttachmentId::new(1),
                ChatThreadId::new(1),
                "..",
                "safe.png",
                "image/png",
                10,
            )
            .is_err()
        );
    }

    #[test]
    fn rejects_duplicate_draft_references() {
        let attachment = ChatAttachmentId::new(1);
        assert!(ChatDraft::new("hello", vec![attachment, attachment], Vec::new()).is_err());
    }

    #[test]
    fn submitting_a_draft_adds_one_message_and_clears_it_atomically() {
        let mut workspace = workspace_with_agent();
        let thread_id = ChatThreadId::new(1);
        workspace
            .execute(DomainCommand::AddChatThread(ChatThread::new(
                thread_id,
                AgentId::new(1),
                Name::new("Implementation").expect("valid name"),
            )))
            .expect("thread is valid");
        workspace
            .execute(DomainCommand::UpdateChatDraft {
                thread_id,
                draft: ChatDraft::new("Please implement this", Vec::new(), Vec::new())
                    .expect("valid draft"),
            })
            .expect("draft is valid");

        workspace
            .execute(DomainCommand::SubmitChatDraft {
                thread_id,
                message_id: ChatMessageId::new(1),
                sent_at: Timestamp::from_unix_millis(42),
            })
            .expect("draft can be submitted");

        let thread = workspace.chat_thread(thread_id).expect("thread exists");
        assert!(thread.draft().is_empty());
        assert_eq!(thread.messages().len(), 1);
        assert_eq!(
            thread.messages()[0].content().as_str(),
            "Please implement this"
        );
        assert!(
            !ChatDraft::new("", Vec::new(), vec![NodeTarget::Agent(AgentId::new(1))])
                .expect("valid mention")
                .is_empty()
        );
    }

    #[test]
    fn mentions_require_a_direct_canvas_connection() {
        let mut workspace = workspace_with_agent();
        let other_id = AgentId::new(2);
        workspace
            .execute(DomainCommand::AddAgent(Agent::new(
                other_id,
                Name::new("Reviewer").expect("valid name"),
                None,
            )))
            .expect("agent is valid");
        let thread_id = ChatThreadId::new(1);
        workspace
            .execute(DomainCommand::AddChatThread(ChatThread::new(
                thread_id,
                AgentId::new(1),
                Name::new("Review").expect("valid name"),
            )))
            .expect("thread is valid");
        let mention = NodeTarget::Agent(other_id);
        let draft =
            ChatDraft::new("Please review", Vec::new(), vec![mention]).expect("valid draft shape");
        assert!(
            workspace
                .execute(DomainCommand::UpdateChatDraft {
                    thread_id,
                    draft: draft.clone(),
                })
                .is_err()
        );

        let position = CanvasPoint::new(0.0, 0.0).expect("valid position");
        let size = CanvasSize::new(400.0, 300.0).expect("valid size");
        let owner_node = Node::new(
            NodeId::new(1),
            NodeTarget::Agent(AgentId::new(1)),
            position,
            size,
        );
        let other_node = Node::new(NodeId::new(2), mention, position, size);
        workspace
            .execute(DomainCommand::AddNode(owner_node))
            .expect("owner node is valid");
        workspace
            .execute(DomainCommand::AddNode(other_node))
            .expect("other node is valid");
        let before = workspace.canvas_layout();
        let after = crate::domain::CanvasLayout::new(
            before.nodes().to_vec(),
            Vec::new(),
            vec![Connection::new(
                ConnectionId::new(1),
                NodeId::new(1),
                NodeId::new(2),
                ConnectionKind::Coordination,
            )],
        );
        workspace
            .execute(DomainCommand::ReplaceCanvas { before, after })
            .expect("connection is valid");

        workspace
            .execute(DomainCommand::UpdateChatDraft { thread_id, draft })
            .expect("connected mention is valid");
    }

    fn workspace_with_agent() -> Workspace {
        let mut workspace = Workspace::new(
            WorkspaceId::new(1),
            Name::new("Workspace").expect("valid name"),
        );
        workspace
            .execute(DomainCommand::AddAgent(Agent::new(
                AgentId::new(1),
                Name::new("Builder").expect("valid name"),
                None,
            )))
            .expect("agent is valid");
        workspace
    }
}
