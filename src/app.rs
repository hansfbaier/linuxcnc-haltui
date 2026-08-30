//! Application state, event loop and key handling.

use std::io;
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use ratatui::backend::Backend;
use ratatui::widgets::{ListState, TableState};
use ratatui::Terminal;

use crate::fmt::{self, FmtArg};
use crate::hal::{self, HalSession, HalType};
use crate::prefs::{self, Prefs, Workmode};
use crate::tree::{self, HalTree};
use crate::ui;
use crate::watch::WatchItem;

pub struct Cli {
    pub fformat: Option<String>,
    pub iformat: Option<String>,
    pub noprefs: bool,
    pub interval: Option<u64>,
    pub watchfile: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tab {
    Show,
    Watch,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Focus {
    Tree,
    Filter,
    ShowText,
    Command,
    Watch,
    Settings,
}

#[derive(Debug)]
pub enum InputAction {
    SetValue(usize),
    AddWatch,
    LoadWatchFile,
    SaveWatchFile(bool),
    SetSetting(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputKind {
    Text,
    BitPick,
    FileDialog,
}

/// State for the file open/save dialog. `entries` is refreshed from disk via
/// [`refresh_dialog`]; each entry is a full path (dirs and files).
#[derive(Debug)]
pub struct FileDialog {
    pub save: bool,
    pub dir: PathBuf,
    pub entries: Vec<PathBuf>,
    pub selected: usize,
    pub scroll: usize,
    /// true = editing the filter (load) / file-name (save) field
    pub field: bool,
    pub error: Option<String>,
}

#[derive(Debug)]
pub struct InputState {
    pub kind: InputKind,
    pub prompt: String,
    pub buffer: String,
    pub cursor: usize,
    /// selected radio in BitPick mode
    pub bit_value: bool,
    pub action: InputAction,
    pub dialog: Option<FileDialog>,
}

impl InputState {
    fn text(prompt: impl Into<String>, buffer: String, action: InputAction) -> Self {
        InputState {
            kind: InputKind::Text,
            prompt: prompt.into(),
            cursor: buffer.chars().count(),
            buffer,
            bit_value: false,
            action,
            dialog: None,
        }
    }

    fn bit_pick(prompt: impl Into<String>, bit_value: bool, action: InputAction) -> Self {
        InputState {
            kind: InputKind::BitPick,
            prompt: prompt.into(),
            buffer: String::new(),
            cursor: 0,
            bit_value,
            action,
            dialog: None,
        }
    }

    /// Open the file dialog. `name` seeds the filter field (load) or the
    /// file-name field (save).
    fn file_dialog(save: bool, dir: PathBuf, name: String, action: InputAction) -> Self {
        let mut dialog = FileDialog {
            save,
            dir,
            entries: Vec::new(),
            selected: 0,
            scroll: 0,
            // load: start browsing the list; save: start with the name field
            field: save,
            error: None,
        };
        refresh_dialog(&mut dialog, &name);
        InputState {
            kind: InputKind::FileDialog,
            prompt: if save {
                "Save watch list".to_string()
            } else {
                "Open watch list".to_string()
            },
            cursor: name.chars().count(),
            buffer: name,
            bit_value: false,
            action,
            dialog: Some(dialog),
        }
    }
}

/// Pad a numeric string so positive values occupy the same width as negative
/// ones: a space stands in for the `-` sign. Custom formats that already
/// produce their own sign or width (leading space / `+`) are left untouched.
fn sign_pad(s: &str) -> String {
    if s.starts_with('-') || s.starts_with('+') || s.starts_with(' ') {
        s.to_string()
    } else {
        format!(" {s}")
    }
}

/// Lexically resolve `.` / `..` components so a navigated directory path
/// doesn't accumulate `..` (e.g. `/a/b/..`). Never touches the filesystem.
fn norm_path(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    if out.as_os_str().is_empty() {
        if p.is_absolute() {
            PathBuf::from("/")
        } else {
            PathBuf::from(".")
        }
    } else {
        out
    }
}

/// Re-scan the dialog's directory into `entries`: ".." (unless at the root),
/// then subdirectories, then files (`.halshow` only on load). In load mode the
/// `filter` narrows the file list; in save mode files are not filtered.
fn refresh_dialog(dialog: &mut FileDialog, filter: &str) {
    dialog.dir = norm_path(&dialog.dir);
    let mut dirs: Vec<String> = Vec::new();
    let mut files: Vec<String> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&dialog.dir) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
            if is_dir {
                dirs.push(name);
            } else if dialog.save || name.to_lowercase().ends_with(".halshow") {
                files.push(name);
            }
        }
    }
    dirs.sort();
    files.sort();
    let fl = filter.trim().to_lowercase();
    let show_all = fl.is_empty();
    let mut entries: Vec<PathBuf> = Vec::new();
    if dialog.dir.parent().is_some() {
        entries.push(dialog.dir.join(".."));
    }
    for d in &dirs {
        entries.push(dialog.dir.join(d));
    }
    if dialog.save {
        for f in &files {
            entries.push(dialog.dir.join(f));
        }
    } else {
        for f in files
            .iter()
            .filter(|f| show_all || f.to_lowercase().contains(&fl))
        {
            entries.push(dialog.dir.join(f));
        }
    }
    dialog.entries = entries;
    if dialog.selected >= dialog.entries.len() {
        dialog.selected = dialog.entries.len().saturating_sub(1);
    }
    if dialog.scroll > dialog.entries.len().saturating_sub(1) {
        dialog.scroll = dialog.entries.len().saturating_sub(1);
    }
    dialog.error = None;
}

pub struct App {
    pub hal: HalSession,
    pub prefs: Prefs,
    pub prefs_path: PathBuf,
    pub prefs_dir: Option<PathBuf>,
    pub use_prefs: bool,
    pub ffmt_override: Option<String>,
    pub ifmt_override: Option<String>,

    pub tree: HalTree,
    pub tree_list: ListState,
    /// rows visible in the tree viewport (updated by the renderer)
    pub tree_page: usize,
    pub watch: Vec<WatchItem>,
    pub watch_state: TableState,
    /// rows visible in the watch viewport (updated by the renderer)
    pub watch_page: usize,
    pub settings_state: ListState,

    pub tab: Tab,
    pub settings_open: bool,
    pub focus: Focus,
    pub input: Option<InputState>,

    pub show_text: String,
    pub show_scroll: usize,
    /// rows visible in the SHOW text viewport (updated by the renderer)
    pub show_page: usize,
    pub shown_node: Option<(HalType, String)>,

    pub command: String,
    pub hist: Vec<String>,
    pub hist_idx: Option<usize>,

