#!/usr/bin/env python3
"""Seeded random differential test of the C and Rust binaries (M9 step 4).

    fuzz.py --c PATH --rust PATH [--ex N] [--vi N] [--seed S] [--jobs J]
            [--save DIR]

Each sequence is generated from its own seed (S + index), so a failure is
reproduced by its seed alone.  Ex sequences run in piped ex mode and compare
exit status, files, leftover temporary files and the whole output.  Vi
sequences type random vi keys on a pty in one burst and compare exit status,
files and leftovers; their screen output depends on read timing (typeahead),
as in M7.  With --save, every failing sequence is written as a scenario
(tests/scenarios/<suite>/ format) for the regression suite.  Uses the
environment and comparison of tests/harness/run.py.  Standard library only.
"""
import argparse
import concurrent.futures
import json
import os
import random
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import run  # noqa: E402

WORDS = ["alpha", "beta", "gamma", "ab", "ba", "x", "aab", "Abc", "delta", "a.b", "a*b", "tab\there", ""]


def text(rnd, lines):
    out = []
    for _ in range(lines):
        out.append(" ".join(rnd.choice(WORDS) for _ in range(rnd.randint(0, 5))))
    return "\n".join(out) + "\n"


def ex_sequence(rnd):
    addr = lambda: rnd.choice(["", "1", "$", ".", "2", "3", "$-1", ".+1", "/a/", "?b?", "/gamma/"])
    addr0 = lambda: rnd.choice(["0", "1", "$", ".", "2"])
    rng = lambda: rnd.choice(["", "%", "1,3", "2,$", ".,.+2", "1", "$", "/a/,/b/", "'a,."])
    pat = lambda: rnd.choice(["a", "b", "^a", "a$", "\\<ab", "[ab]", "a*", "x", "a.b", ".", "^", "$", "gamma"])
    rep = lambda: rnd.choice(["Z", "&&", "<&>", "", "\\U&", "~", "\\1"])
    tline = lambda: " ".join(rnd.choice(WORDS) for _ in range(rnd.randint(0, 3)))
    cmds = [
        lambda: "%sd" % rng(), lambda: "%sy" % rng(), lambda: "%spu" % addr(),
        lambda: "%sco%s" % (rng(), addr0()), lambda: "%sm%s" % (rng(), addr0()),
        lambda: "%ss/%s/%s/%s" % (rng(), pat(), rep(), rnd.choice(["", "g", "g", "p"])),
        lambda: "g/%s/d" % pat(), lambda: "v/%s/d" % pat(),
        lambda: "g/%s/s/%s/%s/g" % (pat(), pat(), rep()),
        lambda: "%sj" % rng(), lambda: "%s>" % rng(), lambda: "%s<" % rng(),
        lambda: "%ska" % addr(), lambda: "%sa\n%s\n." % (addr(), tline()),
        lambda: "%si\n%s\n." % (addr(), tline()), lambda: "%sc\n%s\n." % (rng(), tline()),
        lambda: "%sp" % rng(), lambda: "%snu" % rng(), lambda: "%sl" % rng(),
        lambda: "%s=" % addr(), lambda: "u", lambda: "u",
        lambda: rnd.choice(["set list", "set nolist", "set ic", "set noic", "set nomagic",
                            "set magic", "set sw=4", "set ts=3", "set report=1", "set ai"]),
        lambda: "%s!sort" % rnd.choice(["%", "1,3", "2,$"]), lambda: "%sr other.txt" % addr(),
        lambda: "%sr !echo hi" % addr(), lambda: "w out.txt", lambda: "%s&" % rng(),
        lambda: "%st%s" % (rng(), addr0()), lambda: "%s" % addr(),
    ]
    script = [rnd.choice(cmds)() for _ in range(rnd.randint(2, 10))]
    script += ["w! result.txt", "q!"]
    return {
        "args": ["-e", "f.txt"],
        "files": {"f.txt": text(rnd, rnd.randint(1, 15)), "other.txt": text(rnd, 2)},
        "stdin": "\n".join(script) + "\n",
    }


VI_KEYS = ["h", "j", "k", "l", "w", "b", "e", "W", "B", "0", "$", "^", "G", "1G", "3G", "H", "L", "M",
           "x", "X", "dd", "dw", "D", "J", "~", ".", "u", "U", "p", "P", "yy", "yw", ">>", "<<",
           "ma", "'a", "`a", "d'a", "y'a", "fa", "ta", "Fa", ";", ",", "%", "n", "N", "/a\r", "?b\r",
           "3x", "2dd", "3l", "2j", "rZ", "cwnew\x1b", "iins\x1b", "aapp\x1b", "Aend\x1b",
           "Ostart\x1b", "onext\x1b", "Ibeg\x1b", "s9\x1b", "Cchg\x1b", "\"ayy", "\"ap", "\"bdw", "\"bP",
           "xp", "ddp", "R12\x1b", "5~", "cc line\x1b", "c$tail\x1b", "d$", "d0", "dG", "y$", "\x07"]


def vi_sequence(rnd):
    keys = "".join(rnd.choice(VI_KEYS) for _ in range(rnd.randint(3, 25)))
    return {
        "mode": "pty",
        "args": ["f.txt"],
        "files": {"f.txt": text(rnd, rnd.randint(1, 15))},
        "keys": [[0.6, keys], [0.8, "\x1b\x1b:w! result.txt\r"], [0.4, ":q!\r"]],
        "compare": {"output": False},
        "timeout": 8,
    }


def one(job):
    kind, seed, cbin, rbin = job
    rnd = random.Random(seed)
    sc = ex_sequence(rnd) if kind == "ex" else vi_sequence(rnd)
    want = run.run_one(cbin, sc)
    got = run.run_one(rbin, sc)
    return kind, seed, sc, got, want, run.compare(sc, got, want)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--c", required=True)
    ap.add_argument("--rust", required=True)
    ap.add_argument("--ex", type=int, default=400)
    ap.add_argument("--vi", type=int, default=150)
    ap.add_argument("--seed", type=int, default=20261111)
    ap.add_argument("--jobs", type=int, default=4)
    ap.add_argument("--save")
    ap.add_argument("--only", help="kind:seed, e.g. ex:20261123")
    a = ap.parse_args()
    if not os.path.isdir(run.TERMINFO):
        sys.exit("run tests/tools/build-terminfo.sh first")
    cbin, rbin = os.path.abspath(a.c), os.path.abspath(a.rust)
    if a.only:
        k, s = a.only.split(":")
        jobs = [(k, int(s), cbin, rbin)]
    else:
        jobs = [("ex", a.seed + i, cbin, rbin) for i in range(a.ex)]
        jobs += [("vi", a.seed + 100000 + i, cbin, rbin) for i in range(a.vi)]
    fails = 0
    with concurrent.futures.ProcessPoolExecutor(a.jobs) as pool:
        for kind, seed, sc, got, want, diff in pool.map(one, jobs):
            if not diff:
                continue
            fails += 1
            print("FAIL %s:%d: %s" % (kind, seed, ", ".join(diff)))
            for key in diff:
                run.show_diff(key, got, want)
            if a.save:
                os.makedirs(a.save, exist_ok=True)
                sc = dict(sc, description="fuzz.py --only %s:%d" % (kind, seed))
                with open(os.path.join(a.save, "%s-%d.json" % (kind, seed)), "w") as f:
                    json.dump(sc, f, indent=1)
                    f.write("\n")
    print("%d sequence(s), %d failure(s)" % (len(jobs), fails))
    return 1 if fails else 0


if __name__ == "__main__":
    sys.exit(main())
