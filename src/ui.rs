//! Rendering: tree, tabs, watch table, settings form, help overlay,
//! status line and the self-documenting key-hint footer.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::prelude::*;
use ratatui::widgets::{
    Block, Cell, Clear, List, ListItem, Paragraph, Row, Scrollbar, ScrollbarOrientation,
    ScrollbarState, Table, Tabs, Wrap,
};

use crate::app::{App, Focus, InputKind, Tab};
use crate::hal::HalType;
use crate::tree::{self, TreeNode};
use crate::watch::WatchItem;

// classic ayu-dark palette (ayu-colors dark variant, pitch-black background)
const AYU_BG: Color = Color::Black;
const AYU_PANEL: Color = Color::Rgb(0x1C, 0x23, 0x2B); // line highlight
const AYU_FG: Color = Color::Rgb(0xB3, 0xB1, 0xAD); // foreground
const AYU_DIM: Color = Color::Rgb(0x62, 0x6A, 0x73); // comment
const AYU_ACCENT: Color = Color::Rgb(0xFF, 0xB4, 0x54); // accent orange
const AYU_YELLOW: Color = Color::Rgb(0xE6, 0xB4, 0x50); // operator
const AYU_TEAL: Color = Color::Rgb(0x95, 0xE6, 0xCB); // regexp
const AYU_BLUE: Color = Color::Rgb(0x39, 0xBA, 0xE6); // tag
const AYU_PURPLE: Color = Color::Rgb(0xD2, 0xA6, 0xFF); // entity
const AYU_RED: Color = Color::Rgb(0xF0, 0x71, 0x78); // markup
const AYU_ERROR: Color = Color::Rgb(0xFF, 0x33, 0x33); // invalid
const AYU_SEL_BG: Color = Color::Rgb(0x25, 0x33, 0x40); // selection
const AYU_MATCH_BG: Color = Color::Rgb(0x4C, 0x41, 0x26); // find match

const NAME_PIN: Color = AYU_FG;
const NAME_PARAM: Color = AYU_PURPLE;
const NAME_SIG: Color = AYU_BLUE;
const BIT_TRUE: Color = AYU_YELLOW;
const BIT_FALSE: Color = AYU_RED;
const ERR: Color = AYU_ERROR;

/// Shared bordered block: ayu border + accent title.
fn ayu_block() -> Block<'static> {
    Block::bordered()
        .border_style(Style::default().fg(AYU_DIM))
        .title_style(Style::default().fg(AYU_ACCENT).add_modifier(Modifier::BOLD))
}

pub fn draw(f: &mut Frame, app: &mut App) {
    let area = f.area();
    // ayu-dark base colors
    f.buffer_mut()
        .set_style(area, Style::default().fg(AYU_FG).bg(AYU_BG));
    if app.help {
        draw_help(f, app);
        return;
    }
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // title
            Constraint::Min(3),    // main
            Constraint::Length(1), // status
            Constraint::Length(1), // key hints
        ])
        .split(area);

    draw_title(f, app, rows[0]);
    draw_status(f, app, rows[2]);
    draw_hints(f, app, rows[3]);

    if app.settings_open {
        // settings is a full-screen mode reached via F5
        draw_settings(f, app, rows[1]);
        if let Some(input) = &mut app.input {
            draw_input_popup(f, input);
        }
        return;
    }

    // main: left tree (ratio) | right tabs
    let left_w = ((rows[1].width as f64 * app.prefs.ratio) as u16)
        .clamp(10, rows[1].width.saturating_sub(20));
    let main = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(left_w), Constraint::Min(20)])
        .split(rows[1]);

    draw_tree_panel(f, app, main[0]);
    draw_right(f, app, main[1]);

    if let Some(input) = &mut app.input {
        draw_input_popup(f, input);
    }
}

fn draw_title(f: &mut Frame, app: &App, area: Rect) {
    let left = format!(" haltui — {}", app.title);
    let right = "F1/? help ";
    let width = area.width as usize - left.chars().count().min(area.width as usize);
    let mut line = Line::from(Span::styled(
        left,
        Style::default().fg(AYU_FG).add_modifier(Modifier::BOLD),
    ))
    .left_aligned();
    line.push_span(Span::styled(
        format!("{:>width$}", right, width = width),
        Style::default().fg(AYU_DIM),
    ));
    f.render_widget(Paragraph::new(line), area);
}

