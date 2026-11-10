#!/usr/bin/env python3
"""Run a command on a pseudo-terminal with a fixed size and line speed.

Standard library only.  Used by the capability probe and by the
differential harness (tests/harness/run.py imports run_pty from here).
"""
import argparse
import errno
import fcntl
import os
import pty
import select
import signal
import struct
import sys
import termios
import time

SPEEDS = {n: getattr(termios, "B%d" % n) for n in
          (300, 1200, 2400, 9600, 19200, 38400) if hasattr(termios, "B%d" % n)}


def _setup_slave(fd, rows, cols, speed, raw_out):
    attrs = termios.tcgetattr(fd)
    if raw_out:
        attrs[1] &= ~termios.OPOST
    if speed is not None:
        attrs[4] = SPEEDS[speed]
        attrs[5] = SPEEDS[speed]
    termios.tcsetattr(fd, termios.TCSANOW, attrs)
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))


def run_pty(argv, env, cwd=None, rows=24, cols=80, speed=38400, keys=(),
            timeout=10.0, raw_out=False, settle=0.3, on_tick=None):
    """Run argv on a pty.

    keys is a sequence of (delay_seconds, bytes); each chunk is written after
    waiting delay_seconds of wall time since the previous chunk.  Returns
    (status, output_bytes, timed_out) where status is the raw wait status.
    """
    pid, master = pty.fork()
    if pid == 0:
        try:
            _setup_slave(0, rows, cols, speed, raw_out)
            if cwd:
                os.chdir(cwd)
            os.execve(argv[0], argv, env)
        finally:
            os._exit(127)
    out = bytearray()
    deadline = time.monotonic() + timeout
    pending = list(keys)
    next_at = time.monotonic() + (pending[0][0] if pending else 0)
    status = None
    timed_out = False
    while True:
        now = time.monotonic()
        if now >= deadline:
            timed_out = True
            try:
                os.killpg(pid, signal.SIGKILL)
            except OSError:
                os.kill(pid, signal.SIGKILL)
            break
        if pending and now >= next_at:
            _, data = pending.pop(0)
            if callable(data):
                data(pid, master)
            else:
                os.write(master, data)
            if pending:
                next_at = time.monotonic() + pending[0][0]
        wait = min(deadline, next_at if pending else deadline) - time.monotonic()
        r, _, _ = select.select([master], [], [], max(0.0, min(wait, 0.05)))
        if r:
            try:
                chunk = os.read(master, 65536)
            except OSError as e:
                if e.errno != errno.EIO:
                    raise
                chunk = b""
            if not chunk:
                break
            out += chunk
        wpid, st = os.waitpid(pid, os.WNOHANG)
        if wpid:
            status = st
            # drain whatever is left
            end = time.monotonic() + settle
            while time.monotonic() < end:
                r, _, _ = select.select([master], [], [], 0.05)
                if not r:
                    continue
                try:
                    chunk = os.read(master, 65536)
                except OSError:
                    break
                if not chunk:
                    break
                out += chunk
            break
    if status is None:
        _, status = os.waitpid(pid, 0)
    os.close(master)
    return status, bytes(out), timed_out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--rows", type=int, default=24)
    ap.add_argument("--cols", type=int, default=80)
    ap.add_argument("--speed", type=int, default=38400)
    ap.add_argument("--raw-out", action="store_true",
                    help="clear OPOST so output bytes are not translated")
    ap.add_argument("--timeout", type=float, default=10.0)
    ap.add_argument("argv", nargs="+")
    a = ap.parse_args()
    status, out, to = run_pty(a.argv, dict(os.environ), rows=a.rows,
                              cols=a.cols, speed=a.speed, raw_out=a.raw_out,
                              timeout=a.timeout)
    sys.stdout.buffer.write(out)
    if to:
        sys.stderr.write("ptyrun: timeout\n")
        return 124
    return os.waitstatus_to_exitcode(status) & 0xff


if __name__ == "__main__":
    sys.exit(main())
