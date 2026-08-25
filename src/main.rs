//! haltui — TUI port of the LinuxCNC halshow command.

mod app;
mod fmt;
mod hal;
mod prefs;
mod tree;
mod ui;
mod watch;

use std::io;
use std::process::ExitCode;

use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use app::{App, Cli};

fn usage(prog: &str) {
    println!("Usage:");
    println!("  {prog} [Options] [watchfile]");
    println!("  Options:");
    println!("           --help       (this help)");
    println!("           --fformat    format_string_for_float");
    println!("           --iformat    format_string_for_int");
    println!("           --noprefs    don't use preference file to save settings");
    println!("           --interval   watch update interval in ms");
    println!();
    println!("Notes:");
    println!("       Create watchfile in halshow using: 'File/Save Watch List'.");
    println!("       LinuxCNC must be running for standalone usage.");
}

fn parse_args() -> Result<(Cli, bool), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut cli = Cli {
        fformat: None,
        iformat: None,
        noprefs: false,
        interval: None,
        watchfile: None,
    };
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--help" | "-h" => return Ok((cli, true)),
            "--fformat" => {
                i += 1;
                let v = args.get(i).ok_or("--fformat requires an argument")?;
                cli.fformat = Some(v.clone());
            }
            "--iformat" => {
                i += 1;
                let v = args.get(i).ok_or("--iformat requires an argument")?;
                cli.iformat = Some(v.clone());
            }
            "--interval" => {
                i += 1;
                let v = args.get(i).ok_or("--interval requires an argument")?;
                let n: u64 = v.parse().map_err(|_| format!("bad interval <{v}>"))?;
                cli.interval = Some(n.max(20));
            }
            "--noprefs" => cli.noprefs = true,
            other => {
                if other.starts_with('-') {
                    return Err(format!("unknown option <{other}>"));
                }
                cli.watchfile = Some(other.to_string());
            }
        }
        i += 1;
    }
    Ok((cli, false))
}

fn main() -> ExitCode {
    let prog = std::env::args()
        .next()
        .unwrap_or_else(|| "haltui".to_string());
    let (cli, want_help) = match parse_args() {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{prog}: {e}");
            usage(&prog);
            return ExitCode::from(1);
        }
    };
    if want_help {
        usage(&prog);
        return ExitCode::SUCCESS;
    }

    // restore the terminal even on panic
    let orig_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        orig_hook(info);
    }));

    let mut app = App::new(&cli);
    app.startup(&cli);

    let result = (|| -> io::Result<()> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen)?;
        let backend = CrosstermBackend::new(stdout);
        let mut term = Terminal::new(backend)?;
        let r = app::run(&mut term, &mut app);
        disable_raw_mode()?;
        execute!(io::stdout(), LeaveAlternateScreen)?;
        term.show_cursor()?;
        r
    })();

    app.shutdown();
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("haltui: {e}");
            ExitCode::from(1)
        }
    }
}
