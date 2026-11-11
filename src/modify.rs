//! `modify.c`: low-level text changes (`delete`, `add`, `change`).  No module state.

use crate::blk::{
    BufId, afterdo, beforedo, blkadd, blkdirty, blkget, fetchline, findblk, locate, skiplines,
};
use crate::tio::msg;
use crate::types::{
    BLKSIZE, INFINITY, MARK_FIRST, MARK_UNSET, MAXBLKS, Mark, NMARKS, cstr, mark_at_line, markidx,
    markline,
};
use crate::vars::Editor;

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
        ed.redraw
            .range(markline(frommark), markline(tomark), markline(frommark));
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

    let mut i = findblk(&ed.vars.lnum, markline(frommark), 1);

    let mut m = frommark;
    while m < tomark && ed.vars.lnum[i] < INFINITY {
        let id = blkget(ed, i as i32);
        let lnum = &ed.vars.lnum;
        let c = &mut ed.blk.blk[id].buf.c;

        let mut scan = skiplines(c, 0, markline(m) - lnum[i - 1] - 1);
        scan += markidx(m);

        let mut cpy;
        if markline(tomark) > lnum[i] {
            cpy = BLKSIZE;
        } else if markline(tomark) == markline(m) {
            cpy = scan - markidx(m) + markidx(tomark);
        } else {
            cpy = skiplines(c, scan, markline(tomark) - markline(m));
            cpy += markidx(tomark);
        }

        if cpy < BLKSIZE {
            c.copy_within(cpy.., scan);
            scan += BLKSIZE - cpy;
        }
        c[scan..].fill(0);

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

// C: none (B16)
/// Whether inserting `newtext` at `atmark` would leave a line longer than a
/// block can hold.  C's `add` overflows the block in that case.
fn would_overflow(ed: &mut Editor, atmark: Mark, newtext: &[u8]) -> bool {
    let pre = markidx(atmark);
    let lastpart = newtext.iter().rev().take_while(|&&b| b != b'\n').count();
    let addlines = newtext.contains(&b'\n');
    let rest = if (pre == 0 && lastpart == 0) || markline(atmark) > ed.vars.nlines {
        0
    } else {
        fetchline(ed, markline(atmark)).len() + 1 - pre
    };
    let mut lines = newtext.split(|&b| b == b'\n');
    let first = lines.next().map_or(0, <[u8]>::len);
    if !addlines {
        return pre + first + rest > BLKSIZE - 1;
    }
    pre + first + 1 > BLKSIZE - 1
        || newtext[..newtext.len() - lastpart]
            .split(|&b| b == b'\n')
            .skip(1)
            .any(|l| l.len() + 1 > BLKSIZE - 1)
        || lastpart + rest > BLKSIZE - 1
}

/// The logical block that `add` inserts into for line `l`.  C has no
/// `blkno + 1 < MAXBLKS` bound and reads past `lnum[]` once the table is full.
fn add_block(ed: &Editor, l: i64) -> usize {
    let mut blkno = 1;
    while ed.vars.lnum[blkno] < l && blkno + 1 < MAXBLKS && ed.vars.lnum[blkno + 1] < INFINITY {
        blkno += 1;
    }
    blkno
}

/// The number of blocks `append_text` adds when it copies `parts` into a
/// block that already holds `prefix`, or `usize::MAX` if it never ends.
fn blocks_needed(prefix: &[u8], parts: [&[u8]; 2]) -> usize {
    let mut c = [0u8; BLKSIZE];
    c[..prefix.len()].copy_from_slice(prefix);
    let mut build = prefix.len();
    let mut adds = 0;
    for src in parts {
        let mut nt = 0;
        while nt < src.len() {
            let n = (src.len() - nt).min((BLKSIZE - 1).saturating_sub(build));
            c[build..build + n].copy_from_slice(&src[nt..nt + n]);
            build += n;
            nt += n;
            if nt < src.len() {
                let keep = c[..build]
                    .iter()
                    .rposition(|&b| b == b'\n')
                    .map_or(0, |p| p + 1);
                if keep == 0 && n == 0 {
                    return usize::MAX;
                }
                c.copy_within(keep..build, 0);
                build -= keep;
                adds += 1;
            }
        }
    }
    adds
}

/// Whether adding `newtext` at `atmark` would need more text blocks than the
/// header holds (`MAXBLKS - 1`).  C's `blkadd` then drops the last block and
/// later reads past `lnum[]` (B21).
fn would_exhaust_blocks(ed: &mut Editor, atmark: Mark, newtext: &[u8]) -> bool {
    let used = (1..MAXBLKS).filter(|&i| ed.vars.lnum[i] != INFINITY).count();
    let l = markline(atmark);
    let blkno = add_block(ed, l);
    let blk = blkget(ed, blkno as i32);
    let buf = ed.blk.blk[blk].buf.c;
    let newptr = skiplines(&buf, 0, l - ed.vars.lnum[blkno - 1] - 1) + markidx(atmark);
    let need = blocks_needed(&buf[..newptr], [newtext, cstr(&buf[newptr..])]);
    used.saturating_add(need) > MAXBLKS - 1
}

/// `add`'s refusal when the change does not fit (B16, B21).
fn refuse(ed: &mut Editor, text: &[u8]) -> bool {
    msg(ed, text, &[]);
    ed.vars.toolong = true;
    false
}

/// Copies `src` into block `*blk` at `*build`.  When the block fills up, the
/// partial last line moves to a new block added after it.
fn append_text(ed: &mut Editor, blk: &mut BufId, blkno: &mut usize, build: &mut usize, src: &[u8]) {
    let mut nt = 0;
    while nt < src.len() {
        let c = &mut ed.blk.blk[*blk].buf.c;
        let n = (src.len() - nt).min((BLKSIZE - 1).saturating_sub(*build));
        c[*build..*build + n].copy_from_slice(&src[nt..nt + n]);
        *build += n;
        nt += n;
        if nt < src.len() {
            let keep = c[..*build]
                .iter()
                .rposition(|&b| b == b'\n')
                .map_or(0, |p| p + 1);
            let mut linebuf = [0u8; BLKSIZE];
            let len = *build - keep;
            linebuf[..len].copy_from_slice(&c[keep..*build]);
            c[keep..].fill(0);
            blkdirty(ed, *blk);

            *blkno += 1;
            *blk = blkadd(ed, *blkno as i32);

            ed.blk.blk[*blk].buf.c[..len].copy_from_slice(&linebuf[..len]);
            *build = len;
        }
    }
}

// C: modify.c:add
/// `newtext` has no NUL terminator.  Returns false, changing nothing, when a
/// line would grow past `BLKSIZE - 1` bytes (Appendix B16) or the text would
/// not fit in the header's block table (B21).
pub fn add(ed: &mut Editor, atmark: Mark, newtext: &[u8]) -> bool {
    let newtext = cstr(newtext);

    if newtext.is_empty() {
        return true;
    }

    if would_overflow(ed, atmark, newtext) {
        return refuse(ed, b"Line too long");
    }
    if would_exhaust_blocks(ed, atmark, newtext) {
        return refuse(ed, b"File too big");
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
            ed.redraw.range(l, l, l + addlines);
        } else {
            // The last line was split, so it must be redrawn too.
            ed.redraw.range(l, l + 1, l + addlines + 1);
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

    let mut blkno = add_block(ed, l);
    let mut blk = blkget(ed, blkno as i32);
    let buf = ed.blk.blk[blk].buf.c;

    let mut newptr = skiplines(&buf, 0, markline(atmark) - ed.vars.lnum[blkno - 1] - 1);
    newptr += markidx(atmark);

    let mut build = newptr;
    append_text(ed, &mut blk, &mut blkno, &mut build, newtext);
    append_text(ed, &mut blk, &mut blkno, &mut build, cstr(&buf[newptr..]));

    if ed.vars.lnum[blkno] < ed.vars.nlines
        && ed.vars.lnum[blkno + 1] - ed.vars.lnum[blkno] < (BLKSIZE >> 6) as i64
    {
        let following = blkget(ed, blkno as i32 + 1);
        let fc = &ed.blk.blk[following].buf.c;
        let flen = fc.iter().position(|&b| b == 0).unwrap_or(BLKSIZE);
        if flen + build < BLKSIZE - 1 {
            let ftext = fc[..flen].to_vec();
            let c = &mut ed.blk.blk[blk].buf.c;
            c[build..build + flen].copy_from_slice(&ftext);
            c[build + flen..].fill(0);
            blkdirty(ed, blk);

            blk = following;
            build = 0;
        }
    }

    ed.blk.blk[blk].buf.c[build..].fill(0);
    blkdirty(ed, blk);
    true
}

// C: modify.c:change
pub fn change(ed: &mut Editor, frommark: Mark, tomark: Mark, newtext: &[u8]) {
    let newtext = cstr(newtext);

    if frommark + 1 == tomark && newtext.len() == 1 && newtext[0] != b'\n' {
        let (_, blk, mut text) = locate(ed, markline(frommark));
        let c = &ed.blk.blk[blk].buf.c;
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

    // Refuse before the delete, so nothing is lost (B21).  With the old text
    // still in place this can only overestimate the blocks needed.
    if would_exhaust_blocks(ed, frommark, newtext) {
        refuse(ed, b"File too big");
        return;
    }

    beforedo(ed, false);
    delete(ed, frommark, tomark);
    let _ = add(ed, frommark, newtext);
    ed.vars.rptlabel = b"changed";
    afterdo(ed);
}
