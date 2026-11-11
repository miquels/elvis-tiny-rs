#!/usr/bin/env python3
"""Key timeout characterization (c-behavior.md "Key timeout", B5).

    keytime.py BINARY TERMINFO-DIR

With keytime=1, :map QA x and :map QB dd, types Q, waits 3 s, then A.  If
the file loses its first character, the QA mapping ran (no timeout).
"""
import os, sys, tempfile, shutil
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from ptyrun import run_pty
binary = os.path.abspath(sys.argv[1])
work = tempfile.mkdtemp(prefix="kt.", dir="/tmp")
try:
    with open(os.path.join(work, "f"), "w") as f: f.write("abc\ndef\n")
    env = {"HOME": work, "EXINIT": "set directory=%s keytime=1" % work, "TERM": "vt100",
           "TERMINFO": os.path.abspath(sys.argv[2]), "SHELL": "/bin/sh", "PATH": "/usr/bin:/bin", "LC_ALL": "C"}
    keys = [(0.5, b":map QA x\r"), (0.3, b":map QB dd\r"), (0.3, b"Q"), (3.0, b"A"), (0.5, b"\x1b"), (0.3, b":wq\r")]
    st, out, to = run_pty([binary, "f"], env, cwd=work, keys=keys, timeout=10)
    print("status", os.waitstatus_to_exitcode(st) if st is not None else st, "timeout", to)
    print("file:", open(os.path.join(work, "f")).read().encode())
finally:
    shutil.rmtree(work)
