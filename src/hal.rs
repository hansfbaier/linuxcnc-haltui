//! HAL access: persistent `halcmd -k -f` session for cheap batched
//! reads, plus one-shot `halcmd` spawns for everything else.
//!
//! Session protocol: each command written to the persistent halcmd's
//! stdin produces exactly one output line (value on stdout, error on
//! stderr). stderr is dup2'ed onto the stdout pipe so a batch of N
//! commands yields exactly N response lines, in order.

use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::os::fd::FromRawFd;
use std::os::unix::process::CommandExt;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum HalType {
    Comp,
    Pin,
    Param,
    Sig,
    Funct,
    Thread,
}

impl HalType {
    pub const ALL: [HalType; 6] = [
        HalType::Comp,
        HalType::Pin,
        HalType::Param,
        HalType::Sig,
        HalType::Funct,
        HalType::Thread,
    ];

    /// halcmd keyword (also the tree root path).
    pub fn kw(self) -> &'static str {
        match self {
            HalType::Comp => "comp",
            HalType::Pin => "pin",
            HalType::Param => "param",
            HalType::Sig => "sig",
            HalType::Funct => "funct",
            HalType::Thread => "thread",
        }
    }

    /// Display title used in the tree.
    pub fn title(self) -> &'static str {
        match self {
            HalType::Comp => "Components",
            HalType::Pin => "Pins",
            HalType::Param => "Parameters",
            HalType::Sig => "Signals",
            HalType::Funct => "Functions",
            HalType::Thread => "Threads",
        }
    }

    pub fn from_kw(s: &str) -> Option<HalType> {
        Some(match s {
            "comp" | "component" => HalType::Comp,
            "pin" => HalType::Pin,
            "param" | "parameter" => HalType::Param,
            "sig" | "signal" => HalType::Sig,
            "funct" | "function" => HalType::Funct,
            "thread" => HalType::Thread,
            _ => return None,
        })
    }

    pub fn watchable(self) -> bool {
        matches!(self, HalType::Pin | HalType::Param | HalType::Sig)
    }
}

#[derive(Debug)]
pub struct BatchOut {
    pub line: String,
    pub err: bool,
}

impl BatchOut {
    fn error(msg: impl Into<String>) -> Self {
        BatchOut {
            line: msg.into(),
            err: true,
        }
    }
}

pub struct HalSession {
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    rx: Option<mpsc::Receiver<String>>,
    last_spawn: Instant,
}

impl HalSession {
    pub fn new() -> Self {
        HalSession {
            child: None,
            stdin: None,
            rx: None,
            last_spawn: Instant::now() - Duration::from_secs(10),
        }
    }

