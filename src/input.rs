//! `input.c`: text input mode.  No module state.

use crate::blk::{abortdo, afterdo, beforedo, blksync};
use crate::cmd2::cmd_shift;
use crate::cut::{cut, cutname, paste};
use crate::misc::pfetch;
use crate::modify::{add, change, delete};
use crate::move1::{m_front, m_left, m_match, m_rear, m_updnto};
use crate::move4::m_scroll;
use crate::move5::{m_bword, m_fword};
use crate::opts::Opt;
use crate::os;
use crate::redraw::{idx2col, redraw};
use crate::tio::{beep, getkey};
use crate::vars::Editor;
use crate::vcmd::{v_xchar, v_xit};
use crate::vi::adjmove;
use crate::vi_h::LINEMASK;
use crate::vi_h::{
    BLKSIZE, CMD_SHIFTL, CMD_SHIFTR, MARK_UNSET, Mark, R, WHEN_VIREP, botline, ctrl, markidx,
    markline,
};

/// `key` as the C string `key[2]`: empty when the key is NUL.
fn keystr(key: &[u8; 1]) -> &[u8] {
    if key[0] == 0 { &[] } else { &key[..] }
}

// C: input.c:input
pub fn input(ed: &mut Editor, from: Mark, to: Mark, when: i32) -> R<Mark> {
    let mut from = from;
    let mut to = to;
    let mut key: [u8; 1] = [0];
    let mut m: Mark;
    let mut quit = false;

    if from != to {
        cut(ed, from, to);
    }

    beforedo(ed, false);
    if ed.vars.doingdot {
        if from != to {
            delete(ed, from, to);
        }

        cutname(ed, b'.' as i32);
        ed.vars.cursor = paste(ed, from, false, true) + 1;
    } else {
        if from != to && markline(from) == markline(to) {
            change(ed, to - 1, to, b"$");
        } else {
            if from != to {
                delete(ed, from, to);
            }
            to = from;
        }

        ed.vars.cursor = from;
        if ed.opts.on(Opt::Autoindent)
            && markline(ed.vars.cursor) > 1
            && markidx(ed.vars.cursor) == 0
        {
            pfetch(ed, markline(ed.vars.cursor));
            if ed.vars.plen == 0 {
                pfetch(ed, markline(ed.vars.cursor) - 1);
                let build: Vec<u8> = ed
                    .vars
                    .ptext
                    .iter()
                    .copied()
                    .take_while(|&c| c == b' ' || c == b'\t')
                    .collect();
                if !build.is_empty() {
                    let c = ed.vars.cursor;
                    add(ed, c, &build);
                    ed.vars.cursor += build.len() as Mark;
                }
            }
        }

        loop {
            let c = ed.vars.cursor;
            redraw(ed, c, true);
            key[0] = getkey(ed, when)? as u8;

            if (key[0] == b' ' || key[0] == b'\t') && ed.opts.num(Opt::Wrapmargin) != 0 {
                pfetch(ed, markline(ed.vars.cursor));
                let ptext = ed.vars.ptext.clone();
                let c = ed.vars.cursor;
                if idx2col(ed, c, &ptext, true) > ed.curses.cols - ed.opts.num(Opt::Wrapmargin) {
                    key[0] = b'\n';
                }
            }

            let cursor = ed.vars.cursor;
            match key[0] {
                0 => {
                    key[0] = getkey(ed, 0)? as u8;
                    m = match key[0] {
                        b'h' => m_left(ed, cursor, 0)?,
                        b'j' | b'k' => m_updnto(ed, cursor, 0, key[0] as i8 as i32)?,
                        b'l' => cursor + 1,
                        b'b' => m_bword(ed, cursor, 0, b'b' as i32)?,
                        b'w' => m_fword(ed, cursor, 0, b'w' as i32)?,
                        b'^' => m_front(ed, cursor)?,
                        b'$' => m_rear(ed, cursor)?,
                        k if k == ctrl(b'B') || k == ctrl(b'F') => {
                            m_scroll(ed, cursor, 0, k as i32)?
                        }
                        b'x' => v_xchar(ed, cursor, 0, b'x' as i32)?,
                        b'i' => {
                            from = cursor;
                            to = cursor;
                            cursor
                        }
                        _ => MARK_UNSET,
                    };
                    let c = ed.vars.cursor;
                    m = adjmove(
                        ed,
                        c,
                        m,
                        if key[0] == b'j' || key[0] == b'k' {
                            0x20
                        } else {
                            0
                        },
                    );
                    if key[0] == b'$' || (key[0] == b'l' && m <= ed.vars.cursor) {
                        m += 1;
                    }
                    if m == MARK_UNSET {
                        beep(ed);
                    } else {
                        if to > ed.vars.cursor {
                            let c = ed.vars.cursor;
                            delete(ed, c, to);
                            redraw(ed, c, true);
                        }
                        from = m;
                        to = m;
                        ed.vars.cursor = m;
                    }
                }

                k if k == ctrl(b'Z') => {
                    if getkey(ed, 0)? == ctrl(b'Z') as i32 {
                        quit = true;
                        break;
                    }
                }

                k if k == ctrl(b'[') => break,

                k if k == ctrl(b'U') => {
                    if markline(cursor) == markline(from) {
                        ed.vars.cursor = from;
                    } else {
                        ed.vars.cursor &= LINEMASK;
                    }
                }

                k if k == ctrl(b'D') || k == ctrl(b'T') => {
                    if to > cursor {
                        delete(ed, cursor, to);
                    }
                    ed.vars.mark[27] = cursor;
                    let cmd = if k == ctrl(b'D') {
                        CMD_SHIFTL
                    } else {
                        CMD_SHIFTR
                    };
                    cmd_shift(ed, cursor, cursor, cmd, true, b"")?;
                    if ed.vars.mark[27] != 0 {
                        ed.vars.cursor = ed.vars.mark[27];
                    } else {
                        ed.vars.cursor = m_front(ed, cursor)?;
                    }
                    to = ed.vars.cursor;
                }

                b'\x08' => {
                    if cursor <= from {
                        beep(ed);
                    } else if markidx(cursor) == 0 {
                        ed.vars.cursor -= BLKSIZE as Mark;
                        pfetch(ed, markline(ed.vars.cursor));
                        ed.vars.cursor += ed.vars.plen as Mark;
                    } else {
                        ed.vars.cursor -= 1;
                    }
                }

                k if k == ctrl(b'W') => {
                    m = m_bword(ed, cursor, 1, b'b' as i32)?;
                    if markline(m) == markline(cursor) && m >= from {
                        ed.vars.cursor = m;
                        if from > m {
                            from = m;
                        }
                    } else {
                        beep(ed);
                    }
                }

                b'\n' | b'\r' => {
                    let mut build: Vec<u8> = vec![b'\n'];
                    if ed.opts.on(Opt::Autoindent) {
                        pfetch(ed, markline(ed.vars.cursor));
                        let indent = ed
                            .vars
                            .ptext
                            .iter()
                            .take_while(|&&c| c == b' ' || c == b'\t')
                            .count();
                        build.extend_from_slice(&ed.vars.ptext[..indent]);

                        if indent == ed.vars.ptext.len() && ed.vars.plen > 0 {
                            ed.vars.cursor &= LINEMASK;
                            to = ed.vars.cursor;
                            let c = ed.vars.cursor;
                            let plen = ed.vars.plen as Mark;
                            delete(ed, c, c + plen);
                        }
                    }
                    let c = ed.vars.cursor;
                    if c >= to && when != WHEN_VIREP {
                        add(ed, c, &build);
                    } else {
                        change(ed, c, to, &build);
                    }
                    redraw(ed, c, true);
                    ed.vars.cursor = (c & LINEMASK) + BLKSIZE as Mark + build.len() as Mark - 1;
                    to = ed.vars.cursor;
                }

                k if k == ctrl(b'A') || k == ctrl(b'P') => {
                    if cursor < to {
                        delete(ed, cursor, to);
                    }
                    if k == ctrl(b'A') {
                        cutname(ed, b'.' as i32);
                    }
                    ed.vars.cursor = paste(ed, cursor, false, true) + 1;
                    to = ed.vars.cursor;
                }

                k if k == ctrl(b'V') => {
                    if cursor >= to && when != WHEN_VIREP {
                        add(ed, cursor, b"^");
                    } else {
                        change(ed, cursor, to, b"^");
                        to = cursor + 1;
                    }
                    redraw(ed, cursor, true);
                    key[0] = getkey(ed, 0)? as u8;
                    if key[0] == b'\n' {
                        key[0] = b'\r';
                    }
                    change(ed, cursor, cursor + 1, keystr(&key));
                    ed.vars.cursor += 1;
                    if ed.vars.cursor > to {
                        to = ed.vars.cursor;
                    }
                }

                k if k == ctrl(b'L') || k == ctrl(b'R') => {
                    redraw(ed, MARK_UNSET, false);
                }

                _ => {
                    if cursor >= to && when != WHEN_VIREP {
                        if add(ed, cursor, keystr(&key)) {
                            ed.vars.cursor += 1;
                        }
                        to = ed.vars.cursor;
                    } else {
                        pfetch(ed, markline(cursor));
                        if markidx(cursor) as i32 == ed.vars.plen && !add(ed, cursor, keystr(&key))
                        {
                            ed.vars.cursor -= 1;
                        } else if markidx(cursor) as i32 != ed.vars.plen {
                            change(ed, cursor, cursor + 1, keystr(&key));
                        }
                        ed.vars.cursor += 1;
                    }
                    if ed.opts.on(Opt::Showmatch) && b")}]".contains(&key[0]) {
                        let c = ed.vars.cursor;
                        redraw(ed, c, true);
                        m = m_match(ed, c - 1)?;
                        if markline(m) >= ed.vars.topline
                            && markline(m) <= botline(ed.vars.topline, ed.curses.lines)
                        {
                            redraw(ed, m, true);
                            ed.curses.refresh();
                            os::sleep_secs(1);
                        }
                    }
                }
            }
        }

        if ed.vars.cursor < to {
            let c = ed.vars.cursor;
            delete(ed, c, to);
        }
    }
    afterdo(ed);

    if !ed.vars.doingdot {
        blksync(ed);
        cutname(ed, b'.' as i32);
        let c = ed.vars.cursor;
        cut(ed, from, c);
    }

    if markidx(ed.vars.cursor) != 0 {
        ed.vars.cursor -= 1;
    }
    let c = ed.vars.cursor;
    redraw(ed, c, false);

    if quit {
        abortdo(ed);

        let c = ed.vars.cursor;
        ed.vars.cursor = v_xit(ed, c, 0, b'Z' as i32)?;
    }

    ed.vars.rptlines = 0;
    Ok(ed.vars.cursor)
}
