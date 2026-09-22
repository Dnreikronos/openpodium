use std::mem;
use std::sync::{Arc, Mutex};

use alacritty_terminal::event::{Event, EventListener, WindowSize};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Config, Term, TermMode};
use alacritty_terminal::vte::ansi::{self, Color, CursorShape, NamedColor, Rgb};
use iced::keyboard::{Key, Modifiers, key::Named};
use openpodium::localization::Localizer;

pub(crate) mod session;
mod snapshot;

pub(crate) const CELL_WIDTH: f32 = 8.0;
pub(crate) const CELL_HEIGHT: f32 = 16.0;
pub(crate) const BODY_PADDING: f32 = 10.0;
pub(crate) const HEADER_HEIGHT: f32 = 32.0;
const SCROLLBACK_LINES: usize = 10_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Status {
    Offline,
    Starting,
    Running,
    Exited(String),
    Stopped,
    Failed(String),
}

impl Status {
    pub(crate) fn label(&self, localizer: &Localizer) -> String {
        match self {
            Self::Offline => localizer.text("terminal-offline"),
            Self::Starting => localizer.text("terminal-starting"),
            Self::Running => localizer.text("terminal-running"),
            Self::Exited(detail) => localizer.with_str("terminal-exited", "detail", detail),
            Self::Stopped => localizer.text("terminal-stopped"),
            Self::Failed(detail) => localizer.with_str("terminal-failed", "detail", detail),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct GridSize {
    pub(crate) columns: u16,
    pub(crate) rows: u16,
}

impl GridSize {
    pub(crate) fn for_node(width: f32, height: f32) -> Self {
        let body_width = (width - BODY_PADDING * 2.0).max(CELL_WIDTH);
        let body_height = (height - HEADER_HEIGHT - BODY_PADDING * 2.0).max(CELL_HEIGHT);
        Self {
            columns: ((body_width / CELL_WIDTH).floor() as u16).max(1),
            rows: ((body_height / CELL_HEIGHT).floor() as u16).max(1),
        }
    }
}

impl Dimensions for GridSize {
    fn total_lines(&self) -> usize {
        usize::from(self.rows)
    }

    fn screen_lines(&self) -> usize {
        usize::from(self.rows)
    }

    fn columns(&self) -> usize {
        usize::from(self.columns)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CellColor {
    pub(crate) red: u8,
    pub(crate) green: u8,
    pub(crate) blue: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CellView {
    pub(crate) row: usize,
    pub(crate) column: usize,
    pub(crate) text: String,
    pub(crate) foreground: CellColor,
    pub(crate) background: CellColor,
    pub(crate) bold: bool,
    pub(crate) italic: bool,
    pub(crate) underline: bool,
    pub(crate) strikeout: bool,
    pub(crate) selected: bool,
    pub(crate) hyperlink: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CursorStyle {
    Block,
    Underline,
    Beam,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CursorView {
    pub(crate) row: usize,
    pub(crate) column: usize,
    pub(crate) style: CursorStyle,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct InputMode {
    pub(crate) application_cursor: bool,
    pub(crate) bracketed_paste: bool,
    pub(crate) mouse_reporting: bool,
    pub(crate) alternate_screen: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct View {
    pub(crate) size: GridSize,
    pub(crate) cells: Vec<CellView>,
    pub(crate) cursor: Option<CursorView>,
    pub(crate) mode: InputMode,
    pub(crate) title: Option<String>,
    pub(crate) status: Status,
}

impl View {
    pub(crate) fn offline(size: GridSize) -> Self {
        Self {
            size,
            cells: Vec::new(),
            cursor: None,
            mode: InputMode::default(),
            title: None,
            status: Status::Offline,
        }
    }
}

pub(crate) enum Update {
    PtyWrite(Vec<u8>),
    ClipboardStore(String),
    TitleChanged(Option<String>),
    Bell,
}

#[derive(Clone, Default)]
struct EventQueue(Arc<Mutex<Vec<Event>>>);

impl EventQueue {
    fn drain(&self) -> Vec<Event> {
        self.0
            .lock()
            .map(|mut events| mem::take(&mut *events))
            .unwrap_or_default()
    }
}

impl EventListener for EventQueue {
    fn send_event(&self, event: Event) {
        if let Ok(mut events) = self.0.lock() {
            events.push(event);
        }
    }
}

pub(crate) struct Model {
    parser: ansi::Processor,
    term: Term<EventQueue>,
    events: EventQueue,
    size: GridSize,
    title: Option<String>,
}

impl Model {
    pub(crate) fn new(size: GridSize) -> Self {
        let events = EventQueue::default();
        let config = Config {
            scrolling_history: SCROLLBACK_LINES,
            ..Config::default()
        };
        Self {
            parser: ansi::Processor::new(),
            term: Term::new(config, &size, events.clone()),
            events,
            size,
            title: None,
        }
    }

    pub(crate) fn feed(&mut self, bytes: &[u8]) -> Vec<Update> {
        self.parser.advance(&mut self.term, bytes);
        let mut updates = Vec::new();
        for event in self.events.drain() {
            match event {
                Event::PtyWrite(text) => updates.push(Update::PtyWrite(text.into_bytes())),
                Event::ClipboardStore(_, text) => updates.push(Update::ClipboardStore(text)),
                Event::Title(title) => {
                    self.title = Some(title.clone());
                    updates.push(Update::TitleChanged(Some(title)));
                }
                Event::ResetTitle => {
                    self.title = None;
                    updates.push(Update::TitleChanged(None));
                }
                Event::ColorRequest(index, formatter) => {
                    let color = resolve_color_request(index);
                    updates.push(Update::PtyWrite(formatter(color).into_bytes()));
                }
                Event::TextAreaSizeRequest(formatter) => {
                    let window = WindowSize {
                        num_lines: self.size.rows,
                        num_cols: self.size.columns,
                        cell_width: CELL_WIDTH as u16,
                        cell_height: CELL_HEIGHT as u16,
                    };
                    updates.push(Update::PtyWrite(formatter(window).into_bytes()));
                }
                Event::Bell => updates.push(Update::Bell),
                Event::ClipboardLoad(_, _)
                | Event::MouseCursorDirty
                | Event::CursorBlinkingChange
                | Event::Wakeup
                | Event::Exit
                | Event::ChildExit(_) => {}
            }
        }
        updates
    }

    pub(crate) fn resize(&mut self, size: GridSize) {
        if self.size != size {
            self.size = size;
            self.term.resize(size);
        }
    }

    pub(crate) fn scroll(&mut self, lines: i32) {
        self.term.scroll_display(Scroll::Delta(lines));
    }

    pub(crate) fn begin_selection(&mut self, row: usize, column: usize, right_side: bool) {
        let point = self.display_point(row, column);
        let side = if right_side { Side::Right } else { Side::Left };
        self.term.selection = Some(Selection::new(SelectionType::Simple, point, side));
    }

    pub(crate) fn update_selection(&mut self, row: usize, column: usize, right_side: bool) {
        let point = self.display_point(row, column);
        let side = if right_side { Side::Right } else { Side::Left };
        if let Some(selection) = self.term.selection.as_mut() {
            selection.update(point, side);
        }
    }

    pub(crate) fn clear_selection(&mut self) {
        self.term.selection = None;
    }

    pub(crate) fn selected_text(&self) -> Option<String> {
        self.term.selection_to_string()
    }

    pub(crate) fn input_mode(&self) -> InputMode {
        input_mode(*self.term.mode())
    }

    pub(crate) fn view(&self, status: Status) -> View {
        let content = self.term.renderable_content();
        let display_offset = content.display_offset as i32;
        let selection = content.selection;
        let cursor = if display_offset == 0 && content.cursor.shape != CursorShape::Hidden {
            Some(CursorView {
                row: content.cursor.point.line.0.max(0) as usize,
                column: content.cursor.point.column.0,
                style: match content.cursor.shape {
                    CursorShape::Underline => CursorStyle::Underline,
                    CursorShape::Beam => CursorStyle::Beam,
                    _ => CursorStyle::Block,
                },
            })
        } else {
            None
        };
        let mode = input_mode(content.mode);
        let mut cells =
            Vec::with_capacity(usize::from(self.size.rows) * usize::from(self.size.columns));
        for indexed in content.display_iter {
            if indexed
                .cell
                .flags
                .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
            {
                continue;
            }
            let row = indexed.point.line.0 + display_offset;
            if row < 0 || row >= i32::from(self.size.rows) {
                continue;
            }
            let selected = selection.is_some_and(|range| range.contains(indexed.point));
            let mut foreground = resolve_color(indexed.cell.fg, content.colors);
            let mut background = resolve_color(indexed.cell.bg, content.colors);
            if indexed.cell.flags.contains(Flags::INVERSE) ^ selected {
                mem::swap(&mut foreground, &mut background);
            }
            if indexed.cell.flags.contains(Flags::DIM) {
                foreground = foreground.dimmed();
            }
            let mut text = indexed.cell.c.to_string();
            if let Some(combining) = indexed.cell.zerowidth() {
                text.extend(combining);
            }
            cells.push(CellView {
                row: row as usize,
                column: indexed.point.column.0,
                text,
                foreground,
                background,
                bold: indexed.cell.flags.contains(Flags::BOLD),
                italic: indexed.cell.flags.contains(Flags::ITALIC),
                underline: indexed.cell.flags.intersects(Flags::ALL_UNDERLINES),
                strikeout: indexed.cell.flags.contains(Flags::STRIKEOUT),
                selected,
                hyperlink: indexed.cell.hyperlink().map(|link| link.uri().to_owned()),
            });
        }
        View {
            size: self.size,
            cells,
            cursor,
            mode,
            title: self.title.clone(),
            status,
        }
    }

    fn display_point(&self, row: usize, column: usize) -> Point {
        let row = row.min(usize::from(self.size.rows.saturating_sub(1)));
        let column = column.min(usize::from(self.size.columns.saturating_sub(1)));
        Point::new(
            Line(row as i32 - self.term.grid().display_offset() as i32),
            Column(column),
        )
    }
}

fn input_mode(mode: TermMode) -> InputMode {
    InputMode {
        application_cursor: mode.contains(TermMode::APP_CURSOR),
        bracketed_paste: mode.contains(TermMode::BRACKETED_PASTE),
        mouse_reporting: mode.intersects(
            TermMode::MOUSE_REPORT_CLICK | TermMode::MOUSE_DRAG | TermMode::MOUSE_MOTION,
        ),
        alternate_screen: mode.contains(TermMode::ALT_SCREEN),
    }
}

impl CellColor {
    fn dimmed(self) -> Self {
        Self {
            red: ((u16::from(self.red) * 2) / 3) as u8,
            green: ((u16::from(self.green) * 2) / 3) as u8,
            blue: ((u16::from(self.blue) * 2) / 3) as u8,
        }
    }
}

fn resolve_color(color: Color, overrides: &alacritty_terminal::term::color::Colors) -> CellColor {
    let rgb = match color {
        Color::Spec(rgb) => rgb,
        Color::Indexed(index) => {
            overrides[usize::from(index)].unwrap_or_else(|| resolve_indexed(index))
        }
        Color::Named(name) => overrides[name].unwrap_or_else(|| resolve_named(name)),
    };
    CellColor {
        red: rgb.r,
        green: rgb.g,
        blue: rgb.b,
    }
}

fn resolve_named(color: NamedColor) -> Rgb {
    let index = match color {
        NamedColor::Black | NamedColor::DimBlack => 0,
        NamedColor::Red | NamedColor::DimRed => 1,
        NamedColor::Green | NamedColor::DimGreen => 2,
        NamedColor::Yellow | NamedColor::DimYellow => 3,
        NamedColor::Blue | NamedColor::DimBlue => 4,
        NamedColor::Magenta | NamedColor::DimMagenta => 5,
        NamedColor::Cyan | NamedColor::DimCyan => 6,
        NamedColor::White | NamedColor::DimWhite => 7,
        NamedColor::BrightBlack => 8,
        NamedColor::BrightRed => 9,
        NamedColor::BrightGreen => 10,
        NamedColor::BrightYellow => 11,
        NamedColor::BrightBlue => 12,
        NamedColor::BrightMagenta => 13,
        NamedColor::BrightCyan => 14,
        NamedColor::BrightWhite => 15,
        NamedColor::Background => {
            return Rgb {
                r: 255,
                g: 255,
                b: 255,
            };
        }
        NamedColor::Cursor => {
            return Rgb {
                r: 39,
                g: 42,
                b: 47,
            };
        }
        NamedColor::Foreground | NamedColor::DimForeground | NamedColor::BrightForeground => {
            return Rgb {
                r: 39,
                g: 42,
                b: 47,
            };
        }
    };
    resolve_indexed(index)
}

fn resolve_indexed(index: u8) -> Rgb {
    const ANSI: [Rgb; 16] = [
        Rgb {
            r: 30,
            g: 34,
            b: 42,
        },
        Rgb {
            r: 181,
            g: 44,
            b: 55,
        },
        Rgb {
            r: 43,
            g: 117,
            b: 56,
        },
        Rgb {
            r: 143,
            g: 100,
            b: 0,
        },
        Rgb {
            r: 35,
            g: 96,
            b: 184,
        },
        Rgb {
            r: 142,
            g: 63,
            b: 164,
        },
        Rgb {
            r: 0,
            g: 112,
            b: 126,
        },
        Rgb {
            r: 171,
            g: 178,
            b: 191,
        },
        Rgb {
            r: 92,
            g: 99,
            b: 112,
        },
        Rgb {
            r: 190,
            g: 45,
            b: 60,
        },
        Rgb {
            r: 39,
            g: 124,
            b: 59,
        },
        Rgb {
            r: 151,
            g: 107,
            b: 0,
        },
        Rgb {
            r: 27,
            g: 103,
            b: 199,
        },
        Rgb {
            r: 151,
            g: 60,
            b: 170,
        },
        Rgb {
            r: 0,
            g: 119,
            b: 134,
        },
        Rgb {
            r: 230,
            g: 235,
            b: 241,
        },
    ];
    match index {
        0..=15 => ANSI[usize::from(index)],
        16..=231 => {
            let offset = index - 16;
            let component = |value: u8| if value == 0 { 0 } else { 55 + value * 40 };
            Rgb {
                r: component(offset / 36),
                g: component((offset % 36) / 6),
                b: component(offset % 6),
            }
        }
        _ => {
            let gray = 8 + (index - 232) * 10;
            Rgb {
                r: gray,
                g: gray,
                b: gray,
            }
        }
    }
}

fn resolve_color_request(index: usize) -> Rgb {
    match index {
        0..=255 => resolve_indexed(index as u8),
        256 => resolve_named(NamedColor::Foreground),
        257 => resolve_named(NamedColor::Background),
        258 => resolve_named(NamedColor::Cursor),
        259 => resolve_named(NamedColor::DimBlack),
        260 => resolve_named(NamedColor::DimRed),
        261 => resolve_named(NamedColor::DimGreen),
        262 => resolve_named(NamedColor::DimYellow),
        263 => resolve_named(NamedColor::DimBlue),
        264 => resolve_named(NamedColor::DimMagenta),
        265 => resolve_named(NamedColor::DimCyan),
        266 => resolve_named(NamedColor::DimWhite),
        267 => resolve_named(NamedColor::BrightForeground),
        _ => resolve_named(NamedColor::Background),
    }
}

pub(crate) fn encode_key(
    key: &Key,
    text: Option<&str>,
    modifiers: Modifiers,
    mode: InputMode,
) -> Option<Vec<u8>> {
    if matches!(key, Key::Named(Named::Enter)) && modifiers == Modifiers::SHIFT {
        return Some(if mode.bracketed_paste {
            encode_paste("\n", mode)
        } else {
            b"\x1b[13;2u".to_vec()
        });
    }
    let mut bytes = if modifiers.control() {
        control_bytes(key).or_else(|| named_key_bytes(key, modifiers, mode))?
    } else if let Some(bytes) = named_key_bytes(key, modifiers, mode) {
        bytes
    } else if let Some(text) = text {
        text.as_bytes().to_vec()
    } else if let Key::Character(character) = key {
        character.as_bytes().to_vec()
    } else {
        return None;
    };
    if modifiers.alt() && !bytes.starts_with(b"\x1b") {
        bytes.insert(0, 0x1b);
    }
    Some(bytes)
}

pub(crate) fn encode_paste(text: &str, mode: InputMode) -> Vec<u8> {
    let sanitized = text.replace('\r', "");
    if mode.bracketed_paste {
        format!("\x1b[200~{sanitized}\x1b[201~").into_bytes()
    } else {
        sanitized.into_bytes()
    }
}

pub(crate) fn encode_mouse_wheel(row: usize, column: usize, upward: bool) -> Vec<u8> {
    let button = if upward { 64 } else { 65 };
    format!("\x1b[<{button};{};{}M", column + 1, row + 1).into_bytes()
}

fn control_bytes(key: &Key) -> Option<Vec<u8>> {
    let Key::Character(character) = key else {
        return None;
    };
    let character = character.chars().next()?.to_ascii_lowercase();
    let byte = match character {
        'a'..='z' => character as u8 - b'a' + 1,
        '@' | ' ' => 0,
        '[' => 27,
        '\\' => 28,
        ']' => 29,
        '^' => 30,
        '_' => 31,
        '?' => 127,
        _ => return None,
    };
    Some(vec![byte])
}

fn named_key_bytes(key: &Key, modifiers: Modifiers, mode: InputMode) -> Option<Vec<u8>> {
    let Key::Named(named) = key else {
        return None;
    };
    let sequence = match named {
        Named::Enter => "\r".to_owned(),
        Named::Tab if modifiers.shift() => "\x1b[Z".to_owned(),
        Named::Tab => "\t".to_owned(),
        Named::Backspace => "\x7f".to_owned(),
        Named::Escape => "\x1b".to_owned(),
        Named::ArrowUp => cursor_sequence('A', modifiers, mode),
        Named::ArrowDown => cursor_sequence('B', modifiers, mode),
        Named::ArrowRight => cursor_sequence('C', modifiers, mode),
        Named::ArrowLeft => cursor_sequence('D', modifiers, mode),
        Named::Home => csi_sequence("1", 'H', modifiers),
        Named::End => csi_sequence("1", 'F', modifiers),
        Named::Insert => tilde_sequence(2, modifiers),
        Named::Delete => tilde_sequence(3, modifiers),
        Named::PageUp => tilde_sequence(5, modifiers),
        Named::PageDown => tilde_sequence(6, modifiers),
        Named::F1 => function_sequence('P', modifiers),
        Named::F2 => function_sequence('Q', modifiers),
        Named::F3 => function_sequence('R', modifiers),
        Named::F4 => function_sequence('S', modifiers),
        Named::F5 => tilde_sequence(15, modifiers),
        Named::F6 => tilde_sequence(17, modifiers),
        Named::F7 => tilde_sequence(18, modifiers),
        Named::F8 => tilde_sequence(19, modifiers),
        Named::F9 => tilde_sequence(20, modifiers),
        Named::F10 => tilde_sequence(21, modifiers),
        Named::F11 => tilde_sequence(23, modifiers),
        Named::F12 => tilde_sequence(24, modifiers),
        _ => return None,
    };
    Some(sequence.into_bytes())
}

fn modifier_code(modifiers: Modifiers) -> u8 {
    1 + u8::from(modifiers.shift())
        + u8::from(modifiers.alt()) * 2
        + u8::from(modifiers.control()) * 4
}

fn cursor_sequence(final_char: char, modifiers: Modifiers, mode: InputMode) -> String {
    if modifier_code(modifiers) > 1 {
        csi_sequence("1", final_char, modifiers)
    } else if mode.application_cursor {
        format!("\x1bO{final_char}")
    } else {
        format!("\x1b[{final_char}")
    }
}

fn csi_sequence(prefix: &str, final_char: char, modifiers: Modifiers) -> String {
    let modifier = modifier_code(modifiers);
    if modifier == 1 {
        format!("\x1b[{prefix}{final_char}")
    } else {
        format!("\x1b[{prefix};{modifier}{final_char}")
    }
}

fn tilde_sequence(number: u8, modifiers: Modifiers) -> String {
    let modifier = modifier_code(modifiers);
    if modifier == 1 {
        format!("\x1b[{number}~")
    } else {
        format!("\x1b[{number};{modifier}~")
    }
}

fn function_sequence(final_char: char, modifiers: Modifiers) -> String {
    if modifier_code(modifiers) == 1 {
        format!("\x1bO{final_char}")
    } else {
        csi_sequence("1", final_char, modifiers)
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn model() -> Model {
        Model::new(GridSize {
            columns: 20,
            rows: 4,
        })
    }

    #[test]
    fn parses_styles_wide_glyphs_combining_marks_and_hyperlinks() {
        let mut model = model();
        model.feed(b"plain \x1b[31;1mred\x1b[0m ");
        model.feed("界e\u{301}".as_bytes());
        model.feed(b" \x1b]8;;https://example.com\x1b\\link\x1b]8;;\x1b\\");

        let view = model.view(Status::Running);
        assert!(view.cells.iter().any(|cell| cell.text == "界"));
        assert!(view.cells.iter().any(|cell| cell.text == "e\u{301}"));
        assert!(view.cells.iter().any(|cell| cell.text == "r" && cell.bold));
        assert!(view.cells.iter().any(|cell| {
            cell.text == "l" && cell.hyperlink.as_deref() == Some("https://example.com")
        }));
    }

    #[test]
    fn tracks_alternate_screen_scrollback_selection_and_resize() {
        let mut model = model();
        model.feed(b"one\r\ntwo\r\nthree\r\nfour\r\nfive");
        model.scroll(2);
        model.begin_selection(0, 0, false);
        model.update_selection(0, 2, true);
        assert!(model.selected_text().is_some_and(|text| !text.is_empty()));

        model.feed(b"\x1b[?1049hALT");
        assert!(model.view(Status::Running).mode.alternate_screen);
        model.feed(b"\x1b[?1049l");
        assert!(!model.view(Status::Running).mode.alternate_screen);

        let size = GridSize {
            columns: 12,
            rows: 6,
        };
        model.resize(size);
        assert_eq!(model.view(Status::Running).size, size);
    }

    #[test]
    fn maps_terminal_keys_and_bracketed_paste() {
        let mut mode = InputMode::default();
        assert_eq!(
            encode_key(&Key::Named(Named::ArrowUp), None, Modifiers::empty(), mode),
            Some(b"\x1b[A".to_vec())
        );
        mode.application_cursor = true;
        assert_eq!(
            encode_key(&Key::Named(Named::ArrowUp), None, Modifiers::empty(), mode),
            Some(b"\x1bOA".to_vec())
        );
        assert_eq!(
            encode_key(
                &Key::Character("c".into()),
                Some("c"),
                Modifiers::CTRL,
                mode,
            ),
            Some(vec![3])
        );
        mode.bracketed_paste = true;
        assert_eq!(encode_paste("a\r\nb", mode), b"\x1b[200~a\nb\x1b[201~");
        assert_eq!(encode_mouse_wheel(2, 4, true), b"\x1b[<64;5;3M");
    }

    #[test]
    fn shift_enter_inserts_a_newline_without_submitting() {
        let mut model = model();
        let key = Key::Named(Named::Enter);
        let encode =
            |model: &Model| encode_key(&key, Some("\r"), Modifiers::SHIFT, model.input_mode());

        assert_eq!(encode(&model), Some(b"\x1b[13;2u".to_vec()));
        model.feed(b"\x1b[?2004h");
        assert_eq!(encode(&model), Some(b"\x1b[200~\n\x1b[201~".to_vec()));
        model.feed(b"\x1b[?2004l");
        assert_eq!(encode(&model), Some(b"\x1b[13;2u".to_vec()));
    }

    #[test]
    fn newline_shortcut_preserves_other_enter_combinations() {
        for bracketed_paste in [false, true] {
            let mode = InputMode {
                bracketed_paste,
                ..InputMode::default()
            };
            for (modifiers, expected) in [
                (Modifiers::empty(), b"\r".as_slice()),
                (Modifiers::CTRL, b"\r"),
                (Modifiers::ALT, b"\x1b\r"),
                (Modifiers::CTRL | Modifiers::SHIFT, b"\r"),
                (Modifiers::ALT | Modifiers::SHIFT, b"\x1b\r"),
                (Modifiers::LOGO | Modifiers::SHIFT, b"\r"),
            ] {
                assert_eq!(
                    encode_key(&Key::Named(Named::Enter), Some("\r"), modifiers, mode),
                    Some(expected.to_vec()),
                    "modifiers: {modifiers:?}, bracketed paste: {bracketed_paste}"
                );
            }
        }
    }

    #[test]
    fn accessibility_preserves_dead_key_ime_and_right_to_left_commits() {
        let mode = InputMode::default();
        for committed in ["é", "かな", "مَرْحَبًا", "👩🏽‍💻"] {
            assert_eq!(
                encode_key(
                    &Key::Character(committed.into()),
                    Some(committed),
                    Modifiers::empty(),
                    mode,
                ),
                Some(committed.as_bytes().to_vec())
            );
        }

        let mut model = model();
        model.feed("עברית العربية".as_bytes());
        let rendered = model
            .view(Status::Running)
            .cells
            .into_iter()
            .map(|cell| cell.text)
            .collect::<String>();
        assert!(rendered.contains("עברית"));
        assert!(rendered.contains("العربية"));
    }

    #[test]
    fn light_defaults_preserve_explicit_colors_and_inverse_video() {
        let mut model = model();
        model.feed(b"A\x1b[38;2;12;34;56;48;2;78;90;123mB\x1b[0;7mC");
        let view = model.view(Status::Running);
        let plain = view.cells.iter().find(|cell| cell.text == "A").unwrap();
        assert_eq!(
            plain.background,
            CellColor {
                red: 255,
                green: 255,
                blue: 255
            }
        );
        assert!(
            plain.foreground.red < 50 && plain.foreground.green < 50 && plain.foreground.blue < 50
        );
        let explicit = view.cells.iter().find(|cell| cell.text == "B").unwrap();
        assert_eq!(
            explicit.foreground,
            CellColor {
                red: 12,
                green: 34,
                blue: 56
            }
        );
        assert_eq!(
            explicit.background,
            CellColor {
                red: 78,
                green: 90,
                blue: 123
            }
        );
        let inverse = view.cells.iter().find(|cell| cell.text == "C").unwrap();
        assert_eq!(inverse.foreground, plain.background);
        assert_eq!(inverse.background, plain.foreground);

        let updates = model.feed(b"\x1b]11;?\x07");
        assert!(updates.iter().any(|update| matches!(update,
            Update::PtyWrite(bytes) if String::from_utf8_lossy(bytes).contains("rgb:ffff/ffff/ffff")
        )));
    }

    #[test]
    fn derives_grid_size_from_node_body() {
        assert_eq!(
            GridSize::for_node(360.0, 260.0),
            GridSize {
                columns: 42,
                rows: 13
            }
        );
    }

    proptest! {
        #[test]
        fn arbitrary_terminal_output_preserves_render_invariants(bytes in prop::collection::vec(any::<u8>(), 0..16_384)) {
            let mut model = model();
            let _updates = model.feed(&bytes);
            let view = model.view(Status::Running);

            prop_assert_eq!(view.size, GridSize { columns: 20, rows: 4 });
            prop_assert!(view.cells.iter().all(|cell| cell.row < 4 && cell.column < 20));
        }
    }
}
