//! `cut.c`: cut buffers.

use std::fs::File;
use std::os::fd::AsRawFd;

use crate::blk::{afterdo, beforedo, blkget, blksync, findblk, locate, pfetch, skiplines};
use crate::config::cutname_fmt;
use crate::config::lseek_read;
use crate::fmt::Arg;
use crate::modify::add;
use crate::os;
use crate::tio::msg;
use crate::types::LINEMASK;
use crate::types::{BLKSIZE, Blk, MARK_UNSET, MAXBLKS, Mark, cstr, markidx, markline};
use crate::vars::Editor;

pub const NANNONS: usize = 9;

/// `struct cutbuf`
#[derive(Default, Clone)]
pub struct CutBuf {
    /// Physical blocks holding the text, in order.
    pub phys: Vec<u16>,
    pub start: i32,
    pub end: i32,
    /// The raw descriptor of the `Cut::files` entry holding the blocks, or
    /// `None` for the tmp file.  Several buffers may share one descriptor.
    pub fd: Option<i32>,
    pub lnmode: bool,
}

pub struct Cut {
    /// `named[0..27]` (`a`-`z` and `.`), then `annon[0..NANNONS]`.
    pub bufs: [CutBuf; ANNON + NANNONS],
    pub cbname: u8,
    /// Open descriptors referenced by `CutBuf::fd`.
    pub files: Vec<File>,
}

impl Default for Cut {
    fn default() -> Cut {
        Cut {
            bufs: std::array::from_fn(|_| CutBuf::default()),
            cbname: 0,
            files: Vec::new(),
        }
    }
}

/// Index of the first anonymous buffer in `Cut::bufs`.
const ANNON: usize = 27;

impl Cut {
    // C: cut.c:cutneeds
    /// Stores the blocks the cut buffers use in `need`; returns how many.
    pub fn needs(&self, need: &mut Blk) -> usize {
        let mut n: usize = 0;

        for cb in &self.bufs {
            if cb.fd.is_some() {
                continue;
            }
            for &p in cb.phys.iter().rev() {
                // C overflows `need` here; the extra entries are dropped.
                if n < MAXBLKS {
                    need.set_n(n, p);
                }
                n += 1;
            }
        }

        n.min(MAXBLKS)
    }

    // C: cut.c:cutname
    pub fn select(&mut self, name: u8) {
        self.cbname = name;
    }
}

// C: cut.c:cutfree
fn cutfree(ed: &mut Editor, k: usize) {
    let buf = &mut ed.cut.bufs[k];
    if buf.phys.is_empty() {
        return;
    }
    buf.phys = Vec::new();

    if let Some(fd) = buf.fd {
        if !ed.cut.bufs[..ANNON]
            .iter()
            .any(|b| !b.phys.is_empty() && b.fd == Some(fd))
        {
            ed.cut.files.retain(|f| f.as_raw_fd() != fd);
            let cutfname = cutname_fmt(ed.opts.directory(), os::getpid(), fd);
            let _ = os::unlink(&cutfname);
        }
    }
}

// C: cut.c:cutswitch
pub fn cutswitch(ed: &mut Editor, tmpname: &[u8]) {
    for k in ANNON..ANNON + NANNONS {
        cutfree(ed, k);
    }

    let in_tmp = |b: &CutBuf| !b.phys.is_empty() && b.fd.is_none();
    if !ed.cut.bufs[..ANNON].iter().any(in_tmp) {
        return;
    }

    let fd = match ed.vars.tmpfd.as_ref().map(os::dup) {
        Some(Ok(f)) => {
            let fd = f.as_raw_fd();
            ed.cut.files.push(f);
            Some(fd)
        }
        _ => None,
    };
    let cutfname = cutname_fmt(ed.opts.directory(), os::getpid(), fd.unwrap_or(-1));
    // C: `link(tmpname, cutfname) || unlink(tmpname);`
    if os::link(tmpname, &cutfname).is_ok() {
        let _ = os::unlink(tmpname);
    }

    for b in &mut ed.cut.bufs[..ANNON] {
        if in_tmp(b) {
            b.fd = fd;
        }
    }
}

// C: cut.c:cutend
// Out of line: inlining this into `main` makes the binary larger.
#[inline(never)]
pub fn cutend(ed: &mut Editor) {
    // Named buffers may be keeping an older tmp file alive.
    for k in 0..ANNON {
        cutfree(ed, k);
    }
}

