//! `redraw.c`: screen updates.

use crate::curses;
use crate::misc::{fetchline, pfetch};
use crate::vars::Editor;
use crate::vi_h::{
    BLKSIZE, INFINITY, MARK_UNSET, Mark, botline, ch, markidx, markline, rightcol, schar,
};

#[derive(Default)]
pub struct Redraw {
    pub smartlno: i64,
    pub redrawafter: i64,
    pub preredraw: i64,
    pub postredraw: i64,
    pub idx2col_pcursor: Mark,
    pub idx2col_pcol: Mark,
    pub idx2col_chgs: i64,
    pub redraw_chgs: i64,
}

// C: redraw.c:redrawrange
pub fn redrawrange(ed: &mut Editor, after: i64, pre: i64, post: i64) {
    let r = &mut ed.redraw;
    if after == r.redrawafter {
        r.preredraw -= post - pre;
        if r.postredraw < post {
            r.preredraw += post - r.postredraw;
            r.postredraw = post;
        }
        if r.redrawafter > r.preredraw {
            r.redrawafter = r.preredraw;
        }
        if r.redrawafter < 1 {
            r.redrawafter = 0;
            r.preredraw = INFINITY;
            r.postredraw = INFINITY;
        }
    } else if r.postredraw > 0 {
        r.postredraw = INFINITY;
        r.preredraw = INFINITY;
        if after < r.redrawafter {
            r.redrawafter = after;
        }
    } else {
        r.redrawafter = after;
        r.preredraw = pre;
        r.postredraw = post;
    }
}

// C: redraw.c:idx2col
/// `text` is the line (no NUL).
pub fn idx2col(ed: &mut Editor, curs: Mark, text: &[u8], inputting: bool) -> i32 {
    let tabstop = ed.opts.o_tabstop[0] as i32;
    let list = ed.opts.o_list[0] != 0;
    let r = &mut ed.redraw;
    let mut col: i32 = 0;
    let mut idx: i32 = markidx(curs) as i32;
    let mut t: usize = 0;

    if r.idx2col_chgs == ed.vars.changes
        && ((curs ^ r.idx2col_pcursor) & !(BLKSIZE as i64 - 1)) == 0
    {
        if curs == r.idx2col_pcursor {
            let pcol = r.idx2col_pcol;
            if ch(text, idx as usize) == b'\t' && !inputting && !list {
                return (pcol + tabstop as i64 - (pcol % tabstop as i64) - 1) as i32;
            } else {
                return pcol as i32;
            }
        }

        if curs > r.idx2col_pcursor {
            col = r.idx2col_pcol as i32;
            idx = markidx(curs) as i32 - markidx(r.idx2col_pcursor) as i32;
            t = markidx(r.idx2col_pcursor);
        }
    }

    while idx > 0 {
        let i = schar(ch(text, t));
        if i == 0 {
            break;
        }
        if i == b'\t' as i32 && !list {
            col += tabstop;
            col -= col % tabstop;
        } else if (0..b' ' as i32).contains(&i) || i == 0o177 {
            col += 2;
        } else {
            col += 1;
        }
        t += 1;
        idx -= 1;
    }

    r.idx2col_pcursor = curs;
    r.idx2col_pcol = col as Mark;
    r.idx2col_chgs = ed.vars.changes;

    if ch(text, t) == b'\t' && !inputting && !list {
        col + tabstop - (col % tabstop) - 1
    } else {
        col
    }
}

// C: redraw.c:mark2phys
fn mark2phys(ed: &mut Editor, m: Mark, text: &[u8], inputting: bool) -> i32 {
    let i = idx2col(ed, m, text, inputting);
    while i < ed.vars.leftcol {
        ed.vars.leftcol -= ed.opts.o_sidescroll[0] as i32;
        ed.vars.mustredraw = true;
        redrawrange(ed, 1, INFINITY, INFINITY);
        ed.curses.qaddch(b'\r');
    }
    while i > rightcol(ed.vars.leftcol, ed.curses.cols) {
        ed.vars.leftcol += ed.opts.o_sidescroll[0] as i32;
        ed.vars.mustredraw = true;
        redrawrange(ed, 1, INFINITY, INFINITY);
        ed.curses.qaddch(b'\r');
    }
    ed.vars.physcol = i - ed.vars.leftcol;
    ed.vars.physrow = (markline(m) - ed.vars.topline) as i32;

    ed.vars.physcol
}

