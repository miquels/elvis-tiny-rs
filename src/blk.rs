//! `blk.c`: the block cache for the temporary file, plus do/undo.

use crate::opts::Opt;
use crate::vi_h::Mode;
use std::fs::File;
use std::io::{Seek, SeekFrom};

use crate::recycle::{allocate, garbage};
use crate::redraw::redraw;
use crate::tio::msg;
use crate::vars::Editor;
use crate::vi_h::{
    BLKSIZE, Blk, INFINITY, MARK_UNSET, MAXBLKS, MODIFIED, Mark, mark_last, markline,
};

pub const NBUFS: usize = 5;

/// `struct _blkbuf`
#[derive(Default, Clone)]
pub struct BlkBuf {
    pub buf: Blk,
    pub logical: u16,
    pub dirty: bool,
}

pub struct BlkState {
    pub hdr: Blk,
    pub b4cnt: i32,
    pub blk: [BlkBuf; NBUFS],
    /// Indexes into `blk` (C used pointers).
    pub toonew: Option<usize>,
    pub newtoo: Option<usize>,
    pub recycle: usize,
    pub undocurs: Mark,
    pub oldnlines: i64,
    pub oldlnum: [i64; MAXBLKS],
    pub fetch: FetchCache,
}

/// `fetchline()`'s function-local statics: where the line after the last
/// one fetched starts, while `changes` is unchanged.
#[derive(Default)]
pub struct FetchCache {
    pub nextline: i64,
    pub chglevel: i64,
    /// Offset into the cached block (C kept a pointer).
    pub nextscan: usize,
    /// The cache buffer `nextscan` points into.
    pub nextbuf: BufId,
    pub nextlnum: i64,
}

impl Default for BlkState {
    fn default() -> BlkState {
        BlkState {
            hdr: Blk::default(),
            b4cnt: 0,
            blk: Default::default(),
            toonew: None,
            newtoo: None,
            recycle: 0,
            undocurs: 0,
            oldnlines: 0,
            oldlnum: [0; MAXBLKS],
            fetch: FetchCache::default(),
        }
    }
}

impl BlkState {
    /// The block a `BufId` refers to (`HDR` is the header block).
    pub fn buf(&self, id: BufId) -> &Blk {
        if id == HDR {
            &self.hdr
        } else {
            &self.blk[id].buf
        }
    }
}

/// Index of a cache buffer (`ed.blk.blk[id].buf`).  Valid only until the
/// next call that can recycle a cache entry.
pub type BufId = usize;

/// What `blkget(0)` returns: C returned `&hdr`, which is not a cache buffer.
pub const HDR: BufId = NBUFS;

/// `lseek(fd, off, 0); read(fd, buf, len) == len`.
pub(crate) fn lseek_read(fd: Option<&File>, off: u64, buf: &mut [u8]) -> bool {
    let Some(mut f) = fd else { return false };
    let _ = f.seek(SeekFrom::Start(off));
    crate::config::tread(&mut f, buf) == buf.len() as isize
}

/// `lseek(fd, off, 0); write(fd, buf, len) == len`; with no `off` (C's
/// failed `lseek`) it writes at the current position.
pub(crate) fn lseek_write(fd: Option<&File>, off: Option<u64>, buf: &[u8]) -> bool {
    let Some(mut f) = fd else { return false };
    if let Some(off) = off {
        let _ = f.seek(SeekFrom::Start(off));
    }
    crate::config::twrite(&mut f, buf) == buf.len() as isize
}

/// `while (*scan++ != '\n') {}`, stopping at the end of the block.
pub(crate) fn skipline(c: &[u8], mut i: usize) -> usize {
    while i < c.len() {
        let b = c[i];
        i += 1;
        if b == b'\n' {
            break;
        }
    }
    i
}

/// Skips `n` lines of block text `c`, starting at offset `i`.
pub(crate) fn skiplines(c: &[u8], mut i: usize, n: i64) -> usize {
    for _ in 0..n {
        i = skipline(c, i);
    }
    i
}

/// The first block, from `from` on, that holds line `line` or a later one.
pub(crate) fn findblk(lnum: &[i64], line: i64, from: usize) -> usize {
    let mut i = from;
    while lnum[i] < line {
        i += 1;
    }
    i
}

/// Finds line `line` in the cache: its block number, the buffer holding
/// that block, and the offset where the line starts.
#[inline(never)]
pub(crate) fn locate(ed: &mut Editor, line: i64) -> (usize, BufId, usize) {
    let i = findblk(&ed.vars.lnum, line, 1);
    let id = blkget(ed, i as i32);
    let n = line - ed.vars.lnum[i - 1] - 1;
    (i, id, skiplines(&ed.blk.blk[id].buf.c, 0, n))
}

