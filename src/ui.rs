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

const NAME_PIN: Color = Color::Reset;
const NAME_PARAM: Color = Color::Rgb(110, 52, 0); // #6e3400, halshow param color
const NAME_SIG: Color = Color::Rgb(0, 0, 205); // blue3
const BIT_TRUE: Color = Color::Yellow;
const BIT_FALSE: Color = Color::Rgb(139, 26, 26); // firebrick4
const ERR: Color = Color::Rgb(255, 80, 80);

pub fn draw(f: &mut Frame, app: &mut App) {
    if app.help {
        draw_help(f, app);
        return;
    }
    let area = f.area();
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
        if let Some(input) = &app.input {
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

    if let Some(input) = &app.input {
        draw_input_popup(f, input);
    }
}

fn draw_title(f: &mut Frame, app: &App, area: Rect) {
    let left = format!(" haltui — {}", app.title);
    let right = "F1/? help ";
    let width = area.width as usize - left.chars().count().min(area.width as usize);
    let mut line = Line::from(left).left_aligned();
    line.push_span(Span::styled(
        format!("{:>width$}", right, width = width),
        Style::default().add_modifier(Modifier::DIM),
    ));
    f.render_widget(Paragraph::new(line), area);
}

fn draw_status(f: &mut Frame, app: &App, area: Rect) {
    let style = if app.status_err {
        Style::default().fg(ERR)
    } else {
        Style::default().fg(Color::Yellow)
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
            " ↑↓ nav | → expand | ← collapse | Enter open/add | Space toggle | a add | A add subtree | \
             s show | e/w expand/collapse all | E/W this type | / filter | f full-path | r reload | \
             F2 tree | [ ] resize ".to_string()
        }
        (Focus::Filter, None) => {
            " type = regex filter (live) | f full-path | Ctrl+U clear | Esc/Enter done ".to_string()
        }
        (Focus::ShowText, None) => {
            " ← tree | ↑↓/PgUp/PgDn scroll | a add shown to watch | c command | F3 content | F4 command ".to_string()
        }
        (Focus::Command, None) => {
            " ← tree | Enter run halcmd | ↑↓ history | Esc back | Ctrl+U clear ".to_string()
        }
        (Focus::Watch, None) => {
            " ← tree | ↑↓ sel | Enter set val | s set1 | c clr0 | u unlink | x remove | r reload | e erase | \
             a add | o show in tree | S save | m save multiline | L load ".to_string()
        }
        (Focus::Settings, None) => {
            " ← close + tree | Esc/F5 close | ↑↓ sel | Enter edit/toggle | Apply = Enter on last row ".to_string()
        }
    };
    let global = "1 SHOW  2 WATCH | Tab next tab | F5 settings | q quit";
    let text = format!(" {hints} | {global}");
    f.render_widget(
        Paragraph::new(Span::styled(
            clip(&text, area.width as usize),
            Style::default().fg(Color::Black).bg(Color::Gray),
        )),
        area,
    );
}