// C: redraw.c:drawtext
fn drawtext(ed: &mut Editor, text: &[u8], clr: bool) {
    let leftcol = ed.vars.leftcol;
    let list = ed.opts.o_list[0] != 0;
    let c = &mut ed.curses;
    let mut col: i32;
    let mut i: i32;
    let mut t: usize = 0;
    let mut limitcol = leftcol;
    let tabstop = ed.opts.o_tabstop[0] as i32;

    col = 0;
    loop {
        i = schar(ch(text, t));
        if i == 0 || col >= limitcol {
            break;
        }
        if i == b'\t' as i32 && !list {
            col = col + tabstop - (col % tabstop);
        } else if (0..b' ' as i32).contains(&i) || i == 0o177 {
            col += 2;
        } else {
            col += 1;
        }
        t += 1;
    }

    while col > limitcol {
        c.qaddch(b' ');
        limitcol += 1;
    }

    limitcol = leftcol + c.cols;
    loop {
        i = schar(ch(text, t));
        if i == 0 || col >= limitcol {
            break;
        }
        if i == b'\t' as i32 && !list {
            i = col + tabstop - (col % tabstop);
            if i < limitcol || (c.has_am() && i <= limitcol) {
                if !clr && c.has_pt() && ((i - leftcol) & 7) == 0 {
                    loop {
                        c.qaddch(b'\t');
                        col += 8;
                        if col >= i {
                            break;
                        }
                    }
                    col = i;
                } else {
                    loop {
                        c.qaddch(b' ');
                        col += 1;
                        if col >= i {
                            break;
                        }
                    }
                }
            } else {
                col = limitcol;
                if c.has_am() {
                    c.addch(b'\n');
                }
            }
        } else if (0..b' ' as i32).contains(&i) || i == 0o177 {
            col += 2;
            c.qaddch(b'^');
            if col <= limitcol {
                c.qaddch((i ^ b'@' as i32) as u8);
            }
        } else {
            col += 1;
            c.qaddch(i as u8);
        }
        t += 1;
    }

    if list && col < limitcol {
        c.qaddch(b'$');
        col += 1;
    }
    if clr && col < limitcol {
        c.clrtoeol();
    }
    if !c.has_am() || col <= limitcol {
        c.addch(b'\n');
    }
}

// C: redraw.c:smartdrawtext
fn smartdrawtext(ed: &mut Editor, text: &[u8], lno: i64) {
    ed.curses.mv((lno - ed.vars.topline) as i32, 0);
    drawtext(ed, text, true);
}