fn draw_status(f: &mut Frame, app: &App, area: Rect) {
    let style = if app.status_err {
        Style::default().fg(ERR)
    } else {
        Style::default().fg(AYU_YELLOW)
    };
    let text = clip(&app.status, area.width as usize);
    f.render_widget(Paragraph::new(Span::styled(text, style)), area);
}

fn draw_hints(f: &mut Frame, app: &App, area: Rect) {
    let hints = match (&app.focus, app.input.as_ref()) {
        (_, Some(input)) if input.kind == InputKind::BitPick => {
            " ←→ / ↑↓ toggle | t/f or 1/0 pick | Enter set | Esc cancel ".to_string()
        }
        (_, Some(_)) => " Enter ok | Esc cancel | Ctrl+U clear | F1 help ".to_string(),
        (Focus::Tree, None) => {
            " → expand closed / → pane if scrollable | ← collapse | Enter open/add | Space toggle/a leaf | a add | A add subtree | \
             s show | e/w expand/collapse all | E/W this type | / filter | f full-path | r reload | \
             F2 tree | [ ] resize ".to_string()
        }
        (Focus::Filter, None) => {
            " type = regex filter (live) | Ctrl+U clear | Esc/Enter done ".to_string()
        }
        (Focus::ShowText, None) => {
            " ← tree | ↑↓/PgUp/PgDn scroll | a add shown to watch | c command | F3 content | F4 command ".to_string()
        }
        (Focus::Command, None) => {
            " ← tree | Enter run halcmd | ↑↓ history | Esc back | Ctrl+U clear ".to_string()
        }
        (Focus::Watch, None) => {
            " ← tree | Space toggle bit | Enter set val | s set1 | c clr0 | u unlink | x remove | r reload | e erase | \
             a add | o show in tree | S/m save | L load (file dialog) ".to_string()
        }
        (Focus::Settings, None) => {
            " ← close + tree | Esc/F5 close | ↑↓ sel | Enter edit/toggle | Apply = Enter on last row ".to_string()
        }
    };
    let global = "1 SHOW  2 WATCH | / search | Tab next tab | F5 settings | q quit";
    let text = format!(" {hints} | {global}");
    f.render_widget(
        Paragraph::new(Span::styled(
            clip(&text, area.width as usize),
            Style::default().fg(AYU_FG).bg(AYU_PANEL),
        )),
        area,
    );
}

fn draw_tree_panel(f: &mut Frame, app: &mut App, area: Rect) {
    let block = ayu_block().title("Tree View");
    f.render_widget(block.clone(), area);
    let inner = block.inner(area);
    // list on top, filter entry on its own bottom row (no overlay, so the
    // list's auto-scroll keeps the selection truly visible)
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(inner);
    let list_area = rows[0];
    let filter_area = rows[1];
    let vis = tree::visible(&app.tree.roots);
    // viewport rows for paging
    app.tree_page = list_area.height.max(1) as usize;
    let sel = vis.iter().position(|(p, _, _)| *p == app.tree.selected);
    let items: Vec<ListItem> = vis
        .iter()
        .enumerate()
        .map(|(i, (path, depth, branch))| {
            let node = tree::find_node(&app.tree.roots, path);
            let indent = "  ".repeat(*depth);
            let marker = if *branch {
                if node.map(|n| n.expanded).unwrap_or(false) {
                    "▾ "
                } else {
                    "▸ "
                }
            } else {
                "  "
            };
            // selection pointer only when the node has children
            let cursor = if Some(i) == sel && *branch {
                "▶ "
            } else {
                "  "
            };
            let style = match node {
                Some(n) if n.depth_root() => Style::default().add_modifier(Modifier::BOLD),
                Some(n) if n.leaf => Style::default().fg(kind_color(n.kind)),
                _ => Style::default().fg(AYU_FG),
            };
            let label = node.map(|n| n.name.clone()).unwrap_or_default();
            ListItem::new(Line::from(Span::styled(
                format!("{cursor}{indent}{marker}{label}"),
                style,
            )))
        })
        .collect();
    if app.tree_list.selected().is_none() || app.tree_list.selected() != sel {
        app.tree_list.select(sel);
    }
    let list = List::new(items)
        .highlight_style(
            Style::default()
                .fg(AYU_FG)
                .bg(AYU_SEL_BG)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("");
    f.render_stateful_widget(list, list_area, &mut app.tree_list);

    // filter entry in its own row
    let filter_area = Rect {
        x: filter_area.x + 1,
        width: filter_area.width.saturating_sub(2),
        ..filter_area
    };
    let mut text = String::from("Filter: ");
    if app.tree.filter.is_empty() {
        text.push_str(&styled_placeholder());
        f.render_widget(
            Paragraph::new(Span::styled(text, Style::default().fg(AYU_DIM))),
            filter_area,
        );
    } else {
        text.push_str(&app.tree.filter);
        let style = if app.focus == Focus::Filter {
            Style::default().fg(AYU_FG).bg(AYU_SEL_BG)
        } else {
            Style::default().fg(AYU_FG)
        };
        f.render_widget(Paragraph::new(Span::styled(text, style)), filter_area);
    }
    if app.tree.full_path && filter_area.width > 14 {
        f.render_widget(
            Paragraph::new(Span::styled("[full-path]", Style::default().fg(AYU_TEAL))),
            Rect {
                x: filter_area.x + filter_area.width.saturating_sub(11),
                width: 11,
                ..filter_area
            },
        );
    }
}

fn draw_right(f: &mut Frame, app: &mut App, area: Rect) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(2), Constraint::Min(0)])
        .split(area);
    let tabs = Tabs::new(vec![" SHOW ", " WATCH "])
        .select(app.tab as usize)
        .style(Style::default().fg(AYU_DIM))
        .highlight_style(Style::default().fg(AYU_ACCENT).add_modifier(Modifier::BOLD))
        .divider("|");
    f.render_widget(tabs, rows[0]);
    match app.tab {
        Tab::Show => draw_show(f, app, rows[1]),
        Tab::Watch => draw_watch(f, app, rows[1]),
    }
}