    pub status: String,
    pub status_err: bool,
    pub help: bool,
    pub help_scroll: usize,
    /// incremental search inside the help overlay
    pub help_search: String,
    pub help_search_on: bool,
    pub quit: bool,

    pub last_watch_dir: PathBuf,
    pub last_watch_tail: String,
    pub title: String,

    last_tick: Instant,
}

impl App {
    pub fn new(cli: &Cli) -> Self {
        let (prefs_path, prefs_dir) = prefs::locate();
        let prefs = if cli.noprefs {
            Prefs::default()
        } else {
            prefs::read(&prefs_path)
        };
        let interval = cli.interval.unwrap_or(prefs.watch_interval).max(20);
        let (tab, settings_open) = match prefs.workmode {
            Workmode::Watch => (Tab::Watch, false),
            Workmode::Settings => (Tab::Show, true),
            Workmode::Show => (Tab::Show, false),
        };
        let mut app = App {
            hal: HalSession::new(),
            prefs,
            prefs_path,
            prefs_dir,
            use_prefs: !cli.noprefs,
            ffmt_override: cli.fformat.clone(),
            ifmt_override: cli.iformat.clone(),
            tree: HalTree::new(),
            tree_list: ListState::default(),
            tree_page: 10,
            watch: Vec::new(),
            watch_state: TableState::default(),
            watch_page: 10,
            settings_state: ListState::default(),
            tab,
            settings_open,
            focus: Focus::Tree,
            input: None,
            show_text: String::new(),
            show_scroll: 0,
            show_page: 10,
            shown_node: None,
            command: String::new(),
            hist: Vec::new(),
            hist_idx: None,
            status: "Commands may be tested here but they will NOT be saved".to_string(),
            status_err: false,
            help: false,
            help_scroll: 0,
            help_search: String::new(),
            help_search_on: false,
            quit: false,
            last_watch_dir: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            last_watch_tail: "my.halshow".to_string(),
            title: "Halshow".to_string(),
            last_tick: Instant::now(),
        };
        app.prefs.watch_interval = interval;
        app
    }

    /// Initial HAL/tree/watchlist load.
    pub fn startup(&mut self, cli: &Cli) {
        self.hal.ensure();
        if let Some(f) = &cli.watchfile {
            if Path::new(f).is_file() {
                self.load_watch_file(f);
            } else {
                self.set_status(format!("Cannot read file <{f}>"), true);
            }
        } else if self.use_prefs && self.prefs.auto_save_watchlist {
            let saved: Vec<(Option<HalType>, String)> = self
                .prefs
                .watchlist
                .iter()
                .map(|s| match s.split_once('+') {
                    Some((k, n)) => (HalType::from_kw(k), n.to_string()),
                    None => (None, s.clone()),
                })
                .collect();
            for (kind, name) in saved {
                match kind {
                    Some(k) => {
                        self.watch_add(k, &name, false);
                    }
                    None => {
                        if let Some(k) = self.guess_kind(&name) {
                            self.watch_add(k, &name, false);
                        }
                    }
                }
            }
        }
        self.tree.rebuild(&self.hal);
        if self.tab == Tab::Watch {
            self.poll_watch();
        }
    }

    pub fn set_status(&mut self, msg: String, err: bool) {
        self.status = msg;
        self.status_err = err;
    }

    /// Poll watch values when the interval elapsed.
    pub fn poll_if_due(&mut self) {
        if self.last_tick.elapsed() >= Duration::from_millis(self.prefs.watch_interval.max(20)) {
            self.last_tick = Instant::now();
            if self.tab == Tab::Watch {
                self.poll_watch();
            }
        }
    }

    /// Next tick deadline base (for the main loop).
    pub fn last_tick_instant(&self) -> Instant {
        self.last_tick
    }

    pub fn shutdown(&mut self) {
        if self.use_prefs {
            self.prefs.workmode = if self.settings_open {
                Workmode::Settings
            } else {
                match self.tab {
                    Tab::Show => Workmode::Show,
                    Tab::Watch => Workmode::Watch,
                }
            };
            if self.prefs.auto_save_watchlist {
                self.prefs.watchlist = self.watch.iter().map(|w| w.file_id()).collect();
            }
            if let Err(e) = prefs::write(&self.prefs_path, &self.prefs) {
                eprintln!(
                    "haltui: unable to save settings to {}: {e}",
                    self.prefs_path.display()
                );
            }
        }
        self.hal.kill();
    }

    // ------------------------------------------------------------
    // Watch list operations

    fn guess_kind(&mut self, name: &str) -> Option<HalType> {
        // pin/param via ptype, else sig via stype
        let p = self.hal.batch(&[format!("ptype {name}")]).pop()?;
        if !p.err {
            return Some(HalType::Pin); // could be a param; ptype covers both
        }
        let s = self.hal.batch(&[format!("stype {name}")]).pop()?;
        if !s.err {
            return Some(HalType::Sig);
        }
        None
    }

    fn dtype_of(&mut self, kind: HalType, name: &str) -> Result<String, String> {
        let cmd = match kind {
            HalType::Sig => format!("stype {name}"),
            _ => format!("ptype {name}"),
        };
        let out = self.hal.batch(&[cmd]);
        match out.first() {
            Some(o) if !o.err => Ok(o.line.clone()),
            Some(o) => Err(o.line.clone()),
            None => Err("halcmd unavailable".to_string()),
        }
    }

    fn writable_of(&mut self, kind: HalType, name: &str) -> i8 {
        let show = self.hal.show(kind, name);
        match kind {
            HalType::Pin => hal::pin_writable(&show, name),
            HalType::Param => hal::param_writable(&show, name),
            HalType::Sig => hal::sig_writable(&show),
            _ => 0,
        }
    }

    /// Add item to watch list. Returns true on success.
    pub fn watch_add(&mut self, kind: HalType, name: &str, verbose: bool) -> bool {
        if !kind.watchable() {
            if verbose {
                self.set_status(format!("cannot watch {}s", kind.kw()), true);
            }
            return false;
        }
        if self.watch.iter().any(|w| w.kind == kind && w.name == name) {
            if verbose {
                self.set_status(format!("'{name}' already in list"), true);
            }
            return false;
        }
        let dtype = match self.dtype_of(kind, name) {
            Ok(d) => d,
            Err(e) => {
                if verbose {
                    self.set_status(format!("'{name}': {e}"), true);
                }
                return false;
            }
        };
        let writable = self.writable_of(kind, name);
        self.watch.push(WatchItem {
            kind,
            name: name.to_string(),
            dtype,
            writable,
            value: String::new(),
            error: false,
        });
        // select the new item so the watch table scrolls it into view
        self.watch_state.select(Some(self.watch.len() - 1));
        if verbose {
            self.set_status(format!("'{name}' added"), false);
        }
        true
    }

