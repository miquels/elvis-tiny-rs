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

Not yet characterized. Every item above must be repeated on Linux.
