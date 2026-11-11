# Observed behavior of the C reference

This file records how the C reference binary behaves, per host OS. The Rust
port follows the reference of the host it runs on (IMPLEMENTATION.md 1.6).
Golden scenario results are in `tests/golden/<os>/`.

## macOS

Recorded 2026-11-10.

- Host: macOS 27.2 (build 26B123), arm64.
- Compiler: Apple clang 21.0.0 (clang-2100.4.1.1).
- C library: libSystem.B.dylib 1359.40.3.
- Terminal library: `/usr/lib/libncurses.5.4.dylib` (compatibility 5.4.0).
- Build: `make -f Makefile.mix elvis LIBS=-lncurses EXTRA_CFLAGS="-include sys/ioctl.h -Ishim"`
  in a copy at `target/c-ref/src/` (48 compiler warnings, no errors).
  The `-O0 -g` inventory build is in `target/c-ref/src-O0/`.

### Terminal library (capability probe)

Reports: `tests/golden/macos/capprobe/<term>.<speed>.txt`, produced by
`tests/tools/capprobe.c` run on a pty at 1200 and 38400 baud.

- **No interposition.** Changing the program's own `ospeed` and `PC`
  globals does not change any `tputs` output. ncurses uses its own copies.
- **Padding.** `tputs` removes every well-formed `$<…>` directive (integer,
  fractional, `*`, mandatory `/`) and outputs no pad characters, at both line
  speeds, with or without `xon`. A malformed directive with no closing `>`
  (`\EM$<5` in `pad-cup`) is output literally. So on macOS the adapter in
  section 4.6 must strip well-formed directives and emit nothing for them.
- **`tgoto` order.** `tgoto(cm, col, row)` expands with `%p1` = row and
  `%p2` = col (see `asym-cup`: row 9, col 7 gives `\E[12;57H`).
- **Padding inside `cm`.** `pad-cup` has `cup=…H$<5>`; the bytes after
  `tputs(tgoto(...))` contain no padding and no `$<5>`.
- **`pt`.** `tgetflag("pt")` is 1 only for `screen`, whose entry contains the
  obsolete boolean `OTpt`. It is not derived from `ht`/`it` (xterm has both and
  returns 0). Rust rule: `pt` = the `OTpt` boolean.
- **`ic`/`kH`.** Missing in all fixtures except `ic` in `linux`; `MAYHAVE`
  defaults apply.
- **`ug`.** -1 (absent) for every fixture, so the `us`/`md`/`as` pairs are
  always loaded.
