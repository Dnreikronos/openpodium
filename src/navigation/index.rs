use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use crate::context::{FilePreview, MAX_TEXT_PREVIEW_BYTES, preview_file};
use crate::domain::{
    AgentId, CanvasNodeContent, ChatMessageId, ChatThreadId, Node, NodeId, NodeTarget, ProjectPath,
    TaskId, Workspace, WorkspaceId,
};
use crate::git::project_paths;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SearchKind {
    Workspace,
    Agent,
    Task,
    ChatMessage,
    Note,
    ProjectPath,
    Text,
}

impl SearchKind {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Workspace => "Workspace",
            Self::Agent => "Agent",
            Self::Task => "Task",
            Self::ChatMessage => "Message",
            Self::Note => "Note",
            Self::ProjectPath => "Path",
            Self::Text => "Text",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContentTarget {
    Task(TaskId),
    ProjectPath(ProjectPath),
    Chat {
        agent_id: AgentId,
        thread_id: ChatThreadId,
        message_id: ChatMessageId,
        character_offset: usize,
    },
    Note {
        character_offset: usize,
    },
    Text {
        character_offset: usize,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchTarget {
    pub workspace_id: WorkspaceId,
    pub floor_id: Option<u64>,
    pub node_id: Option<NodeId>,
    pub content: Option<ContentTarget>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchDocument {
    pub key: String,
    pub kind: SearchKind,
    pub title: String,
    pub detail: String,
    pub body: String,
    pub target: SearchTarget,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchResult {
    pub document: SearchDocument,
    pub score: i64,
    pub character_offset: usize,
}

#[derive(Debug, Clone, Default)]
pub struct SearchIndex {
    workspaces: BTreeMap<WorkspaceId, Arc<Vec<SearchDocument>>>,
}

impl SearchIndex {
    pub fn replace_workspace(&mut self, workspace_id: WorkspaceId, documents: Vec<SearchDocument>) {
        self.workspaces.insert(workspace_id, Arc::new(documents));
    }

    pub fn remove_workspace(&mut self, workspace_id: WorkspaceId) {
        self.workspaces.remove(&workspace_id);
    }

    pub fn documents_for(&self, workspace_id: WorkspaceId) -> &[SearchDocument] {
        self.workspaces
            .get(&workspace_id)
            .map_or(&[], |documents| documents.as_slice())
    }

    pub fn search(&self, query: &str, limit: usize) -> Vec<SearchResult> {
        let query = query.trim();
        if query.is_empty() || limit == 0 {
            return Vec::new();
        }
        let mut results = self
            .workspaces
            .values()
            .flat_map(|documents| documents.iter())
            .filter_map(|document| match_document(document, query))
            .collect::<Vec<_>>();
        results.sort_by(|left, right| {
            right
                .score
                .cmp(&left.score)
                .then_with(|| left.document.key.cmp(&right.document.key))
        });
        results.truncate(limit);
        results
    }
}

pub fn index_workspace(workspace: &Workspace) -> Vec<SearchDocument> {
    let workspace_id = workspace.id();
    let workspace_detail = workspace
        .settings()
        .working_directory()
        .map_or_else(String::new, |directory| directory.as_str().to_owned());
    let mut documents = vec![SearchDocument {
        key: format!("workspace:{}", workspace_id.get()),
        kind: SearchKind::Workspace,
        title: workspace.name().to_owned(),
        detail: workspace_detail.clone(),
        body: workspace_detail,
        target: SearchTarget {
            workspace_id,
            floor_id: None,
            node_id: None,
            content: None,
        },
    }];

    let layout = workspace.all_canvas_layout();
    for agent in workspace.agents() {
        let node = node_for_target(&layout, NodeTarget::Agent(agent.id()));
        documents.push(SearchDocument {
            key: format!("agent:{}:{}", workspace_id.get(), agent.id().get()),
            kind: SearchKind::Agent,
            title: agent.name().to_string(),
            detail: format!("{} · {}", workspace.name(), agent.state()),
            body: agent.program().label().to_owned(),
            target: target_for_node(workspace, node, None),
        });
    }

    for task in workspace.tasks() {
        let node = node_for_target(&layout, NodeTarget::Task(task.id())).or_else(|| {
            task.assignee()
                .and_then(|agent| node_for_target(&layout, NodeTarget::Agent(agent)))
        });
        documents.push(SearchDocument {
            key: format!("task:{}:{}", workspace_id.get(), task.id().get()),
            kind: SearchKind::Task,
            title: task.title().to_string(),
            detail: format!("{} · {}", workspace.name(), task.state()),
            body: task.prompt().as_str().to_owned(),
            target: target_for_node(workspace, node, Some(ContentTarget::Task(task.id()))),
        });
    }

    for thread in workspace.chat_threads() {
        let node = node_for_target(&layout, NodeTarget::Agent(thread.agent_id()));
        for message in thread.messages() {
            documents.push(SearchDocument {
                key: format!(
                    "message:{}:{}:{}",
                    workspace_id.get(),
                    thread.id().get(),
                    message.id().get()
                ),
                kind: SearchKind::ChatMessage,
                title: thread.name().to_string(),
                detail: workspace.name().to_owned(),
                body: message.content().as_str().to_owned(),
                target: target_for_node(
                    workspace,
                    node,
                    Some(ContentTarget::Chat {
                        agent_id: thread.agent_id(),
                        thread_id: thread.id(),
                        message_id: message.id(),
                        character_offset: 0,
                    }),
                ),
            });
        }
    }

    for node in layout.nodes() {
        index_node(workspace, node, &mut documents);
    }
    if let Some(directory) = workspace.settings().working_directory() {
        index_project_paths(
            workspace,
            None,
            &PathBuf::from(directory.as_str()),
            &mut documents,
        );
    }
    for (floor_id, floor) in &workspace.floors().entries {
        if floor.lifecycle == crate::domain::FloorLifecycle::Available {
            index_project_paths(
                workspace,
                Some(*floor_id),
                &PathBuf::from(floor.directory.as_str()),
                &mut documents,
            );
        }
    }
    documents
}

fn index_project_paths(
    workspace: &Workspace,
    floor_id: Option<u64>,
    checkout: &std::path::Path,
    documents: &mut Vec<SearchDocument>,
) {
    let Ok(paths) = project_paths(checkout) else {
        return;
    };
    let floor_key = floor_id.map_or_else(|| "main".to_owned(), |id| id.to_string());
    let floor_label = floor_id
        .and_then(|id| workspace.floors().entries.get(&id))
        .map_or("main", |floor| floor.name.as_str());
    documents.extend(paths.into_iter().map(|path| SearchDocument {
        key: format!(
            "project-path:{}:{floor_key}:{}",
            workspace.id().get(),
            path.as_str()
        ),
        kind: SearchKind::ProjectPath,
        title: path.as_str().to_owned(),
        detail: format!("{} · {floor_label}", workspace.name()),
        body: String::new(),
        target: SearchTarget {
            workspace_id: workspace.id(),
            floor_id,
            node_id: None,
            content: Some(ContentTarget::ProjectPath(path.clone())),
        },
    }));
}

fn index_node(workspace: &Workspace, node: &Node, documents: &mut Vec<SearchDocument>) {
    let workspace_id = workspace.id();
    let base_target = target_for_node(workspace, Some(node.id()), None);
    match node.content() {
        CanvasNodeContent::Note { path, title } => {
            documents.push(path_document(workspace, node, path));
            let body = note_body(workspace, node.id(), path);
            documents.push(SearchDocument {
                key: format!("note:{}:{}", workspace_id.get(), node.id().get()),
                kind: SearchKind::Note,
                title: title.to_string(),
                detail: path.as_str().to_owned(),
                body,
                target: SearchTarget {
                    content: Some(ContentTarget::Note {
                        character_offset: 0,
                    }),
                    ..base_target
                },
            });
        }
        CanvasNodeContent::FileTree { root } => {
            documents.push(path_document(workspace, node, root))
        }
        CanvasNodeContent::Artifact { path } | CanvasNodeContent::Diff { path, .. } => {
            documents.push(path_document(workspace, node, path));
        }
        CanvasNodeContent::Text { markdown } => documents.push(SearchDocument {
            key: format!("text:{}:{}", workspace_id.get(), node.id().get()),
            kind: SearchKind::Text,
            title: "Canvas text".to_owned(),
            detail: workspace.name().to_owned(),
            body: markdown.as_str().to_owned(),
            target: SearchTarget {
                content: Some(ContentTarget::Text {
                    character_offset: 0,
                }),
                ..base_target
            },
        }),
        CanvasNodeContent::Reference(_)
        | CanvasNodeContent::Portal(_)
        | CanvasNodeContent::Shape(_)
        | CanvasNodeContent::Arrow(_)
        | CanvasNodeContent::Freehand(_) => {}
    }
}

fn path_document(workspace: &Workspace, node: &Node, path: &ProjectPath) -> SearchDocument {
    SearchDocument {
        key: format!(
            "path:{}:{}:{}",
            workspace.id().get(),
            node.id().get(),
            path.as_str()
        ),
        kind: SearchKind::ProjectPath,
        title: path.as_str().to_owned(),
        detail: workspace.name().to_owned(),
        body: String::new(),
        target: target_for_node(
            workspace,
            Some(node.id()),
            Some(ContentTarget::ProjectPath(path.clone())),
        ),
    }
}

fn note_body(workspace: &Workspace, node_id: NodeId, path: &ProjectPath) -> String {
    let Some(checkout) = workspace.node_directory(node_id) else {
        return String::new();
    };
    match preview_file(
        &PathBuf::from(checkout.as_str()),
        path,
        MAX_TEXT_PREVIEW_BYTES,
    ) {
        Ok(FilePreview::Text { content, .. }) => content,
        Ok(FilePreview::Missing | FilePreview::Binary { .. } | FilePreview::TooLarge { .. })
        | Err(_) => String::new(),
    }
}

fn node_for_target(layout: &crate::domain::CanvasLayout, target: NodeTarget) -> Option<NodeId> {
    layout
        .nodes()
        .iter()
        .find(|node| node.reference() == Some(target))
        .map(Node::id)
}

fn target_for_node(
    workspace: &Workspace,
    node_id: Option<NodeId>,
    content: Option<ContentTarget>,
) -> SearchTarget {
    SearchTarget {
        workspace_id: workspace.id(),
        floor_id: node_id.and_then(|node| workspace.floors().node_floors.get(&node).copied()),
        node_id,
        content,
    }
}

fn match_document(document: &SearchDocument, query: &str) -> Option<SearchResult> {
    let title =
        fuzzy_match(&document.title, query).map(|matched| (matched.score + 1_000, 0, false));
    let detail =
        fuzzy_match(&document.detail, query).map(|matched| (matched.score + 250, 0, false));
    let body =
        fuzzy_match(&document.body, query).map(|matched| (matched.score, matched.offset, true));
    let (score, character_offset, body_match) = [title, detail, body]
        .into_iter()
        .flatten()
        .max_by_key(|value| value.0)?;
    let mut document = document.clone();
    if body_match && let Some(content) = document.target.content.as_mut() {
        match content {
            ContentTarget::Chat {
                character_offset: offset,
                ..
            }
            | ContentTarget::Note {
                character_offset: offset,
            }
            | ContentTarget::Text {
                character_offset: offset,
            } => *offset = character_offset,
            ContentTarget::Task(_) | ContentTarget::ProjectPath(_) => {}
        }
    }
    Some(SearchResult {
        document,
        score,
        character_offset,
    })
}

#[derive(Debug, Clone, Copy)]
struct FuzzyMatch {
    score: i64,
    offset: usize,
}

fn fuzzy_match(value: &str, query: &str) -> Option<FuzzyMatch> {
    let haystack: Vec<(char, usize)> = value
        .chars()
        .enumerate()
        .flat_map(|(offset, character)| {
            character
                .to_lowercase()
                .map(move |normalized| (normalized, offset))
        })
        .collect();
    let needle: Vec<char> = query.chars().flat_map(char::to_lowercase).collect();
    if needle.is_empty() {
        return None;
    }
    let mut positions = Vec::with_capacity(needle.len());
    let mut cursor = 0;
    for wanted in needle {
        let relative = haystack[cursor..]
            .iter()
            .position(|(candidate, _)| *candidate == wanted)?;
        let position = cursor + relative;
        positions.push(position);
        cursor = position + 1;
    }
    let first = positions[0];
    let consecutive = positions
        .windows(2)
        .filter(|pair| pair[1] == pair[0] + 1)
        .count() as i64;
    let boundary = positions
        .iter()
        .filter(|position| {
            **position == 0 || !haystack[position.saturating_sub(1)].0.is_alphanumeric()
        })
        .count() as i64;
    let span = positions.last().copied().unwrap_or(first) - first + 1;
    Some(FuzzyMatch {
        score: consecutive * 40 + boundary * 25 - span as i64 - first as i64,
        offset: haystack[first].1,
    })
}

#[cfg(test)]
mod tests {
    use crate::domain::{Name, WorkspaceDirectory};

    use super::*;

    fn document(key: &str, title: &str) -> SearchDocument {
        SearchDocument {
            key: key.to_owned(),
            kind: SearchKind::Workspace,
            title: title.to_owned(),
            detail: String::new(),
            body: String::new(),
            target: SearchTarget {
                workspace_id: WorkspaceId::new(1),
                floor_id: None,
                node_id: None,
                content: None,
            },
        }
    }

    #[test]
    fn fuzzy_search_prefers_consecutive_word_boundary_matches_and_stable_keys() {
        let mut index = SearchIndex::default();
        index.replace_workspace(
            WorkspaceId::new(1),
            vec![
                document("b", "Command palette"),
                document("a", "Command post"),
                document("c", "Canvas mapping"),
            ],
        );
        let results = index.search("cmp", 10);
        assert_eq!(
            results
                .iter()
                .map(|result| result.document.key.as_str())
                .collect::<Vec<_>>(),
            vec!["a", "b", "c"]
        );
    }

    #[test]
    fn replacing_one_workspace_does_not_touch_another() {
        let mut index = SearchIndex::default();
        index.replace_workspace(WorkspaceId::new(1), vec![document("one", "One")]);
        index.replace_workspace(WorkspaceId::new(2), vec![document("two", "Two")]);
        index.replace_workspace(WorkspaceId::new(1), vec![document("new", "New")]);
        assert_eq!(index.documents_for(WorkspaceId::new(1))[0].key, "new");
        assert_eq!(index.documents_for(WorkspaceId::new(2))[0].key, "two");
    }

    #[test]
    fn workspace_index_contains_workspace_metadata() {
        let mut workspace = Workspace::new(WorkspaceId::new(7), Name::new("OpenPodium").unwrap());
        workspace
            .execute(crate::domain::DomainCommand::UpdateWorkspaceSettings(
                crate::domain::WorkspaceSettings::new(
                    Name::new("OpenPodium").unwrap(),
                    None,
                    Some(WorkspaceDirectory::new("/tmp/openpodium").unwrap()),
                    None,
                ),
            ))
            .unwrap();
        let documents = index_workspace(&workspace);
        assert_eq!(documents[0].title, "OpenPodium");
        assert_eq!(documents[0].body, "/tmp/openpodium");
    }

    #[test]
    fn content_matches_keep_unicode_character_offsets_but_title_matches_start_at_zero() {
        let mut message = document("message", "General thread");
        message.body = "Alpha Éclair".to_owned();
        message.target.content = Some(ContentTarget::Chat {
            agent_id: AgentId::new(1),
            thread_id: ChatThreadId::new(2),
            message_id: ChatMessageId::new(3),
            character_offset: 0,
        });
        let mut index = SearchIndex::default();
        index.replace_workspace(WorkspaceId::new(1), vec![message]);

        let body = index.search("écl", 1).pop().unwrap();
        assert!(matches!(
            body.document.target.content,
            Some(ContentTarget::Chat {
                character_offset: 6,
                ..
            })
        ));

        let title = index.search("thread", 1).pop().unwrap();
        assert!(matches!(
            title.document.target.content,
            Some(ContentTarget::Chat {
                character_offset: 0,
                ..
            })
        ));
    }
}
