#!/bin/sh
# Compile the terminfo fixture sources with the host tic into
# target/terminfo-fixtures (compiled formats differ between ncurses versions).
set -eu
root=$(cd "$(dirname "$0")/../.." && pwd)
src="$root/tests/fixtures/terminfo"
out="$root/target/terminfo-fixtures"
rm -rf "$out"
mkdir -p "$out"
for t in xterm vt100 linux screen; do
  tic -x -o "$out" "$src/$t.ti"
done
# The synthetic entries use= the compiled vt100 entry.
TERMINFO="$out" tic -x -o "$out" "$src/synthetic.ti"
