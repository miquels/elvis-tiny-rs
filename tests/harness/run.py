#!/usr/bin/env python3
"""Differential test harness for the elvis-tiny Rust port.

    run.py [--c PATH] [--rust PATH] --suite NAME [--update-golden]
           [--repeat N] [--only SCENARIO] [--keep]

Scenarios live in tests/scenarios/<suite>/<name>.json and golden results in
tests/golden/<os>/<suite>/<name>.json.  Each binary given is run on every
scenario and compared with the golden result; with --update-golden the C
result is written as the new golden file instead.  --suite all runs every
suite.  Standard library only.

Scenario keys (all optional except one of "stdin"/"keys"):
  mode          "piped" (default) or "pty"
  progname      name the binary is invoked as (symlink), default "elvis"
  args          list of arguments
  files         {name: content}; content is a latin-1 string, or
                {"repeat": str, "count": n}, or {"mode": 0o444, "data": ...},
                or {"dir": true}
  stdin         latin-1 string fed to stdin (piped mode)
  keys          list of [delay_seconds, latin-1 string] (pty mode), or
                [delay, {"signal": "INT"|"WINCH", "rows": r, "cols": c}]
  env           {name: value or null to unset}; applied after the defaults
  term, rows, cols, speed, timeout
  compare       {"output": bool, "files": bool, "status": bool}
  divergences   list of Appendix B IDs this scenario depends on
  os            list of host OS names ("linux", "macos") the scenario runs on;
                default all
  leftover      expected list of leftover temporary-file kinds
  description   free text
"""
import argparse
import json
import os
import platform
import shutil
import signal
import struct
import subprocess
import sys
import tempfile
import time

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
sys.path.insert(0, os.path.join(ROOT, "tests", "tools"))
from ptyrun import run_pty  # noqa: E402

SCEN_DIR = os.path.join(ROOT, "tests", "scenarios")
OSNAME = {"Darwin": "macos", "Linux": "linux"}[platform.system()]
GOLD_DIR = os.path.join(ROOT, "tests", "golden", OSNAME)
TERMINFO = os.path.join(ROOT, "target", "terminfo-fixtures")
FIXBIN = os.path.join(ROOT, "tests", "fixtures", "bin")


def lat(s):
    return s.encode("latin-1")


def esc(data):
    """Readable escaping that keeps one output line per text line."""
    out = []
    line = []
    for b in data:
        if b == 0x0A:
            line.append("\\n")
            out.append("".join(line))
            line = []
        elif 0x20 <= b < 0x7F and b != 0x5C:
            line.append(chr(b))
        elif b == 0x1B:
            line.append("\\e")
        elif b == 0x0D:
            line.append("\\r")
        else:
            line.append("\\x%02x" % b)
    if line:
        out.append("".join(line))
    return out


def tmpkind(name):
    if name.startswith("elv_"):
        return "cutbuf"
    if name.startswith("elv"):
        return "tmpfile"
    if name.startswith("so") and len(name) == 8:
        return "scratch"
    return None


def make_file(path, spec):
    if isinstance(spec, dict) and spec.get("dir"):
        os.mkdir(path)
        if "mode" in spec:
            os.chmod(path, spec["mode"])
        return
    mode = None
    if isinstance(spec, dict) and "repeat" in spec:
        data = lat(spec["repeat"]) * spec["count"]
    elif isinstance(spec, dict):
        mode = spec.get("mode")
        data = lat(spec["data"]) if "data" in spec else lat(spec["repeat"]) * spec["count"]
    else:
        data = lat(spec)
    with open(path, "wb") as f:
        f.write(data)
    if mode is not None:
        os.chmod(path, mode)


def snapshot(work):
    files, left = {}, []
    for name in sorted(os.listdir(work)):
        p = os.path.join(work, name)
        kind = tmpkind(name)
        if kind:
            left.append(kind)
            continue
        if os.path.isdir(p):
            files[name] = "<dir>"
            continue
        try:
            with open(p, "rb") as f:
                files[name] = esc(f.read())
        except OSError as e:
            files[name] = "<unreadable %s>" % e.errno
    return files, sorted(left)


