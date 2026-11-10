//! `vcmd.c`: visual-mode commands.

use crate::blk::{afterdo, beforedo, undo};
use crate::cmd1::{cmd_edit, cmd_file, cmd_tag, cmd_xit};
use crate::cmd2::{cmd_join, cmd_shift, cmd_substitute};
use crate::curses::{resume_curses, suspend_curses};
use crate::cut::{cut, cutname, paste};
use crate::ex::doexcmd;
use crate::input::input;
use crate::misc::pfetch;
use crate::modify::{add, change, delete};
use crate::move1::m_front;
use crate::move2::atol;
use crate::os::take_sigint;
use crate::redraw::redraw;
use crate::system::{elvis_system, filter};
use crate::tio::{Arg, exrefresh, getkey, msg, sprintf, vgets};
use crate::vars::Editor;
use crate::vi_h::{
    BLKSIZE, CMD_EDIT, CMD_FILE, CMD_JOIN, CMD_SHIFTL, CMD_SHIFTR, CMD_SUBAGAIN, CMD_TAG, CMD_XIT,
    Cmd, Interrupted, MARK_UNSET, MODE_COLON, MODE_EX, MODE_VI, Mark, R, WHEN_VIINP, WHEN_VIREP,
    ch, cstr, default, mark_at_line, markidx, markline,
};

#[derive(Default)]
pub struct Vcmd {
    pub v_overtype_width: i64,
    pub v_increment_sign: i32,
}

const LINEMASK: Mark = !(BLKSIZE as Mark - 1);

/// `static char flip[]` in `v_ulcase`.
const V_ULCASE_FLIP: &[u8] = b"aAbBcCdDeEfFgGhHiIjJkKlLmMnNoOpPqQrRsStTuUvVwWxXyYzZ[](){}<>";

// C: vcmd.c:v_quit
pub fn v_quit(ed: &mut Editor) -> R<Mark> {
    let lines = ed.curses.lines;
    ed.curses.mv(lines - 1, 0);
    ed.vars.mode = MODE_EX;
    Ok(ed.vars.cursor)
}

// C: vcmd.c:v_redraw
pub fn v_redraw(ed: &mut Editor) -> R<Mark> {
    redraw(ed, MARK_UNSET, false);
    Ok(ed.vars.cursor)
}

// C: vcmd.c:v_1ex
pub fn v_1ex(ed: &mut Editor, _m: Mark, text: &[u8]) -> R<Mark> {
    ed.vars.exwrote = ed.vars.mode == MODE_COLON;
    doexcmd(ed, text)?;
    exrefresh(ed);

    if ed.vars.mode != MODE_VI && ed.vars.mode != MODE_COLON {
        return Ok(ed.vars.cursor);
    }

    if ed.vars.exwrote {
        ed.vars.mode = MODE_VI;
        msg(ed, b"[Hit <RETURN> to continue]", &[]);
        if getkey(ed, 0)? == b':' as i32 {
            ed.vars.mode = MODE_COLON;
            ed.curses.addch(b'\n');
        } else {
            redraw(ed, MARK_UNSET, false);
        }
    }

    Ok(ed.vars.cursor)
}

// C: vcmd.c:v_undo
pub fn v_undo(ed: &mut Editor, _m: Mark) -> R<Mark> {
    if undo(ed) {
        redraw(ed, MARK_UNSET, false);
    }
    Ok(ed.vars.cursor)
}

// C: vcmd.c:v_xchar
pub fn v_xchar(ed: &mut Editor, m: Mark, cnt: i64, cmd: i32) -> R<Mark> {
    let mut m = m;
    let mut cnt = cnt;
    default(&mut cnt, 1);

    if cmd == b'X' as i32 {
        if (markidx(m) as i64) < cnt {
            return Ok(MARK_UNSET);
        }
        m -= cnt;
    }

    pfetch(ed, markline(m));
    if markidx(m.saturating_add(cnt)) as i64 > ed.vars.plen as i64 {
        cnt = ed.vars.plen as i64 - markidx(m) as i64;
    }
    if cnt == 0 {
        return Ok(MARK_UNSET);
    }

    beforedo(ed, false);
    cut(ed, m, m + cnt);
    delete(ed, m, m + cnt);
    afterdo(ed);
    Ok(m)
}

