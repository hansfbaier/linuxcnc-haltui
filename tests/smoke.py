#!/usr/bin/env python3
"""Smoke-test haltui through a pty: spawn, drive keys, assert screens."""

import fcntl
import os
import pty
import select
import struct
import sys
import termios
import time

BIN = os.path.join(os.path.dirname(__file__), "..", "target/debug/haltui")


def run(keys_script, cols=120, rows=40, timeout=15):
    pid, fd = pty.fork()
    if pid == 0:
        os.environ["TERM"] = "xterm-256color"
        os.execv(BIN, [BIN, "--noprefs"])
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))
    out = b""
    screens = []

    def pump(t):
        nonlocal out
        end = time.time() + t
        while time.time() < end:
            r, _, _ = select.select([fd], [], [], 0.1)
            if r:
                try:
                    chunk = os.read(fd, 65536)
                except OSError:
                    return False
                if chunk:
                    out += chunk
        return True

    for action, delay in keys_script:
        if not pump(delay):
            break
        screens.append((action, out.decode("utf-8", "replace")))
        if action:
            os.write(fd, action.encode())
    pump(0.5)
    screens.append(("final", out.decode("utf-8", "replace")))
    try:
        os.close(fd)
    except OSError:
        pass
    # ensure child dead
    for _ in range(30):
        p, st = os.waitpid(pid, os.WNOHANG)
        if p:
            break
        time.sleep(0.1)
    else:
        os.kill(pid, 9)
    return screens


def visible(text):
    # strip ANSI escapes
    import re

    return re.sub(r"\x1b\[[0-9;?]*[a-zA-Z]|\x1b[()][0-9A-B]", "", text)


def main():
    script = [
        ("", 1.0),  # startup + tree load
        ("1", 0.3),  # SHOW tab
        ("2", 0.3),  # WATCH tab
        ("3", 0.3),  # SETTINGS tab
        ("\t", 0.3),  # tab cycle
        ("F2", 0.2),  # focus tree — need escape seq
    ]
    # F2 as escape sequence (xterm)
    script = [
        ("", 1.0),
        ("\x1b[15~", 0.3),  # F5 → settings screen
        ("\x1b", 0.3),  # Esc → close settings
        ("2", 0.3),
        ("1", 0.3),
        ("/", 0.2),
        ("f", 0.2),
        ("\x1b", 0.3),  # filter: toggle fullpath, esc
        ("e", 0.4),  # expand all
        ("w", 0.3),  # collapse all
        ("\x1b[1~", 0.4),  # Home (no-op mostly)
        ("?", 0.4),  # help
        ("\x1b", 0.2),
        ("q", 0.4),  # quit
    ]
    screens = run(script)
    ok = True
    for action, screen in screens:
        t = visible(screen)
        checks = []
        if "haltui" in t:
            checks.append("title")
        if "Tree View" in t:
            checks.append("tree-panel")
        if "SHOW" in t:
            checks.append("tabs")
        if "F1" in t or "help" in t.lower():
            checks.append("hints")
        print(f"--- after {action!r}: {checks}")
        if not checks and action:
            print("WARN: nothing recognized after", action)
            print(t[-2000:])
            ok = False
    # final screen checks
    t = visible(screens[-1][1])
    for needle in ["Halshow", "SHOW", "WATCH", "Filter"]:
        if needle in t:
            print("found:", needle)
        else:
            print("MISSING:", needle)
            ok = False
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
