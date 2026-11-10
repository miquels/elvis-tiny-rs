//! `modify.c`: low-level text changes (`delete`, `add`, `change`).  No module state.

use crate::blk::{afterdo, beforedo, blkadd, blkdirty, blkget, skipline};
use crate::redraw::redrawrange;
use crate::vars::Editor;
use crate::vi_h::{
    BLKSIZE, INFINITY, MARK_FIRST, MARK_UNSET, Mark, NMARKS, cstr, mark_at_line, markidx, markline,
};

// C: modify.c:delete
pub fn delete(ed: &mut Editor, frommark: Mark, tomark: Mark) {
    let mut tomark = tomark;

    if frommark == tomark {
        return;
    }

    ed.vars.changes += 1;
    ed.vars.significant = true;

    if markline(frommark) != markline(tomark) {
        ed.vars.mustredraw = true;
        redrawrange(ed, markline(frommark), markline(tomark), markline(frommark));
    }

    let l = markline(tomark);
    for i in 0..NMARKS {
        let m = &mut ed.vars.mark[i];
        if *m < frommark {
            continue;
        } else if *m < tomark {
            *m = MARK_UNSET;
        } else if markline(*m) == l {
            if markline(frommark) == l {
                *m -= markidx(tomark) as i64 - markidx(frommark) as i64;
            } else {
                *m -= markidx(tomark) as i64;
            }
        } else {
            *m -= mark_at_line(l - markline(frommark));
        }
    }

    if markidx(frommark) == 0 && markidx(tomark) == 0 {
        ed.vars.rptlines = markline(tomark) - markline(frommark);
        ed.vars.rptlabel = b"deleted";
    }

    let l = markline(frommark);
    let mut i = 1;
    while ed.vars.lnum[i] < l {
        i += 1;
    }

    let mut m = frommark;
    while m < tomark && ed.vars.lnum[i] < INFINITY {
        let id = blkget(ed, i as i32);
        let lnum = &ed.vars.lnum;
        let c = &mut ed.blk.blk[id].buf.c;

        let mut scan = 0;
        let mut l = markline(m) - lnum[i - 1] - 1;
        while l > 0 {
            scan = skipline(c, scan);
            l -= 1;
        }
        scan += markidx(m);

        let mut cpy;
        if markline(tomark) > lnum[i] {
            cpy = BLKSIZE;
        } else if markline(tomark) == markline(m) {
            cpy = scan - markidx(m) + markidx(tomark);
        } else {
            cpy = scan;
            let mut l = markline(tomark) - markline(m);
            while l > 0 {
                cpy = skipline(c, cpy);
                l -= 1;
            }
            cpy += markidx(tomark);
        }

        while cpy < BLKSIZE {
            c[scan] = c[cpy];
            scan += 1;
            cpy += 1;
        }
        while scan < BLKSIZE {
            c[scan] = 0;
            scan += 1;
        }

        tomark -= mark_at_line(lnum[i] + 1 - markline(m));

        if c[0] != 0 {
            i += 1;
        }

        blkdirty(ed, id);

        m = mark_at_line(ed.vars.lnum[i - 1] + 1);
    }

    if ed.vars.nlines == 0 {
        let id = blkadd(ed, 1);
        ed.blk.blk[id].buf.c[0] = b'\n';
        blkdirty(ed, id);
        ed.vars.cursor = MARK_FIRST;
    }
}