    fn poll_watch(&mut self) {
        if self.watch.is_empty() {
            return;
        }
        let cmds: Vec<String> = self
            .watch
            .iter()
            .map(|w| match w.kind {
                HalType::Sig => format!("gets {}", w.name),
                _ => format!("getp {}", w.name),
            })
            .collect();
        let outs = self.hal.batch(&cmds);
        // format outside the borrow of self.watch
        let formatted: Vec<(bool, String)> = self
            .watch
            .iter()
            .zip(outs.iter())
            .map(|(w, o)| {
                if o.err {
                    (true, "----".to_string())
                } else {
                    (false, self.format_value(&w.dtype, &o.line))
                }
            })
            .collect();
        for (w, (err, val)) in self.watch.iter_mut().zip(formatted) {
            w.error = err;
            w.value = val;
        }
    }

    fn format_value(&self, dtype: &str, raw: &str) -> String {
        if dtype == "bit" {
            return raw.to_string();
        }
        let fmt_override = if dtype == "float" || dtype == "hal_float" {
            self.ffmt_override.as_deref()
        } else {
            self.ifmt_override.as_deref()
        };
        if let Some(f) = fmt_override {
            if let Ok(v) = raw.parse::<f64>() {
                let out = if dtype == "float" || dtype == "hal_float" {
                    fmt::apply(f, &FmtArg::Float(v))
                } else {
                    fmt::apply(f, &FmtArg::Int(v as i64))
                };
                return sign_pad(&out);
            }
            return sign_pad(raw);
        }
        if dtype == "float" || dtype == "hal_float" {
            if !self.prefs.ffmts.is_empty() {
                if let Ok(v) = raw.parse::<f64>() {
                    return sign_pad(&fmt::apply(&self.prefs.ffmts, &FmtArg::Float(v)));
                }
            }
        } else if matches!(dtype, "s32" | "u32") && !self.prefs.ifmts.is_empty() {
            if let Ok(v) = raw.parse::<i64>() {
                return sign_pad(&fmt::apply(&self.prefs.ifmts, &FmtArg::Int(v)));
            }
        }
        sign_pad(raw)
    }

    pub fn watch_set(&mut self, idx: usize, val: &str) {
        if idx >= self.watch.len() {
            return;
        }
        let (kind, name) = {
            let w = &self.watch[idx];
            (w.kind, w.name.clone())
        };
        let cmd = match kind {
            HalType::Sig => "sets",
            _ => "setp",
        };
        let out = HalSession::exec(&[cmd, &name, val]);
        self.set_status(
            out.trim().to_string(),
            out.contains("ERROR") || out.contains("not found"),
        );
        self.poll_watch();
    }

    pub fn watch_unlink(&mut self, idx: usize) {
        if idx >= self.watch.len() {
            return;
        }
        let (kind, name) = {
            let w = &self.watch[idx];
            (w.kind, w.name.clone())
        };
        let _ = HalSession::exec(&["unlinkp", &name]);
        // halcmd's unlinkp prints nothing on success in some versions —
        // detect the result by re-reading the pin's writability instead
        let new_writable = self.writable_of(kind, &name);
        self.watch[idx].writable = new_writable;
        if new_writable != -1 {
            self.set_status(format!("'{name}' unlinked"), false);
        } else {
            self.set_status(format!("could not unlink '{name}'"), true);
        }
    }

    pub fn watch_remove(&mut self, idx: usize) {
        if idx >= self.watch.len() {
            return;
        }
        let name = self.watch[idx].name.clone();
        self.watch.remove(idx);
        if self
            .watch_state
            .selected()
            .is_some_and(|s| s >= self.watch.len())
            && !self.watch.is_empty()
        {
            self.watch_state.select(Some(self.watch.len() - 1));
        }
        self.set_status(format!("'{name}' removed from list"), false);
    }

    pub fn watch_erase(&mut self) {
        self.watch.clear();
        self.watch_state.select(None);
        self.set_status("Watchlist cleared".to_string(), false);
    }

    pub fn reload_watch(&mut self) {
        let items: Vec<(HalType, String)> = self
            .watch
            .iter()
            .map(|w| (w.kind, w.name.clone()))
            .collect();
        let sel = self.watch_state.selected();
        self.watch.clear();
        for (k, n) in items {
            self.watch_add(k, &n, false);
        }
        // preserve the previous selection (watch_add selects the new item)
        if let Some(s) = sel {
            self.watch_state
                .select(Some(s.min(self.watch.len().saturating_sub(1))));
        }
        self.poll_watch();
    }

    // ------------------------------------------------------------
    // Watch file load/save