// C: vcmd.c:v_mark
pub fn v_mark(ed: &mut Editor, m: Mark, _count: i64, key: i32) -> R<Mark> {
    if key < b'a' as i32 || key > b'z' as i32 {
        msg(ed, b"Marks must be from a to z", &[]);
    } else {
        ed.vars.mark[(key - b'a' as i32) as usize] = m;
    }
    Ok(m)
}

// C: vcmd.c:v_ulcase
pub fn v_ulcase(ed: &mut Editor, m: Mark, cnt: i64) -> R<Mark> {
    let mut cnt = cnt;
    default(&mut cnt, 1);

    pfetch(ed, markline(m));

    let flip = V_ULCASE_FLIP;
    let mut newtext: Vec<u8> = Vec::new();
    let mut j: i64 = 0;
    let mut i = markidx(m);
    while j < cnt && ch(&ed.vars.ptext, i) != 0 {
        let c = ed.vars.ptext[i];
        let pos = flip.iter().position(|&f| f == c);
        let mut n = match pos {
            Some(p) => flip[p ^ 1],
            None => 0,
        };
        if n == 0 {
            n = c;
        }
        newtext.push(n);
        j += 1;
        i += 1;
    }

    let start = markidx(m);
    if j > 0 && newtext[..] != ed.vars.ptext[start..start + newtext.len()] {
        beforedo(ed, false);
        change(ed, m, m + j, &newtext);
        afterdo(ed);
    }

    Ok(m + j)
}

// C: vcmd.c:v_replace
pub fn v_replace(ed: &mut Editor, m: Mark, cnt: i64, key: i32) -> R<Mark> {
    let mut cnt = cnt;
    let mut key = key;
    default(&mut cnt, 1);

    if key == b'\r' as i32 {
        key = b'\n' as i32;
    }

    // make sure the resulting line isn't too long
    let room = BLKSIZE as i64 - 2 - markidx(m) as i64;
    if cnt > room {
        cnt = room;
    }

    let text: Vec<u8> = vec![key as u8; cnt.max(0) as usize];

    pfetch(ed, markline(m));
    let idx = markidx(m) as i64;
    if idx + cnt > ed.vars.plen as i64 {
        cnt = ed.vars.plen as i64 - idx;
    }

    beforedo(ed, false);
    change(ed, m, m + cnt, &text);
    afterdo(ed);

    if ch(&text, 0) == b'\n' {
        Ok((m & LINEMASK) + cnt * BLKSIZE as i64)
    } else {
        Ok(m + cnt - 1)
    }
}

// C: vcmd.c:v_overtype
pub fn v_overtype(ed: &mut Editor, m: Mark) -> R<Mark> {
    if ed.vars.doingdot {
        let width = ed.vcmd.v_overtype_width;
        if width < 0 {
            msg(ed, b"Can't repeat a multi-line overtype command", &[]);
            return Ok(MARK_UNSET);
        }

        if width == 0 {
            return Ok(m);
        }

        return v_subst(ed, m, width);
    }

    beforedo(ed, false);
    let end = input(ed, m, m, WHEN_VIREP)?;
    afterdo(ed);

    // only a single-line overtype can be repeated with the dot key
    if markline(end) == markline(m) && end >= m - 1 {
        ed.vcmd.v_overtype_width = end - m + 1;
    } else {
        ed.vcmd.v_overtype_width = -1;
    }

    Ok(end)
}

// C: vcmd.c:v_selcut
pub fn v_selcut(ed: &mut Editor, m: Mark, _cnt: i64, key: i32) -> R<Mark> {
    cutname(ed, key);
    Ok(m)
}

