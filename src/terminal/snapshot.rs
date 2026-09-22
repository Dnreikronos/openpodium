use std::fmt::Write;

use alacritty_terminal::grid::Scroll;
use serde::{Deserialize, Serialize};

use super::{CellView, CursorStyle, GridSize, Model, Status};

const PREFIX: &[u8] = b"\0OPENPODIUM_SCREEN_V1\n";

#[derive(Serialize, Deserialize)]
struct Screen {
    columns: u16,
    rows: u16,
    ansi: String,
}

/// Capture the active screen, not a selection highlight or a scrolled viewport.
/// Its size is bounded by the terminal grid, independent of output volume.
pub(super) fn capture(model: &mut Model) -> Vec<u8> {
    let selection = model.term.selection.take();
    let offset = model.term.grid().display_offset();
    model.term.scroll_display(Scroll::Bottom);
    let view = model.view(Status::Offline);
    model.term.scroll_display(Scroll::Delta(offset as i32));
    model.term.selection = selection;

    // Disable wrapping while painting so the bottom-right cell cannot scroll.
    let mut ansi = String::from("\x1b[?7l");
    if view.mode.alternate_screen {
        ansi.push_str("\x1b[?1049h");
    }
    let mut previous: Option<&CellView> = None;
    for cell in &view.cells {
        write!(ansi, "\x1b[{};{}H", cell.row + 1, cell.column + 1).unwrap();
        if previous.is_none_or(|previous| !same_style(previous, cell)) {
            let foreground = cell.foreground;
            let background = cell.background;
            write!(
                ansi,
                "\x1b[0;38;2;{};{};{};48;2;{};{};{}m",
                foreground.red,
                foreground.green,
                foreground.blue,
                background.red,
                background.green,
                background.blue
            )
            .unwrap();
            for (enabled, code) in [
                (cell.bold, 1),
                (cell.italic, 3),
                (cell.underline, 4),
                (cell.strikeout, 9),
            ] {
                if enabled {
                    write!(ansi, "\x1b[{code}m").unwrap();
                }
            }
            write!(
                ansi,
                "\x1b]8;;{}\x1b\\",
                safe_osc(cell.hyperlink.as_deref().unwrap_or_default())
            )
            .unwrap();
        }
        ansi.push_str(&cell.text);
        previous = Some(cell);
    }
    ansi.push_str("\x1b]8;;\x1b\\\x1b[0m\x1b[?7h");
    for (enabled, mode) in [
        (view.mode.application_cursor, 1),
        (view.mode.bracketed_paste, 2004),
        (view.mode.mouse_reporting, 1000),
    ] {
        if enabled {
            write!(ansi, "\x1b[?{mode}h").unwrap();
        }
    }
    if let Some(cursor) = view.cursor {
        let shape = match cursor.style {
            CursorStyle::Block => 2,
            CursorStyle::Underline => 4,
            CursorStyle::Beam => 6,
        };
        write!(
            ansi,
            "\x1b[{};{}H\x1b[{shape} q\x1b[?25h",
            cursor.row + 1,
            cursor.column + 1
        )
        .unwrap();
    } else {
        ansi.push_str("\x1b[?25l");
    }
    if let Some(title) = view.title {
        write!(ansi, "\x1b]2;{}\x07", safe_osc(&title)).unwrap();
    }
    let screen = Screen {
        columns: view.size.columns,
        rows: view.size.rows,
        ansi,
    };
    let mut bytes = PREFIX.to_vec();
    bytes.extend(serde_json::to_vec(&screen).expect("screen contains only strings and integers"));
    bytes
}

pub(super) fn restore(size: GridSize, bytes: &[u8]) -> Model {
    let Some(json) = bytes.strip_prefix(PREFIX) else {
        let mut model = Model::new(size);
        let _ = model.feed(bytes);
        return model;
    };
    let Ok(screen) = serde_json::from_slice::<Screen>(json) else {
        return Model::new(size);
    };
    if screen.rows == 0
        || screen.columns == 0
        || usize::from(screen.rows) * usize::from(screen.columns) > 1_000_000
    {
        return Model::new(size);
    }
    let mut model = Model::new(GridSize {
        columns: screen.columns,
        rows: screen.rows,
    });
    let _ = model.feed(screen.ansi.as_bytes());
    model.resize(size);
    model
}

fn same_style(left: &CellView, right: &CellView) -> bool {
    left.foreground == right.foreground
        && left.background == right.background
        && left.bold == right.bold
        && left.italic == right.italic
        && left.underline == right.underline
        && left.strikeout == right.strikeout
        && left.hyperlink == right.hyperlink
}

fn safe_osc(value: &str) -> String {
    value
        .chars()
        .filter(|character| !character.is_control())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_preserves_styles_modes_cursor_unicode_and_title() {
        let size = GridSize {
            columns: 20,
            rows: 4,
        };
        let mut model = Model::new(size);
        model.feed(
            b"\x1b[?1049h\x1b[?1h\x1b[?2004h\x1b[?1000h\x1b]2;Project\x07\x1b[31;44;1;3;4;9m",
        );
        model.feed("界e\u{301}".as_bytes());
        model.feed(b"\x1b]8;;https://example.test\x1b\\link\x1b]8;;\x1b\\\x1b[0m\x1b[4;20H!\x1b[2;5H\x1b[6 q");
        let expected = model.view(Status::Offline);
        let payload = capture(&mut model);
        assert_eq!(restore(size, &payload).view(Status::Offline), expected);
        model.feed(b"\x1b[?25l");
        let payload = capture(&mut model);
        assert_eq!(
            restore(size, &payload).view(Status::Offline),
            model.view(Status::Offline)
        );
    }

    #[test]
    fn capture_leaves_scrollback_and_selection_unchanged() {
        let size = GridSize {
            columns: 20,
            rows: 4,
        };
        let mut model = Model::new(size);
        model.feed(b"one\r\ntwo\r\nthree\r\nfour\r\nfive");
        let expected = model.view(Status::Offline);
        model.scroll(1);
        model.begin_selection(0, 0, false);
        model.update_selection(0, 2, true);
        let selected = model.view(Status::Offline);
        let payload = capture(&mut model);
        assert_eq!(model.view(Status::Offline), selected);
        assert_eq!(restore(size, &payload).view(Status::Offline), expected);
    }
}