    pub fn load_watch_file(&mut self, path: &str) {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) => {
                self.set_status(format!("Cannot read <{path}>: {e}"), true);
                return;
            }
        };
        // backup auto-saved watchlist
        let backup = self
            .prefs_dir
            .as_ref()
            .map(|d| d.join(".halshow_watchlist_backup"));
        if let Some(b) = &backup {
            let _ = std::fs::write(b, crate::watch::file_text(&self.watch, true));
        }
        self.watch.clear();
        self.watch_state.select(None);
        for (kind, name) in crate::watch::parse_file(&text) {
            match kind {
                Some(k) => {
                    self.watch_add(k, &name, false);
                }
                None => {
                    if let Some(k) = self.guess_kind(&name) {
                        self.watch_add(k, &name, false);
                    }
                }
            }
        }
        self.tab = Tab::Watch;
        self.focus = Focus::Watch;
        let p = Path::new(path);
        self.last_watch_tail = p
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string());
        self.last_watch_dir = p
            .parent()
            .map(|d| d.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));
        self.title = self.last_watch_tail.clone();
        let backup_note = match &backup {
            Some(b) => format!(", saved backup for old watchlist in {}", b.display()),
            None => String::new(),
        };
        self.set_status(
            format!("{} loaded{backup_note}", self.last_watch_tail),
            false,
        );
        self.poll_watch();
    }

    fn save_watch_prompt(&mut self, multiline: bool) {
        if self.watch.is_empty() {
            self.set_status("Watchlist empty, nothing to save".to_string(), true);
            return;
        }
        let dir = if self.last_watch_dir.as_os_str().is_empty() {
            PathBuf::from(".")
        } else {
            self.last_watch_dir.clone()
        };
        self.input = Some(InputState::file_dialog(
            true,
            dir,
            self.last_watch_tail.clone(),
            InputAction::SaveWatchFile(multiline),
        ));
    }

    pub fn save_watch_file(&mut self, path: &str, multiline: bool) {
        let text = crate::watch::file_text(&self.watch, multiline);
        if let Err(e) = std::fs::write(path, text) {
            self.set_status(format!("Cannot write <{path}>: {e}"), true);
            return;
        }
        let p = Path::new(path);
        self.last_watch_tail = p
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string());
        self.last_watch_dir = p
            .parent()
            .map(|d| d.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));
        self.title = self.last_watch_tail.clone();
        self.set_status(format!("{} saved", self.last_watch_tail), false);
    }

    // ------------------------------------------------------------
    // SHOW tab

    pub fn show_node(&mut self, kind: HalType, name: &str) {
        self.shown_node = Some((kind, name.to_string()));
        self.show_text = self.hal.show(kind, name);
        self.show_scroll = 0;
    }

    /// Live-preview the current tree selection in the SHOW tab (no Enter
    /// needed). Roots show the whole type list, matching halshow's click
    /// behaviour.
    fn preview_selection(&mut self) {
        if self.tab != Tab::Show {
            return;
        }
        let sel = self
            .tree
            .selected_node()
            .map(|n| (n.kind, tree::full_name(&n.path).unwrap_or("").to_string()));
        if let Some((kind, name)) = sel {
            self.show_node(kind, &name);
        }
    }

    fn open_help(&mut self) {
        self.help = true;
        self.help_scroll = 0;
        self.help_search.clear();
        self.help_search_on = false;
    }

    /// First help line at/after `from` (wrapping) containing the query.
    fn help_match_after(&self, from: usize, wrap: bool) -> Option<usize> {
        let q = self.help_search.to_lowercase();
        if q.is_empty() {
            return None;
        }
        let lines: Vec<&str> = crate::ui::HELP.lines().collect();
        for (i, l) in lines.iter().enumerate().skip(from) {
            if l.to_lowercase().contains(&q) {
                return Some(i);
            }
        }
        if wrap {
            for (i, l) in lines.iter().enumerate().take(from) {
                if l.to_lowercase().contains(&q) {
                    return Some(i);
                }
            }
        }
        None
    }

    fn help_jump_to_match(&mut self, from: usize, wrap: bool) {
        if let Some(i) = self.help_match_after(from, wrap) {
            self.help_scroll = i;
        }
    }

    fn run_command(&mut self) {
        let cmd = self.command.trim().to_string();
        self.command.clear();
        self.hist_idx = None;
        if cmd.is_empty() {
            return;
        }
        if self.hist.last() != Some(&cmd) {
            self.hist.push(cmd.clone());
        }
        let toks = hal::tokenize(&cmd);
        let args: Vec<&str> = toks.iter().map(|s| s.as_str()).collect();
        self.show_text = HalSession::exec(&args);
        self.show_scroll = 0;
    }

    fn cmd_history(&mut self, dir: i32) {
        if self.hist.is_empty() {
            return;
        }
        let len = self.hist.len() as i32;
        let idx = match self.hist_idx {
            None => {
                if dir < 0 {
                    len - 1
                } else {
                    return;
                }
            }
            Some(i) => (i as i32 + dir).clamp(0, len - 1),
        };
        self.hist_idx = Some(idx as usize);
        self.command = self.hist[idx as usize].clone();
    }

    // ------------------------------------------------------------
    // Key handling

    pub fn on_key(&mut self, key: KeyEvent) {
        // help overlay swallows everything but its own keys
        if self.help {
            if self.help_search_on {
                match key.code {
                    KeyCode::Esc => {
                        self.help_search_on = false;
                        self.help_search.clear();
                    }
                    KeyCode::Enter => {
                        // next match after the current line, wrapping
                        self.help_jump_to_match(self.help_scroll + 1, true);
                    }
                    KeyCode::Backspace => {
                        self.help_search.pop();
                        self.help_jump_to_match(self.help_scroll, false);
                    }
                    KeyCode::Char(c) => {
                        self.help_search.push(c);
                        self.help_jump_to_match(self.help_scroll, false);
                    }
                    _ => {}
                }
            } else {
                match key.code {
                    KeyCode::Esc | KeyCode::F(1) | KeyCode::Char('?') => self.help = false,
                    KeyCode::Char('/') => {
                        self.help_search_on = true;
                        self.help_search.clear();
                    }
                    KeyCode::Up => self.help_scroll = self.help_scroll.saturating_sub(1),
                    KeyCode::Down => self.help_scroll += 1,
                    KeyCode::PageUp => self.help_scroll = self.help_scroll.saturating_sub(10),
                    KeyCode::PageDown => self.help_scroll += 10,
                    _ => {}
                }
            }
            return;
        }
        // input prompt swallows everything
        if self.input.is_some() {
            self.on_input_key(key);
            return;
        }
        // F1 opens help from anywhere
        if key.code == KeyCode::F(1) {
            self.open_help();
            return;
        }
        // settings screen is a full-screen mode, not a tab
        if self.settings_open {
            match key.code {
                KeyCode::F(5) | KeyCode::Esc => self.close_settings(),
                KeyCode::Left => {
                    self.settings_open = false;
                    self.focus = Focus::Tree;
                }
                _ => self.on_settings_key(key),
            }
            return;
        }
        match self.focus {
            Focus::Filter => {
                self.on_filter_key(key);
                return;
            }
            Focus::Command => {
                self.on_command_key(key);
                return;
            }
            _ => {}
        }
        // global keys
        match key.code {
            KeyCode::Char('q') => {
                self.quit = true;
                return;
            }
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.quit = true;
                return;
            }
            KeyCode::Char('?') => {
                self.open_help();
                return;
            }
            KeyCode::Char('/') => {
                // search: start a fresh search — clear the previous filter,
                // unfilter the tree, and focus the filter entry
                self.tree.filter.clear();
                self.tree.rebuild(&self.hal);
                self.focus = Focus::Filter;
                return;
            }
            KeyCode::Tab => {
                self.next_tab();
                return;
            }
            KeyCode::BackTab => {
                self.prev_tab();
                return;
            }
            KeyCode::Char('1') => {
                self.tab = Tab::Show;
                self.focus = Focus::ShowText;
                self.preview_selection();
                return;
            }
            KeyCode::Char('2') => {
                self.tab = Tab::Watch;
                self.focus = Focus::Watch;
                return;
            }
            KeyCode::F(5) => {
                self.open_settings();
                return;
            }
            KeyCode::F(2) => {
                self.focus = Focus::Tree;
                return;
            }
            KeyCode::F(3) => {
                self.focus_content();
                return;
            }
            KeyCode::F(4) => {
                self.tab = Tab::Show;
                self.focus = Focus::Command;
                return;
            }
            KeyCode::Char('[') => {
                self.prefs.ratio = (self.prefs.ratio - 0.05).clamp(0.12, 0.9);
                return;
            }
            KeyCode::Char(']') => {
                self.prefs.ratio = (self.prefs.ratio + 0.05).clamp(0.12, 0.9);
                return;
            }
            _ => {}
        }
        match self.focus {
            Focus::Tree => self.on_tree_key(key),
            Focus::ShowText => self.on_show_key(key),
            Focus::Watch => self.on_watch_key(key),
            Focus::Settings => self.on_settings_key(key),
            _ => {}
        }
    }

    fn focus_content(&mut self) {
        self.focus = match self.tab {
            Tab::Show => Focus::ShowText,
            Tab::Watch => Focus::Watch,
        };
    }

    fn next_tab(&mut self) {
        self.tab = match self.tab {
            Tab::Show => Tab::Watch,
            Tab::Watch => Tab::Show,
        };
        self.focus_content();
        if self.tab == Tab::Show {
            self.preview_selection();
        }
    }

    fn prev_tab(&mut self) {
        self.tab = match self.tab {
            Tab::Show => Tab::Watch,
            Tab::Watch => Tab::Show,
        };
        self.focus_content();
        if self.tab == Tab::Show {
            self.preview_selection();
        }
    }

    fn open_settings(&mut self) {
        self.settings_open = true;
        self.focus = Focus::Settings;
        if self.settings_state.selected().is_none() {
            self.settings_state.select(Some(0));
        }
    }

    fn close_settings(&mut self) {
        self.settings_open = false;
        self.focus_content();
    }

    fn on_input_key(&mut self, key: KeyEvent) {
        if self
            .input
            .as_ref()
            .is_some_and(|i| i.kind == InputKind::FileDialog)
        {
            self.on_dialog_key(key);
            return;
        }
        let Some(input) = &mut self.input else { return };
        match key.code {
            KeyCode::Esc => {
                self.input = None;
                return;
            }
            KeyCode::Enter => {
                let input = self.input.take().unwrap();
                let action = input.action;
                let buffer = match input.kind {
                    InputKind::Text => input.buffer,
                    InputKind::BitPick => {
                        if input.bit_value {
                            "1".to_string()
                        } else {
                            "0".to_string()
                        }
                    }
                    InputKind::FileDialog => input.buffer,
                };
                self.apply_action(action, &buffer);
                return;
            }
            _ => {}
        }
        match input.kind {
            InputKind::BitPick => match key.code {
                KeyCode::Left
                | KeyCode::Right
                | KeyCode::Up
                | KeyCode::Down
                | KeyCode::Tab
                | KeyCode::BackTab => input.bit_value = !input.bit_value,
                KeyCode::Char('t') | KeyCode::Char('T') | KeyCode::Char('1') => {
                    input.bit_value = true
                }
                KeyCode::Char('f') | KeyCode::Char('F') | KeyCode::Char('0') => {
                    input.bit_value = false
                }
                _ => {}
            },
            InputKind::Text => match key.code {
                KeyCode::Left => input.cursor = input.cursor.saturating_sub(1),
                KeyCode::Right => {
                    input.cursor = (input.cursor + 1).min(input.buffer.chars().count())
                }
                KeyCode::Home => input.cursor = 0,
                KeyCode::End => input.cursor = input.buffer.chars().count(),
                KeyCode::Backspace => {
                    if input.cursor > 0 {
                        input.cursor -= 1;
                        remove_char_at(&mut input.buffer, input.cursor);
                    }
                }
                KeyCode::Delete => remove_char_at(&mut input.buffer, input.cursor),
                KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    input.buffer.clear();
                    input.cursor = 0;
                }
                KeyCode::Char(c) => {
                    let pos = input
                        .buffer
                        .char_indices()
                        .nth(input.cursor)
                        .map(|(i, _)| i)
                        .unwrap_or(input.buffer.len());
                    input.buffer.insert(pos, c);
                    input.cursor += 1;
                }
                _ => {}
            },
            InputKind::FileDialog => {}
        }
    }

    /// File open/save dialog key handling. The dialog owns a directory
    /// listing; Tab switches between the list and the filter/file-name field.
    fn on_dialog_key(&mut self, key: KeyEvent) {
        let mut input = match self.input.take() {
            Some(i) if i.kind == InputKind::FileDialog => i,
            other => {
                self.input = other;
                return;
            }
        };
        let mut dialog = match input.dialog.take() {
            Some(d) => d,
            None => {
                self.input = None;
                return;
            }
        };
        let mut confirm: Option<String> = None;
        let mut cancel = false;

        let n = dialog.entries.len();
        match key.code {
            KeyCode::Esc => cancel = true,
            KeyCode::Tab => dialog.field = !dialog.field,
            // list navigation always works, whether or not the field is focused
            KeyCode::Up => dialog.selected = dialog.selected.saturating_sub(1),
            KeyCode::Down => dialog.selected = (dialog.selected + 1).min(n.saturating_sub(1)),
            KeyCode::PageUp => dialog.selected = dialog.selected.saturating_sub(10),
            KeyCode::PageDown => dialog.selected = (dialog.selected + 10).min(n.saturating_sub(1)),
            KeyCode::Home => dialog.selected = 0,
            KeyCode::End => dialog.selected = n.saturating_sub(1),
            KeyCode::Left => {
                if dialog.field && input.cursor > 0 {
                    input.cursor -= 1;
                } else if let Some(parent) = dialog.dir.parent() {
                    dialog.dir = parent.to_path_buf();
                    dialog.selected = 0;
                    dialog.scroll = 0;
                    refresh_dialog(&mut dialog, &input.buffer);
                }
            }
            KeyCode::Right => {
                if dialog.field {
                    input.cursor = (input.cursor + 1).min(input.buffer.chars().count());
                }
            }
            KeyCode::Backspace => {
                if dialog.field && input.cursor > 0 && !input.buffer.is_empty() {
                    input.cursor -= 1;
                    remove_char_at(&mut input.buffer, input.cursor);
                    if !dialog.save {
                        refresh_dialog(&mut dialog, &input.buffer);
                    }
                } else if let Some(parent) = dialog.dir.parent() {
                    dialog.dir = parent.to_path_buf();
                    dialog.selected = 0;
                    dialog.scroll = 0;
                    refresh_dialog(&mut dialog, &input.buffer);
                }
            }
            KeyCode::Delete => {
                if dialog.field {
                    remove_char_at(&mut input.buffer, input.cursor);
                    if !dialog.save {
                        refresh_dialog(&mut dialog, &input.buffer);
                    }
                }
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                dialog.field = true;
                input.buffer.clear();
                input.cursor = 0;
                if !dialog.save {
                    refresh_dialog(&mut dialog, &input.buffer);
                }
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                // typing focuses the filter/file-name field
                dialog.field = true;
                let pos = input
                    .buffer
                    .char_indices()
                    .nth(input.cursor)
                    .map(|(i, _)| i)
                    .unwrap_or(input.buffer.len());
                input.buffer.insert(pos, c);
                input.cursor += 1;
                if !dialog.save {
                    refresh_dialog(&mut dialog, &input.buffer);
                }
            }
            KeyCode::Enter => {
                if dialog.field {
                    let name = input.buffer.trim().to_string();
                    if name.is_empty() {
                        dialog.error = Some(if dialog.save {
                            "Enter a file name".to_string()
                        } else {
                            "Type a filter or pick a file".to_string()
                        });
                    } else if dialog.save {
                        confirm = Some(dialog.dir.join(&name).to_string_lossy().into_owned());
                    } else {
                        let cand = if Path::new(&name).is_absolute() {
                            PathBuf::from(&name)
                        } else {
                            dialog.dir.join(&name)
                        };
                        if cand.is_dir() {
                            dialog.dir = cand;
                            dialog.selected = 0;
                            dialog.scroll = 0;
                            refresh_dialog(&mut dialog, &input.buffer);
                        } else if cand.is_file() {
                            confirm = Some(cand.to_string_lossy().into_owned());
                        } else if let Some(p) = dialog.entries.get(dialog.selected) {
                            // the typed text is a filter: open the highlighted file
                            if p.is_file() {
                                confirm = Some(p.to_string_lossy().into_owned());
                            } else {
                                dialog.error = Some(format!("no such file: {}", cand.display()));
                            }
                        } else {
                            dialog.error = Some(format!("no such file: {}", cand.display()));
                        }
                    }
                } else if let Some(p) = dialog.entries.get(dialog.selected) {
                    if p.is_dir() {
                        dialog.dir = p.clone();
                        dialog.selected = 0;
                        dialog.scroll = 0;
                        refresh_dialog(&mut dialog, &input.buffer);
                    } else if dialog.save {
                        // picking an existing file seeds the name field
                        input.buffer = p
                            .file_name()
                            .map(|s| s.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        input.cursor = input.buffer.chars().count();
                        dialog.field = true;
                    } else {
                        confirm = Some(p.to_string_lossy().into_owned());
                    }
                }
            }
            _ => {}
        }

        if cancel {
            self.input = None;
            return;
        }

        if let Some(path) = confirm {
            self.input = None;
            let action = input.action;
            self.apply_action(action, &path);
        } else {
            input.dialog = Some(dialog);
            self.input = Some(input);
        }
    }

    fn apply_action(&mut self, action: InputAction, buffer: &str) {
        match action {
            InputAction::SetValue(idx) => {
                self.watch_set(idx, buffer.trim());
            }
            InputAction::AddWatch => {
                let text = buffer.trim().to_string();
                let (kind, name) = match text.split_once('+') {
                    Some((k, n)) => match HalType::from_kw(k) {
                        Some(k) => (k, n.trim().to_string()),
                        None => {
                            let Some(k) = self.guess_kind(&text) else {
                                self.set_status(
                                    format!("'{text}': not a pin, param or signal"),
                                    true,
                                );
                                return;
                            };
                            (k, text.clone())
                        }
                    },
                    None => {
                        let Some(k) = self.guess_kind(&text) else {
                            self.set_status(format!("'{text}': not a pin, param or signal"), true);
                            return;
                        };
                        (k, text.clone())
                    }
                };
                self.watch_add(kind, &name, true);
            }
            InputAction::LoadWatchFile => {
                let path = buffer.trim().to_string();
                if !path.is_empty() {
                    self.load_watch_file(&path);
                }
            }
            InputAction::SaveWatchFile(multiline) => {
                let path = buffer.trim().to_string();
                if !path.is_empty() {
                    self.save_watch_file(&path, multiline);
                }
            }
            InputAction::SetSetting(idx) => {
                self.apply_setting(idx, buffer.trim());
            }
        }
    }

    fn apply_setting(&mut self, idx: usize, val: &str) {
        match idx {
            0 => {
                if let Ok(v) = val.parse::<u64>() {
                    if v >= 1 {
                        self.prefs.watch_interval = v;
                        self.set_status(format!("Update interval set to {v} ms"), false);
                        return;
                    }
                }
                self.set_status("Value out of range (min 1 ms)".to_string(), true);
            }
            1 => {
                if let Ok(v) = val.parse::<u16>() {
                    self.prefs.col1_width = v.clamp(8, 120);
                    self.set_status(
                        format!("Column width set to {}", self.prefs.col1_width),
                        false,
                    );
                    return;
                }
                self.set_status("Value out of range".to_string(), true);
            }
            2 => {
                self.prefs.ffmts = val.to_string();
                self.set_status("Float format set".to_string(), false);
            }
            3 => {
                self.prefs.ifmts = val.to_string();
                self.set_status("Integer format set".to_string(), false);
            }
            _ => {}
        }
    }

    fn on_filter_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc | KeyCode::Enter => {
                self.focus = Focus::Tree;
                self.tree.rebuild(&self.hal);
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.tree.filter.clear();
                self.tree.rebuild(&self.hal);
            }
            KeyCode::Backspace => {
                self.tree.filter.pop();
                self.tree.rebuild(&self.hal);
            }
            KeyCode::Char(c) => {
                self.tree.filter.push(c);
                self.tree.rebuild(&self.hal);
            }
            _ => {}
        }
    }

    fn on_command_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.focus = Focus::ShowText;
            }
            KeyCode::Left => {
                self.focus = Focus::Tree;
            }
            KeyCode::Char('?') => {
                self.open_help();
            }
            KeyCode::Enter => self.run_command(),
            KeyCode::Up => self.cmd_history(-1),
            KeyCode::Down => self.cmd_history(1),
            KeyCode::Backspace => {
                self.command.pop();
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.command.clear();
            }
            KeyCode::Char(c) => self.command.push(c),
            _ => {}
        }
    }

    fn on_tree_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Up => {
                self.tree_move(-1);
                self.preview_selection();
            }
            KeyCode::Down => {
                self.tree_move(1);
                self.preview_selection();
            }
            KeyCode::PageUp => {
                self.tree_move(-(self.tree_page.max(1) as i32));
                self.preview_selection();
            }
            KeyCode::PageDown => {
                self.tree_move(self.tree_page.max(1) as i32);
                self.preview_selection();
            }
            KeyCode::Right => {
                let Some(n) = self.tree.selected_node().cloned() else {
                    return;
                };
                if !n.expanded && n.is_branch() {
                    // closed parent: expand it in place
                    self.tree_set_expanded(&n.path, true);
                } else if self.tab != Tab::Show {
                    // expanded branch or leaf: hand focus to the right panel
                    self.preview_selection();
                    self.focus_content();
                } else if self.show_text.lines().count() > self.show_page.max(1) {
                    // SHOW tab: only jump to the pane when there is
                    // something to do there — the output is scrollable
                    // (more than one visible page)
                    self.preview_selection();
                    self.focus_content();
                }
            }
            KeyCode::Left => {
                let mut moved = false;
                if let Some(n) = self.tree.selected_node().cloned() {
                    if n.expanded && n.is_branch() {
                        self.tree_set_expanded(&n.path, false);
                    } else if let Some(parent) = tree::parent_path(&n.path) {
                        self.tree.selected = parent;
                        moved = true;
                    }
                }
                if moved {
                    self.preview_selection();
                }
            }
            KeyCode::Char(' ') => {
                if let Some(n) = self.tree.selected_node().cloned() {
                    if n.leaf && self.tab == Tab::Watch {
                        let kind = n.kind;
                        let name = tree::full_name(&n.path).unwrap_or(&n.name).to_string();
                        self.watch_add(kind, &name, true);
                        self.poll_watch();
                    } else {
                        let path = n.path.clone();
                        let expanded = n.expanded;
                        self.tree_set_expanded(&path, !expanded);
                    }
                }
            }
            KeyCode::Enter => {
                if let Some(n) = self.tree.selected_node() {
                    let kind = n.kind;
                    let name = tree::full_name(&n.path).unwrap_or(&n.name).to_string();
                    let leaf = n.leaf;
                    match self.tab {
                        Tab::Show => self.show_node(kind, &name),
                        Tab::Watch if leaf => {
                            self.watch_add(kind, &name, true);
                            self.poll_watch();
                        }
                        Tab::Watch => {
                            self.set_status(
                                format!("cannot watch non-leaf {} node", kind.kw()),
                                true,
                            );
                        }
                    }
                }
            }
            KeyCode::Char('a') => {
                if let Some(n) = self.tree.selected_node() {
                    let kind = n.kind;
                    let name = tree::full_name(&n.path).unwrap_or(&n.name).to_string();
                    if n.leaf {
                        self.watch_add(kind, &name, true);
                    } else {
                        self.set_status(format!("cannot watch non-leaf {} node", kind.kw()), true);
                    }
                }
            }
            KeyCode::Char('A') => {
                if let Some(n) = self.tree.selected_node() {
                    let path = n.path.clone();
                    let leaves = tree::collect_leaves(&self.tree.roots, &path);
                    let mut added = 0;
                    for (kind, name) in leaves {
                        if self.watch_add(kind, &name, false) {
                            added += 1;
                        }
                    }
                    self.set_status(format!("{added} item(s) added"), false);
                }
            }
            KeyCode::Char('s') => {
                if let Some(n) = self.tree.selected_node() {
                    let kind = n.kind;
                    let name = tree::full_name(&n.path).unwrap_or(&n.name).to_string();
                    self.tab = Tab::Show;
                    self.focus = Focus::ShowText;
                    self.show_node(kind, &name);
                }
            }
            KeyCode::Char('e') => self.tree.expand_all(),
            KeyCode::Char('w') => self.tree.collapse_all(),
            KeyCode::Char('E') => {
                if let Some(n) = self.tree.selected_node() {
                    let kind = n.kind;
                    self.tree.expand_kind(kind);
                }
            }
            KeyCode::Char('W') => {
                if let Some(n) = self.tree.selected_node() {
                    let kind = n.kind;
                    self.tree.collapse_kind(kind);
                }
            }
            KeyCode::Char('f') => {
                self.tree.full_path = !self.tree.full_path;
                self.tree.rebuild(&self.hal);
            }
            KeyCode::Char('r') => self.tree.rebuild(&self.hal),
            _ => {}
        }
    }

    fn tree_move(&mut self, delta: i32) {
        let vis = tree::visible(&self.tree.roots);
        let Some(cur) = vis.iter().position(|(p, _, _)| *p == self.tree.selected) else {
            if let Some((p, _, _)) = vis.first() {
                self.tree.selected = p.clone();
            }
            return;
        };
        let idx = (cur as i32 + delta).clamp(0, vis.len() as i32 - 1) as usize;
        self.tree.selected = vis[idx].0.clone();
    }

    fn tree_set_expanded(&mut self, path: &str, value: bool) {
        fn set(nodes: &mut [crate::tree::TreeNode], path: &str, value: bool) -> bool {
            for n in nodes {
                if n.path == path {
                    n.expanded = value;
                    return true;
                }
                if set(&mut n.children, path, value) {
                    return true;
                }
            }
            false
        }
        set(&mut self.tree.roots, path, value);
    }

    fn on_show_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Left => self.focus = Focus::Tree,
            KeyCode::Up => self.show_scroll = self.show_scroll.saturating_sub(1),
            KeyCode::Down => self.show_scroll += 1,
            KeyCode::PageUp => {
                self.show_scroll = self.show_scroll.saturating_sub(self.show_page.max(1));
            }
            KeyCode::PageDown => self.show_scroll += self.show_page.max(1),
            KeyCode::Home => self.show_scroll = 0,
            KeyCode::Char('a') => {
                if let Some((kind, name)) = self.shown_node.clone() {
                    self.watch_add(kind, &name, true);
                }
            }
            KeyCode::Char('c') => {
                self.focus = Focus::Command;
            }
            _ => {}
        }
    }

    fn on_watch_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Left => self.focus = Focus::Tree,
            KeyCode::Up => {
                if let Some(s) = self.watch_state.selected() {
                    self.watch_state.select(Some(s.saturating_sub(1)));
                } else if !self.watch.is_empty() {
                    self.watch_state.select(Some(0));
                }
            }
            KeyCode::Down => {
                if let Some(s) = self.watch_state.selected() {
                    let next = (s + 1).min(self.watch.len().saturating_sub(1));
                    self.watch_state.select(Some(next));
                } else if !self.watch.is_empty() {
                    self.watch_state.select(Some(0));
                }
            }
            KeyCode::PageUp => {
                if let Some(s) = self.watch_state.selected() {
                    self.watch_state
                        .select(Some(s.saturating_sub(self.watch_page.max(1))));
                } else if !self.watch.is_empty() {
                    self.watch_state.select(Some(0));
                }
            }
            KeyCode::PageDown => {
                if let Some(s) = self.watch_state.selected() {
                    let next = (s + self.watch_page.max(1)).min(self.watch.len().saturating_sub(1));
                    self.watch_state.select(Some(next));
                } else if !self.watch.is_empty() {
                    self.watch_state.select(Some(0));
                }
            }
            KeyCode::Enter => {
                let Some(idx) = self.watch_state.selected() else {
                    return;
                };
                if idx >= self.watch.len() {
                    return;
                }
                let w = &self.watch[idx];
                if w.writable == 1 {
                    let label = w.name.clone();
                    if w.dtype == "bit" {
                        self.input = Some(InputState::bit_pick(
                            format!("Set {label} (bit)"),
                            w.value == "TRUE",
                            InputAction::SetValue(idx),
                        ));
                    } else {
                        self.input = Some(InputState::text(
                            format!("Set {label}"),
                            w.value.trim().to_string(),
                            InputAction::SetValue(idx),
                        ));
                    }
                } else if w.writable == -1 {
                    self.watch_unlink(idx);
                }
            }
            KeyCode::Char('s') => {
                let Some(idx) = self.watch_state.selected() else {
                    return;
                };
                if idx < self.watch.len()
                    && self.watch[idx].writable == 1
                    && self.watch[idx].dtype == "bit"
                {
                    self.watch_set(idx, "1");
                }
            }
            KeyCode::Char('c') => {
                let Some(idx) = self.watch_state.selected() else {
                    return;
                };
                if idx < self.watch.len()
                    && self.watch[idx].writable == 1
                    && self.watch[idx].dtype == "bit"
                {
                    self.watch_set(idx, "0");
                }
            }
            KeyCode::Char(' ') => {
                let Some(idx) = self.watch_state.selected() else {
                    return;
                };
                if idx < self.watch.len() {
                    let (writable, dtype, value) = {
                        let w = &self.watch[idx];
                        (w.writable, w.dtype.clone(), w.value.clone())
                    };
                    if writable == 1 && dtype == "bit" {
                        let new_val = if value == "TRUE" { "0" } else { "1" };
                        self.watch_set(idx, new_val);
                    }
                }
            }
            KeyCode::Char('u') => {
                if let Some(idx) = self.watch_state.selected() {
                    self.watch_unlink(idx);
                }
            }
            KeyCode::Char('x') | KeyCode::Delete => {
                if let Some(idx) = self.watch_state.selected() {
                    self.watch_remove(idx);
                }
            }
            KeyCode::Char('r') => self.reload_watch(),
            KeyCode::Char('e') => self.watch_erase(),
            KeyCode::Char('a') => {
                self.input = Some(InputState::text(
                    "Add to watch (pin/param/sig name)",
                    String::new(),
                    InputAction::AddWatch,
                ));
            }
            KeyCode::Char('o') => {
                let Some(idx) = self.watch_state.selected() else {
                    return;
                };
                if idx >= self.watch.len() {
                    return;
                }
                let (kind, name) = {
                    let w = &self.watch[idx];
                    (w.kind, w.name.clone())
                };
                let path = format!("{}+{}", kind.kw(), name);
                self.tree.reveal(&path);
                self.tab = Tab::Show;
                self.focus = Focus::ShowText;
                self.show_node(kind, &name);
            }
            KeyCode::Char('L') => {
                let dir = if self.last_watch_dir.as_os_str().is_empty() {
                    PathBuf::from(".")
                } else {
                    self.last_watch_dir.clone()
                };
                self.input = Some(InputState::file_dialog(
                    false,
                    dir,
                    String::new(),
                    InputAction::LoadWatchFile,
                ));
            }
            KeyCode::Char('S') => {
                self.save_watch_prompt(false);
            }
            KeyCode::Char('m') => {
                self.save_watch_prompt(true);
            }
            _ => {}
        }
    }

    fn on_settings_key(&mut self, key: KeyEvent) {
        let n = 6; // 5 settings + Apply
        let sel = self.settings_state.selected().unwrap_or(0);
        match key.code {
            KeyCode::Up => self.settings_state.select(Some(sel.saturating_sub(1))),
            KeyCode::Down => self.settings_state.select(Some((sel + 1).min(n - 1))),
            KeyCode::Enter | KeyCode::Char(' ') => match sel {
                0..=3 => {
                    let def = match sel {
                        0 => self.prefs.watch_interval.to_string(),
                        1 => self.prefs.col1_width.to_string(),
                        2 => self.prefs.ffmts.clone(),
                        _ => self.prefs.ifmts.clone(),
                    };
                    self.input = Some(InputState::text(
                        "Edit setting",
                        def.clone(),
                        InputAction::SetSetting(sel),
                    ));
                }
                4 => {
                    // remember watchlist toggle
                    self.prefs.auto_save_watchlist = !self.prefs.auto_save_watchlist;
                    self.set_status("Remember watchlist toggled".to_string(), false);
                }
                _ => {
                    // Apply
                    self.poll_watch();
                    self.set_status("Settings applied".to_string(), false);
                }
            },
            _ => {}
        }
    }
}