// C: vcmd.c:v_paste
pub fn v_paste(ed: &mut Editor, m: Mark, _cnt: i64, cmd: i32) -> R<Mark> {
    beforedo(ed, false);
    let m = paste(ed, m, cmd == b'p' as i32, false);
    afterdo(ed);
    Ok(m)
}

// C: vcmd.c:v_yank
pub fn v_yank(ed: &mut Editor, m: Mark, n: Mark) -> R<Mark> {
    cut(ed, m, n);
    Ok(m)
}

// C: vcmd.c:v_delete
pub fn v_delete(ed: &mut Editor, m: Mark, n: Mark) -> R<Mark> {
    if n <= m {
        return Ok(MARK_UNSET);
    }

    beforedo(ed, false);
    cut(ed, m, n);
    delete(ed, m, n);
    afterdo(ed);
    Ok(m)
}

// C: vcmd.c:v_insert
pub fn v_insert(ed: &mut Editor, m: Mark, cnt: i64, key: i32) -> R<Mark> {
    let mut m = m;
    let mut cnt = cnt;
    let mut after = false;
    default(&mut cnt, 1);

    beforedo(ed, false);
    match u8::try_from(key) {
        Ok(b'i') => {
            after = false;
        }
        Ok(b'a') => {
            pfetch(ed, markline(m));
            if ed.vars.plen > 0 {
                m += 1;
            }
            after = true;
        }
        Ok(b'I') => {
            m = m_front(ed, m, 1)?;
            after = false;
        }
        Ok(b'A') => {
            pfetch(ed, markline(m));
            m = (m & LINEMASK) + ed.vars.plen as Mark;
            after = true;
        }
        Ok(b'O') => {
            m &= LINEMASK;
            add(ed, m, b"\n");
            after = false;
        }
        Ok(b'o') => {
            m = (m & LINEMASK) + BLKSIZE as Mark;
            add(ed, m, b"\n");
            after = false;
        }
        _ => {}
    }

    let wasdot = ed.vars.doingdot;
    let mut reps = cnt;
    while reps > 0 {
        if take_sigint() {
            return Err(Interrupted);
        }
        m = input(ed, m, m, WHEN_VIINP)?;
        if after {
            m += 1;
        }
        reps -= 1;
        ed.vars.doingdot = true;
    }
    if after {
        m -= 1;
    }

    ed.vars.doingdot = wasdot;
    afterdo(ed);

    Ok(m)
}

// C: vcmd.c:v_change
pub fn v_change(ed: &mut Editor, m: Mark, n: Mark) -> R<Mark> {
    let (mut m, mut n) = (m, n);
    if m > n {
        std::mem::swap(&mut m, &mut n);
    }

    // for line mode, retain the last newline char
    let lnmode = markidx(m) == 0 && markidx(n) == 0 && m != n;
    if lnmode {
        n -= BLKSIZE as Mark;
        pfetch(ed, markline(n));
        n = (n & LINEMASK) + ed.vars.plen as Mark;
    }

    beforedo(ed, false);
    cut(ed, m, n);
    let m = input(ed, m, n, WHEN_VIINP)?;
    afterdo(ed);

    Ok(m)
}

// C: vcmd.c:v_subst
pub fn v_subst(ed: &mut Editor, m: Mark, cnt: i64) -> R<Mark> {
    let mut cnt = cnt;
    default(&mut cnt, 1);

    pfetch(ed, markline(m));
    if markidx(m) as i64 + cnt > ed.vars.plen as i64 {
        cnt = ed.vars.plen as i64 - markidx(m) as i64;
    }

    beforedo(ed, false);
    cut(ed, m, m + cnt);
    let m = input(ed, m, m + cnt, WHEN_VIINP)?;
    afterdo(ed);
    Ok(m)
}

