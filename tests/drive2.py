#!/usr/bin/env python3
"""haltui interaction test: tree nav, watch add, setp, command entry,
prefs save. Needs HAL running with at least one RW param."""

import fcntl
import importlib
import os
import pty
import select
import struct
import subprocess
import sys
import tempfile
import termios
import time

# pyte lives in the test venv; load it dynamically so static
# analyzers without the venv on their path don't flag it.
pyte = importlib.import_module("pyte")

BIN = os.path.join(os.path.dirname(__file__), "..", "target/debug/haltui")


class Haltui:
    def __init__(self, args=None, cols=120, rows=40):
        self.pid, self.fd = pty.fork()
        if self.pid == 0:
            os.environ["TERM"] = "xterm-256color"
            os.execv(BIN, [BIN] + (args or []))
        fcntl.ioctl(self.fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))
        self.out = b""

    def pump(self, t=0.4):
        end = time.time() + t
        while time.time() < end:
            r, _, _ = select.select([self.fd], [], [], 0.05)
            if r:
                try:
                    chunk = os.read(self.fd, 65536)
                except OSError:
                    return
                if chunk:
                    self.out += chunk

    def send(self, s, delay=0.3):
        self.pump(delay)
        os.write(self.fd, s.encode())
        self.pump(delay)

    def screen(self):
        """Decode the pty stream through a terminal emulator and join
        all visible lines into one string for substring checks."""
        scr = pyte.Screen(120, 40)
        stream = pyte.Stream(scr)
        stream.feed(self.out.decode("utf-8", "replace"))
        return "\n".join(l.rstrip() for l in scr.display)

    def close(self):
        try:
            os.close(self.fd)
        except OSError:
            pass
        for _ in range(20):
            p, _ = os.waitpid(self.pid, os.WNOHANG)
            if p:
                return
            time.sleep(0.1)
        os.kill(self.pid, 9)


def expect(name, cond):
    print(("PASS: " if cond else "FAIL: ") + name)
    return cond


def main():
    ok = True
    # ensure the harmless math fixture component is present (idempotent;
    # the test only reads/writes abs.0.*, never machine pins)
    subprocess.run(["halcmd", "loadrt", "abs"], capture_output=True)
    time.sleep(0.6)
    h = Haltui(args=["--noprefs"])
    h.pump(1.2)
    ok &= expect("SHOW tab initial", "HAL show output" in h.screen())

    h.send("w", 0.5)  # collapse all → the six roots fit on screen
    ok &= expect(
        "tree roots",
        all(
            t in h.screen()
            for t in [
                "Components",
                "Pins",
                "Parameters",
                "Signals",
                "Functions",
                "Threads",
            ]
        ),
    )

    # deterministic nav: filter "tmax" (only param nodes survive), then
    # expand — visible: comp, pin, param, abs, 0, tmax, test-thread, tmax
    h.send("/", 0.3)
    h.send("tmax", 0.6)
    h.send("\x1b", 0.3)  # back to tree
    h.send("e", 0.5)
    ok &= expect("tree expanded shows tmax", "tmax" in h.screen())

    h.send("\x1b[B" * 5, 0.4)  # Down x5 → first tmax leaf
    h.send("a", 0.8)  # add to watch
    s = h.screen()
    ok &= expect("watch add status", "added" in s)

    h.send("2", 0.8)  # WATCH tab
    s = h.screen()
    ok &= expect("watch row shown", "abs.0.tmax" in s)

    # set value: Enter → input prefilled → clear → 42
    h.send("\r", 0.4)
    h.send("\x15")  # Ctrl+U clear
    h.send("42", 0.2)
    h.send("\r", 0.8)
    s = h.screen()
    ok &= expect("watch value 42", "42" in s)

    got = subprocess.run(
        ["halcmd", "getp", "abs.0.tmax"], capture_output=True, text=True
    )
    ok &= expect("halcmd sees 42", got.stdout.strip() == "42")

    # command entry: F4 → show param
    h.send("\x1b[14~", 0.3)  # F4
    h.send("show param abs.0.tmax", 0.3)
    h.send("\r", 0.8)
    s = h.screen()
    ok &= expect("command output shown", "Parameters:" in s and "tmax" in s)

    # help overlay
    h.send("?", 0.4)
    ok &= expect("help overlay", "haltui help" in h.screen())
    h.send("\x1b", 0.3)

    # filter: '/' + clear + type, tree filters live
    h.send("/", 0.3)
    h.send("\x15", 0.2)  # Ctrl+U clears previous filter
    h.send("abs", 0.6)
    s = h.screen()
    ok &= expect("filter live", "abs" in s)
    h.send("\x1b", 0.3)

    h.send("q", 0.5)
    h.close()
    subprocess.run(["halcmd", "setp", "abs.0.tmax", "0"], check=False)

    # ---- prefs round trip test
    d = tempfile.mkdtemp()
    env = dict(os.environ, CONFIG_DIR=d)
    p = subprocess.Popen(
        [BIN],
        env=env,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    time.sleep(1.2)
    p.terminate()
    p.wait()
    pf = os.path.join(d, "halshow.preferences")
    ok &= expect("prefs file written", os.path.isfile(pf))
    if os.path.isfile(pf):
        txt = open(pf).read()
        ok &= expect("prefs has workmode", "set ::workmode showhal" in txt)
        ok &= expect("prefs has ratio", "placeFrames 0.3" in txt)
        ok &= expect("prefs has interval", "set ::watchInterval" in txt)
    subprocess.run(["rm", "-rf", d])

    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