fn draw_show(f: &mut Frame, app: &mut App, area: Rect) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(1)])
        .split(area);
    let block = ayu_block().title(" HAL show output ");
    let inner = block.inner(rows[0]);
    // viewport rows for paging
    app.show_page = inner.height.max(1) as usize;
    let lines: Vec<&str> = app.show_text.lines().collect();
    let total = lines.len();
    let max_scroll = total.saturating_sub(inner.height as usize);
    app.show_scroll = app.show_scroll.min(max_scroll);
    let text = lines
        .iter()
        .skip(app.show_scroll)
        .take(inner.height as usize)
        .cloned()
        .collect::<Vec<_>>()
        .join("\n");
    let para = Paragraph::new(text)
        .block(block)
        .wrap(Wrap { trim: false })
        .style(Style::default().fg(AYU_FG));
    f.render_widget(para, rows[0]);
    if total > 0 && inner.height as usize > 0 {
        let mut sb_state = ScrollbarState::new(total).position(app.show_scroll);
        let sb = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(Some("↑"))
            .end_symbol(Some("↓"))
            .thumb_symbol("┃");
        f.render_stateful_widget(sb, rows[0], &mut sb_state);
    }
    // command entry
    let mut cmd = String::from("HAL command: ");
    if app.command.is_empty() {
        cmd.push_str(&styled_placeholder_cmd());
    } else {
        cmd.push_str(&app.command);
        cmd.push(' ');
    }
    let style = if app.focus == Focus::Command {
        Style::default().fg(AYU_FG).bg(AYU_SEL_BG)
    } else {
        Style::default().fg(AYU_FG)
    };
    f.render_widget(
        Paragraph::new(Span::styled(clip(&cmd, rows[1].width as usize), style)),
        rows[1],
    );
}