// C: vcmd.c:v_join
pub fn v_join(ed: &mut Editor, m: Mark, cnt: i64) -> R<Mark> {
    let mut cnt = cnt;
    default(&mut cnt, 1);

    pfetch(ed, markline(m));
    let joint = (m & LINEMASK) + ed.vars.plen as Mark;

    cmd_join(
        ed,
        m,
        m.saturating_add(mark_at_line(cnt.min(i64::MAX / BLKSIZE as i64))),
        CMD_JOIN,
        false,
        b"",
    )?;
    ed.vars.mustredraw = true;

    Ok(joint)
}

// C: vcmd.c:shift_help
fn shift_help(ed: &mut Editor, m: Mark, n: Mark, excmd: Cmd) -> R<Mark> {
    // adjust for inclusive endmarks in ex
    let n = n - BLKSIZE as Mark;

    cmd_shift(ed, m, n, excmd, false, b"")?;
    Ok(m)
}

// C: vcmd.c:v_lshift
pub fn v_lshift(ed: &mut Editor, m: Mark, n: Mark) -> R<Mark> {
    shift_help(ed, m, n, CMD_SHIFTL)
}

// C: vcmd.c:v_rshift
pub fn v_rshift(ed: &mut Editor, m: Mark, n: Mark) -> R<Mark> {
    shift_help(ed, m, n, CMD_SHIFTR)
}

// C: vcmd.c:v_filter
pub fn v_filter(ed: &mut Editor, m: Mark, n: Mark) -> R<Mark> {
    let mut cmdln: Vec<u8> = Vec::new();

    // adjust for inclusive endmarks in ex
    let n = n - BLKSIZE as Mark;

    if vgets(ed, b'!', &mut cmdln, 100)? > 0 {
        filter(ed, m, n, cstr(&cmdln))?;
    }

    redraw(ed, MARK_UNSET, false);
    Ok(m)
}

// C: vcmd.c:v_status
pub fn v_status(ed: &mut Editor) -> R<Mark> {
    let cursor = ed.vars.cursor;
    cmd_file(ed, cursor, cursor, CMD_FILE, false, b"")?;
    Ok(ed.vars.cursor)
}

// C: vcmd.c:v_again
pub fn v_again(ed: &mut Editor, m: Mark, n: Mark) -> R<Mark> {
    cmd_substitute(ed, m, n - BLKSIZE as Mark, CMD_SUBAGAIN, true, b"")?;
    Ok(ed.vars.cursor)
}

// C: vcmd.c:v_switch
pub fn v_switch(ed: &mut Editor) -> R<Mark> {
    if ch(&ed.vars.prevorig, 0) == 0 {
        msg(ed, b"No previous file", &[]);
    } else {
        let name = cstr(&ed.vars.prevorig).to_vec();
        let cursor = ed.vars.cursor;
        cmd_edit(ed, cursor, cursor, CMD_EDIT, false, &name)?;
    }
    Ok(ed.vars.cursor)
}

// C: vcmd.c:v_tag
pub fn v_tag(ed: &mut Editor, keyword: &[u8], m: Mark, _cnt: i64) -> R<Mark> {
    ed.vars.cursor = m;

    let cursor = ed.vars.cursor;
    cmd_tag(ed, cursor, cursor, CMD_TAG, false, keyword)?;

    Ok(ed.vars.cursor)
}

// C: vcmd.c:v_keyword
pub fn v_keyword(ed: &mut Editor, keyword: &[u8], m: Mark, _cnt: i64) -> R<Mark> {
    let lines = ed.curses.lines;
    ed.curses.mv(lines - 1, 0);
    ed.curses
        .addstr(b"---------------------------------------------------------\n");
    ed.curses.clrtoeol();
    ed.curses.refresh();
    let prg = cstr(&ed.opts.o_keywordprg).to_vec();
    let mut cmdline = sprintf(b"%s %s", &[Arg::S(&prg), Arg::S(keyword)]);
    // C wrote into char cmdline[130] without a bound.
    cmdline.truncate(129);
    let waswarn = ed.opts.o_warn[0];
    ed.opts.o_warn[0] = 0;
    suspend_curses(ed);
    if elvis_system(ed, Some(&cmdline)) != 0 {
        ed.curses.addstr(b"<<< failed >>>\n");
    }
    resume_curses(ed, false);
    ed.vars.mode = MODE_VI;
    redraw(ed, MARK_UNSET, false);
    ed.opts.o_warn[0] = waswarn;

    Ok(m)
}