    fn spawn(&mut self) {
        self.last_spawn = Instant::now();
        // Merge child stdout+stderr into one pipe so response lines
        // arrive in command order. The pipe is created manually:
        // pre_exec dup2's the write end onto fds 1 and 2, and the
        // parent keeps the read end as a plain File.
        let mut fds = [0i32; 2];
        if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
            return;
        }
        let read_end = unsafe { File::from_raw_fd(fds[0]) };
        let write_fd = fds[1];
        let mut cmd = Command::new("halcmd");
        cmd.args(["-k", "-f"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        unsafe {
            cmd.pre_exec(move || {
                libc::dup2(write_fd, libc::STDOUT_FILENO);
                libc::dup2(write_fd, libc::STDERR_FILENO);
                libc::close(write_fd);
                Ok(())
            });
        }
        if let Ok(mut child) = cmd.spawn() {
            if let Some(stdin) = child.stdin.take() {
                let (tx, rx) = mpsc::channel();
                std::thread::spawn(move || {
                    let mut reader = BufReader::new(read_end);
                    let mut line = String::new();
                    loop {
                        line.clear();
                        match reader.read_line(&mut line) {
                            Ok(0) | Err(_) => break,
                            Ok(_) => {
                                while line.ends_with('\n') || line.ends_with('\r') {
                                    line.pop();
                                }
                                if tx.send(line.clone()).is_err() {
                                    break;
                                }
                            }
                        }
                    }
                });
                self.child = Some(child);
                self.stdin = Some(stdin);
                self.rx = Some(rx);
            }
        }
    }

    fn alive(&mut self) -> bool {
        match self.child.as_mut() {
            Some(c) => matches!(c.try_wait(), Ok(None)),
            None => false,
        }
    }

    pub fn kill(&mut self) {
        let Some(mut c) = self.child.take() else {
            self.stdin = None;
            self.rx = None;
            return;
        };
        // Ask halcmd to quit so it runs hal_exit() and unregisters its
        // transient component; otherwise a SIGKILL leaks a stale
        // "halcmd<pid>" comp in HAL shared memory.
        if let Some(mut stdin) = self.stdin.take() {
            let _ = stdin.write_all(b"quit\n");
            let _ = stdin.flush();
        }
        let mut waited = 0u32;
        while waited < 500 {
            match c.try_wait() {
                Ok(Some(_)) => {
                    self.rx = None;
                    return;
                }
                Ok(None) => {
                    std::thread::sleep(Duration::from_millis(20));
                    waited += 20;
                }
                Err(_) => break,
            }
        }
        let _ = c.kill();
        let _ = c.wait();
        self.rx = None;
    }

    /// (Re)spawn the session if dead, throttled to one attempt per 2 s.
    pub fn ensure(&mut self) {
        if !self.alive() && self.last_spawn.elapsed() > Duration::from_secs(2) {
            self.kill();
            self.spawn();
        }
    }

    /// Drop the session so the next use (re)spawns immediately.
    pub fn restart(&mut self) {
        self.kill();
        self.last_spawn = Instant::now() - Duration::from_secs(10);
    }

    /// True when a real LinuxCNC session is available. A bare `halcmd` call
    /// initializes an empty HAL on its own (transient component `halcmd<pid>`),
    /// so a success exit code is not enough: we check for a `linuxcnc`/`halrun`
    /// process, or for HAL containing any non-transient component.
    pub fn available(&self) -> bool {
        if proc_running("linuxcnc") || proc_running("halrun") {
            return true;
        }
        comps_available(&Self::exec(&["list", "comp"]))
    }

    /// Send N commands, read N response lines.
    pub fn batch(&mut self, cmds: &[String]) -> Vec<BatchOut> {
        if cmds.is_empty() {
            return Vec::new();
        }
        self.ensure();
        let Some(stdin) = self.stdin.as_mut() else {
            return (0..cmds.len())
                .map(|_| BatchOut::error("halcmd unavailable"))
                .collect();
        };
        let mut buf = String::with_capacity(64 * cmds.len());
        for c in cmds {
            buf.push_str(c);
            buf.push('\n');
        }
        if stdin
            .write_all(buf.as_bytes())
            .and_then(|_| stdin.flush())
            .is_err()
        {
            self.kill();
            return (0..cmds.len())
                .map(|_| BatchOut::error("halcmd session died"))
                .collect();
        }
        let Some(rx) = self.rx.as_ref() else {
            self.kill();
            return (0..cmds.len())
                .map(|_| BatchOut::error("halcmd session died"))
                .collect();
        };
        let mut outs = Vec::with_capacity(cmds.len());
        for _ in 0..cmds.len() {
            match rx.recv_timeout(Duration::from_secs(2)) {
                Ok(line) => {
                    let (msg, err) = strip_err_prefix(&line);
                    outs.push(BatchOut { line: msg, err });
                }
                Err(_) => {
                    self.kill();
                    while outs.len() < cmds.len() {
                        outs.push(BatchOut::error("halcmd session died"));
                    }
                    break;
                }
            }
        }
        outs
    }

    /// One-shot halcmd: `halcmd <args...>`. Returns merged output.
    pub fn exec(args: &[&str]) -> String {
        match Command::new("halcmd").args(args).output() {
            Ok(out) => {
                let mut s = String::from_utf8_lossy(&out.stdout).into_owned();
                if !out.stderr.is_empty() {
                    if !s.is_empty() && !s.ends_with('\n') {
                        s.push('\n');
                    }
                    s.push_str(&String::from_utf8_lossy(&out.stderr));
                }
                s
            }
            Err(e) => format!("halcmd: {e}"),
        }
    }

    pub fn list(&self, t: HalType) -> Vec<String> {
        Self::exec(&["list", t.kw()])
            .split_whitespace()
            .map(|s| s.to_string())
            .collect()
    }

    pub fn show(&self, t: HalType, name: &str) -> String {
        Self::exec(&["show", t.kw(), name])
    }
}

/// Whether a process whose executable basename is `name` is running.
fn proc_running(name: &str) -> bool {
    let Ok(out) = Command::new("ps").args(["-e", "-o", "args="]).output() else {
        return false;
    };
    let text = String::from_utf8_lossy(&out.stdout);
    text.lines().any(|l| {
        l.split_whitespace()
            .any(|t| t.rsplit('/').next().map(|b| b == name).unwrap_or(false))
    })
}

/// Does a `halcmd list comp` output indicate a real session? Transient
/// `halcmd<pid>` components (created by halcmd itself) are ignored; a
/// `halcmd: ...` prefix means the binary is missing or failed to run.
fn comps_available(comps: &str) -> bool {
    if comps.starts_with("halcmd:") {
        return false;
    }
    comps.split_whitespace().any(|c| !c.starts_with("halcmd"))
}

/// halcmd error lines from a `-f` session look like `<stdin>:7: message`.
/// Returns (message, was_error).
fn strip_err_prefix(line: &str) -> (String, bool) {
    if let Some(rest) = line.strip_prefix("<stdin>") {
        // rest == ":7: message"
        if let Some(pos) = rest.find(": ") {
            return (rest[pos + 2..].to_string(), true);
        }
    }
    (line.to_string(), false)
}

/// Shell-like tokenizer for the arbitrary HAL command entry.
pub fn tokenize(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if let Some(q) = quote {
            if c == q {
                quote = None;
            } else {
                cur.push(c);
            }
        } else if c == '"' || c == '\'' {
            quote = Some(c);
        } else if c.is_whitespace() {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
        } else if c == '\\' {
            if let Some(n) = chars.next() {
                cur.push(n);
            }
        } else {
            cur.push(c);
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

// --- writability detection (mirrors halshow.tcl) ----------------------

/// 1 = writable, -1 = writable but linked to a signal, 0 = not writable.
pub fn pin_writable(show_out: &str, name: &str) -> i8 {
    for line in show_out.lines() {
        let toks: Vec<&str> = line.split_whitespace().collect();
        if toks.len() >= 5 && toks[4] == name {
            if toks[2] == "IN" {
                if toks.len() >= 6 && (toks[5] == "<==" || toks[5] == "<=>") {
                    return -1;
                }
                return 1;
            }
            return 0;
        }
    }
    0
}

pub fn param_writable(show_out: &str, name: &str) -> i8 {
    for line in show_out.lines() {
        let toks: Vec<&str> = line.split_whitespace().collect();
        if toks.len() >= 5 && toks[4] == name {
            return if toks[2] == "RW" { 1 } else { 0 };
        }
    }
    0
}

/// Signals are writable unless an output pin ("<==") is linked.
pub fn sig_writable(show_out: &str) -> i8 {
    if show_out.contains("<=") {
        0
    } else {
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comps_available_detects_real_session() {
        // empty / only transient halcmd comps -> not running
        assert!(!comps_available(""));
        assert!(!comps_available("halcmd12345"));
        assert!(!comps_available("halcmd1 halcmd2"));
        // any real component -> running
        assert!(comps_available("halcmd1 abs"));
        assert!(comps_available("abs"));
        assert!(comps_available("motmod\nrio"));
        // missing binary / spawn failure -> not running
        assert!(!comps_available("halcmd: No such file or directory"));
    }

    #[test]
    fn session_batch() {
        let mut s = HalSession::new();
        s.ensure();
        let outs = s.batch(&[
            "getp abs.0.tmax".to_string(),
            "getp abs.0.tmax-increased".to_string(),
        ]);
        assert_eq!(outs.len(), 2);
        assert!(!outs[0].err, "err: {}", outs[0].line);
        assert!(!outs[1].err, "err: {}", outs[1].line);
        assert_eq!(outs[0].line.parse::<i64>().is_ok(), true);
        // error path
        let errs = s.batch(&["getp nope.nope".to_string()]);
        assert!(errs[0].err);
    }
}