fn draw_watch(f: &mut Frame, app: &mut App, area: Rect) {
    // viewport rows for paging (panel minus border)
    app.watch_page = area.height.saturating_sub(2).max(1) as usize;
    if app.watch.is_empty() {
        let hint = "Watchlist empty.\n<-- Select a leaf in the tree, press Enter (WATCH tab) or 'a'.\n    Press 'a' here to add by name.";
        f.render_widget(
            Paragraph::new(hint)
                .block(ayu_block().title(" WATCH "))
                .style(Style::default().fg(AYU_DIM)),
            area,
        );
        return;
    }
    let name_w = app.prefs.col1_width.clamp(8, 120) as usize;
    let widths = [
        Constraint::Length(name_w as u16),
        Constraint::Min(14),
        Constraint::Length(12),
    ];
    let rows: Vec<Row> = app.watch.iter().map(|w| watch_row(w, name_w)).collect();
    let table = Table::new(rows, widths)
        .block(ayu_block().title(" WATCH "))
        .column_spacing(1)
        .row_highlight_style(
            Style::default()
                .fg(AYU_FG)
                .bg(AYU_SEL_BG)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▶ ")
        .style(if app.focus == Focus::Watch {
            Style::default().fg(AYU_FG)
        } else {
            Style::default().fg(AYU_DIM)
        });
    let sel = app.watch_state.selected();
    if sel.is_none() || sel.unwrap_or(0) >= app.watch.len() {
        app.watch_state
            .select(if app.watch.is_empty() { None } else { Some(0) });
    }
    f.render_stateful_widget(table, area, &mut app.watch_state);
}

fn watch_row(w: &WatchItem, name_w: usize) -> Row<'static> {
    let name_color = match w.kind {
        HalType::Pin => NAME_PIN,
        HalType::Param => NAME_PARAM,
        HalType::Sig => NAME_SIG,
        _ => NAME_PIN,
    };
    // value indicator
    let indicator = if w.error {
        Span::styled("● ", Style::default().fg(AYU_DIM))
    } else if w.dtype == "bit" {
        let color = if w.value == "TRUE" {
            BIT_TRUE
        } else {
            BIT_FALSE
        };
        Span::styled("● ", Style::default().fg(color))
    } else {
        Span::styled("  ", Style::default().fg(AYU_FG))
    };
    let name = Span::styled(
        clip(&w.name, name_w.saturating_sub(2)),
        Style::default().fg(name_color),
    );
    let value = if w.error {
        Span::styled("----", Style::default().fg(AYU_DIM))
    } else if w.dtype == "bit" {
        let color = if w.value == "TRUE" {
            BIT_TRUE
        } else {
            BIT_FALSE
        };
        Span::styled(w.value.clone(), Style::default().fg(color))
    } else {
        Span::styled(w.value.clone(), Style::default().fg(AYU_FG))
    };
    let actions = match (w.dtype.as_str(), w.writable) {
        ("bit", 1) => "Set|Clr",
        ("bit", -1) => "unlink",
        ("bit", _) => "–",
        (_, 1) => "set val",
        (_, -1) => "unlink",
        _ => "–",
    };
    let action_span = Span::styled(
        actions,
        Style::default().fg(AYU_DIM).add_modifier(Modifier::DIM),
    );
    Row::new(vec![
        Cell::from(Line::from(vec![indicator, name])),
        Cell::from(value),
        Cell::from(Line::from(action_span)),
    ])
}