fn draw_tree_panel(f: &mut Frame, app: &mut App, area: Rect) {
    let block = Block::bordered().title("Tree View");
    let vis = tree::visible(&app.tree.roots);
    let items: Vec<ListItem> = vis
        .iter()
        .map(|(path, depth, branch)| {
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
            let style = match node {
                Some(n) if n.depth_root() => Style::default().add_modifier(Modifier::BOLD),
                Some(n) if n.leaf => Style::default().fg(kind_color(n.kind)),
                _ => Style::default(),
            };
            let label = node.map(|n| n.name.clone()).unwrap_or_default();
            ListItem::new(Line::from(Span::styled(
                format!("{indent}{marker}{label}"),
                style,
            )))
        })
        .collect();
    let sel = vis.iter().position(|(p, _, _)| *p == app.tree.selected);
    if app.tree_list.selected().is_none() || app.tree_list.selected() != sel {
        app.tree_list.select(sel);
    }
    let list = List::new(items)
        .block(block)
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▶ ");
    f.render_stateful_widget(list, area, &mut app.tree_list);

    // filter entry at the bottom of the tree panel
    let filter_area = Rect {
        x: area.x + 1,
        y: area.y + area.height.saturating_sub(2),
        width: area.width.saturating_sub(2),
        height: 1,
    };
    let mut text = String::from("Filter: ");
    if app.tree.filter.is_empty() {
        text.push_str(&styled_placeholder());
        let style = if app.focus == Focus::Filter {
            Style::default().fg(Color::Gray)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        f.render_widget(Paragraph::new(Span::styled(text, style)), filter_area);
    } else {
        text.push_str(&app.tree.filter);
        let style = if app.focus == Focus::Filter {
            Style::default().fg(Color::White).bg(Color::DarkGray)
        } else {
            Style::default().fg(Color::White)
        };
        f.render_widget(Paragraph::new(Span::styled(text, style)), filter_area);
    }
    if app.tree.full_path && filter_area.width > 14 {
        f.render_widget(
            Paragraph::new(Span::styled(
                "[full-path]",
                Style::default().fg(Color::Cyan),
            )),
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
        .highlight_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
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
    let block = Block::bordered().title(" HAL show output ");
    let inner = block.inner(rows[0]);
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
        .style(if app.focus == Focus::ShowText {
            Style::default().fg(Color::White)
        } else {
            Style::default().fg(Color::Gray)
        });
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
        Style::default().fg(Color::White).bg(Color::DarkGray)
    } else {
        Style::default().fg(Color::White)
    };
    f.render_widget(
        Paragraph::new(Span::styled(clip(&cmd, rows[1].width as usize), style)),
        rows[1],
    );
}

fn draw_watch(f: &mut Frame, app: &mut App, area: Rect) {
    if app.watch.is_empty() {
        let hint = "Watchlist empty.\n<-- Select a leaf in the tree, press Enter (WATCH tab) or 'a'.\n    Press 'a' here to add by name.";
        f.render_widget(
            Paragraph::new(hint)
                .block(Block::bordered().title(" WATCH "))
                .style(Style::default().fg(Color::DarkGray)),
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
        .block(Block::bordered().title(" WATCH "))
        .column_spacing(1)
        .row_highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▶ ")
        .style(if app.focus == Focus::Watch {
            Style::default().fg(Color::White)
        } else {
            Style::default().fg(Color::Gray)
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
        Span::styled("● ", Style::default().fg(Color::DarkGray))
    } else if w.dtype == "bit" {
        let color = if w.value == "TRUE" {
            BIT_TRUE
        } else {
            BIT_FALSE
        };
        Span::styled("● ", Style::default().fg(color))
    } else {
        Span::raw("  ")
    };
    let name = Span::styled(
        clip(&w.name, name_w.saturating_sub(2)),
        Style::default().fg(name_color),
    );
    let value = if w.error {
        Span::styled("----", Style::default().fg(Color::DarkGray))
    } else if w.dtype == "bit" {
        let color = if w.value == "TRUE" {
            BIT_TRUE
        } else {
            BIT_FALSE
        };
        Span::styled(w.value.clone(), Style::default().fg(color))
    } else {
        Span::raw(w.value.clone())
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
        Style::default().fg(Color::Gray).add_modifier(Modifier::DIM),
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
                    Style::default().add_modifier(Modifier::BOLD),
                ))
            } else {
                Line::from(vec![
                    Span::raw(*label),
                    Span::styled(
                        format!(
                            "{:>width$}",
                            val,
                            width = (40usize).saturating_sub(label.len())
                        ),
                        Style::default().fg(Color::Cyan),
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
        .block(Block::bordered().title(" SETTINGS "))
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▶ ")
        .style(if app.focus == Focus::Settings {
            Style::default().fg(Color::White)
        } else {
            Style::default().fg(Color::Gray)
        });
    f.render_stateful_widget(list, area, &mut app.settings_state);
    // storage info
    let info = if app.use_prefs {
        format!("(Settings stored in: {})", app.prefs_path.display())
    } else {
        "\"--noprefs\" option set. Settings will not be saved!".to_string()
    };
    let style = if app.use_prefs {
        Style::default().fg(Color::DarkGray)
    } else {
        Style::default().fg(Color::Red)
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

fn draw_input_popup(f: &mut Frame, input: &crate::app::InputState) {
    let area = f.area();
    let (w, h) = match input.kind {
        InputKind::BitPick => ((area.width * 3 / 4).min(60), 7),
        InputKind::Text => ((area.width * 3 / 4).min(90), 5),
    };
    let rect = Rect {
        x: (area.width.saturating_sub(w)) / 2,
        y: (area.height.saturating_sub(h)) / 2,
        width: w,
        height: h,
    };
    f.render_widget(Clear, rect);
    f.render_widget(
        Block::bordered()
            .title(format!(" {} ", input.prompt))
            .style(Style::default().bg(Color::Black)),
        rect,
    );
    match input.kind {
        InputKind::BitPick => {
            let sel = Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD);
            let unsel = Style::default().fg(Color::Gray);
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
                        Span::raw(" "),
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
                    Style::default().fg(Color::DarkGray),
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
                Paragraph::new(Span::styled(text, Style::default().fg(Color::White))),
                inner,
            );
            f.set_cursor_position(Position::new(inner.x + cursor_x as u16, inner.y));
        }
    }
}

// ----------------------------------------------------------------
// Help overlay: the full, self-documenting key map.

const HELP: &str = r#"haltui — TUI port of LinuxCNC halshow
=====================================

GLOBAL
  q / Ctrl+C          quit (settings + watchlist saved to the prefs file)
  F1 or ?             toggle this help
  Tab / Shift+Tab     next / previous tab
  1 / 2               jump to SHOW / WATCH tab
  F2                  focus tree panel
  F3                  focus current tab content
  F4                  focus HAL command entry
  F5                  open / close the settings screen
  [ / ]               shrink / grow tree panel
  r                   reload (tree: refresh from HAL, watch: re-read writability)

TREE PANEL
  ↑ ↓                 move selection
  →                   expand node / enter first child
  ←                   collapse node / go to parent
  Space               toggle expand
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
  type                live regex filter on node names (or full path with 'f')
  f                   toggle full-path matching
  Ctrl+U              clear filter
  Esc / Enter         return to tree

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
  Enter               set value (writable items); unlink (linked items)
  s / c               set bit to 1 / clear bit to 0
  u                   unlink pin from signal
  x / Delete          remove item
  r                   reload watch (re-detect writability)
  e                   erase watch list
  a                   add item by name ("pin axis.0.pos" or just the name)
  o                   show item in tree + SHOW tab
  S                   save watch list (one line per file, halshow default)
  m                   save watch list (multiline format)
  L                   load watch list (.halshow file)

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
    let block = Block::bordered().title(" haltui help — F1 / ? / Esc closes ");
    let inner = block.inner(rect);
    let lines: Vec<&str> = HELP.lines().collect();
    let max_scroll = lines.len().saturating_sub(inner.height as usize);
    app.help_scroll = app.help_scroll.min(max_scroll);
    let text = lines
        .iter()
        .skip(app.help_scroll)
        .take(inner.height as usize)
        .cloned()
        .collect::<Vec<_>>()
        .join("\n");
    f.render_widget(
        Paragraph::new(text).block(block).wrap(Wrap { trim: false }),
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
    f.render_widget(
        Paragraph::new(Span::styled(
            " ↑↓/PgUp/PgDn scroll | F1/?/Esc close ",
            Style::default().fg(Color::Black).bg(Color::Gray),
        )),
        hint_area,
    );
}

// ----------------------------------------------------------------

fn kind_color(kind: HalType) -> Color {
    match kind {
        HalType::Pin => NAME_PIN,
        HalType::Param => NAME_PARAM,
        HalType::Sig => NAME_SIG,
        _ => Color::Reset,
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
