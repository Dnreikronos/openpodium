use std::collections::{BTreeMap, BTreeSet};

use iced::keyboard::{Key, Modifiers, key::Named};
use iced::widget::{button, column, container, row, scrollable, text, text_input};
use iced::{Element, Fill};
use openpodium::domain::{TimelineEventId, WorkspaceId};
use openpodium::navigation::{
    CommandId, CommandRegistry, Platform, SearchDocument, SearchIndex, SearchResult, Shortcut,
};

const MAX_RESULTS: usize = 30;
pub const INPUT_ID: &str = "navigation-palette-input";

#[derive(Debug, Clone)]
pub enum Message {
    Open,
    Close,
    QueryChanged(String),
    MoveSelection(bool),
    ActivateSelection,
    Activate(Item),
    BeginRebind(CommandId),
    BindingChanged(String),
    SaveBinding,
    Unbind,
    IndexBuilt {
        workspace_id: WorkspaceId,
        revision: Option<TimelineEventId>,
        documents: Result<Vec<SearchDocument>, String>,
    },
    SearchCompleted {
        generation: u64,
        results: Result<Vec<SearchResult>, String>,
    },
}

#[derive(Debug, Clone)]
pub enum Item {
    Search(SearchResult),
    Command(CommandId),
}

#[derive(Default)]
pub struct UiState {
    pub open: bool,
    pub query: String,
    pub selected: usize,
    pub index: SearchIndex,
    pub stale: BTreeSet<WorkspaceId>,
    pub busy: Option<WorkspaceId>,
    pub indexed_revisions: BTreeMap<WorkspaceId, Option<TimelineEventId>>,
    pub refresh_ticks: u16,
    pub rebinding: Option<CommandId>,
    pub binding_draft: String,
    pub search_results: Vec<SearchResult>,
    pub search_generation: u64,
}

impl UiState {
    pub fn mark_stale(&mut self, workspace_id: WorkspaceId) {
        self.stale.insert(workspace_id);
    }

    pub fn move_selection(&mut self, forward: bool, item_count: usize) {
        if item_count == 0 {
            self.selected = 0;
        } else if forward {
            self.selected = (self.selected + 1) % item_count;
        } else if self.selected == 0 {
            self.selected = item_count - 1;
        } else {
            self.selected -= 1;
        }
    }
}

pub fn items(state: &UiState, commands: &CommandRegistry) -> Vec<Item> {
    if let Some(query) = state.query.strip_prefix('>') {
        let query = query.trim();
        commands
            .commands()
            .filter(|command| fuzzy_contains(command.label, query))
            .take(MAX_RESULTS)
            .map(|command| Item::Command(command.id))
            .collect()
    } else {
        state
            .search_results
            .iter()
            .cloned()
            .map(Item::Search)
            .collect()
    }
}

pub fn shortcut_from_key(key: &Key, modifiers: Modifiers) -> Option<Shortcut> {
    let (key, shift) = match key.as_ref() {
        Key::Character("+") => ("plus".to_owned(), false),
        Key::Character("-") => ("minus".to_owned(), false),
        Key::Character(value) if value.chars().count() == 1 => {
            (value.to_string(), modifiers.shift())
        }
        Key::Named(Named::ArrowLeft) => ("arrowleft".to_owned(), modifiers.shift()),
        Key::Named(Named::ArrowRight) => ("arrowright".to_owned(), modifiers.shift()),
        Key::Named(Named::ArrowUp) => ("arrowup".to_owned(), modifiers.shift()),
        Key::Named(Named::Backspace) => ("backspace".to_owned(), modifiers.shift()),
        Key::Named(Named::Delete) => ("delete".to_owned(), modifiers.shift()),
        Key::Named(Named::Enter) => ("enter".to_owned(), modifiers.shift()),
        Key::Named(Named::Escape) => ("escape".to_owned(), modifiers.shift()),
        Key::Named(Named::Tab) => ("tab".to_owned(), modifiers.shift()),
        Key::Named(Named::Space) => ("space".to_owned(), modifiers.shift()),
        _ => return None,
    };
    Shortcut::new(
        if cfg!(target_os = "macos") {
            modifiers.command()
        } else {
            modifiers.control()
        },
        shift,
        modifiers.alt(),
        key,
    )
    .ok()
}

pub fn view<'a>(state: &'a UiState, commands: &'a CommandRegistry) -> Element<'a, Message> {
    let current_items = items(state, commands);
    let mut results = column![].spacing(4);
    if current_items.is_empty() {
        results = results.push(text(if state.query.is_empty() {
            "Type to search, or enter > to discover commands"
        } else {
            "No matching results"
        }));
    }
    for (index, item) in current_items.into_iter().enumerate() {
        let selected = index == state.selected;
        let (label, detail) = match &item {
            Item::Search(result) => (
                format!(
                    "{}{} · {}",
                    if selected { "› " } else { "" },
                    result.document.title,
                    result.document.kind.label()
                ),
                result.document.detail.clone(),
            ),
            Item::Command(id) => {
                let shortcut = commands.binding(*id).map_or_else(
                    || "Unbound".to_owned(),
                    |value| value.display(Platform::current()),
                );
                (
                    format!("{}{}", if selected { "› " } else { "" }, id.label()),
                    shortcut,
                )
            }
        };
        let mut item_row = row![
            button(column![text(label), text(detail).size(12)])
                .on_press(Message::Activate(item.clone()))
                .width(Fill)
        ]
        .spacing(8);
        if let Item::Command(id) = item {
            item_row = item_row.push(button("Rebind").on_press(Message::BeginRebind(id)));
        }
        results = results.push(item_row);
    }

    let mut palette = column![
        row![
            text_input(
                "Search workspaces, agents, tasks, messages, notes, and paths",
                &state.query
            )
            .id(INPUT_ID)
            .on_input(Message::QueryChanged)
            .width(Fill),
            button("Close").on_press(Message::Close),
        ]
        .spacing(8),
        scrollable(results).height(320),
    ]
    .spacing(10);
    if let Some(command) = state.rebinding {
        palette = palette.push(
            row![
                text(format!("Rebind {}", command.label())),
                text_input("Primary+Shift+key", &state.binding_draft)
                    .on_input(Message::BindingChanged),
                button("Save").on_press(Message::SaveBinding),
                button("Unbind").on_press(Message::Unbind),
            ]
            .spacing(8),
        );
    }
    container(palette)
        .width(Fill)
        .padding(16)
        .style(container::rounded_box)
        .into()
}

fn fuzzy_contains(value: &str, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    let mut query = query.chars().flat_map(char::to_lowercase);
    let mut wanted = query.next();
    for candidate in value.chars().flat_map(char::to_lowercase) {
        if wanted == Some(candidate) {
            wanted = query.next();
            if wanted.is_none() {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_mode_discovers_actions_with_fuzzy_text() {
        let mut state = UiState {
            query: "> nxtw".to_owned(),
            ..UiState::default()
        };
        let commands = CommandRegistry::new();
        let found = items(&state, &commands);
        assert!(
            found
                .iter()
                .any(|item| matches!(item, Item::Command(CommandId::NextWorkspace)))
        );

        state.query = ">".to_owned();
        assert_eq!(items(&state, &commands).len(), CommandId::ALL.len());
    }
}