fn draw_settings(f: &mut Frame, app: &mut App, area: Rect) {
    let p = &app.prefs;
    let fmt_float = if app.ffmt_override.is_some() {
        format!("{} (disabled by --fformat)", p.ffmts)
    } else {
        p.ffmts.clone()
    };
    let fmt_int = if app.ifmt_override.is_some() {
        format!("{} (disabled by --iformat)", p.ifmts)
    } else {
        p.ifmts.clone()
    };
    let rows_text = [
        ("Update interval (in ms)", p.watch_interval.to_string()),
        (
            "Column width for value in watch tab",
            p.col1_width.to_string(),
        ),
        ("Override format string for Float", fmt_float),
        ("Override format string for Integer", fmt_int),
        ("Remember watchlist", bool_txt(p.auto_save_watchlist)),
        ("Apply", "".to_string()),
    ];
    let items: Vec<ListItem> = rows_text
        .iter()
        .map(|(label, val)| {
            let text = if *label == "Apply" {
                Line::from(Span::styled(
                    " Apply",
                    Style::default().fg(AYU_ACCENT).add_modifier(Modifier::BOLD),
                ))
            } else {
                Line::from(vec![
                    Span::styled(*label, Style::default().fg(AYU_FG)),
                    Span::styled(
                        format!(
                            "{:>width$}",
                            val,
                            width = (40usize).saturating_sub(label.len())
                        ),
                        Style::default().fg(AYU_TEAL),
                    ),
                ])
            };
            ListItem::new(text)
        })
        .collect();
    if app.settings_state.selected().is_none() {
        app.settings_state.select(Some(0));
    }
    let list = List::new(items)
        .block(ayu_block().title(" SETTINGS "))
        .highlight_style(
            Style::default()
                .fg(AYU_FG)
                .bg(AYU_SEL_BG)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▶ ")
        .style(if app.focus == Focus::Settings {
            Style::default().fg(AYU_FG)
        } else {
            Style::default().fg(AYU_DIM)
        });
    f.render_stateful_widget(list, area, &mut app.settings_state);
    // storage info
    let info = if app.use_prefs {
        format!("(Settings stored in: {})", app.prefs_path.display())
    } else {
        "\"--noprefs\" option set. Settings will not be saved!".to_string()
    };
    let style = if app.use_prefs {
        Style::default().fg(AYU_DIM)
    } else {
        Style::default().fg(AYU_ERROR)
    };
    let info_area = Rect {
        x: area.x + 2,
        y: area.y + area.height.saturating_sub(2),
        width: area.width.saturating_sub(4),
        height: 1,
    };
    f.render_widget(
        Paragraph::new(Span::styled(clip(&info, info_area.width as usize), style)),
        info_area,
    );
}

fn draw_input_popup(f: &mut Frame, input: &mut crate::app::InputState) {
    if input.kind == InputKind::FileDialog {
        draw_file_dialog(f, input);
        return;
    }
    let area = f.area();
    let (w, h) = match input.kind {
        InputKind::BitPick => ((area.width * 3 / 4).min(60), 7),
        InputKind::Text => ((area.width * 3 / 4).min(90), 5),
        InputKind::FileDialog => unreachable!("handled above"),
    };
    let rect = Rect {
        x: (area.width.saturating_sub(w)) / 2,
        y: (area.height.saturating_sub(h)) / 2,
        width: w,
        height: h,
    };
    f.render_widget(Clear, rect);
    f.render_widget(
        ayu_block()
            .title(format!(" {} ", input.prompt))
            .style(Style::default().bg(AYU_PANEL)),
        rect,
    );
    match input.kind {
        InputKind::BitPick => {
            let sel = Style::default().fg(AYU_ACCENT).add_modifier(Modifier::BOLD);
            let unsel = Style::default().fg(AYU_DIM);
            let row = |f: &mut Frame, y: u16, mark: &str, label: &str, style: Style| {
                let r = Rect {
                    x: rect.x + 4,
                    y: rect.y + y,
                    width: rect.width.saturating_sub(6),
                    height: 1,
                };
                f.render_widget(
                    Paragraph::new(Line::from(vec![
                        Span::styled(mark, style),
                        Span::styled(" ", style),
                        Span::styled(label, style),
                    ])),
                    r,
                );
            };
            let (t_style, f_style) = if input.bit_value {
                (sel, unsel)
            } else {
                (unsel, sel)
            };
            row(
                f,
                1,
                if input.bit_value { "(•)" } else { "( )" },
                "TRUE",
                t_style,
            );
            row(
                f,
                2,
                if !input.bit_value { "(•)" } else { "( )" },
                "FALSE",
                f_style,
            );
            let hint = Rect {
                x: rect.x + 2,
                y: rect.y + 4,
                width: rect.width.saturating_sub(4),
                height: 1,
            };
            f.render_widget(
                Paragraph::new(Span::styled(
                    " ←→ / ↑↓ toggle   t/f or 1/0 pick   Enter set   Esc cancel",
                    Style::default().fg(AYU_DIM),
                )),
                hint,
            );
        }
        InputKind::Text => {
            let inner = Rect {
                x: rect.x + 2,
                y: rect.y + 2,
                width: rect.width - 4,
                height: 1,
            };
            let mut text = input.buffer.clone();
            text.push(' ');
            let cursor_x = input.cursor.min(inner.width.saturating_sub(1) as usize);
            f.render_widget(
                Paragraph::new(Span::styled(text, Style::default().fg(AYU_FG))),
                inner,
            );
            f.set_cursor_position(Position::new(inner.x + cursor_x as u16, inner.y));
        }
        InputKind::FileDialog => unreachable!("handled above"),
    }
}

/// File open/save dialog: directory listing with a filter (load) or
/// file-name (save) field. The selection is scrolled to stay visible here so
/// the key handler doesn't need to know the viewport size.
fn draw_file_dialog(f: &mut Frame, input: &mut crate::app::InputState) {
    let area = f.area();
    let w = (area.width * 3 / 4).min(90);
    let h = (area.height.saturating_sub(2)).min(24);
    let rect = Rect {
        x: area.width.saturating_sub(w) / 2,
        y: area.height.saturating_sub(h) / 2,
        width: w,
        height: h,
    };
    f.render_widget(Clear, rect);
    f.render_widget(
        ayu_block()
            .title(format!(" {} ", input.prompt))
            .style(Style::default().bg(AYU_PANEL)),
        rect,
    );
    let Some(dialog) = &mut input.dialog else {
        return;
    };
    let inner = Rect {
        x: rect.x + 2,
        y: rect.y + 1,
        width: rect.width.saturating_sub(4),
        height: rect.height.saturating_sub(2),
    };
    // current directory
    f.render_widget(
        Paragraph::new(Span::styled(
            dialog.dir.display().to_string(),
            Style::default().fg(AYU_DIM),
        )),
        Rect {
            x: inner.x,
            y: inner.y,
            width: inner.width,
            height: 1,
        },
    );
    let list_top = inner.y + 1;
    let field_y = inner.y + inner.height - 2;
    let hint_y = inner.y + inner.height - 1;
    let list_h = field_y.saturating_sub(list_top) as usize;
    let n = dialog.entries.len();
    if n > 0 {
        let vis = list_h.max(1);
        if dialog.selected >= dialog.scroll + vis {
            dialog.scroll = dialog.selected + 1 - vis;
        }
        if dialog.selected < dialog.scroll {
            dialog.scroll = dialog.selected;
        }
        if dialog.scroll > n.saturating_sub(vis) {
            dialog.scroll = n.saturating_sub(vis);
        }
        for i in 0..vis {
            let idx = dialog.scroll + i;
            if idx >= n {
                break;
            }
            let y = list_top + i as u16;
            if y >= field_y {
                break;
            }
            let p = &dialog.entries[idx];
            let name = if p.ends_with("..") {
                // the parent-directory entry: file_name() is None for ".."
                "..".to_string()
            } else {
                p.file_name()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| p.to_string_lossy().into_owned())
            };
            let is_dir = p.is_dir();
            let label = if is_dir { format!("{name}/") } else { name };
            let selected = idx == dialog.selected;
            let style = if selected {
                Style::default().fg(AYU_FG).bg(AYU_SEL_BG)
            } else if is_dir {
                Style::default().fg(AYU_BLUE)
            } else {
                Style::default().fg(AYU_FG)
            };
            f.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::styled(
                        if is_dir { "▶ " } else { "  " },
                        Style::default().fg(AYU_DIM),
                    ),
                    Span::styled(label, style),
                ])),
                Rect {
                    x: inner.x,
                    y,
                    width: inner.width,
                    height: 1,
                },
            );
        }
    } else {
        f.render_widget(
            Paragraph::new(Span::styled(
                if dialog.save {
                    "(empty directory)"
                } else {
                    "no *.halshow files here"
                },
                Style::default().fg(AYU_DIM),
            )),
            Rect {
                x: inner.x,
                y: list_top,
                width: inner.width,
                height: 1,
            },
        );
    }
    // filter / file-name field
    let label = if dialog.save { "Name:  " } else { "Filter: " };
    let field_style = if dialog.field { AYU_FG } else { AYU_DIM };
    let mut field_text = input.buffer.clone();
    field_text.push(' ');
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                label,
                Style::default().fg(AYU_ACCENT).add_modifier(Modifier::BOLD),
            ),
            Span::styled(field_text, Style::default().fg(field_style)),
        ])),
        Rect {
            x: inner.x,
            y: field_y,
            width: inner.width,
            height: 1,
        },
    );
    if dialog.field {
        let lab_w = label.chars().count() as u16;
        let cursor_x = input.cursor.min(inner.width.saturating_sub(lab_w) as usize);
        f.set_cursor_position(Position::new(inner.x + lab_w + cursor_x as u16, field_y));
    }
    // hint (or error)
    let hint = match &dialog.error {
        Some(e) => (e.clone(), AYU_ERROR),
        None => (
            "↑↓ move   Enter open/save   ← parent   type to filter/name   Esc cancel".to_string(),
            AYU_DIM,
        ),
    };
    f.render_widget(
        Paragraph::new(Span::styled(hint.0, Style::default().fg(hint.1))),
        Rect {
            x: inner.x,
            y: hint_y,
            width: inner.width,
            height: 1,
        },
    );
}

