use std::collections::BTreeMap;
use std::error::Error;
use std::fmt::{self, Display, Formatter};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    MacOs,
    Other,
}

impl Platform {
    pub const fn current() -> Self {
        if cfg!(target_os = "macos") {
            Self::MacOs
        } else {
            Self::Other
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Shortcut {
    primary: bool,
    shift: bool,
    alt: bool,
    key: String,
}

impl Shortcut {
    pub fn new(
        primary: bool,
        shift: bool,
        alt: bool,
        key: impl Into<String>,
    ) -> Result<Self, ShortcutError> {
        let key = normalize_key(&key.into())?;
        Ok(Self {
            primary,
            shift,
            alt,
            key,
        })
    }

    pub const fn primary(&self) -> bool {
        self.primary
    }

    pub const fn shift(&self) -> bool {
        self.shift
    }

    pub const fn alt(&self) -> bool {
        self.alt
    }

    pub fn key(&self) -> &str {
        &self.key
    }

    pub fn parse(value: &str) -> Result<Self, ShortcutError> {
        let mut primary = false;
        let mut shift = false;
        let mut alt = false;
        let mut key = None;
        for part in value.split('+') {
            match part.trim().to_ascii_lowercase().as_str() {
                "primary" | "cmd" | "ctrl" => primary = true,
                "shift" => shift = true,
                "alt" | "option" => alt = true,
                "" => return Err(ShortcutError::Invalid),
                value if key.is_none() => key = Some(value.to_owned()),
                _ => return Err(ShortcutError::Invalid),
            }
        }
        Self::new(primary, shift, alt, key.ok_or(ShortcutError::Invalid)?)
    }

    pub fn storage_value(&self) -> String {
        let mut parts = Vec::new();
        if self.primary {
            parts.push("Primary");
        }
        if self.shift {
            parts.push("Shift");
        }
        if self.alt {
            parts.push("Alt");
        }
        parts.push(&self.key);
        parts.join("+")
    }

    pub fn display(&self, platform: Platform) -> String {
        let mut parts = Vec::new();
        if self.primary {
            parts.push(match platform {
                Platform::MacOs => "Cmd",
                Platform::Other => "Ctrl",
            });
        }
        if self.shift {
            parts.push("Shift");
        }
        if self.alt {
            parts.push(match platform {
                Platform::MacOs => "Option",
                Platform::Other => "Alt",
            });
        }
        parts.push(&self.key);
        parts.join("+")
    }
}

fn normalize_key(value: &str) -> Result<String, ShortcutError> {
    let value = value.trim().to_ascii_lowercase();
    let valid_named = matches!(
        value.as_str(),
        "arrowleft"
            | "arrowright"
            | "arrowup"
            | "arrowdown"
            | "backspace"
            | "delete"
            | "enter"
            | "escape"
            | "tab"
            | "space"
            | "plus"
            | "minus"
            | "0"
    );
    if value.chars().count() == 1 || valid_named {
        Ok(value)
    } else {
        Err(ShortcutError::Invalid)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShortcutError {
    Invalid,
}

impl Display for ShortcutError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter
            .write_str("shortcut must contain modifiers and one character or supported named key")
    }
}

impl Error for ShortcutError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CommandId {
    OpenPalette,
    NextWorkspace,
    PreviousWorkspace,
    NextAttention,
    PreviousAttention,
    NextNode,
    PreviousNode,
    NextConnection,
    PreviousConnection,
    PanLeft,
    PanRight,
    PanUp,
    PanDown,
    ZoomIn,
    ZoomOut,
    ResetZoom,
    FocusContent,
    FocusCanvas,
    Undo,
    Redo,
    Duplicate,
    Delete,
    Copy,
    Paste,
    ToggleSidebar,
}

impl CommandId {
    pub const ALL: [Self; 25] = [
        Self::OpenPalette,
        Self::NextWorkspace,
        Self::PreviousWorkspace,
        Self::NextAttention,
        Self::PreviousAttention,
        Self::NextNode,
        Self::PreviousNode,
        Self::NextConnection,
        Self::PreviousConnection,
        Self::PanLeft,
        Self::PanRight,
        Self::PanUp,
        Self::PanDown,
        Self::ZoomIn,
        Self::ZoomOut,
        Self::ResetZoom,
        Self::FocusContent,
        Self::FocusCanvas,
        Self::Undo,
        Self::Redo,
        Self::Duplicate,
        Self::Delete,
        Self::Copy,
        Self::Paste,
        Self::ToggleSidebar,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::OpenPalette => "open_palette",
            Self::NextWorkspace => "next_workspace",
            Self::PreviousWorkspace => "previous_workspace",
            Self::NextAttention => "next_attention",
            Self::PreviousAttention => "previous_attention",
            Self::NextNode => "next_node",
            Self::PreviousNode => "previous_node",
            Self::NextConnection => "next_connection",
            Self::PreviousConnection => "previous_connection",
            Self::PanLeft => "pan_left",
            Self::PanRight => "pan_right",
            Self::PanUp => "pan_up",
            Self::PanDown => "pan_down",
            Self::ZoomIn => "zoom_in",
            Self::ZoomOut => "zoom_out",
            Self::ResetZoom => "reset_zoom",
            Self::FocusContent => "focus_content",
            Self::FocusCanvas => "focus_canvas",
            Self::Undo => "undo",
            Self::Redo => "redo",
            Self::Duplicate => "duplicate",
            Self::Delete => "delete",
            Self::Copy => "copy",
            Self::Paste => "paste",
            Self::ToggleSidebar => "toggle_sidebar",
        }
    }

    pub fn from_storage_id(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|command| command.as_str() == value)
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::OpenPalette => "Open command palette",
            Self::NextWorkspace => "Next workspace",
            Self::PreviousWorkspace => "Previous workspace",
            Self::NextAttention => "Next agent needing attention",
            Self::PreviousAttention => "Previous agent needing attention",
            Self::NextNode => "Next canvas node",
            Self::PreviousNode => "Previous canvas node",
            Self::NextConnection => "Next connected node",
            Self::PreviousConnection => "Previous connected node",
            Self::PanLeft => "Pan canvas left",
            Self::PanRight => "Pan canvas right",
            Self::PanUp => "Pan canvas up",
            Self::PanDown => "Pan canvas down",
            Self::ZoomIn => "Zoom in",
            Self::ZoomOut => "Zoom out",
            Self::ResetZoom => "Reset zoom",
            Self::FocusContent => "Focus selected content",
            Self::FocusCanvas => "Focus canvas",
            Self::Undo => "Undo canvas edit",
            Self::Redo => "Redo canvas edit",
            Self::Duplicate => "Duplicate selection",
            Self::Delete => "Delete selection",
            Self::Copy => "Copy selection",
            Self::Paste => "Paste selection",
            Self::ToggleSidebar => "Show or hide the workspace rail",
        }
    }

