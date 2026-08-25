#!/usr/bin/env python3
"""haltui watch-file tests: load .halshow, add by name, erase, save
formats, settings apply. Needs HAL with abs.0.* pins/params loaded."""

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
    d = tempfile.mkdtemp()
    wf = os.path.join(d, "test.halshow")
    with open(wf, "w") as f:
        f.write("# halshow watchlist created 123\n\n")
        f.write("pin+abs.0.in\nparam+abs.0.tmax\n")

    h = Haltui(args=["--noprefs", wf])
    h.pump(1.5)
    s = h.screen()
    ok &= expect("load switches to WATCH tab", "Watchlist empty" not in s)
    ok &= expect("pin row", "abs.0.in" in s)
    ok &= expect("param row", "abs.0.tmax" in s)
    ok &= expect("title has file", "test.halshow" in s)

    # add by name: watch focus, 'a', type "sig+nonexist" should fail,
    # then valid param
    h.send("a", 0.3)
    h.send("param+abs.0.tmax-increased", 0.2)
    h.send("\r", 0.8)
    ok &= expect("add by name status", "added" in h.screen())

    # save oneline to new file
    wf2 = os.path.join(d, "out.halshow")
    h.send("S", 0.3)  # save oneline
    h.send("\x15", 0.2)  # clear
    h.send(wf2, 0.2)
    h.send("\r", 0.8)
    ok &= expect("save status", "saved" in h.screen())
    if os.path.isfile(wf2):
        txt = open(wf2).read().strip()
        toks = txt.split()
        ok &= expect(
            "save oneline format",
            len(txt.splitlines()) == 1
            and "pin+abs.0.in" in toks
            and "param+abs.0.tmax" in toks
            and "param+abs.0.tmax-increased" in toks,
        )
    else:
        ok &= expect("save file exists", False)

    # multiline save
    wf3 = os.path.join(d, "outm.halshow")
    h.send("m", 0.3)
    h.send("\x15", 0.2)
    h.send(wf3, 0.2)
    h.send("\r", 0.8)
    if os.path.isfile(wf3):
        txt = open(wf3).read()
        ok &= expect("multiline header", txt.startswith("# halshow watchlist created"))
        ok &= expect(
            "multiline items", "pin+abs.0.in\n" in txt and "param+abs.0.tmax\n" in txt
        )
    else:
        ok &= expect("multiline file exists", False)

    # erase
    h.send("e", 0.6)
    ok &= expect("erase clears list", "Watchlist empty" in h.screen())

    # settings: interval change (F5 opens the settings screen)
    h.send("\x1b[15~", 0.3)  # F5 → settings
    h.send("\r", 0.3)  # edit interval field
    h.send("\x15", 0.2)
    h.send("50", 0.2)
    h.send("\r", 0.6)
    ok &= expect("interval applied", "Update interval set to 50 ms" in h.screen())

    h.send("q", 0.5)
    h.close()
    subprocess.run(["halcmd", "setp", "abs.0.tmax", "0"], check=False)
    subprocess.run(["rm", "-rf", d])
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