// C: modify.c:add
/// `newtext` has no NUL terminator.
pub fn add(ed: &mut Editor, atmark: Mark, newtext: &[u8]) {
    let newtext = cstr(newtext);
    let mut nt = 0;

    if newtext.is_empty() {
        return;
    }

    ed.vars.changes += 1;
    ed.vars.significant = true;

    let mut addlines: i64 = 0;
    let mut lastpart: i64 = 0;
    for &b in newtext {
        if b == b'\n' {
            addlines += 1;
            lastpart = 0;
        } else {
            lastpart += 1;
        }
    }

    if lastpart == 0 && markidx(atmark) == 0 {
        ed.vars.rptlines = addlines;
        ed.vars.rptlabel = b"added";
    }

    let l = markline(atmark);

    if addlines > 0 {
        ed.vars.mustredraw = true;
        if markidx(atmark) == 0 && lastpart == 0 {
            redrawrange(ed, l, l, l + addlines);
        } else {
            // The last line was split, so it must be redrawn too.
            redrawrange(ed, l, l + 1, l + addlines + 1);
        }
    }

    for i in 0..NMARKS {
        let m = &mut ed.vars.mark[i];
        if *m < atmark {
            continue;
        } else if markline(*m) > l {
            *m += mark_at_line(addlines);
        } else if addlines > 0 {
            *m += mark_at_line(addlines) + lastpart - markidx(atmark) as i64;
        } else {
            *m += lastpart;
        }
    }

    let mut blkno = 1;
    while ed.vars.lnum[blkno] < l && ed.vars.lnum[blkno + 1] < INFINITY {
        blkno += 1;
    }
    let mut blk = blkget(ed, blkno as i32);
    let buf = ed.blk.blk[blk].buf.c;

    let mut newptr = 0;
    let mut l = markline(atmark) - ed.vars.lnum[blkno - 1] - 1;
    while l > 0 {
        newptr = skipline(&buf, newptr);
        l -= 1;
    }
    newptr += markidx(atmark);

    let mut build = newptr;
    let mut linebuf = [0u8; BLKSIZE];
    let mut scan;

    while nt < newtext.len() {
        {
            let c = &mut ed.blk.blk[blk].buf.c;
            while nt < newtext.len() && build < BLKSIZE - 1 {
                c[build] = newtext[nt];
                build += 1;
                nt += 1;
            }
        }
        if nt < newtext.len() {
            let c = &mut ed.blk.blk[blk].buf.c;
            scan = BLKSIZE;
            while build > 0 && c[build - 1] != b'\n' {
                scan -= 1;
                build -= 1;
                linebuf[scan] = c[build];
            }

            while build < BLKSIZE {
                c[build] = 0;
                build += 1;
            }
            blkdirty(ed, blk);

            blkno += 1;
            blk = blkadd(ed, blkno as i32);

            let c = &mut ed.blk.blk[blk].buf.c;
            build = 0;
            while scan < BLKSIZE {
                c[build] = linebuf[scan];
                build += 1;
                scan += 1;
            }
        }
    }

    while newptr < BLKSIZE && buf[newptr] != 0 {
        {
            let c = &mut ed.blk.blk[blk].buf.c;
            while newptr < BLKSIZE && buf[newptr] != 0 && build < BLKSIZE - 1 {
                c[build] = buf[newptr];
                build += 1;
                newptr += 1;
            }
        }
        if newptr < BLKSIZE && buf[newptr] != 0 {
            let c = &mut ed.blk.blk[blk].buf.c;
            scan = BLKSIZE;
            while build > 0 && c[build - 1] != b'\n' {
                scan -= 1;
                build -= 1;
                linebuf[scan] = c[build];
            }

            while build < BLKSIZE {
                c[build] = 0;
                build += 1;
            }
            blkdirty(ed, blk);

            blkno += 1;
            blk = blkadd(ed, blkno as i32);

            let c = &mut ed.blk.blk[blk].buf.c;
            build = 0;
            while scan < BLKSIZE {
                c[build] = linebuf[scan];
                build += 1;
                scan += 1;
            }
        }
    }

    if ed.vars.lnum[blkno] < ed.vars.nlines
        && ed.vars.lnum[blkno + 1] - ed.vars.lnum[blkno] < (BLKSIZE >> 6) as i64
    {
        let following = blkget(ed, blkno as i32 + 1);
        let fc = &ed.blk.blk[following].buf.c;
        let flen = fc.iter().position(|&b| b == 0).unwrap_or(BLKSIZE);
        if flen + build < BLKSIZE - 1 {
            let ftext = fc[..flen].to_vec();
            let c = &mut ed.blk.blk[blk].buf.c;
            for &b in &ftext {
                c[build] = b;
                build += 1;
            }
            while build < BLKSIZE {
                c[build] = 0;
                build += 1;
            }
            blkdirty(ed, blk);

            blk = following;
            build = 0;
        }
    }

    let c = &mut ed.blk.blk[blk].buf.c;
    while build < BLKSIZE {
        c[build] = 0;
        build += 1;
    }
    blkdirty(ed, blk);
}

// C: modify.c:change
pub fn change(ed: &mut Editor, frommark: Mark, tomark: Mark, newtext: &[u8]) {
    let newtext = cstr(newtext);

    if frommark + 1 == tomark && newtext.len() == 1 && newtext[0] != b'\n' {
        let l = markline(frommark);
        let mut i = 1;
        while ed.vars.lnum[i] < l {
            i += 1;
        }

        let blk = blkget(ed, i as i32);

        let c = &ed.blk.blk[blk].buf.c;
        let mut text = 0;
        let mut n = l - ed.vars.lnum[i - 1] - 1;
        while n > 0 {
            text = skipline(c, text);
            n -= 1;
        }

        text += markidx(frommark);
        if c[text] == newtext[0] {
            return;
        } else if c[text] != b'\n' {
            ed.vars.changes += 1;
            ed.vars.significant = true;
            beforedo(ed, false);
            ed.blk.blk[blk].buf.c[text] = newtext[0];
            blkdirty(ed, blk);
            afterdo(ed);
            return;
        }
        // else it is a complex change involving a newline
    }

    beforedo(ed, false);
    delete(ed, frommark, tomark);
    add(ed, frommark, newtext);
    ed.vars.rptlabel = b"changed";
    afterdo(ed);
}
