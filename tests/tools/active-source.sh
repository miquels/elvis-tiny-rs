#!/bin/sh
# Resolve the active preprocessor branches of elvis-tiny-c with the Linux
# settings from Makefile.mix (used on both Linux and macOS).
# Output: target/c-ref/active/<file>
set -eu
root=$(cd "$(dirname "$0")/../.." && pwd)
src="$root/elvis-tiny-c"
out="$root/target/c-ref/active"
rm -rf "$out"
mkdir -p "$out"

flags="-DM_SYSV -DCRUNCH -DNO_MKEXRC -DNO_CURSORSHAPE -DNO_CHARATTR
 -DNO_SHOWMODE -DNO_MODELINE -DNO_OPTCOLS -DNO_DIGRAPH -DNO_ABBR -DNO_AT
 -DNO_SENTENCE -DNO_ERRLIST -DUSE_MKSTEMP -DUSE_SNPRINTF
 -UNO_EXTENSIONS -UNO_RECYCLE -UNO_MAGIC -UNO_CHARSEARCH -UNO_SHOWMATCH
 -UDEBUG -UTRACE -Ulint -UDATE -UCS_IBMPC -UCS_LATIN1 -UCS_SPECIAL
 -UVERBOSE -USORT -UMKEXRC -UVIREC -USYSEXRC -UWILDCARD_NO_MAIN
 -Uxelos -Ubsd -UM_I86 -U__TURBOC__ -UM68000 -UOSK
 -DUNIXV=1 -DBSD=0 -DUNIX7=0 -DMSDOS=0 -DTOS=0 -DAMIGA=0 -DCOHERENT=0
 -DMINIX=0 -DANY_UNIX=1 -DMICROSOFT=0 -DTURBOC=0
 -DTIOCGWINSZ -DSIGWINCH -DSIGQUIT -DVSUSP -DVLNEXT -DO_APPEND
 -UTIOCSLTC -UVSWTCH -UO_BINARY"

files="main.c vars.c blk.c recycle.c tmp.c misc.c modify.c cut.c regexp.c
 regsub.c opts.c ex.c cmd1.c cmd2.c system.c tio.c input.c move1.c move2.c
 move3.c move4.c move5.c vcmd.c vi.c redraw.c curses.c
 config.h vi.h curses.h regexp.h"

status=0
for f in $files; do
  # unifdef exits 1 when output differs from input; 2 means an error.
  # shellcheck disable=SC2086
  unifdef $flags -o "$out/$f" "$src/$f" || rc=$?
  if [ "${rc:-0}" -ge 2 ]; then echo "unifdef failed on $f" >&2; status=2; fi
  rc=0
done
exit $status
