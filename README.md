# haltui

A rust TUI (text user interface) port of the LinuxCNC `halshow` command.
It shows the running HAL configuration in a tree, lets you inspect HAL
objects, watch live pin/signal/parameter values, and set writable
values — all from the keyboard.

`haltui` talks to HAL through the standard `halcmd` program, so it works
with any installed LinuxCNC and needs nothing but `halcmd` on `$PATH`.

<img width="1904" height="1112" alt="image" src="https://github.com/user-attachments/assets/376f06e3-e9c2-419b-b7a8-8417f476f0fe" />

<img width="1902" height="1115" alt="image" src="https://github.com/user-attachments/assets/527b4be6-0e26-4a10-9f35-b5d628e59a9d" />

## Features

- **Tree view** of Components / Pins / Parameters / Signals / Functions /
  Threads, with live regex filtering (per-segment or full-path) that
  auto-reveals matching branches.
- **SHOW tab** — `hal show` output for any node, plus an arbitrary
  `halcmd` command entry with history.
- **WATCH tab** — watchlist of pins/signals/parameters with polled values
  (bit values as colored dots), set/clear for bits, set-value for
  writable items, unlink pins, load/save watch lists.
- **Settings screen** (`F5`) — polling interval, watch name column width,
  float / integer format overrides, remember-watchlist.
- Reads and writes the **same preference file as halshow**
  (`halshow.preferences`), both directions compatible.
- Loads and saves **.halshow watch files** (oneline and multiline
  formats), byte-compatible with halshow's File menu.
- Every function reachable by keyboard; a contextual key-hint bar and a
  full help overlay (`F1` / `?`) document all bindings.

## Build

```sh
cd haltui
cargo build --release
# binary at target/release/haltui
```

Dependencies: `rustc`/`cargo` (edition 2021), crates `ratatui`,
`crossterm`, `regex`, `libc`. Linux only (uses `dup2`/`pipe` via libc).

## Run

```sh
haltui [options] [watchfile]

  --help       show usage
  --fformat F  format string for float values (overrides settings)
  --iformat F  format string for integer values (overrides settings)
  --noprefs    don't read or write the preference file
  --interval N watch update interval in ms (default 100)

# examples
haltui                          # inspect HAL
haltui my.halshow               # start with a watch list
haltui --fformat %.3f --noprefs
```

LinuxCNC must be running for HAL objects to exist; without it haltui
still starts and shows an empty tree, retrying the HAL connection.

## Keyboard reference

Full list is in-app: press `F1` or `?` (self-documenting). Summary:

| Key | Action |
| ----- | -------- |
| `q` / `Ctrl+C` | quit (prefs + watchlist saved) |
| `F1` / `?` | help overlay (`/` searches it incrementally) |
| `Tab` / `Shift+Tab` | next / previous tab |
| `1` `2` | SHOW / WATCH tab |
| `F2` `F3` `F4` | focus tree / content / command entry |
| `F5` | open / close the settings screen |
| `/` | fresh tree search from any panel (clears previous filter) |
| `[` `]` | shrink / grow tree panel |

**Tree:** `↑↓` move / `PgUp PgDn` page (with SHOW tab active, the pane
live-previews the selection), `→` expand a closed node; on an open node it
jumps to the right pane when useful (WATCH list, or SHOW output longer
than one page), `←` collapse / parent, `Space` toggle, `Enter` show or
add-to-watch, `a` add leaf, `A` add subtree, `s` show in SHOW, `e`/`w`
expand/collapse all, `E`/`W` per type, `/` filter, `f` full-path, `r` reload.

**SHOW:** `↑↓ PgUp PgDn Home` scroll, `a` add shown item, `c` focus command.

**WATCH:** `↑↓` select / `PgUp PgDn` page, `Space` toggle a writable bit,
`Enter` set value, `s`/`c` set/clear bit, `u` unlink, `x` remove, `r`
reload, `e` erase, `a` add by name, `o` show in tree, `S` save (oneline),
`m` save (multiline), `L` load. Newly added items are selected so they
are scrolled into view.

**Settings screen:** `↑↓` select field, `Enter`/`Space` edit or toggle,
`Esc`/`F5` close, `←` close + back to tree.

## Preference file

Same locations and format as halshow.tcl:

1. `$CONFIG_DIR/halshow.preferences`
2. directory of the `.ini` of a running `linuxcnc` process
3. `~/.halshow_preferences`

`haltui` writes a Tcl file halshow can read, and parses the file halshow
writes — geometry, split ratio, workmode, watch interval, column width,
format strings, remember-list, and the saved watchlist. (The home-form
watchlist and `alwaysOnTop` flag are preserved on round-trip for halshow
compatibility even though a terminal has no "always on top" concept.)

## Architecture

```
src/main.rs     CLI parsing, terminal setup, panic-safe teardown
src/app.rs      App state, event loop, key dispatch
src/hal.rs      halcmd session (persistent -k -f child, batched reads)
src/tree.rs     HAL tree model, filtering, navigation
src/watch.rs    watch items + .halshow file load/save
src/prefs.rs    halshow.preferences read/write + location
src/fmt.rs      mini printf-style formatter for ffmts/ifmts
src/ui.rs       ratatui rendering + help overlay + key-hint footer
```

The HAL session keeps one `halcmd -k -f` child alive and batches
`getp`/`gets`/`ptype`/`stype` reads (one output line per command), while
`show`, `list`, `setp`, `sets`, `unlinkp` and arbitrary commands run as
one-shot `halcmd` invocations.

## Tests

```sh
cargo test              # unit tests: formatter, prefs, watch parse, session
python3 tests/smoke.py  # TUI smoke: renders, tabs, help, filter, quit
python3 tests/drive2.py # tree nav, watch add, set 42, command entry, prefs
python3 tests/drive3.py # watch files load/save, erase, settings
```

The Python drivers spawn haltui in a pty and decode output with `pyte`
(pip install pyte). `drive2.py`/`drive3.py` expect a few HAL objects:
`halcmd loadrt threads name1=test-thread period1=1000000` and
`halcmd loadrt abs`.

## License

GPL-2.0 (same as LinuxCNC halshow).
