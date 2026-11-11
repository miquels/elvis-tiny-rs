#!/usr/bin/env python3
"""Check that every exit path restores the terminal (M8).

    sttycheck.py TERMINFO-DIR BINARY...

Runs `stty -a` before and after the editor inside one pty.  A binary whose
path contains "panic-test" (built with RUSTFLAGS="--cfg elvis_test_panic")
also gets the forced-panic case (^_).  Standard library only.
"""
import os, sys, tempfile, shutil
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from ptyrun import run_pty
terminfo = os.path.abspath(sys.argv[1])
bins = sys.argv[2:]
cases = [
    # name, args, keys, extra env, rows
    ("quit", ["f"], [(0.8, b":q\r")], {}, 24),
    ("wq", ["f"], [(0.8, b"x:wq\r")], {}, 24),
    ("ZZ", ["f"], [(0.8, b"ZZ")], {}, 24),
    ("shell-escape", ["f"], [(0.8, b":!true\r"), (0.5, b"\r"), (0.5, b":q\r")], {}, 24),
    ("ex-mode", ["-e", "f"], [(0.8, b"q\r")], {}, 24),
    ("r-flag", ["-r"], [], {}, 24),
    ("term-unknown", ["f"], [], {"TERM": "nosuchterm"}, 24),
    ("missing-cup", ["f"], [], {"TERM": "missing-cup"}, 24),
    ("tmpdir-fail", ["f"], [], {"EXINIT": "set directory=/nonexistent"}, 24),
    ("small-screen", ["f"], [], {}, 1),
    ("panic", ["f"], [(0.8, b"\x1f"), (0.3, b"")], {}, 24),
]
work = tempfile.mkdtemp(prefix="st.", dir="/tmp")
fails = 0
try:
    for b in bins:
        print("==", b)
        for name, args, keys, extra, rows in cases:
            if name == "panic" and "panic-test" not in b:
                continue
            with open(os.path.join(work, "f"), "w") as f:
                f.write("hello\nworld\n")
            env = {"HOME": work, "EXINIT": "set directory=%s" % work, "TERM": "vt100",
                   "TERMINFO": terminfo, "SHELL": "/bin/sh", "PATH": "/usr/bin:/bin", "LC_ALL": "C"}
            env.update(extra)
            script = ('stty -a > "$W/before" 2>&1; "$@"; echo $? > "$W/st"; stty -a > "$W/after" 2>&1')
            env["W"] = work
            st, out, to = run_pty(["/bin/sh", "-c", script, "sh", os.path.abspath(b)] + args,
                                  env, cwd=work, rows=rows, keys=keys, timeout=8)
            rd = lambda n: open(os.path.join(work, n)).read() if os.path.exists(os.path.join(work, n)) else "<missing>"
            before, after, code = rd("before"), rd("after"), rd("st").strip()
            ok = before == after and before != "<missing>" and not to
            tail = out[-40:]
            if name == "panic":
                i = out.find(b"panicked")
                tail = out[max(0, i - 30):i + 30]
            if not ok:
                fails += 1
            print("  %-4s %-13s status=%-4s restored=%s timeout=%s tail=%r" % ("ok" if ok else "FAIL", name, code, before == after, to, tail))
            for n in os.listdir(work):
                if n != "f":
                    os.unlink(os.path.join(work, n))
finally:
    shutil.rmtree(work)
print("%d failure(s)" % fails)
