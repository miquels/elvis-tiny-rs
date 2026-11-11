#!/usr/bin/env python3
"""Compare the elv* temporary-file image of the C and Rust binaries (M3).

    tmpimage.py C-BINARY RUST-BINARY TERMINFO-DIR

Each fixture file is loaded by both binaries in turn on a pty (same file, so
the same inode and temporary-file name); the image is copied while the editor
waits for input.  Standard library only.
"""
import os, sys, tempfile, shutil
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from ptyrun import run_pty
cbin, rbin, terminfo = (os.path.abspath(a) for a in sys.argv[1:4])
work = tempfile.mkdtemp(prefix="ti.", dir="/tmp")
cases = {
    "small": b"one\ntwo\nthree\n",
    "nul": b"a\0b\nc\0\0d\n",
    "nonl": b"no trailing newline",
    "near": (b"x" * 1022 + b"\n") * 3,
    "over": b"y" * 1500 + b"\n",
    "blocks": b"".join(b"line %05d of the block test\n" % i for i in range(3000)),
    "exact": (b"z" * 99 + b"\n") * 5000,
    "high": bytes(range(1, 256)).replace(b"\n", b"") + b"\n",
    "empty": b"",
}
snap = {}
def grab(binary, name):
    got = {}
    def copy(pid, master):
        for n in os.listdir(work):
            if n.startswith("elv"):
                with open(os.path.join(work, n), "rb") as f:
                    got[n] = f.read()
    env = {"HOME": work, "EXINIT": "set directory=%s" % work, "TERM": "vt100",
           "TERMINFO": terminfo, "SHELL": "/bin/sh", "PATH": "/usr/bin:/bin", "LC_ALL": "C"}
    st, out, to = run_pty([binary, name], env, cwd=work,
                          keys=[(1.0, copy), (0.2, b"\r\x1b:q!\r"), (0.5, b"\r:q!\r")], timeout=10)
    left = sorted(n for n in os.listdir(work) if n.startswith(("elv", "so")))
    for n in left:
        os.unlink(os.path.join(work, n))
    got["<status>"] = (os.waitstatus_to_exitcode(st) if st is not None else None, to, len(left))
    return got
fails = 0
try:
    for name, data in cases.items():
        with open(os.path.join(work, name), "wb") as f:
            f.write(data)
        c = grab(cbin, name)
        r = grab(rbin, name)
        ok = c == r and len(c) == 2
        if not ok:
            fails += 1
        sz = {k: len(v) for k, v in c.items()}
        print("%-4s %-7s C=%s Rust=%s" % ("ok" if ok else "FAIL", name, sz, {k: len(v) for k, v in r.items()}))
        if not ok and c and r:
            (cn, cv), (rn, rv) = next(iter(c.items())), next(iter(r.items()))
            diff = [i for i in range(min(len(cv), len(rv))) if cv[i] != rv[i]]
            print("     names %s %s, first differing byte %s" % (cn, rn, diff[:5]))
finally:
    shutil.rmtree(work)
print("%d failure(s)" % fails)
sys.exit(1 if fails else 0)