    pub fn default_shortcut(self) -> Option<Shortcut> {
        let value = match self {
            Self::OpenPalette => "Primary+k",
            Self::NextWorkspace => "Primary+]",
            Self::PreviousWorkspace => "Primary+[",
            Self::NextAttention => "Primary+Shift+]",
            Self::PreviousAttention => "Primary+Shift+[",
            Self::NextNode => "Primary+ArrowDown",
            Self::PreviousNode => "Primary+ArrowUp",
            Self::NextConnection => "Primary+ArrowRight",
            Self::PreviousConnection => "Primary+ArrowLeft",
            Self::PanLeft => "ArrowLeft",
            Self::PanRight => "ArrowRight",
            Self::PanUp => "ArrowUp",
            Self::PanDown => "ArrowDown",
            Self::ZoomIn => "plus",
            Self::ZoomOut => "minus",
            Self::ResetZoom => "0",
            Self::FocusContent => "Enter",
            Self::FocusCanvas => "Primary+Shift+Escape",
            Self::Undo => "Primary+z",
            Self::Redo => "Primary+Shift+z",
            Self::Duplicate => "Primary+d",
            Self::Delete => "Delete",
            Self::Copy => "Primary+c",
            Self::Paste => "Primary+v",
            Self::ToggleSidebar => "Primary+b",
        };
        Shortcut::parse(value).ok()
    }
}

#[derive(Debug, Clone)]
pub struct CommandBinding {
    pub id: CommandId,
    pub label: &'static str,
    pub shortcut: Option<Shortcut>,
}

#[derive(Debug, Clone)]
pub struct CommandRegistry {
    bindings: BTreeMap<CommandId, Option<Shortcut>>,
}

impl Default for CommandRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl CommandRegistry {
    pub fn new() -> Self {
        Self {
            bindings: CommandId::ALL
                .into_iter()
                .map(|id| (id, id.default_shortcut()))
                .collect(),
        }
    }

    pub fn binding(&self, id: CommandId) -> Option<&Shortcut> {
        self.bindings.get(&id).and_then(Option::as_ref)
    }

