//! `cut.c`: cut buffers.

use std::fs::File;
use std::os::fd::AsRawFd;

use crate::blk::{afterdo, beforedo, blkget, blksync, findblk, locate, lseek_read, skiplines};
use crate::config::cutname_fmt;
use crate::misc::pfetch;
use crate::modify::add;
use crate::os;
use crate::tio::{Arg, msg};
use crate::vars::Editor;
use crate::vi_h::LINEMASK;
use crate::vi_h::{BLKSIZE, Blk, MARK_UNSET, MAXBLKS, Mark, cstr, markidx, markline};

pub const NANNONS: usize = 9;

/// `struct cutbuf`
#[derive(Default, Clone)]
pub struct CutBuf {
    /// Block numbers holding the text; `None` is C's null pointer.
    pub phys: Option<Vec<i16>>,
    pub nblks: i32,
    pub start: i32,
    pub end: i32,
    /// C's descriptor number: -1 means "use tmpfd", otherwise the raw fd of
    /// an entry in `Cut::files`.  Several buffers may share one descriptor,
    /// so the `File` itself lives in `Cut::files`.  Starts at 0 like a C
    /// static.
    pub fd: i32,
    pub lnmode: bool,
}

#[derive(Default)]
pub struct Cut {
    pub named: [CutBuf; 27],
    pub annon: [CutBuf; NANNONS],
    pub cbname: u8,
    /// Open descriptors referenced by `CutBuf::fd`.
    pub files: Vec<File>,
}

/// Cut buffers are addressed as `0..27` for `named` and `27..` for `annon`.
const ANNON: usize = 27;

fn cbref(cut: &Cut, k: usize) -> &CutBuf {
    if k < ANNON {
        &cut.named[k]
    } else {
        &cut.annon[k - ANNON]
    }
}

fn cbmut(cut: &mut Cut, k: usize) -> &mut CutBuf {
    if k < ANNON {
        &mut cut.named[k]
    } else {
        &mut cut.annon[k - ANNON]
    }
}

// C: cut.c:cutneeds
pub fn cutneeds(ed: &mut Editor, need: &mut Blk) -> i32 {
    let mut n: usize = 0;

    for cb in ed.cut.named.iter().chain(ed.cut.annon.iter()) {
        if cb.fd > 0 {
            continue;
        }
        let Some(phys) = cb.phys.as_ref() else {
            continue;
        };
        let mut i = cb.nblks;
        while i > 0 {
            i -= 1;
            // C overflows `need` here; the extra entries are dropped.
            if n < MAXBLKS {
                need.set_n(n, phys[i as usize] as u16);
            }
            n += 1;
        }
    }

    n.min(MAXBLKS) as i32
}

// C: cut.c:cutfree
fn cutfree(ed: &mut Editor, k: usize) {
    let buf = cbmut(&mut ed.cut, k);

    if buf.nblks <= 0 {
        return;
    }

    buf.nblks = 0;
    buf.phys = None;
    let fd = buf.fd;

    let mut i = 0;
    if fd >= 0 {
        while i < 27 {
            if ed.cut.named[i].nblks > 0 && ed.cut.named[i].fd == fd {
                break;
            }
            i += 1;
        }
    }

    if fd >= 0 && i == 27 {
        ed.cut.files.retain(|f| f.as_raw_fd() != fd);
        let cutfname = cutname_fmt(cstr(&ed.opts.o_directory), os::getpid(), fd);
        let _ = os::unlink(&cutfname);
    }
}

// C: cut.c:cutswitch
pub fn cutswitch(ed: &mut Editor, tmpname: &[u8]) {
    for i in 0..NANNONS {
        cutfree(ed, ANNON + i);
    }

    let mut i = 0;
    while i < 27 {
        if ed.cut.named[i].nblks > 0 && ed.cut.named[i].fd < 0 {
            break;
        }
        i += 1;
    }

    if i == 27 {
        return;
    }

    let fd = match ed.vars.tmpfd.as_ref().map(os::dup) {
        Some(Ok(f)) => {
            let fd = f.as_raw_fd();
            ed.cut.files.push(f);
            fd
        }
        _ => -1,
    };
    let cutfname = cutname_fmt(cstr(&ed.opts.o_directory), os::getpid(), fd);
    // C: `link(tmpname, cutfname) || unlink(tmpname);`
    if os::link(tmpname, &cutfname).is_ok() {
        let _ = os::unlink(tmpname);
    }

    while i < 27 {
        if ed.cut.named[i].nblks > 0 && ed.cut.named[i].fd < 0 {
            ed.cut.named[i].fd = fd;
        }
        i += 1;
    }
}

// C: cut.c:cutend
// Out of line: inlining this into `main` makes the binary larger.
#[inline(never)]
pub fn cutend(ed: &mut Editor) {
    // Named buffers may be keeping an older tmp file alive.
    for i in 0..27 {
        cutfree(ed, i);
    }
}

// C: cut.c:cutname
pub fn cutname(ed: &mut Editor, name: i32) {
    ed.cut.cbname = name as u8;
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
        ed.cut.annon.rotate_right(1);

        k = ANNON;
        ed.cut.annon[0].nblks = 0;
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
    cbmut(&mut ed.cut, k).fd = -1;

    cbmut(&mut ed.cut, k).lnmode = lnmode;

    if markidx(from) == 0 && markidx(to) == 0 {
        ed.vars.rptlines = markline(to) - markline(from);
        ed.vars.rptlabel = b"yanked";
    }

    blksync(ed);

    let (mut first, mut id, mut scan) = locate(ed, markline(from));
    scan += markidx(from);

    cbmut(&mut ed.cut, k).start = scan as i32;

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
    let cb = cbmut(&mut ed.cut, k);
    cb.end = scan as i32;

    cb.nblks = (last - first) as i32;
    if cb.end > 0 {
        cb.nblks += 1;
    }
    let mut phys = Vec::with_capacity(cb.nblks as usize);
    for _ in 0..cb.nblks {
        phys.push(hdr.n(first) as i16);
        first += 1;
    }
    cb.phys = Some(phys);
}

// C: cut.c:readcutblk
fn readcutblk(ed: &mut Editor, k: usize, blkno: usize) {
    let cb = cbref(&ed.cut, k);
    let off = cb.phys.as_ref().map_or(0, |p| p[blkno]) as i64 * BLKSIZE as i64;

    let fd = if cb.fd >= 0 {
        ed.cut.files.iter().find(|f| f.as_raw_fd() == cb.fd)
    } else {
        ed.vars.tmpfd.as_ref()
    };

    if lseek_read(fd, off, &mut ed.vars.tmpblk.c) != BLKSIZE as isize {
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

    if cbref(&ed.cut, k).nblks == 0 {
        if cbname != 0 {
            msg(ed, b"Cut buffer \"%c is empty", &[Arg::C(cbname)]);
        } else {
            msg(ed, b"Cut buffer is empty", &[]);
        }
        ed.cut.cbname = 0;
        return MARK_UNSET;
    }
    ed.cut.cbname = 0;

    if cbref(&ed.cut, k).lnmode {
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

    let (nblks, start, end) = {
        let cb = cbref(&ed.cut, k);
        (cb.nblks, cb.start, cb.end)
    };

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
            readcutblk(ed, k, i as usize);
            tmpblk_cut(ed, end);
            let t = tmpblk_text(ed, 0);
            add(ed, at, &t);
            i -= 1;
        }

        while i > 0 {
            readcutblk(ed, k, i as usize);
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
