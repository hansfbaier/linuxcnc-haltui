//! Read/write the halshow preferences file. The on-disk format is the
//! Tcl fragment written by halshow.tcl; haltui reads and writes the
//! same format so both tools can share one settings file.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Workmode {
    Show,
    Watch,
    Settings,
}

impl Workmode {
    pub fn as_str(self) -> &'static str {
        match self {
            Workmode::Show => "showhal",
            Workmode::Watch => "watchhal",
            Workmode::Settings => "settings",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Prefs {
    pub geometry: String,
    pub ratio: f64,
    pub old_w_leftf: u16,
    pub workmode: Workmode,
    pub watch_interval: u64,
    pub col1_width: u16,
    pub ffmts: String,
    pub ifmts: String,
    pub always_on_top: bool,
    pub auto_save_watchlist: bool,
    pub watchlist: Vec<String>,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs {
            geometry: "800x600+100+100".to_string(),
            ratio: 0.3,
            old_w_leftf: 160,
            workmode: Workmode::Show,
            watch_interval: 100,
            col1_width: 100,
            ffmts: String::new(),
            ifmts: String::new(),
            always_on_top: false,
            auto_save_watchlist: true,
            watchlist: Vec::new(),
        }
    }
}

/// Locate the preferences file the way halshow does:
/// 1. $CONFIG_DIR/halshow.preferences
/// 2. directory of the .ini of a running `linuxcnc` process
/// 3. ~/.halshow_preferences
pub fn locate() -> (PathBuf, Option<PathBuf>) {
    if let Ok(dir) = env::var("CONFIG_DIR") {
        if !dir.is_empty() {
            return (
                PathBuf::from(&dir).join("halshow.preferences"),
                Some(PathBuf::from(dir)),
            );
        }
    }
    if let Some(dir) = ini_dir_from_ps() {
        return (dir.join("halshow.preferences"), Some(dir));
    }
    let home = env::var("HOME").unwrap_or_else(|_| ".".to_string());
    (PathBuf::from(home).join(".halshow_preferences"), None)
}

fn ini_dir_from_ps() -> Option<PathBuf> {
    let out = Command::new("ps")
        .args(["-e", "-o", "stat=", "-o", "command="])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    for line in text.lines() {
        if !line.trim_start().starts_with('S') {
            continue;
        }
        let Some(pos) = line.find("linuxcnc ") else {
            continue;
        };
        for tok in line[pos + "linuxcnc ".len()..].split_whitespace() {
            if tok.starts_with('/') && tok.ends_with(".ini") {
                if let Some(parent) = Path::new(tok).parent() {
                    return Some(parent.to_path_buf());
                }
            }
        }
    }
    None
}

pub fn read(path: &Path) -> Prefs {
    let mut p = Prefs::default();
    let text = match fs::read_to_string(path) {
        Ok(t) => t,
        Err(_) => return p,
    };
    let mut in_watchlist = false;
    for raw in text.lines() {
        let line = raw.trim();
        if in_watchlist {
            if line == "}" {
                in_watchlist = false;
                continue;
            }
            p.watchlist
                .extend(line.split_whitespace().map(String::from));
            continue;
        }
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(rest) = line.strip_prefix("set ::watchlist {") {
            in_watchlist = true;
            let rest = rest.trim();
            if rest.ends_with('}') {
                in_watchlist = false;
                p.watchlist.extend(
                    rest.strip_suffix('}')
                        .unwrap_or(rest)
                        .split_whitespace()
                        .map(String::from),
                );
            } else {
                p.watchlist
                    .extend(rest.split_whitespace().map(String::from));
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("placeFrames") {
            if let Ok(r) = rest.trim().parse::<f64>() {
                p.ratio = r;
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("wm geometry .") {
            p.geometry = rest.trim().to_string();
            continue;
        }
        let Some(rest) = line.strip_prefix("set ::") else {
            continue;
        };
        let Some((name, val)) = rest.split_once(|c: char| c.is_whitespace()) else {
            continue;
        };
        let val = unbrace(val.trim());
        match name {
            "ratio" => {
                if let Ok(r) = val.parse() {
                    p.ratio = r;
                }
            }
            "old_w_leftf" => {
                if let Ok(v) = val.parse() {
                    p.old_w_leftf = v;
                }
            }
            "workmode" => match val.as_str() {
                "watchhal" => p.workmode = Workmode::Watch,
                "settings" => p.workmode = Workmode::Settings,
                _ => p.workmode = Workmode::Show,
            },
            "watchInterval" => {
                if let Ok(v) = val.parse() {
                    p.watch_interval = v;
                }
            }
            "col1_width" => {
                if let Ok(v) = val.parse() {
                    p.col1_width = v;
                }
            }
            "ffmts" => p.ffmts = val,
            "ifmts" => p.ifmts = val,
            "alwaysOnTop" => p.always_on_top = val == "1" || val == "true",
            "autoSaveWatchlist" => p.auto_save_watchlist = val == "1" || val == "true",
            _ => {}
        }
    }
    p
}

fn unbrace(s: &str) -> String {
    if s.len() >= 2 && s.starts_with('{') && s.ends_with('}') {
        s[1..s.len() - 1].to_string()
    } else {
        s.to_string()
    }
}

pub fn write(path: &Path, p: &Prefs) -> std::io::Result<()> {
    let mut s = String::new();
    s.push_str("# Halshow settings\n");
    s.push_str("# This file is generated automatically.\n");
    s.push_str(&format!("wm geometry . {}\n", p.geometry));
    s.push_str(&format!("placeFrames {}\n", p.ratio));
    s.push_str(&format!("set ::ratio {}\n", p.ratio));
    s.push_str(&format!("set ::old_w_leftf {}\n", p.old_w_leftf));
    if p.auto_save_watchlist && !p.watchlist.is_empty() {
        s.push_str("set ::watchlist {\n");
        for item in &p.watchlist {
            s.push_str(&format!("    {}\n", item));
        }
        s.push_str("}\n");
    }
    s.push_str(&format!("set ::workmode {}\n", p.workmode.as_str()));
    s.push_str(&format!("set ::watchInterval {}\n", p.watch_interval));
    s.push_str(&format!("set ::col1_width {}\n", p.col1_width));
    s.push_str(&format!("set ::ffmts {{{}}}\n", p.ffmts));
    s.push_str(&format!("set ::ifmts {{{}}}\n", p.ifmts));
    s.push_str(&format!(
        "set ::alwaysOnTop {}\n",
        if p.always_on_top { 1 } else { 0 }
    ));
    s.push_str(&format!(
        "set ::autoSaveWatchlist {}\n",
        if p.auto_save_watchlist { 1 } else { 0 }
    ));
    fs::write(path, s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_halshow_file() {
        let sample = "# Halshow settings\n\
            wm geometry . 700x475+185+244\n\
            placeFrames 0.3\n\
            set ::ratio 0.3\n\
            set ::old_w_leftf 160\n\
            set ::watchlist {\n    pin+axis.0.pos\n    sig+estop\n}\n\
            set ::workmode watchhal\n\
            set ::watchInterval 250\n\
            set ::col1_width 120\n\
            set ::ffmts {%5.2f}\n\
            set ::ifmts {}\n\
            set ::alwaysOnTop 0\n\
            set ::autoSaveWatchlist 1\n";
        let p = parse_test(sample);
        assert_eq!(p.ratio, 0.3);
        assert_eq!(p.workmode, Workmode::Watch);
        assert_eq!(p.watch_interval, 250);
        assert_eq!(p.col1_width, 120);
        assert_eq!(p.ffmts, "%5.2f");
        assert!(p.ifmts.is_empty());
        assert_eq!(p.watchlist, vec!["pin+axis.0.pos", "sig+estop"]);
        assert!(p.auto_save_watchlist);
    }

    fn parse_test(text: &str) -> Prefs {
        let dir = std::env::temp_dir().join(format!("haltui_test_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("prefs");
        std::fs::write(&path, text).unwrap();
        let p = read(&path);
        let _ = std::fs::remove_dir_all(&dir);
        p
    }
}