def run_one(binary, sc):
    # Keep the path short: o_directory is a 30-byte array in C and longer
    # values overflow into the next option (Appendix B).
    top = os.path.realpath(tempfile.mkdtemp(prefix="eh.", dir="/tmp"))
    work = os.path.join(top, "w")
    bindir = os.path.join(top, "bin")
    os.mkdir(work)
    os.mkdir(bindir)
    try:
        for name, spec in sc.get("files", {}).items():
            make_file(os.path.join(work, name), spec)
        prog = os.path.join(bindir, sc.get("progname", "elvis"))
        os.symlink(os.path.abspath(binary), prog)
        env = {
            "HOME": work,
            "EXINIT": "set directory=%s" % work,
            "TERM": sc.get("term", "vt100"),
            "TERMINFO": TERMINFO,
            "SHELL": "/bin/sh",
            "PATH": "%s:/usr/bin:/bin" % FIXBIN,
            "LC_ALL": "C",
        }
        for k, v in sc.get("env", {}).items():
            if v is None:
                env.pop(k, None)
            else:
                env[k] = v.replace("$WORK", work)
        argv = [prog] + [a.replace("$WORK", work) for a in sc.get("args", [])]
        timeout = sc.get("timeout", 10)
        if sc.get("mode", "piped") == "pty":
            keys = []
            for delay, data in sc.get("keys", []):
                if isinstance(data, dict):
                    keys.append((delay, _signaller(data)))
                else:
                    keys.append((delay, lat(data)))
            status, out, to = run_pty(argv, env, cwd=work,
                                      rows=sc.get("rows", 24),
                                      cols=sc.get("cols", 80),
                                      speed=sc.get("speed", 38400),
                                      keys=keys, timeout=timeout)
        else:
            try:
                p = subprocess.run(argv, input=lat(sc.get("stdin", "")),
                                   stdout=subprocess.PIPE,
                                   stderr=subprocess.STDOUT, env=env, cwd=work,
                                   timeout=timeout, start_new_session=True)
                status, out, to = p.returncode, p.stdout, False
            except subprocess.TimeoutExpired as e:
                status, out, to = None, e.stdout or b"", True
        if status is not None and sc.get("mode", "piped") == "pty":
            status = os.waitstatus_to_exitcode(status)
        out = out.replace(lat(work), b"<WORK>").replace(lat(top), b"<TOP>")
        # restore permissions so the snapshot and cleanup can proceed
        for name in os.listdir(work):
            p = os.path.join(work, name)
            if not os.path.islink(p):
                os.chmod(p, 0o755 if os.path.isdir(p) else 0o644)
        files, left = snapshot(work)
        return {
            "status": "timeout" if to else status,
            "files": files,
            "leftover": left,
            "output": esc(out),
        }
    finally:
        shutil.rmtree(top, ignore_errors=True)


def _signaller(spec):
    def fire(pid, master):
        if "rows" in spec:
            import fcntl
            import termios
            fcntl.ioctl(master, termios.TIOCSWINSZ,
                        struct.pack("HHHH", spec["rows"], spec["cols"], 0, 0))
        if spec.get("signal") == "WINCH" and "rows" in spec:
            return  # the kernel sends SIGWINCH to the foreground group
        os.kill(pid, getattr(signal, "SIG" + spec["signal"]))
    return fire


def compare(sc, got, want):
    cmp = {"output": True, "files": True, "status": True}
    cmp.update(sc.get("compare", {}))
    diffs = []
    for key in ("status", "files", "output"):
        if cmp[key] and got[key] != want[key]:
            diffs.append(key)
    if got["leftover"] != want["leftover"]:
        diffs.append("leftover")
    return diffs


def show_diff(key, got, want):
    import difflib
    a = want[key] if isinstance(want[key], list) else json.dumps(want[key], indent=1).splitlines()
    b = got[key] if isinstance(got[key], list) else json.dumps(got[key], indent=1).splitlines()
    for line in difflib.unified_diff(a, b, "expected", "actual", lineterm="", n=2):
        print("      " + line)


def suites(name):
    if name == "all":
        return sorted(d for d in os.listdir(SCEN_DIR)
                      if os.path.isdir(os.path.join(SCEN_DIR, d)))
    return [name]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--c")
    ap.add_argument("--rust")
    ap.add_argument("--suite", required=True)
    ap.add_argument("--only")
    ap.add_argument("--update-golden", action="store_true")
    ap.add_argument("--repeat", type=int, default=1)
    a = ap.parse_args()
    if not os.path.isdir(TERMINFO):
        sys.exit("run tests/tools/build-terminfo.sh first")
    if a.update_golden and not a.c:
        sys.exit("--update-golden needs --c")
    bins = [(n, p) for n, p in (("C", a.c), ("Rust", a.rust)) if p]
    if not bins:
        sys.exit("give --c and/or --rust")
    fails = total = 0
    for suite in suites(a.suite):
        sdir = os.path.join(SCEN_DIR, suite)
        for fn in sorted(os.listdir(sdir)):
            if not fn.endswith(".json"):
                continue
            name = fn[:-5]
            if a.only and a.only != name:
                continue
            with open(os.path.join(sdir, fn)) as f:
                sc = json.load(f)
            if OSNAME not in sc.get("os", [OSNAME]):
                continue
            gpath = os.path.join(GOLD_DIR, suite, fn)
            want = None
            if os.path.exists(gpath):
                with open(gpath) as f:
                    want = json.load(f)
            for label, binary in bins:
                for rep in range(a.repeat):
                    total += 1
                    got = run_one(binary, sc)
                    if a.update_golden and label == "C" and rep == 0:
                        os.makedirs(os.path.dirname(gpath), exist_ok=True)
                        with open(gpath, "w") as f:
                            json.dump(got, f, indent=1)
                            f.write("\n")
                        want = got
                        print("UPDATE %s/%s" % (suite, name))
                        continue
                    if want is None:
                        fails += 1
                        print("NOGOLD %s/%s [%s]" % (suite, name, label))
                        continue
                    d = compare(sc, got, want)
                    if d:
                        fails += 1
                        print("FAIL %s/%s [%s run %d]: %s" % (suite, name, label, rep + 1, ", ".join(d)))
                        for key in d:
                            show_diff(key, got, want)
                    else:
                        print("ok   %s/%s [%s run %d]" % (suite, name, label, rep + 1))
    print("%d run(s), %d failure(s)" % (total, fails))
    return 1 if fails else 0


if __name__ == "__main__":
    sys.exit(main())