- **Lookup.** `TERMINFO=/nonexistent` still finds system entries (ncurses
  falls back to `/usr/share/terminfo`). An unknown name returns 0 from
  `tgetent` (exit 3, "Unrecognized TERM type"). The -1 case ("Can't read
  /etc/termcap", exit 2) could not be produced on macOS because the system
  database is always present.

### Exit statuses and messages

| Case | Status | Output | Scenario |
|------|--------|--------|----------|
| `-r` | 0 | `Use the \`virec\` program to recover lost files` | startup/r-flag |
| `TERM` unset | 1 | `Environment variable TERM must be set` | startup/term-unset |
| Unknown terminal | 3 | `Unrecognized TERM type` | startup/term-unknown |
| Missing `cm` / `ce` | 1 | `This termcap entry lacks the :cm=: capability` | caps/missing-cup, caps/missing-el |
| End of ex script input | 1 | (clears last line, restores terminal) | startup/eof-of-script |
| File name over 128 bytes | 1 | `file name too long` | startup/name-too-long |
| Temp directory unwritable | 9 | `FAIL` path in `tmp.c` | files/tmpdir-unwritable |
| Screen too small at startup | **SIGSEGV (-11)** | `Screen too small` then crash | startup/small-screen |
| Normal quit | 0 | | most scenarios |

The startup crash is undefined behavior: `getsize()` runs inside
`starttcap()` before `initscr()` sets `stdscr`, so `endwin()` →
`suspend_curses()` → `tputs(KE, …, faddch)` writes through a null `stdscr`
(Appendix B, B4). Resizing to a too-small window later (`resize/too-small`)
does not exit: `getsize(SIGWINCH)` keeps the old size because the new one
fails the `lines >= 2 && cols >= 30` test.

"Too many files on command line" (exit 1) was not exercised; it needs over
1024 bytes of arguments.

### Signals

- `SIGINT` in vi command mode (`signals/sigint-vi-cmd`): shows
  `[Press <RETURN> to continue]`; the next key returns to vi mode.
- `SIGINT` during `:g` in ex mode (`signals/sigint-global`): exit status 0 after
  `:q!`. Whether the signal lands mid-command depends on timing, so only the
  status is compared.
- `SIGQUIT` is ignored (`signals/sigquit-ignored`).
- `^Z` is an ordinary key: `VSUSP` is cleared and there is no `SIGTSTP`
  handling. Typing `^Z` in vi mode does not suspend.

### Key timeout

With `keytime=1`, `:map QA x` and `:map QB dd`, typing `Q`, waiting 3 seconds,
then `A` runs the `QA` mapping. The `alarm()` never interrupts the read,
because macOS `signal()` installs `SIGALRM` with `SA_RESTART`. The timeout
never takes effect on macOS. The port implements the intended timeout
(Appendix B, B5).

### Window size

`SIGWINCH` with a valid new size redraws (`resize/grow`). `SIGWINCH` without a
size change is harmless (`resize/winch-noop`).

### Temporary files

- Name: `{directory}/elv{sum:x}{ino:04x}{dev:03x}`; `ino` and `dev` are
  printed in full (the widths are minimums). Example: `elv426c13517100000e`
  for inode 0x26c13517 on device 0x100000e.
- Mode 0600, one link (the `mkstemp` name is unlinked after `link()`).
- Header block: native-endian (little-endian on arm64) `u16`s; `n[0]` is the
  low 16 bits of the inode (0x3517 above), `n[1]` = 1 for a one-block file.
- Block 1 holds the text, NUL-padded.
- A one-line yank to a named buffer creates no `elv_` cut file.
- No temporary files remain after a normal exit; `:q!` after an interrupt
  also leaves none. The `tmpdir-unwritable` failure (exit 9) leaves none.

### Standard input not a terminal

Piped ex mode (`-e` with a script on stdin and output to a pipe) works:
`tcgetattr(2)`/`tcsetattr(2)` fail silently, the size comes from the terminfo
`lines`/`cols`, and `ks`/`ke` are still written to stdout. End of input exits
with status 1.

### Option overflow

`:set directory=<40 bytes>` overflows the 30-byte `o_directory` array into
`o_keywordprg` (`options/directory-overflow`; Appendix B, B3). The harness
uses short `/tmp/eh.*` work directories so ordinary scenarios stay under the
limit.

## Linux

Recorded 2026-11-11.

- Host: Debian 13.7 (trixie), x86_64, kernel 6.12.116.
- Compiler: gcc 14.2.0 (Debian 14.2.0-19).
- C library: glibc 2.41 (2.41-12+deb13u4).
- Terminal library: `libtinfo.so.6.5` (ncurses 6.5+20250216).
- Build: `make -f Makefile.mix elvis` with the unchanged Linux settings in a
  copy at `target/c-ref/src/` (no warnings, no errors). The `-O0 -g`
  inventory build is in `target/c-ref/src-O0/`.

Everything below that is not listed here behaves as on macOS (same
messages, exit statuses, signal handling, option overflow and piped mode).
Apart from the padding NULs, the Linux golden outputs equal the macOS ones
byte for byte, except `signals/sigint-global`, whose output depends on
when the signal lands (only the status is compared).

### Terminal library (capability probe)

Reports: `tests/golden/linux/capprobe/<term>.<speed>.txt` (`capprobe` with
`ospeed` 0, as on macOS) and `<term>.<speed>.pad.txt` (`ospeed` set to the
pty's `B*` code: 9 for 1200, 15 for 38400). Fixtures compiled by the host
`tic` use the `x/xterm` directory layout (macOS: `78/xterm`).

- **Interposition.** The program's own `ospeed` and `PC` globals replace
  libtinfo's (ELF copy relocation; libtinfo exports both as data symbols).
  `tgetent` writes them: `ospeed` becomes the tty's `B*` code and `PC` the
  first byte of `pad` (0 for every fixture). elvis then sets `ospeed` from
  `cfgetospeed()` in `resume_curses`. So the C reference pads at the tty's
  line speed: at 38400 baud on `vt100` every cursor move (`cup=…$<5>`) is
  followed by 21 NULs. In piped mode `tcgetattr(2)` fails, `ospeed` is 0
  and nothing is padded.
- **Padding.** Without a `SCREEN` (the termcap interface never creates
  one), ncurses 6.5 pads every well-formed `$<…>` delay whatever `xon` and
  `pb` say, and `NCURSES_NO_PADDING` has no effect. Measured with
  `tests/tools/padprobe.c`, which counts pad characters for each `ospeed`: the
  count is `(tenths / 10) * baud / 9000` in C `int` arithmetic, where
  `tenths` is the delay in tenths of a millisecond (times `affcnt` for `*`).
  `$<.5>` gives 0. The product wraps at `B4000000` (`$<1000>` gives 0).
  Unknown codes (16, 4096, 4112, negative) give no padding; the `B*` table
  runs from `B0` to `B4000000`. `npc` (xterm) emits no pad characters and
  sleeps instead. There is no BSD-style leading-digit padding (`tputs("50")`
  outputs "50"). Malformed directives are output as on macOS.
- **`me` (sgr0 trimming).** ncurses 6.5's `_nc_trim_sgr0` strips the
  delays from the `sgr` expansions before comparing (`strip_padding` in
  `set_attribute_9`), so `vt100` gives `me=\E[0m`, not macOS's
  `\E[0m$<2>`. Checked with crafted entries (results in
  `terminfo::tests::trim_sgr0_matches_libtinfo`): trailing delays of every
  form vanish, a delay in the middle keeps `sgr0` unchanged, an `rmacs`
  with its own delay is not found, and the final `strstr` branch removes
  the first `strlen(off)` bytes (`chop_out(tmp, i, j)` takes a length).
- **`tgoto` order, `pt`, `ic`/`kH`, `ug`.** As on macOS.
- **Lookup.** The search order is `TERMINFO`, `$HOME/.terminfo`,
  `TERMINFO_DIRS`, then `/etc/terminfo`, `/lib/terminfo`,
  `/usr/share/terminfo`. Even with every database directory hidden (bind
  mounts in a user namespace), `tgetent` returns 0 (exit 3, "Unrecognized
  TERM type"), never -1, so "Can't read /etc/termcap" (exit 2) cannot be
  produced on Linux either.

### Exit statuses and messages

As on macOS, including the startup crash: "Screen too small" is followed
by SIGSEGV (status -11 in the harness, 139 from a shell). `stty -a` after
every exit path equals `stty -a` before it (quit, `:wq`, `ZZ`, `:!`,
ex-mode quit, `-r`, unknown terminal, missing `cm`, unwritable temporary
directory, and the crash; `tests/tools/sttycheck.py`).

`:w! /dev/full` reports `Wrote "/dev/full"  3 lines, 14 characters`: C
ignores the `ENOSPC` from `write()` (`files/write-dev-full`).

### Key timeout

As on macOS: glibc's `signal()` also installs `SIGALRM` with `SA_RESTART`,
so with `keytime=1`, `:map QA x` and `:map QB dd` (`tests/tools/keytime.py`), typing `Q`, waiting 3
seconds and typing `A` still runs the `QA` mapping (Appendix B, B5).

### Temporary files

Name, mode, header and text blocks as on macOS, with the Linux `st_dev`
(for example `elv91330029`: sum 9, inode low digits, device 0x029 on this
host's file system). C and Rust produce identical names and byte-identical
images (`tests/tools/tmpimage.py`) for: a small file, NUL bytes, no trailing newline, lines of 1022
bytes, a 1500-byte line, a 3000-line file over many blocks, exactly 500000
bytes, bytes 1 to 255, and an empty file.

### Block table limits

The header holds `MAXBLKS - 1` = 511 text blocks, and C never checks it.
Loading a file that needs more (for example 974 lines of 513 bytes, under
500000 bytes) writes past `hdr.n[]` and `lnum[]` and crashes (SIGSEGV).
`:%s/a/b/g` on a 400000-byte file of 80-byte lines splits blocks until
`blkadd` drops the last block and `add` reads past `lnum[]` (SIGSEGV).
The macOS build runs the same code with the same limits, so it is
expected to fail the same way there; that has not been tried (Appendix B,
B21).

### Regular expressions (M5)

`tests/tools/redriver.c` links the reference build's `regexp.o` and
`regsub.o` and runs `tests/fixtures/regexp/cases.txt` (320 cases from
`tests/tools/retable.py`: literals, `.`, closures, anchors, word
boundaries, classes, ten subexpressions, nested closures, the empty
pattern, `magic` off, `ignorecase`, bytes 128 to 255, 1001-byte subjects,
every compile error, and `regsub` with `&`, `\1`…`\9`, `~`, `\U`/`\L`/`\u`/`\l`/`\E`).
Report: `tests/golden/linux/regexp/report.txt`, identical in three runs.

- Groups of cases run in separate processes, because some inputs make C
  write outside its buffers: case 107 (`[%E0-%EF]`, B14) crashes; error
  paths such as "Too many []s" corrupt the heap for later cases in the
  same process.
- A B14 class writes `bmap[-4..-1]`, which with this struct layout lands
  in the top byte of `minlen`, `first`, `bol` and `program[0]` (the class
  count). The port reproduces those bytes; `bol` is compared as a truth
  value because C only tests it that way.
- Cases 157, 212 and 248 (a B14 class, then `\(…\)` and one more token)
  report "Mismatched \)" in the driver only, depending on the pattern's
  length. The C editor compiles the same patterns and reports "Not found",
  as the port does.

### Random sequences (M9)

`tests/harness/fuzz.py` with the default seeds (400 ex-mode and 150 vi-key
sequences): the vi sequences all match; the 14 ex sequences that differ
fall into three families, and a minimal reproduction of each family is a
memory error in an AddressSanitizer build of the reference
(`-O0 -fsanitize=address`, outside `elvis-tiny-c/`). In each of the 14 C
also crashes, leaves its temporary file, or differs between its `-O0` and
`-O2` builds. They are kept as `tests/scenarios/regress/` scenarios with the
reviewed Rust result as reference.

- `:s` with `g` and a pattern that can match the empty string (`a*`, `$`):
  global-buffer-overflow in `regsub` (B11).
- `co`, `t` or `m` with a destination past the last line, e.g. `1t2` in a
  one-line file: stack-buffer-overflow in `add` (B22). `1m2` works.
- A failed `pu` sets the cursor to `MARK_UNSET`. The autoprint of line 0
  leaves `fetchline`'s next-line shortcut one line off (`1p` then prints
  line 2, and `q!` does not quit). A later command that walks to the last
  line reads past it: global-buffer-overflow in `fetchline` (B23). C then
  crashes, stops reading its input, or leaves its temporary file; the
  `-O0` and `-O2` builds differ.

The same sequences found two port bugs, both fixed: lines past the end were
returned with their NUL bytes (now empty, as C's callers see them; B13), and
the first B22 check also refused `1m2`.