// C: vcmd.c:v_increment
pub fn v_increment(ed: &mut Editor, keyword: &[u8], m: Mark, cnt: i64) -> R<Mark> {
    let mut cnt = cnt;
    default(&mut cnt, 1);

    if !ed.vars.doingdot {
        ed.vcmd.v_increment_sign = getkey(ed, 0)?;
    }

    let sign = ed.vcmd.v_increment_sign;
    let newcnt = if sign == b'+' as i32 || sign == b'#' as i32 {
        atol(keyword).checked_add(cnt)
    } else if sign == b'-' as i32 {
        atol(keyword).checked_sub(cnt)
    } else if sign == b'=' as i32 {
        Some(cnt)
    } else {
        return Ok(MARK_UNSET);
    };
    // C overflows a long here (undefined); treat it as a failed command.
    let Some(cnt) = newcnt else {
        return Ok(MARK_UNSET);
    };
    let newval = sprintf(b"%ld", &[Arg::D(cnt)]);

    beforedo(ed, false);
    change(ed, m, m + keyword.len() as Mark, &newval);
    afterdo(ed);

    Ok(m)
}

// C: vcmd.c:v_xit
pub fn v_xit(ed: &mut Editor, m: Mark, _cnt: i64, key: i32) -> R<Mark> {
    if key != b'Z' as i32 {
        return Ok(MARK_UNSET);
    }

    let lines = ed.curses.lines;
    ed.curses.mv(lines - 1, 0);
    ed.curses.clrtoeol();

    cmd_xit(ed, m, m, CMD_XIT, false, b"")?;

    Ok(m)
}

// C: vcmd.c:v_undoline
pub fn v_undoline(ed: &mut Editor, m: Mark) -> R<Mark> {
    if markline(m) != ed.vars.u_line {
        return Ok(MARK_UNSET);
    }

    beforedo(ed, false);
    ed.vars.u_text.extend_from_slice(b"\n");
    let text = ed.vars.u_text.clone();
    let u_line = ed.vars.u_line;
    change(ed, mark_at_line(u_line), mark_at_line(u_line + 1), &text);
    afterdo(ed);

    ed.vars.u_line = -1;

    Ok(m & LINEMASK)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flip_table_pairs() {
        assert_eq!(V_ULCASE_FLIP.len() % 2, 0);
        for p in V_ULCASE_FLIP.chunks(2) {
            let a = V_ULCASE_FLIP.iter().position(|&f| f == p[0]).unwrap();
            assert_eq!(V_ULCASE_FLIP[a ^ 1], p[1]);
        }
    }

    #[test]
    fn mark_and_xit_reject_bad_keys() {
        let mut ed = Editor::new();
        assert_eq!(v_mark(&mut ed, 5000, 0, b'c' as i32).unwrap(), 5000);
        assert_eq!(ed.vars.mark[2], 5000);
        assert_eq!(v_xit(&mut ed, 5000, 0, b'Q' as i32).unwrap(), MARK_UNSET);
        assert_eq!(v_undoline(&mut ed, mark_at_line(3)).unwrap(), MARK_UNSET);
    }

    #[test]
    fn overtype_dot_width() {
        let mut ed = Editor::new();
        ed.vars.doingdot = true;
        ed.vcmd.v_overtype_width = 0;
        assert_eq!(v_overtype(&mut ed, 4096).unwrap(), 4096);
    }
}
