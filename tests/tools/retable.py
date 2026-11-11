#!/usr/bin/env python3
"""Write the regexp parity table tests/fixtures/regexp/cases.txt (M5).

    retable.py > tests/fixtures/regexp/cases.txt

One case per line, four tab-separated fields:

    flags  pattern  subject  replacement

flags: `m` magic on (else off), `i` ignorecase, `b` regexec with bolflag
(else not at the beginning of a line), `+` continue the previous case's
group.  A replacement of `-` means no regsub.  In every field `%HH` is the
byte 0xHH and `%%` a percent sign.  Each group starts from a fresh state
(a new process in C, a new Editor in Rust); within a group the previous
regexp (empty pattern) and the previous substitution (`~`) carry over.
tests/tools/redriver.c (C) and regexp::tests::c_driver_table (Rust) both
read this file.  Deterministic: a fixed list followed by combinations from
a fixed seed.
"""
import random

FIXED = [
    # literals, ., anchors
    ("mb", "abc", "xxabcxx", "-"), ("mb", "abc", "ab", "-"), ("mb", "a.c", "abc a.c", "-"),
    ("mb", "^abc", "abcabc", "-"), ("m", "^abc", "abcabc", "-"), ("mb", "abc$", "abcabc", "-"),
    ("mb", "^$", "", "-"), ("mb", "^$", "x", "-"), ("mb", "$", "abc", "-"), ("mb", "^", "abc", "-"),
    ("mb", "a^b", "a^b", "-"), ("mb", "a$b", "a$b", "-"), ("mb", "x", "", "-"),
    # closures
    ("mb", "ab*c", "ac abc abbbc", "-"), ("mb", "ab\\+c", "ac abc", "-"), ("mb", "ab\\?c", "abbc abc", "-"),
    ("mb", "a*", "bbb", "-"), ("mb", "a*", "aaab", "-"), ("mb", ".*", "anything", "-"),
    ("mb", "a.*b", "axxbyyb", "-"), ("mb", "x\\+", "aaa", "-"), ("mb", "\\(ab\\)*c", "ababc", "-"),
    ("mb", "a**", "aaa", "-"), ("mb", "*a", "a", "-"), ("mb", "\\+a", "a", "-"), ("mb", "a\\(*\\)", "a", "-"),
    # word boundaries
    ("mb", "\\<the", "other the", "-"), ("mb", "the\\>", "the theme", "-"), ("mb", "\\<a\\>", "ba a ab", "-"),
    ("mb", "\\<", "  x", "-"), ("mb", "\\>", "x  ", "-"), ("mb", "\\<_x", "a _x", "-"),
    # classes
    ("mb", "[abc]", "xxbxx", "-"), ("mb", "[^abc]", "abcd", "-"), ("mb", "[a-c]*", "abcabd", "-"),
    ("mb", "[]a]", "x]a", "-"), ("mb", "[^]a]", "]ab", "-"), ("mb", "[a-]", "x-", "-"),
    ("mb", "[-a]", "x-", "-"), ("mb", "[z-a]", "m", "-"), ("mb", "[abc", "a", "-"), ("mb", "[", "a", "-"),
    ("mb", "[0-9][0-9]*", "abc 123 x", "-"), ("mb", "[A-Za-z_][A-Za-z0-9_]*", " _id9 ", "-"),
    ("mb", "[.*]", "a*b", "-"), ("mb", "[\\]]", "a]b", "-"), ("mb", "[\\\\]", "a\\b", "-"),
    # subexpressions
    ("mb", "\\(a\\)\\(b\\)\\(c\\)", "xabc", "-"), ("mb", "\\(a*\\)\\(b\\)", "xaab", "-"),
    ("mb", "\\(\\(a\\)b\\)", "ab", "-"),
    ("mb", "\\(1\\)\\(2\\)\\(3\\)\\(4\\)\\(5\\)\\(6\\)\\(7\\)\\(8\\)\\(9\\)", "123456789", "\\9\\8\\7\\6\\5\\4\\3\\2\\1"),
    ("mb", "\\(1\\)\\(2\\)\\(3\\)\\(4\\)\\(5\\)\\(6\\)\\(7\\)\\(8\\)\\(9\\)\\(0\\)", "1234567890", "-"),
    ("mb", "\\(a", "a", "-"), ("mb", "a\\)", "a", "-"), ("mb", "\\(\\)", "x", "-"), ("mb", "\\(x\\)*y", "xxy", "-"),
    # nested closures
    ("mb", "\\(a*\\)*", "aaa", "-"), ("mb", "\\(ab*\\)*c", "abbabc", "-"), ("mb", "\\(a\\+\\)\\+b", "aaab", "-"),
    # magic off
    ("b", "a.c", "abc a.c", "-"), ("b", "a\\.c", "abc a.c", "-"), ("b", "a*", "a*", "-"),
    ("b", "a\\*", "aaa", "-"), ("b", "[ab]", "x[ab]", "-"), ("b", "\\[ab]", "xb", "-"),
    ("b", "^a$", "a", "-"), ("b", "\\(a\\)", "a", "&\\1"), ("b", "a", "a", "x"), ("+b", "a", "a", "&~"),
    # ignorecase
    ("mib", "abc", "xABCx", "-"), ("mib", "[a-c]*", "AbC", "-"), ("mib", "\\<THE\\>", "in the end", "-"),
    ("mib", "x", "X", "<&>"), ("mb", "ABC", "abc", "-"), ("mib", "[^a]", "Ab", "-"),
    # empty pattern: the previous regexp
    ("mb", "", "x", "-"), ("mb", "abc", "abc", "-"), ("+mb", "", "zzabcz", "-"), ("+mb", "q", "q", "-"),
    ("+mb", "", "aqa", "-"), ("+mib", "", "AQA", "-"), ("mb", "\\(", "x", "-"), ("+mb", "", "x", "-"),
    # substitutions
    ("mb", "b", "abc", "[&]"), ("mb", "b", "abc", "\\&"), ("mb", "b", "abc", "~~"), ("mb", "\\(b\\)", "abc", "\\1\\1"),
    ("mb", "\\(b\\)", "abc", "\\2"), ("mb", "b", "abc", "x\\ny"), ("mb", "b", "abc", "\\u&"), ("mb", "b", "abc", "\\U&x"),
    ("mb", "b", "abc", "\\L&"), ("mb", "b", "abc", "\\l&"), ("mb", "\\(b\\)\\(c\\)", "abc", "\\u\\1\\U\\2\\Ex"),
    ("mb", "b", "abc", "%5C"), ("mb", "b", "abc", "a\\"), ("mb", "b", "abc", "\\\\"), ("mb", "b", "abc", "~"),
    ("mb", "b", "abc", "<&>"), ("+mb", "c", "abc", "~-~"), ("+mb", "c", "abc", ""), ("+mb", "c", "abc", "~"),
    ("+b", "c", "abc", "~&"), ("+mb", "c", "abc", "\\~"), ("+mb", "c", "abc", "~\\1~"),
    # bytes 128 to 255
    ("mb", "%E9", "caf%E9", "-"), ("mb", "[%E0-%EF]", "x%E5y", "-"), ("mb", "[^%80-%FF]*", "ab%FFc", "-"),
    ("mb", ".%FF", "a%FF", "-"), ("mib", "%C9", "%E9", "-"), ("mb", "\\<%E9", "a %E9", "-"),
    ("mb", "[%80%FF]", "%7F%FF", "-"), ("mb", "%80*", "%80%80x", "-"), ("mb", "x", "%80x%FF", "<&>"),
    ("mb", "[a%E9]\\+", "%E9a%E9b", "-"),
    # errors and odd syntax
    ("mb", "\\", "a", "-"), ("mb", "a\\", "a", "-"), ("mb", "\\(\\(\\(\\(\\(\\(\\(\\(\\(\\(\\(a\\)\\)\\)\\)\\)\\)\\)\\)\\)\\)\\)", "a", "-"),
    ("mb", "[a][b][c][d][e][f][g][h][i][j][k]", "abcdefghijk", "-"), ("mb", "\\q", "q", "-"),
    ("mb", "\\{", "{", "-"), ("mb", "a\\|b", "a|b", "-"),
    # long subject
    ("mb", "z$", "a" * 1000 + "z", "-"), ("mb", "\\(a*\\)z", "a" * 1000 + "z", "-"),
    ("mb", "[b-y]", "a" * 1022 + "m", "<&>"),
]