impl InputAction {}

fn remove_char_at(s: &mut String, idx: usize) {
    if let Some((pos, _)) = s.char_indices().nth(idx) {
        s.remove(pos);
    }
}

/// Main loop: draw, wait for events or the watch tick.
pub fn run<B: Backend<Error = io::Error>>(term: &mut Terminal<B>, app: &mut App) -> io::Result<()> {
    loop {
        if app.quit {
            break;
        }
        app.poll_if_due();
        term.draw(|f| ui::draw(f, app))?;
        let interval = Duration::from_millis(app.prefs.watch_interval.max(20));
        let deadline = app.last_tick_instant() + interval;
        let wait = deadline.saturating_duration_since(Instant::now());
        if event::poll(wait)? {
            match event::read()? {
                Event::Key(k) => app.on_key(k),
                Event::Resize(_, _) => {}
                _ => {}
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{norm_path, sign_pad};
    use std::path::{Path, PathBuf};

    #[test]
    fn norm_resolves_dot_dot() {
        assert_eq!(norm_path(Path::new("/a/b/..")), PathBuf::from("/a"));
        assert_eq!(norm_path(Path::new("/a/b/../../c")), PathBuf::from("/c"));
        assert_eq!(norm_path(Path::new("/")), PathBuf::from("/"));
        assert_eq!(norm_path(Path::new("/a/./b")), PathBuf::from("/a/b"));
        assert_eq!(norm_path(Path::new("/a/../..")), PathBuf::from("/"));
    }

    #[test]
    fn sign_pad_aligns_positive_and_negative() {
        assert_eq!(sign_pad("12.5"), " 12.5");
        assert_eq!(sign_pad("-12.5"), "-12.5");
        assert_eq!(sign_pad("42"), " 42");
        assert_eq!(sign_pad(" 12.5"), " 12.5");
        assert_eq!(sign_pad("+12.5"), "+12.5");
        assert_eq!(sign_pad("0"), " 0");
    }
}
