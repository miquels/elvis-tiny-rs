//! `misc.c`: line fetching and the regexp error hook.

use crate::blk::{BufId, locate};
use crate::tio::{Arg, msg};
use crate::vars::Editor;
use crate::vi_h::BLKSIZE;

/// `fetchline()`'s function-local statics.
#[derive(Default)]
pub struct Misc {
    pub fetchline_nextline: i64,
    pub fetchline_chglevel: i64,
    /// Offset into the cached block (C kept a pointer).
    pub fetchline_nextscan: usize,
    /// The cache buffer `fetchline_nextscan` points into.
    pub fetchline_nextbuf: BufId,
    pub fetchline_nextlnum: i64,
}

// C: misc.c:fetchline
/// Returns a copy of line `line` without its NUL terminator.
pub fn fetchline(ed: &mut Editor, line: i64) -> Vec<u8> {
    let id;
    let mut scan;

    // `blkget` bumps `changes` whenever it reuses a buffer, so an unchanged
    // `changes` means the saved buffer index and offset are still good.
    if ed.vars.changes == ed.misc.fetchline_chglevel && line == ed.misc.fetchline_nextline {
        id = ed.misc.fetchline_nextbuf;
        scan = ed.misc.fetchline_nextscan;
    } else {
        let i;
        (i, id, scan) = locate(ed, line);
        ed.misc.fetchline_nextlnum = ed.vars.lnum[i];
    }

    let c = &ed.blk.buf(id).c;
    let start = scan.min(BLKSIZE);
    scan = start;
    while scan < BLKSIZE && c[scan] != b'\n' {
        scan += 1;
    }
    let text = c[start..scan].to_vec();

    if line < ed.misc.fetchline_nextlnum {
        ed.misc.fetchline_nextline = line + 1;
        ed.misc.fetchline_chglevel = ed.vars.changes;
        ed.misc.fetchline_nextscan = scan + 1;
        ed.misc.fetchline_nextbuf = id;
    } else {
        ed.misc.fetchline_nextline = 0;
    }

    // fetchline() and pfetch() share state; force pfetch() to refetch.
    ed.vars.pchgs = 0;

    text
}

// C: misc.c:pfetch
/// Copies line `l` into `ed.vars.ptext` (no NUL) and sets `plen`, `pline`, `pchgs`.
pub fn pfetch(ed: &mut Editor, l: i64) {
    if l != ed.vars.pline || ed.vars.changes != ed.vars.pchgs {
        ed.vars.pline = l;
        ed.vars.ptext = fetchline(ed, ed.vars.pline);
        ed.vars.plen = ed.vars.ptext.len() as i32;
        ed.vars.pchgs = ed.vars.changes;
    }
}

// C: misc.c:regerror
pub fn regerror(ed: &mut Editor, txt: &[u8]) {
    msg(ed, b"RE error: %s", &[Arg::S(txt)]);
}