# atoms that take a closure, and atoms that do not (elvis rejects `\)*`)
CLOSABLE = ["a", "b", ".", "[ab]", "[^a]", "x", "[a-c]", "%E9", "[%E0-%FF]"]
PLAIN = ["\\(a\\)", "\\(b*\\)", "\\(a\\|b\\)", "\\<", "\\>"]
CLOS = ["", "", "", "*", "\\+", "\\?"]
SUBJ = "ab xa bab %E9a aab b "
REPL = ["-", "-", "&", "<&>", "\\1", "~", "[\\1&]", "\\u&"]


def main():
    rows = list(FIXED)
    rnd = random.Random(1988)
    while len(rows) < 320:
        parts = []
        for _ in range(rnd.randint(1, 4)):
            if rnd.random() < 0.3:
                # a closure here is an error 1 time in 10
                parts.append(rnd.choice(PLAIN) + ("*" if rnd.random() < 0.1 else ""))
            else:
                parts.append(rnd.choice(CLOSABLE) + rnd.choice(CLOS))
        pat = "".join(parts)
        if rnd.random() < 0.15:
            pat = "^" + pat
        if rnd.random() < 0.15:
            pat += "$"
        toks = SUBJ.split(" ")
        subj = "".join(rnd.choice(toks) + rnd.choice(["", " "]) for _ in range(rnd.randint(1, 6)))
        flags = "m" + ("i" if rnd.random() < 0.2 else "") + ("b" if rnd.random() < 0.8 else "")
        rows.append((flags, pat, subj, rnd.choice(REPL)))
    for r in rows:
        assert all("\t" not in f and "\n" not in f for f in r)
        print("\t".join(r))


if __name__ == "__main__":
    main()