// C: blk.c:blkinit
pub fn blkinit(ed: &mut Editor) {
    for i in 0..NBUFS {
        ed.blk.blk[i].logical = 0;
        ed.blk.blk[i].dirty = false;
    }
    for i in 0..MAXBLKS {
        ed.blk.hdr.set_n(i, 0);
    }
}

// C: blk.c:blkget
pub fn blkget(ed: &mut Editor, logical: i32) -> BufId {
    if logical == 0 {
        return HDR;
    }

    for this in 0..NBUFS {
        if ed.blk.blk[this].logical as i32 == logical {
            ed.blk.newtoo = ed.blk.toonew;
            ed.blk.toonew = Some(this);
            return this;
        }
    }

    let mut this;
    loop {
        this = ed.blk.recycle;
        ed.blk.recycle += 1;
        if ed.blk.recycle == NBUFS {
            ed.blk.recycle = 0;
        }
        if Some(this) != ed.blk.toonew && Some(this) != ed.blk.newtoo {
            break;
        }
    }

    blkflush(ed, this);

    ed.blk.blk[this].logical = logical as u16;
    let phys = ed.blk.hdr.n(logical as usize);
    if phys != 0 {
        if !lseek_read(
            ed.vars.tmpfd.as_ref(),
            phys as u64 * BLKSIZE as u64,
            &mut ed.blk.blk[this].buf.c,
        ) {
            msg(ed, b"Error reading back from tmp file!", &[]);
        }
    } else {
        ed.blk.blk[this].buf.c = [0; BLKSIZE];
    }

    // Not really a change, but it may invalidate the shortcuts that
    // `changes` protects.
    ed.vars.changes += 1;

    ed.blk.blk[this].dirty = false;

    ed.blk.newtoo = ed.blk.toonew;
    ed.blk.toonew = Some(this);
    this
}

// C: blk.c:blkflush
fn blkflush(ed: &mut Editor, this: usize) {
    if ed.blk.blk[this].logical != 0 && ed.blk.blk[this].buf.c[0] == 0 {
        blkdirty(ed, this);
    }

    let logical = ed.blk.blk[this].logical as usize;
    if logical == 0 || ed.blk.hdr.n(logical) != 0 && !ed.blk.blk[this].dirty {
        return;
    }

    let seekpos = allocate(ed);
    let physical = (seekpos / BLKSIZE as i64) as u16;
    let seekpos = (seekpos >= 0).then_some(seekpos as u64);

    if !lseek_write(ed.vars.tmpfd.as_ref(), seekpos, &ed.blk.blk[this].buf.c) {
        msg(ed, b"Trouble writing to tmp file", &[]);
    }
    ed.blk.blk[this].dirty = false;

    let logical = ed.blk.blk[this].logical as usize;
    ed.blk.hdr.set_n(logical, physical);
}

// C: blk.c:blkdirty
pub fn blkdirty(ed: &mut Editor, id: BufId) {
    let i = id;

    // A block ending with line INFINITY was allocated needlessly by
    // tmpstart(); forget it.
    if ed.vars.lnum[ed.blk.blk[i].logical as usize] == INFINITY {
        ed.blk.blk[i].logical = 0;
        ed.blk.blk[i].dirty = false;
        return;
    }

    let c = &ed.blk.blk[i].buf.c;
    let mut j: i64 = 0;
    let mut scan = 0;
    while scan < BLKSIZE && c[scan] != 0 {
        if c[scan] == b'\n' {
            j += 1;
        }
        scan += 1;
    }

    let mut k = ed.blk.blk[i].logical as usize;
    let lnum = &mut ed.vars.lnum;
    j += lnum[k - 1] - lnum[k];
    if j != 0 {
        ed.vars.nlines += j;
        while k < MAXBLKS && lnum[k] != INFINITY {
            lnum[k] += j;
            k += 1;
        }
    }

    if ed.blk.blk[i].buf.c[0] != 0 {
        ed.blk.blk[i].dirty = true;
    } else {
        let k0 = ed.blk.blk[i].logical;
        for j in 0..NBUFS {
            if ed.blk.blk[j].logical >= k0 {
                ed.blk.blk[j].logical -= 1;
            }
        }

        ed.blk.blk[i].logical = 0;
        ed.blk.blk[i].dirty = false;
        let mut k = k0 as usize;
        while k < MAXBLKS - 1 {
            let n = ed.blk.hdr.n(k + 1);
            ed.blk.hdr.set_n(k, n);
            ed.vars.lnum[k] = ed.vars.lnum[k + 1];
            k += 1;
        }
        ed.blk.hdr.set_n(MAXBLKS - 1, 0);
        ed.vars.lnum[MAXBLKS - 1] = INFINITY;
    }
}