// ----------------------------------------------------------------
// Help overlay: the full, self-documenting key map.

pub const HELP: &str = r#"haltui — TUI port of LinuxCNC halshow
=====================================

SEARCH THIS HELP
  /                   incremental search (case-insensitive substring)
  type               jump to first match, Enter = next match
  Esc                leave search

GLOBAL
  q / Ctrl+C          quit (settings + watchlist saved to the prefs file)
  F1 or ?             toggle this help
  Tab / Shift+Tab     next / previous tab
  1 / 2               jump to SHOW / WATCH tab
  F2                  focus tree panel
  F3                  focus current tab content
  F4                  focus HAL command entry
  F5                  open / close the settings screen
  /                   fresh search from any panel (clears previous filter)
  [ / ]               shrink / grow tree panel
  r                   reload (tree: refresh from HAL, watch: re-read writability)

TREE PANEL
  ↑ ↓                 move selection (live-previews in SHOW tab)
  PgUp / PgDn         page through the tree
  →                   expand closed node; open node jumps to the right pane
                      when useful (WATCH list, or scrollable SHOW output)
  ←                   collapse node / go to parent
  Space               toggle expand (adds a leaf to watch in WATCH tab)
  Enter               open node in SHOW tab (or add to watch in WATCH tab)
  a                   add selected leaf to watch list
  A                   add all leaves below selected node to watch list
  s                   show selected node in SHOW tab
  e / w               expand / collapse all
  E / W               expand / collapse current type (Pins, Signals, ...)
  /                   focus filter entry
  f                   toggle full-path regex matching
  Ctrl+U (filter)     clear filter