    pub fn commands(&self) -> impl Iterator<Item = CommandBinding> + '_ {
        CommandId::ALL.into_iter().map(|id| CommandBinding {
            id,
            label: id.label(),
            shortcut: self.bindings.get(&id).cloned().flatten(),
        })
    }

    pub fn command_for(&self, shortcut: &Shortcut) -> Option<CommandId> {
        CommandId::ALL
            .into_iter()
            .find(|id| self.binding(*id) == Some(shortcut))
    }

    pub fn rebind(
        &mut self,
        id: CommandId,
        shortcut: Option<Shortcut>,
    ) -> Result<(), BindingError> {
        if let Some(shortcut) = shortcut.as_ref()
            && let Some(existing) = self.command_for(shortcut)
            && existing != id
        {
            return Err(BindingError::Conflict {
                shortcut: shortcut.clone(),
                command: existing,
            });
        }
        self.bindings.insert(id, shortcut);
        Ok(())
    }

    pub fn apply_stored(
        &mut self,
        values: impl IntoIterator<Item = (String, Option<String>)>,
    ) -> Vec<String> {
        let mut warnings = Vec::new();
        let mut parsed = Vec::new();
        for (id, value) in values {
            let Some(command) = CommandId::from_storage_id(&id) else {
                continue;
            };
            let shortcut = match value {
                Some(value) => match Shortcut::parse(&value) {
                    Ok(shortcut) => Some(shortcut),
                    Err(error) => {
                        warnings.push(format!("Ignored invalid shortcut for {id}: {error}"));
                        continue;
                    }
                },
                None => None,
            };
            parsed.push((id, command, shortcut));
        }
        for (_, command, _) in &parsed {
            self.bindings.insert(*command, None);
        }
        for (id, command, shortcut) in parsed {
            if let Err(error) = self.rebind(command, shortcut) {
                warnings.push(format!("Ignored shortcut for {id}: {error}"));
                if let Err(default_error) = self.rebind(command, command.default_shortcut()) {
                    warnings.push(format!(
                        "Could not restore the default shortcut for {id}: {default_error}"
                    ));
                }
            }
        }
        warnings
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BindingError {
    Conflict {
        shortcut: Shortcut,
        command: CommandId,
    },
}

impl Display for BindingError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Conflict { shortcut, command } => write!(
                formatter,
                "{} is already assigned to {}",
                shortcut.storage_value(),
                command.label()
            ),
        }
    }
}

impl Error for BindingError {}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every command must be reachable and rebindable from the palette, and a
    /// duplicate default binding would silently shadow one of them.
    #[test]
    fn every_command_has_a_distinct_default_binding() {
        use std::collections::BTreeSet;

        let mut seen = BTreeSet::new();
        for command in CommandId::ALL {
            let shortcut = command
                .default_shortcut()
                .unwrap_or_else(|| panic!("{} has no default binding", command.as_str()));
            assert!(
                seen.insert(shortcut.storage_value()),
                "{} repeats a default binding",
                command.as_str()
            );
            assert_eq!(CommandId::from_storage_id(command.as_str()), Some(command));
        }

        assert_eq!(
            CommandId::ToggleSidebar
                .default_shortcut()
                .unwrap()
                .storage_value(),
            "Primary+b"
        );
    }

    #[test]
    fn shortcut_round_trip_and_platform_display_are_stable() {
        let shortcut = Shortcut::parse("Primary+Shift+K").unwrap();
        assert_eq!(shortcut.storage_value(), "Primary+Shift+k");
        assert_eq!(shortcut.display(Platform::MacOs), "Cmd+Shift+k");
        assert_eq!(shortcut.display(Platform::Other), "Ctrl+Shift+k");
    }

    #[test]
    fn duplicate_bindings_are_rejected_without_mutation() {
        let mut registry = CommandRegistry::new();
        let shortcut = registry.binding(CommandId::OpenPalette).unwrap().clone();
        let before = registry.binding(CommandId::NextWorkspace).cloned();
        let error = registry
            .rebind(CommandId::NextWorkspace, Some(shortcut))
            .unwrap_err();
        assert!(matches!(error, BindingError::Conflict { .. }));
        assert_eq!(registry.binding(CommandId::NextWorkspace).cloned(), before);
    }

    #[test]
    fn unknown_stored_commands_survive_loading_without_affecting_registry() {
        let mut registry = CommandRegistry::new();
        let warnings = registry.apply_stored([
            ("future_command".to_owned(), Some("Primary+j".to_owned())),
            ("open_palette".to_owned(), Some("Primary+p".to_owned())),
        ]);
        assert!(warnings.is_empty());
        assert_eq!(registry.binding(CommandId::OpenPalette).unwrap().key(), "p");
    }

    #[test]
    fn stored_reassignment_is_independent_of_database_row_order() {
        let mut registry = CommandRegistry::new();
        let warnings = registry.apply_stored([
            ("open_palette".to_owned(), Some("Primary+]".to_owned())),
            ("next_workspace".to_owned(), None),
        ]);
        assert!(warnings.is_empty());
        assert_eq!(
            registry.command_for(&Shortcut::parse("Primary+]").unwrap()),
            Some(CommandId::OpenPalette)
        );
        assert!(registry.binding(CommandId::NextWorkspace).is_none());
    }
}