// C: redraw.c:redraw
pub fn redraw(ed: &mut Editor, curs: Mark, inputting: bool) {
    let mut text: Vec<u8>;
    let mut l: i64;
    let mut i: i32;
    let lines = ed.curses.lines;

    if curs == MARK_UNSET {
        if ed.vars.topline < 1 || ed.vars.topline > ed.vars.nlines {
            ed.vars.topline = 1;
        } else {
            ed.curses.mv(lines - 1, 0);
            ed.curses.clrtoeol();
        }
        ed.vars.leftcol = 0;
        ed.vars.mustredraw = true;
        ed.redraw.redrawafter = INFINITY;
        ed.redraw.preredraw = 0;
        ed.redraw.postredraw = 0;
        ed.redraw.redraw_chgs = 0;
        ed.redraw.smartlno = 0;
        return;
    }

    l = markline(curs);
    text = fetchline(ed, l);
    mark2phys(ed, curs, &text, inputting);

    let bot = |ed: &Editor| botline(ed.vars.topline, ed.curses.lines);

    if l >= ed.vars.topline && l <= bot(ed) {
        if ed.redraw.redraw_chgs != ed.vars.changes && !ed.vars.mustredraw {
            smartdrawtext(ed, &text, l);
        }
    } else if l < ed.vars.topline
        && l > ed.vars.topline - lines as i64
        && (ed.curses.has(curses::SR) || ed.curses.has(curses::AL))
    {
        if !ed.vars.mustredraw {
            ed.curses.mv(0, 0);
            while l < ed.vars.topline {
                ed.vars.topline -= 1;
                if ed.curses.has(curses::SR) {
                    ed.curses.put(curses::SR);
                } else {
                    ed.curses.insertln();
                }
                let top = ed.vars.topline;
                text = fetchline(ed, top);
                drawtext(ed, &text, false);
                ed.curses.put(curses::UP);
            }

            ed.curses.mv(lines - 1, 0);
            ed.curses.clrtoeol();
        } else {
            ed.vars.topline = l;
            ed.redraw.redrawafter = INFINITY;
            ed.redraw.preredraw = 0;
            ed.redraw.postredraw = 0;
        }
    } else if l > ed.vars.topline && l < bot(ed) + lines as i64 {
        let r = &ed.redraw;
        if !ed.vars.mustredraw
            || (r.redrawafter == r.preredraw && r.preredraw == bot(ed) && r.postredraw == l)
        {
            ed.curses.mv(lines - 1, 0);
            ed.curses.clrtoeol();
            while l > bot(ed) {
                ed.vars.topline += 1;
                let b = bot(ed);
                text = fetchline(ed, b);
                drawtext(ed, &text, false);
            }
            ed.vars.mustredraw = false;
        } else {
            ed.vars.topline = l - (lines as i64 - 2);
            ed.redraw.redrawafter = INFINITY;
            ed.redraw.preredraw = 0;
            ed.redraw.postredraw = 0;
        }
    } else {
        ed.vars.topline = l - (lines as i64 / 2) - 1;
        if ed.vars.topline < 1 {
            ed.vars.topline = 1;
        }
        ed.vars.mustredraw = true;
        ed.redraw.redrawafter = INFINITY;
        ed.redraw.preredraw = 0;
        ed.redraw.postredraw = 0;
    }

    if ed.vars.mustredraw {
        if ed.redraw.redrawafter == INFINITY {
            ed.redraw.redrawafter = 0;
            ed.redraw.preredraw = INFINITY;
            ed.redraw.postredraw = INFINITY;
        }

        {
            let r = &mut ed.redraw;
            if r.smartlno >= r.redrawafter {
                if r.smartlno < r.preredraw {
                    r.smartlno = 0;
                } else {
                    r.smartlno += r.postredraw - r.preredraw;
                }
            }
        }

        if ed.redraw.preredraw < ed.redraw.postredraw && ed.redraw.preredraw <= bot(ed) {
            if ed.redraw.preredraw < ed.vars.topline {
                l = ed.vars.topline;
            } else {
                l = ed.redraw.preredraw;
            }

            if l + ed.redraw.postredraw - ed.redraw.preredraw > bot(ed)
                || !ed.curses.has(curses::AL)
            {
                ed.redraw.preredraw = INFINITY;
                ed.redraw.postredraw = INFINITY;
            } else {
                ed.curses.mv((l - ed.vars.topline) as i32, 0);
                i = (ed.redraw.postredraw - ed.redraw.preredraw) as i32;
                while i > 0 {
                    ed.curses.insertln();
                    i -= 1;
                }

                ed.curses.mv(lines - 1, 0);
                ed.curses.clrtoeol();
            }
        }

        if ed.redraw.preredraw > ed.redraw.postredraw && ed.redraw.postredraw <= bot(ed) {
            if ed.redraw.preredraw > bot(ed) || !ed.curses.has(curses::DL) {
                ed.redraw.postredraw = INFINITY;
                ed.redraw.preredraw = INFINITY;
            } else {
                ed.curses.mv(lines - 1, 0);
                ed.curses.clrtoeol();

                ed.curses
                    .mv((ed.redraw.postredraw - ed.vars.topline) as i32, 0);
                l = ed.redraw.postredraw;
                while l < ed.redraw.preredraw && l <= bot(ed) {
                    ed.curses.deleteln();
                    l += 1;
                }

                i = (lines as i64 - 1 + (ed.redraw.postredraw - ed.redraw.preredraw)) as i32;
                ed.curses.mv(i, 0);
                l = ed.vars.topline + i as i64;
                while l <= bot(ed) {
                    ed.curses.clrtoeol();

                    if l <= ed.vars.nlines {
                        text = fetchline(ed, l);
                        drawtext(ed, &text, false);
                    } else {
                        ed.curses.addstr(b"~\n");
                    }
                    l += 1;
                }
            }
        }

        l = markline(curs);
        pfetch(ed, l);
        let ptext = ed.vars.ptext.clone();
        smartdrawtext(ed, &ptext, l);

        if ed.redraw.redrawafter < ed.vars.topline {
            l = ed.vars.topline;
        } else {
            l = ed.redraw.redrawafter;
        }
        ed.curses.mv((l - ed.vars.topline) as i32, 0);

        while l <= bot(ed) && l < ed.redraw.postredraw {
            if l == ed.redraw.smartlno {
                ed.curses.qaddch(b'\n');
                l += 1;
                continue;
            }

            if l <= ed.vars.nlines {
                text = fetchline(ed, l);
                drawtext(ed, &text, true);
            } else {
                ed.curses.qaddch(b'~');
                ed.curses.clrtoeol();
                ed.curses.addch(b'\n');
            }
            l += 1;
        }

        ed.vars.mustredraw = false;
    }

    ed.redraw.redrawafter = INFINITY;
    ed.redraw.preredraw = 0;
    ed.redraw.postredraw = 0;

    ed.curses
        .mv((markline(curs) - ed.vars.topline) as i32, ed.vars.physcol);
    ed.curses.wqrefresh();

    ed.redraw.redraw_chgs = ed.vars.changes;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redrawrange_combines_like_c() {
        let mut ed = Editor::new();
        redrawrange(&mut ed, 5, 6, 7);
        assert_eq!(
            (
                ed.redraw.redrawafter,
                ed.redraw.preredraw,
                ed.redraw.postredraw
            ),
            (5, 6, 7)
        );
        redrawrange(&mut ed, 5, 6, 8);
        assert_eq!(
            (
                ed.redraw.redrawafter,
                ed.redraw.preredraw,
                ed.redraw.postredraw
            ),
            (5, 5, 8)
        );
        redrawrange(&mut ed, 3, 4, 4);
        assert_eq!(
            (
                ed.redraw.redrawafter,
                ed.redraw.preredraw,
                ed.redraw.postredraw
            ),
            (3, INFINITY, INFINITY)
        );
    }

    #[test]
    fn idx2col_tabs_and_controls() {
        let mut ed = Editor::new();
        let line = b"\tab\x01c\xe9d";
        let m = |i: i64| 1024 + i;
        assert_eq!(idx2col(&mut ed, m(0), line, false), 7);
        assert_eq!(idx2col(&mut ed, m(0), line, true), 0);
        assert_eq!(idx2col(&mut ed, m(1), line, false), 8);
        assert_eq!(idx2col(&mut ed, m(4), line, false), 12);
        assert_eq!(idx2col(&mut ed, m(6), line, false), 14);
        // shortcut path agrees with a fresh count
        ed.vars.changes += 1;
        assert_eq!(idx2col(&mut ed, m(6), line, false), 14);
        ed.opts.o_list[0] = 1;
        ed.vars.changes += 1;
        assert_eq!(idx2col(&mut ed, m(1), line, false), 2);
    }

    #[test]
    fn drawtext_output() {
        let mut ed = Editor::new();
        ed.curses.cols = 80;
        drawtext(&mut ed, b"a\tb\x01", false);
        assert_eq!(ed.curses.kbuf, b"a       b^A\n\r");
        ed.curses.kbuf.clear();
        ed.vars.leftcol = 1;
        drawtext(&mut ed, b"\x01xy", false);
        assert_eq!(ed.curses.kbuf, b" xy\n\r");
    }
}