FILTER ENTRY
  type                live regex filter; matching branches auto-reveal
  Ctrl+U              clear filter
  Esc / Enter         return to tree (toggle full-path with 'f' back in the tree)

SHOW TAB
  ←                   back to tree view
  ↑ ↓ PgUp PgDn Home  scroll output
  a                   add shown item to watch list
  c                   focus HAL command entry

HAL COMMAND ENTRY
  ←                   back to tree view
  type                arbitrary halcmd command (e.g. "show sig estop")
  Enter               execute (output appears above)
  ↑ ↓                 command history
  Esc                 back to output
  Ctrl+U              clear line

WATCH TAB
  ←                   back to tree view
  ↑ ↓                 select item
  PgUp / PgDn         page through the list
  Space               toggle a writable bit value
  Enter               set value (writable items); unlink (linked items)
  s / c               set bit to 1 / clear bit to 0
  u                   unlink pin from signal
  x / Delete          remove item
  r                   reload watch (re-detect writability)
  e                   erase watch list
  a                   add item by name ("pin axis.0.pos" or just the name)
  o                   show item in tree + SHOW tab
  S                   save watch list via file dialog (halshow default format)
  m                   save watch list via file dialog (multiline format)
  L                   load watch list via file dialog (.halshow file)

  FILE DIALOG (S / m / L)
  ↑↓ / PgUp / PgDn    move through the directory listing (always)
  Enter               open a directory, or pick / save the selected file
  ← / Backspace       go to the parent directory (Backspace edits the name first)
  type                focus the filter / name field and type in it
  Tab                 toggle the field focus (Enter then confirms / opens)
  Esc                 cancel

  Newly added items are selected so they are scrolled into view.

SETTINGS SCREEN (F5)
  ←                   close + back to tree view
  Esc / F5            close + back to content
  ↑ ↓                 select field
  Enter / Space       edit value / toggle bools / Apply (last row)
  Values: update interval (ms), name column width, float format (%5.2f),
  integer format (%08x), remember watchlist.

INPUT PROMPT
  Enter               confirm
  Esc                 cancel
  Ctrl+U              clear (text inputs)

BIT VALUE DIALOG (writable bit pin/param/signal, Enter on a watch row)
  ←→ / ↑↓            toggle TRUE / FALSE
  t / f, 1 / 0       pick TRUE / FALSE directly
  Enter               set the chosen value
  Esc                 cancel

FILES
  Preferences:  $CONFIG_DIR/halshow.preferences, or the directory of the
  .ini of a running linuxcnc process, or ~/.halshow_preferences.
  Compatible with halshow.tcl (same format, both ways).

CLI
  haltui [--fformat F] [--iformat F] [--noprefs] [--interval MS] [watchfile]
"#;

