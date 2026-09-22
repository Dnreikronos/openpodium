use std::fmt::Write;

use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line, Point};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::vte::ansi::{Color, NamedColor, Rgb};
use serde::{Deserialize, Serialize};

use super::{CellView, CursorStyle, DefaultColor, GridSize, Model, Status};

const PREFIX: &[u8] = b"\0OPENPODIUM_SCREEN_V1\n";
const HISTORY_BYTES: usize = 256 * 1024;

#[derive(Serialize, Deserialize)]
struct Screen {
    columns: u16,
    rows: u16,
    ansi: String,
    #[serde(default)]
    history: Vec<String>,
    #[serde(default)]
    semantic_colors: bool,
}

/// Capture the active screen and a bounded tail of complete scrollback rows.
pub(super) fn capture(model: &mut Model) -> Vec<u8> {
    let selection = model.term.selection.take();
    let offset = model.term.grid().display_offset();
    model.term.scroll_display(Scroll::Bottom);
    let view = model.view(Status::Offline);
    let history = capture_history(model);
    model.term.scroll_display(Scroll::Delta(offset as i32));
    model.term.selection = selection;

    // Disable wrapping while painting so the bottom-right cell cannot scroll.
    let mut ansi = String::from("\x1b[?7l");
    if view.mode.alternate_screen {
        ansi.push_str("\x1b[?1049h");
    }
    paint_cells(&mut ansi, &view.cells, true);
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
        history,
        semantic_colors: true,
    };
    let mut bytes = PREFIX.to_vec();
    bytes.extend(serde_json::to_vec(&screen).expect("screen contains only strings and integers"));
    bytes
}

