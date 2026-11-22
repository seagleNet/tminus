#!/usr/bin/env python3
"""Measure t-'s output, CPU and wakeups per font and effect, in a pty.

usage: scripts/bench.py BIN  ->  one JSON line per scenario
env: SECS (measured seconds per scenario, default 5),
     ONLY (substring filter on "WxH" and scenario name, e.g. "250x70 nixie matrix")

Don't build or run other heavy work meanwhile (it skews CPU), and alternate
the binaries being compared over a few runs: effects are random.
"""

import fcntl
import json
import os
import pty
import signal
import struct
import sys
import termios
import threading
import time
from pathlib import Path

BIN = sys.argv[1]
SECS, WARM = float(os.environ.get("SECS", "5")), 1.0
SCEN = [
    ("block idle", ["-f", "block", "-e", "none"]),
    ("block decrypt", ["-f", "block", "-e", "decrypt"]),
    ("block matrix", ["-f", "block", "-e", "matrix"]),
    ("block binarypath", ["-f", "block", "-e", "binarypath"]),
    ("nixie idle", ["-f", "nixie", "-e", "none"]),
    ("nixie decrypt", ["-f", "nixie", "-e", "decrypt"]),
    ("nixie matrix", ["-f", "nixie", "-e", "matrix"]),
    ("nixie binarypath", ["-f", "nixie", "-e", "binarypath"]),
]
SIZES = [(120, 40), (250, 70)]


def stat(pid):
    f = Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()
    cpu = (int(f[11]) + int(f[12])) / os.sysconf("SC_CLK_TCK")
    status = Path(f"/proc/{pid}/status").read_text().splitlines()
    vol = int(
        next(line for line in status if line.startswith("voluntary_ctxt")).split()[1]
    )
    return cpu, vol


def run(args, w, h):
    pid, fd = pty.fork()
    if pid == 0:
        os.environ["XDG_RUNTIME_DIR"] = "/tmp/tmb"
        os.makedirs("/tmp/tmb", exist_ok=True)
        os.environ["TERM"] = "xterm-256color"
        os.execv(
            BIN,
            [
                BIN,
                "10m",
                "--no-config",
                "--every",
                "1",
                "--no-bell",
                "--no-notify",
                "--fps",
                "60",
            ]
            + args,
        )
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", h, w, 0, 0))
    count = [0]
    stop = [False]

    def drain():
        answered = False
        while not stop[0]:
            try:
                b = os.read(fd, 65536)
            except OSError:
                break
            count[0] += len(b)
            if not answered and b"\x1b[c" in b:
                os.write(fd, b"\x1b]11;rgb:0c0c/0c0c/0c0c\x1b\\\x1b[?62c")
                answered = True

    t = threading.Thread(target=drain, daemon=True)
    t.start()
    time.sleep(WARM)
    c0, v0, b0, t0 = *stat(pid), count[0], time.time()
    time.sleep(SECS)
    c1, v1, b1, t1 = *stat(pid), count[0], time.time()
    os.kill(pid, signal.SIGTERM)
    os.waitpid(pid, 0)
    stop[0] = True  # only now: undrained, its last writes block and waitpid hangs
    dt = t1 - t0
    return {
        "cpu": 100 * (c1 - c0) / dt,
        "kib": (b1 - b0) / dt / 1024,
        "wake": (v1 - v0) / dt,
    }


ONLY = os.environ.get("ONLY")
for w, h in SIZES:
    if ONLY and f"{w}x{h}" not in ONLY:
        continue
    for name, a in SCEN:
        if ONLY and name not in ONLY:
            continue
        print(
            json.dumps({"name": name, "size": f"{w}x{h}", **run(a, w, h)}), flush=True
        )