fn draw_help(f: &mut Frame, app: &mut App) {
    let area = f.area();
    let w = (area.width * 4 / 5).min(96);
    let h = (area.height * 4 / 5).max(10);
    let rect = Rect {
        x: (area.width.saturating_sub(w)) / 2,
        y: (area.height.saturating_sub(h)) / 2,
        width: w,
        height: h,
    };
    f.render_widget(Clear, rect);
    let block = ayu_block()
        .title(" haltui help — F1 / ? / Esc closes ")
        .style(Style::default().bg(AYU_BG));
    let inner = block.inner(rect);
    let lines: Vec<&str> = HELP.lines().collect();
    let max_scroll = lines.len().saturating_sub(inner.height as usize);
    app.help_scroll = app.help_scroll.min(max_scroll);
    let query = app.help_search.clone();
    let text_lines: Vec<Line> = lines
        .iter()
        .skip(app.help_scroll)
        .take(inner.height as usize)
        .map(|l| help_line(l, &query))
        .collect();
    f.render_widget(
        Paragraph::new(text_lines)
            .block(block)
            .wrap(Wrap { trim: false }),
        rect,
    );
    if lines.len() > inner.height as usize {
        let mut sb_state = ScrollbarState::new(lines.len()).position(app.help_scroll);
        let sb = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(Some("↑"))
            .end_symbol(Some("↓"))
            .thumb_symbol("┃");
        f.render_stateful_widget(sb, rect, &mut sb_state);
    }
    let hint_area = Rect {
        x: area.x,
        y: area.y + area.height.saturating_sub(1),
        width: area.width,
        height: 1,
    };
    let hint = if app.help_search_on {
        format!(
            " search: {} | Enter next match | Esc back ",
            app.help_search
        )
    } else {
        " ↑↓/PgUp/PgDn scroll | / search | Enter next match | F1/?/Esc close ".to_string()
    };
    f.render_widget(
        Paragraph::new(Span::styled(
            clip(&hint, hint_area.width as usize),
            Style::default().fg(AYU_FG).bg(AYU_PANEL),
        )),
        hint_area,
    );
}

/// Style base for a help line: title/separator in accent, section
/// headers in teal, body in the default ayu foreground.
fn help_base_style(line: &str) -> Style {
    let t = line.trim_end();
    if t.starts_with("haltui —") || (!t.is_empty() && t.chars().all(|c| c == '=')) {
        return Style::default().fg(AYU_ACCENT).add_modifier(Modifier::BOLD);
    }
    if !t.starts_with(' ') && !t.is_empty() {
        // section header: non-indented line with an uppercase run ≥ 2
        let upper_run = t
            .chars()
            .take_while(|c| c.is_uppercase() || c.is_ascii_digit() || " /().'".contains(*c))
            .count();
        if upper_run >= 2 {
            return Style::default().fg(AYU_TEAL).add_modifier(Modifier::BOLD);
        }
    }
    Style::default().fg(AYU_FG)
}

/// Help line with base styling applied and every case-insensitive
/// occurrence of `query` highlighted.
fn help_line<'a>(line: &'a str, query: &str) -> Line<'a> {
    let base = help_base_style(line);
    if query.is_empty() || !line.is_ascii() || !query.is_ascii() {
        return Line::from(Span::styled(line, base));
    }
    let lower = line.to_ascii_lowercase();
    let q = query.to_ascii_lowercase();
    let mut spans: Vec<Span> = Vec::new();
    let mut pos = 0usize;
    while let Some(rel) = lower[pos..].find(&q) {
        let start = pos + rel;
        let end = start + q.len();
        if start > pos {
            spans.push(Span::styled(&line[pos..start], base));
        }
        spans.push(Span::styled(
            &line[start..end],
            base.fg(AYU_YELLOW).bg(AYU_MATCH_BG),
        ));
        pos = end;
    }
    if pos < line.len() {
        spans.push(Span::styled(&line[pos..], base));
    }
    Line::from(spans)
}

// ----------------------------------------------------------------

fn kind_color(kind: HalType) -> Color {
    match kind {
        HalType::Pin => NAME_PIN,
        HalType::Param => NAME_PARAM,
        HalType::Sig => NAME_SIG,
        _ => AYU_FG,
    }
}

impl TreeNode {
    fn depth_root(&self) -> bool {
        self.path.find(['+', '.']).is_none()
    }
}

fn bool_txt(b: bool) -> String {
    if b { "1" } else { "0" }.to_string()
}

fn clip(s: &str, width: usize) -> String {
    if s.chars().count() <= width {
        s.to_string()
    } else {
        s.chars().take(width.saturating_sub(1)).collect::<String>() + "…"
    }
}

fn styled_placeholder() -> String {
    "Filter tree (regex)".to_string()
}

fn styled_placeholder_cmd() -> String {
    "e.g. show sig estop".to_string()
}