fn paint_cells(ansi: &mut String, cells: &[CellView], absolute_rows: bool) {
    let mut previous: Option<&CellView> = None;
    for cell in cells {
        if absolute_rows {
            write!(ansi, "\x1b[{};{}H", cell.row + 1, cell.column + 1).unwrap();
        } else {
            write!(ansi, "\x1b[{}G", cell.column + 1).unwrap();
        }
        if previous.is_none_or(|previous| !same_style(previous, cell)) {
            let inverse = matches!(
                cell.foreground.default_role,
                Some(DefaultColor::Background | DefaultColor::DimBackground)
            ) || matches!(
                cell.background.default_role,
                Some(DefaultColor::Foreground | DefaultColor::DimForeground)
            );
            let (foreground, background) = if inverse {
                (cell.background, cell.foreground)
            } else {
                (cell.foreground, cell.background)
            };
            ansi.push_str("\x1b[0m");
            if inverse {
                ansi.push_str("\x1b[7m");
            }
            if matches!(
                cell.foreground.default_role,
                Some(DefaultColor::DimForeground | DefaultColor::DimBackground)
            ) {
                ansi.push_str("\x1b[2m");
            }
            for (color, channel, default) in [(foreground, 38, 39), (background, 48, 49)] {
                if color.default_role.is_some() {
                    write!(ansi, "\x1b[{default}m").unwrap();
                } else {
                    write!(
                        ansi,
                        "\x1b[{channel};2;{};{};{}m",
                        color.red, color.green, color.blue
                    )
                    .unwrap();
                }
            }
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
}

fn capture_history(model: &mut Model) -> Vec<String> {
    let mut history = Vec::new();
    let mut bytes = 0;
    let count = model.term.grid().history_size();
    let rows = usize::from(model.size.rows);
    let mut captured = 0;
    'pages: while captured < count {
        let page_rows = (count - captured).min(rows);
        model.term.scroll_display(Scroll::Delta(page_rows as i32));
        let view = model.view(Status::Offline);
        for row in (0..page_rows).rev() {
            let cells = view
                .cells
                .iter()
                .filter(|cell| cell.row == row)
                .cloned()
                .collect::<Vec<_>>();
            let mut line = String::new();
            paint_cells(&mut line, &cells, false);
            line.push_str("\x1b]8;;\x1b\\\x1b[0m\r\n");
            if bytes + line.len() > HISTORY_BYTES {
                break 'pages;
            }
            bytes += line.len();
            history.push(line);
        }
        captured += page_rows;
    }
    model.term.scroll_display(Scroll::Bottom);
    history.reverse();
    history
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
    let _ = model.feed(b"\x1b[?7l");
    for line in &screen.history {
        let _ = model.feed(line.as_bytes());
    }
    if !screen.history.is_empty() {
        for _ in 1..screen.rows {
            let _ = model.feed(b"\r\n");
        }
    }
    let _ = model.feed(screen.ansi.as_bytes());
    if !screen.semantic_colors {
        migrate_legacy_colors(&mut model);
    }
    model.resize(size);
    model
}

fn migrate_legacy_colors(model: &mut Model) {
    let grid = model.term.grid_mut();
    for row in grid.topmost_line().0..grid.screen_lines() as i32 {
        for column in 0..grid.columns() {
            let cell = &mut grid[Point::new(Line(row), Column(column))];
            // The old encoder flattened defaults (and inverse/dim styles) into RGB.
            cell.fg = match cell.fg {
                Color::Spec(Rgb {
                    r: 26,
                    g: 28,
                    b: 31,
                }) => {
                    cell.flags.insert(Flags::DIM);
                    Color::Named(NamedColor::Foreground)
                }
                Color::Spec(Rgb {
                    r: 170,
                    g: 170,
                    b: 170,
                }) => {
                    cell.flags.insert(Flags::DIM);
                    Color::Named(NamedColor::Background)
                }
                color => legacy_default(color),
            };
            cell.bg = legacy_default(cell.bg);
        }
    }
}

fn legacy_default(color: Color) -> Color {
    match color {
        Color::Spec(Rgb {
            r: 39,
            g: 42,
            b: 47,
        }) => Color::Named(NamedColor::Foreground),
        Color::Spec(Rgb {
            r: 255,
            g: 255,
            b: 255,
        }) => Color::Named(NamedColor::Background),
        color => color,
    }
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
    fn legacy_snapshots_recover_default_colors_in_screen_and_history() {
        let size = GridSize {
            columns: 20,
            rows: 4,
        };
        let json = serde_json::json!({
            "columns": size.columns,
            "rows": size.rows,
            "history": ["\u{1b}[0;38;2;39;42;47;48;2;255;255;255mhistory\u{1b}[0m\r\n"],
            "ansi": "\u{1b}[1;1H\u{1b}[0;38;2;39;42;47;48;2;255;255;255mA\u{1b}[0;38;2;255;255;255;48;2;39;42;47mB\u{1b}[0;38;2;26;28;31;48;2;255;255;255mC\u{1b}[0;38;2;170;170;170;48;2;39;42;47mD\u{1b}[0;38;2;12;34;56;48;2;65;43;21mE"
        });
        let mut payload = PREFIX.to_vec();
        payload.extend(serde_json::to_vec(&json).unwrap());
        let mut model = restore(size, &payload);
        let view = model.view(Status::Offline);
        for (cell, foreground, background) in [
            ('A', DefaultColor::Foreground, DefaultColor::Background),
            ('B', DefaultColor::Background, DefaultColor::Foreground),
            ('C', DefaultColor::DimForeground, DefaultColor::Background),
            ('D', DefaultColor::DimBackground, DefaultColor::Foreground),
        ] {
            let cell = view
                .cells
                .iter()
                .find(|value| value.text == cell.to_string())
                .unwrap();
            assert_eq!(cell.foreground.default_role, Some(foreground));
            assert_eq!(cell.background.default_role, Some(background));
        }
        let colored = view.cells.iter().find(|cell| cell.text == "E").unwrap();
        assert_eq!(colored.foreground.default_role, None);
        assert_eq!(colored.foreground.red, 12);
        assert_eq!(colored.background.default_role, None);
        assert_eq!(colored.background.red, 65);
        let saved = capture(&mut model);
        assert_eq!(restore(size, &saved).view(Status::Offline), view);
        model.scroll(1);
        let history = model.view(Status::Offline);
        let first = &history.cells[0];
        assert_eq!(first.text, "h");
        assert_eq!(
            first.foreground.default_role,
            Some(DefaultColor::Foreground)
        );
        assert_eq!(
            first.background.default_role,
            Some(DefaultColor::Background)
        );
    }

    #[test]
    fn raw_output_keeps_explicit_colors_matching_legacy_defaults() {
        let size = GridSize {
            columns: 20,
            rows: 4,
        };
        let model = restore(size, b"\x1b[38;2;39;42;47;48;2;255;255;255mA");
        let cell = &model.view(Status::Offline).cells[0];
        assert_eq!(cell.foreground.default_role, None);
        assert_eq!(cell.background.default_role, None);
    }

    #[test]
    fn snapshot_retains_theme_defaults_explicit_colors_and_dim_inverse_styles() {
        let size = GridSize {
            columns: 30,
            rows: 4,
        };
        let mut model = Model::new(size);
        model.feed(b"plain \x1b[2mdim\x1b[0;7minverse\x1b[2m dim\x1b[0;38;2;39;42;47;48;2;255;255;255m rgb");
        let expected = model.view(Status::Offline);
        let payload = capture(&mut model);
        assert_eq!(restore(size, &payload).view(Status::Offline), expected);
    }

    #[test]
    fn restored_terminal_can_scroll_back_to_earlier_output() {
        let size = GridSize {
            columns: 20,
            rows: 4,
        };
        let mut original = Model::new(size);
        original.feed(b"one\r\ntwo\r\nthree\r\nfour\r\nfive\r\nsix");
        let payload = capture(&mut original);
        let mut restored = restore(size, &payload);
        original.scroll(2);
        restored.scroll(2);
        assert_eq!(
            restored.view(Status::Offline),
            original.view(Status::Offline)
        );
    }

    #[test]
    fn history_preserves_styled_unicode_across_partial_and_complete_pages() {
        for lines in [5, 7, 8, 9, 21] {
            let size = GridSize {
                columns: 20,
                rows: 4,
            };
            let mut original = Model::new(size);
            for line in 0..lines {
                original.feed(format!("\x1b[{}m{line}: 界e\u{301}\r\n", 31 + line % 6).as_bytes());
            }
            let mut restored = restore(size, &capture(&mut original));
            let count = original.term.grid().history_size();
            assert_eq!(restored.term.grid().history_size(), count);
            for _ in 0..=count {
                assert_eq!(
                    restored.view(Status::Offline),
                    original.view(Status::Offline)
                );
                original.scroll(1);
                restored.scroll(1);
            }
        }
    }

    #[test]
    fn old_screen_only_snapshots_remain_readable() {
        let size = GridSize {
            columns: 20,
            rows: 4,
        };
        let mut original = Model::new(size);
        original.feed(b"legacy");
        let payload = capture(&mut original);
        let mut json: serde_json::Value =
            serde_json::from_slice(payload.strip_prefix(PREFIX).unwrap()).unwrap();
        json.as_object_mut().unwrap().remove("history");
        let mut legacy = PREFIX.to_vec();
        legacy.extend(serde_json::to_vec(&json).unwrap());
        assert_eq!(
            restore(size, &legacy).view(Status::Offline),
            original.view(Status::Offline)
        );
    }

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
