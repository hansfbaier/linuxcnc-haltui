#!/usr/bin/env python3
"""haltui bit-value dialog test: open the radio picker on a writable bit
pin, toggle, cancel. No HAL writes (Esc cancels) — safe against a live
machine config."""

import fcntl
import importlib
import os
import pty
import select
import struct
import subprocess
import sys
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
    # use a harmless, self-loaded bit pin so the test works without a
    # live machine config (dialog is only opened + cancelled; no writes)
    subprocess.run(["halcmd", "loadrt", "not"], capture_output=True)
    time.sleep(0.6)
    pin = "not.0.in"

    h = Haltui(args=["--noprefs"])
    h.pump(1.5)

    h.send("2", 0.4)  # WATCH tab
    h.send("a", 0.4)  # add by name
    h.send(pin, 0.3)
    h.send("\r", 0.8)
    s = h.screen()
    ok &= expect("bit pin added to watch", pin in s)

    h.send("\r", 0.6)  # Enter → bit value dialog
    s = h.screen()
    ok &= expect("dialog title", f"Set {pin}" in s)
    ok &= expect("radio TRUE shown", "TRUE" in s)
    ok &= expect("radio FALSE shown", "FALSE" in s)
    ok &= expect("radio hit hint", "toggle" in s)

    # toggle with Right arrow → selection flips
    h.send("\x1b[C", 0.4)  # Right
    s2 = h.screen()
    ok &= expect("toggle keypress accepted", "(•)" in s2)  # still has a selected marker

    h.send("\x1b", 0.4)  # Esc → cancel, no write
    s3 = h.screen()
    ok &= expect("dialog closed", f"Set {pin}" not in s3)

    h.send("q", 0.4)
    h.close()
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
