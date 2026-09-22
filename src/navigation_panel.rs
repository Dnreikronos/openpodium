use std::collections::{BTreeMap, BTreeSet};

use iced::keyboard::{Key, Modifiers, key::Named};
use iced::widget::{button, column, container, row, scrollable, text, text_input};
use iced::{Element, Fill};
use openpodium::domain::{TimelineEventId, WorkspaceId};
use openpodium::navigation::{
    CommandId, CommandRegistry, Platform, SearchDocument, SearchIndex, SearchResult, Shortcut,
};

use crate::app::shell;

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

fn key_cap<'a>(label: impl text::IntoFragment<'a>) -> Element<'a, Message> {
    container(text(label).size(11))
        .style(shell::key_cap)
        .padding([2, 6])
        .into()
}

pub fn view<'a>(state: &'a UiState, commands: &'a CommandRegistry) -> Element<'a, Message> {
    let current_items = items(state, commands);
    let mut results = column![].spacing(2);
    if current_items.is_empty() {
        results = results.push(
            container(
                text(if state.query.is_empty() {
                    "Type to search, or enter > to discover commands"
                } else {
                    "No matching results"
                })
                .size(13)
                .style(shell::muted_text),
            )
            .padding([14, 10]),
        );
    }
    for (index, item) in current_items.into_iter().enumerate() {
        let selected = index == state.selected;
        // The highlighted row carries the selection now, so the title no
        // longer has to wear a "›" marker to say which one is active.
        let (title, kind, detail) = match &item {
            Item::Search(result) => (
                result.document.title.clone(),
                Some(result.document.kind.label().to_owned()),
                result.document.detail.clone(),
            ),
            Item::Command(id) => (
                id.label().to_owned(),
                None,
                commands.binding(*id).map_or_else(
                    || "Unbound".to_owned(),
                    |value| value.display(Platform::current()),
                ),
            ),
        };
        let mut heading = row![text(title).size(13)].spacing(6);
        if let Some(kind) = kind {
            heading = heading.push(text(kind).size(11).style(shell::subtle_text));
        }
        let entry: Element<'_, Message> = match item {
            Item::Command(id) => row![
                button(
                    row![
                        heading.width(Fill),
                        container(key_cap(detail)).align_y(iced::Alignment::Center),
                    ]
                    .spacing(8)
                    .align_y(iced::Alignment::Center),
                )
                .style(shell::navigation_button(selected))
                .padding([8, 10])
                .width(Fill)
                .on_press(Message::Activate(Item::Command(id))),
                button(text("Rebind").size(12))
                    .style(shell::utility_button)
                    .padding([6, 10])
                    .on_press(Message::BeginRebind(id)),
            ]
            .spacing(6)
            .into(),
            search => button(
                column![
                    heading,
                    text(detail)
                        .size(11)
                        .style(shell::muted_text)
                        .wrapping(iced::widget::text::Wrapping::None),
                ]
                .spacing(1),
            )
            .style(shell::navigation_button(selected))
            .padding([8, 10])
            .width(Fill)
            .on_press(Message::Activate(search))
            .into(),
        };
        results = results.push(entry);
    }

    let mut palette = column![
        row![
            text_input(
                "Search workspaces, agents, tasks, messages, notes, and paths",
                &state.query
            )
            .id(INPUT_ID)
            .size(15)
            .style(shell::search_input)
            .on_input(Message::QueryChanged)
            .width(Fill),
            button(key_cap("esc"))
                .style(shell::utility_button)
                .padding(2)
                .on_press(Message::Close),
        ]
        .spacing(10)
        .align_y(iced::Alignment::Center),
        container(iced::widget::Space::new().width(Fill).height(1))
            .style(shell::rule)
            .height(1),
        scrollable(results).height(340),
    ]
    .spacing(10);
    if let Some(command) = state.rebinding {
        palette = palette.push(
            column![
                text(format!("Rebind {}", command.label()))
                    .size(12)
                    .style(shell::muted_text),
                row![
                    text_input("Primary+Shift+key", &state.binding_draft)
                        .padding([8, 10])
                        .style(shell::input)
                        .on_input(Message::BindingChanged)
                        .width(Fill),
                    button(text("Save").size(13))
                        .style(shell::primary_button)
                        .padding([8, 14])
                        .on_press(Message::SaveBinding),
                    button(text("Unbind").size(13))
                        .style(shell::secondary_button)
                        .padding([8, 14])
                        .on_press(Message::Unbind),
                ]
                .spacing(6)
                .align_y(iced::Alignment::Center),
            ]
            .spacing(6),
        );
    }

    // A dimmed scrim plus a centered card, so the palette reads as a modal
    // over the workspace instead of a bar that shoves the canvas downward.
    let scrim = iced::widget::mouse_area(
        container(iced::widget::Space::new().width(Fill).height(Fill))
            .width(Fill)
            .height(Fill)
            .style(shell::scrim),
    )
    .on_press(Message::Close);
    let card = container(iced::widget::opaque(
        container(palette)
            .style(shell::card)
            .width(Fill)
            .max_width(680)
            .padding(16),
    ))
    .width(Fill)
    .height(Fill)
    .align_x(iced::Alignment::Center)
    .padding([84, 24]);
    iced::widget::stack![scrim, card].into()
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

    #[test]
    fn accessibility_shortcuts_use_the_logical_character_from_the_active_layout() {
        let shortcut = shortcut_from_key(&Key::Character("ж".into()), Modifiers::CTRL).unwrap();
        assert_eq!(shortcut.key(), "ж");
    }
}