// C: blk.c:blkadd
pub fn blkadd(ed: &mut Editor, logical: i32) -> BufId {
    let logical_u = logical as usize;
    let mut i = MAXBLKS - 1;
    while i > logical_u {
        let n = ed.blk.hdr.n(i - 1);
        ed.blk.hdr.set_n(i, n);
        ed.vars.lnum[i] = ed.vars.lnum[i - 1];
        i -= 1;
    }
    ed.blk.hdr.set_n(logical_u, 0);
    ed.vars.lnum[logical_u] = ed.vars.lnum[logical_u - 1];

    for i in 0..NBUFS {
        if ed.blk.blk[i].logical as i32 >= logical {
            ed.blk.blk[i].logical += 1;
        }
    }

    blkget(ed, logical)
}

// C: blk.c:blksync
pub fn blksync(ed: &mut Editor) {
    for i in 0..NBUFS {
        blkflush(ed, i);
    }
    if ed.opts.on(Opt::Sync) {
        crate::os::sync();
    }
}

// C: blk.c:beforedo
pub fn beforedo(ed: &mut Editor, forundo: bool) {
    let was = ed.blk.b4cnt;
    ed.blk.b4cnt += 1;
    if was > 0 {
        return;
    }

    blksync(ed);

    garbage(ed);

    if !lseek_write(ed.vars.tmpfd.as_ref(), Some(0), &ed.blk.hdr.c) {
        msg(ed, b"Trouble writing header to tmp file ", &[]);
    }

    if forundo {
        std::mem::swap(&mut ed.vars.lnum, &mut ed.blk.oldlnum);
        std::mem::swap(&mut ed.vars.nlines, &mut ed.blk.oldnlines);
    } else {
        ed.blk.oldlnum = ed.vars.lnum;
        ed.blk.oldnlines = ed.vars.nlines;
    }

    ed.blk.undocurs = ed.vars.cursor;
}

// C: blk.c:afterdo
pub fn afterdo(ed: &mut Editor) {
    ed.blk.b4cnt -= 1;
    if ed.blk.b4cnt != 0 {
        // After abortdo() the count may be off; keep it sane.
        if ed.blk.b4cnt < 0 {
            ed.blk.b4cnt = 0;
        }
        return;
    }

    if markline(ed.vars.cursor) > ed.vars.nlines {
        ed.vars.cursor = mark_last(ed.vars.nlines);
    }

    if ed.vars.significant {
        ed.vars.viflags.setflag(MODIFIED);
    }
}

// C: blk.c:abortdo
pub fn abortdo(ed: &mut Editor) {
    if ed.blk.b4cnt > 0 {
        ed.blk.b4cnt = 1;
        afterdo(ed);
    }

    if ed.vars.mode == Mode::Colon {
        ed.vars.mode = Mode::Vi;
    }
    if ed.vars.mode == Mode::Vi {
        redraw(ed, MARK_UNSET, false);
    }
}

// C: blk.c:undo
pub fn undo(ed: &mut Editor) -> bool {
    let mut oldhdr = Blk::default();

    if !ed.vars.viflags.tstflag(MODIFIED) {
        msg(ed, b"You haven't modified this file yet.", &[]);
        return false;
    }

    if !lseek_read(ed.vars.tmpfd.as_ref(), 0, &mut oldhdr.c) {
        msg(ed, b"Trouble rereading the old header from tmp file", &[]);
    }

    ed.vars.cursor = ed.blk.undocurs;
    beforedo(ed, true);
    afterdo(ed);

    blkinit(ed);

    ed.blk.hdr = oldhdr;

    ed.vars.changes += 1;

    true
}

// C: misc.c:fetchline
/// Returns a copy of line `line` without its NUL terminator.
pub fn fetchline(ed: &mut Editor, line: i64) -> Vec<u8> {
    let id;
    let mut scan;

    // `blkget` bumps `changes` whenever it reuses a buffer, so an unchanged
    // `changes` means the saved buffer index and offset are still good.
    if ed.vars.changes == ed.blk.fetch.chglevel && line == ed.blk.fetch.nextline {
        id = ed.blk.fetch.nextbuf;
        scan = ed.blk.fetch.nextscan;
    } else {
        let i;
        (i, id, scan) = locate(ed, line);
        ed.blk.fetch.nextlnum = ed.vars.lnum[i];
    }

    let c = &ed.blk.buf(id).c;
    let start = scan.min(BLKSIZE);
    scan = start;
    while scan < BLKSIZE && c[scan] != b'\n' {
        scan += 1;
    }
    let text = c[start..scan].to_vec();

    if line < ed.blk.fetch.nextlnum {
        ed.blk.fetch.nextline = line + 1;
        ed.blk.fetch.chglevel = ed.vars.changes;
        ed.blk.fetch.nextscan = scan + 1;
        ed.blk.fetch.nextbuf = id;
    } else {
        ed.blk.fetch.nextline = 0;
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