// C: cut.c:cut
pub fn cut(ed: &mut Editor, from: Mark, to: Mark) {
    let lnmode = markidx(from) == 0 && markidx(to) == 0;

    let cbname = ed.cut.cbname;
    let k;
    if cbname == 0 {
        cutfree(ed, ANNON + NANNONS - 1);

        // The last (just freed) buffer rotates to the front and is reset
        // below, which matches C's shift-and-reuse.
        ed.cut.bufs[ANNON..].rotate_right(1);

        k = ANNON;
    } else if cbname.is_ascii_lowercase() {
        k = (cbname - b'a') as usize;
        cutfree(ed, k);
    } else if cbname == b'.' {
        k = 26;
        cutfree(ed, k);
    } else {
        msg(ed, b"Invalid cut buffer name: \"%c", &[Arg::C(cbname)]);
        ed.cut.cbname = 0;
        return;
    }
    ed.cut.cbname = 0;
    ed.cut.bufs[k].fd = None;
    ed.cut.bufs[k].lnmode = lnmode;

    if markidx(from) == 0 && markidx(to) == 0 {
        ed.vars.rptlines = markline(to) - markline(from);
        ed.vars.rptlabel = b"yanked";
    }

    blksync(ed);

    let (first, mut id, mut scan) = locate(ed, markline(from));
    scan += markidx(from);

    ed.cut.bufs[k].start = scan as i32;

    let line = markline(to);
    let last = findblk(&ed.vars.lnum, line, first);
    if last != first {
        id = blkget(ed, last as i32);
    }
    scan = skiplines(&ed.blk.blk[id].buf.c, 0, line - ed.vars.lnum[last - 1] - 1);
    if markline(to) <= ed.vars.nlines {
        scan += markidx(to);
    }

    let hdr = &ed.blk.hdr;
    let cb = &mut ed.cut.bufs[k];
    cb.end = scan as i32;
    let nblks = last - first + (cb.end > 0) as usize;
    cb.phys = (first..first + nblks).map(|i| hdr.n(i)).collect();
}

// C: cut.c:readcutblk
fn readcutblk(ed: &mut Editor, k: usize, blkno: usize) {
    let cb = &ed.cut.bufs[k];
    let off = cb.phys[blkno] as u64 * BLKSIZE as u64;

    let fd = match cb.fd {
        Some(fd) => ed.cut.files.iter().find(|f| f.as_raw_fd() == fd),
        None => ed.vars.tmpfd.as_ref(),
    };

    if !lseek_read(fd, off, &mut ed.vars.tmpblk.c) {
        msg(ed, b"Error reading back from tmp file for pasting!", &[]);
    }
}

/// `tmpblk.c[from]` up to its NUL, copied so it can be passed to `add`.
fn tmpblk_text(ed: &Editor, from: usize) -> Vec<u8> {
    cstr(&ed.vars.tmpblk.c[from.min(BLKSIZE)..]).to_vec()
}

fn tmpblk_cut(ed: &mut Editor, end: i32) {
    if let Some(b) = ed.vars.tmpblk.c.get_mut(end as usize) {
        *b = 0;
    }
}

// C: cut.c:paste
pub fn paste(ed: &mut Editor, at: Mark, after: bool, retend: bool) -> Mark {
    let mut at = at;
    let cbname = ed.cut.cbname;

    let k = match cbname {
        b'A'..=b'Z' => (cbname - b'A') as usize,
        b'a'..=b'z' => (cbname - b'a') as usize,
        b'1'..=b'9' => ANNON + (cbname - b'1') as usize,
        b'.' => 26,
        0 => ANNON,
        _ => {
            msg(ed, b"Invalid cut buffer name: \"%c", &[Arg::C(cbname)]);
            ed.cut.cbname = 0;
            return MARK_UNSET;
        }
    };

    if ed.cut.bufs[k].phys.is_empty() {
        if cbname != 0 {
            msg(ed, b"Cut buffer \"%c is empty", &[Arg::C(cbname)]);
        } else {
            msg(ed, b"Cut buffer is empty", &[]);
        }
        ed.cut.cbname = 0;
        return MARK_UNSET;
    }
    ed.cut.cbname = 0;

    if ed.cut.bufs[k].lnmode {
        at &= LINEMASK;
        if after {
            at += BLKSIZE as i64;
        }
    } else if after {
        // With markidx(at) == 0 we might be pasting into an empty line.
        if markidx(at) == 0 {
            pfetch(ed, markline(at));
            if ed.vars.plen != 0 {
                at += 1;
            }
        } else {
            at += 1;
        }
    }

    // mark[27] stays in sync with the changes add() makes.
    ed.vars.mark[27] = at;

    let cb = &ed.cut.bufs[k];
    let (nblks, start, end) = (cb.phys.len(), cb.start, cb.end);

    if nblks == 1 {
        readcutblk(ed, k, 0);

        if end != 0 {
            tmpblk_cut(ed, end);
        }

        beforedo(ed, false);
        let t = tmpblk_text(ed, start as usize);
        add(ed, at, &t);
        afterdo(ed);
    } else {
        beforedo(ed, false);
        let mut i = nblks - 1;

        if end > 0 {
            readcutblk(ed, k, i);
            tmpblk_cut(ed, end);
            let t = tmpblk_text(ed, 0);
            add(ed, at, &t);
            i -= 1;
        }

        while i > 0 {
            readcutblk(ed, k, i);
            let t = tmpblk_text(ed, 0);
            add(ed, at, &t);
            i -= 1;
        }

        readcutblk(ed, k, 0);
        let t = tmpblk_text(ed, start as usize);
        add(ed, at, &t);
        afterdo(ed);
    }

    ed.vars.rptlines = markline(ed.vars.mark[27]) - markline(at);
    ed.vars.rptlabel = b"pasted";

    if retend {
        return ed.vars.mark[27] - 1;
    }
    at
}
