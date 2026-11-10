//! `move4.c`: screen-relative motions and scrolling.  No module state.

use crate::opts::Opt;
use crate::redraw::redraw;
use crate::tio::{getkey, msg};
use crate::types::{
    MARK_FIRST, MARK_UNSET, Mark, R, When, botline, ctrl, default, mark_at_line, mark_last,
    markidx, markline,
};
use crate::vars::Editor;

const CTRL_B: i32 = ctrl(b'B') as i32;
const CTRL_D: i32 = ctrl(b'D') as i32;
const CTRL_E: i32 = ctrl(b'E') as i32;
const CTRL_F: i32 = ctrl(b'F') as i32;
const CTRL_U: i32 = ctrl(b'U') as i32;
const CTRL_Y: i32 = ctrl(b'Y') as i32;

// C: move4.c:m_row
pub fn m_row(ed: &mut Editor, _m: Mark, cnt: i64, key: i32) -> R<Mark> {
    let mut cnt = cnt;
    default(&mut cnt, 1);

    cnt -= 1;
    let topline = ed.vars.topline;
    let lines = ed.curses.lines;
    match u8::try_from(key) {
        Ok(b'H') => cnt += topline,
        Ok(b'M') => cnt = topline + ((lines - 1) / 2) as i64,
        Ok(b'L') => cnt = botline(topline, lines) - cnt,
        _ => {}
    }

    Ok(mark_at_line(cnt))
}

// C: move4.c:m_z
pub fn m_z(ed: &mut Editor, m: Mark, cnt: i64, key: i32) -> R<Mark> {
    let mut m = m;
    let mut key = key;
    let newtop;

    if cnt < 0 || cnt > ed.vars.nlines {
        return Ok(MARK_UNSET);
    }
    if cnt != 0 {
        m = mark_at_line(cnt);
        newtop = cnt;
    } else {
        newtop = markline(m);
    }

    // allow a "window size" number to be entered, but ignore it
    while key >= b'0' as i32 && key <= b'9' as i32 {
        key = getkey(ed, When::RAW)?;
    }

    let lines = ed.curses.lines as i64;
    let newtop = match u8::try_from(key) {
        Ok(b'\n' | b'\r' | b'+') => newtop,
        Ok(b'.' | b'z') => newtop - lines / 2,
        Ok(b'-') => newtop - (lines - 1),
        _ => return Ok(MARK_UNSET),
    };

    if newtop >= 1 {
        ed.vars.topline = newtop;
    } else {
        ed.vars.topline = 1;
    }
    ed.vars.mustredraw = true;

    Ok(m)
}

// C: move4.c:m_scroll
pub fn m_scroll(ed: &mut Editor, m: Mark, cnt: i64, key: i32) -> R<Mark> {
    let mut m = m;
    let mut cnt = cnt;
    let lines = ed.curses.lines as i64;

    match key {
        CTRL_F | CTRL_B => {
            default(&mut cnt, 1);
            ed.vars.mustredraw = true;
            // keeps one old line on screen
            cnt = cnt.saturating_mul(lines - 1) - 1;
        }
        CTRL_E | CTRL_Y => {
            default(&mut cnt, 1);
        }
        CTRL_U | CTRL_D => {
            if cnt == 0 {
                cnt = ed.opts.scroll();
            } else {
                if cnt > lines - 1 {
                    cnt = lines - 1;
                }
                ed.opts.set_num(Opt::Scroll, cnt as i32);
            }
        }
        _ => {}
    }

    match key {
        CTRL_B | CTRL_Y | CTRL_U => {
            cnt = ed.vars.topline.saturating_sub(cnt);
            if cnt < 1 {
                cnt = 1;
                m = MARK_FIRST;
            }
            let tmp = mark_at_line(cnt) + markidx(m) as Mark;
            redraw(ed, tmp, false);
            let bot = botline(ed.vars.topline, ed.curses.lines);
            if markline(m) > bot {
                m = mark_at_line(bot);
            }
        }
        CTRL_F | CTRL_E | CTRL_D => {
            let bot = botline(ed.vars.topline, ed.curses.lines);
            cnt = bot.saturating_add(cnt);
            if cnt > ed.vars.nlines {
                cnt = ed.vars.nlines;
                m = mark_last(ed.vars.nlines);
            }
            let tmp = mark_at_line(cnt) + markidx(m) as Mark;
            redraw(ed, tmp, false);
            if markline(m) < ed.vars.topline {
                m = mark_at_line(ed.vars.topline);
            }
        }
        _ => {}
    }

    if key == CTRL_B || key == CTRL_F {
        ed.vars.changes += 1;

        // the paging commands must clear the status line explicitly
        msg(ed, b"", &[]);
    }

    Ok(m)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn row() {
        let mut ed = Editor::new();
        ed.vars.topline = 10;
        ed.curses.lines = 24;
        assert_eq!(m_row(&mut ed, 0, 0, b'H' as i32).unwrap(), mark_at_line(10));
        assert_eq!(m_row(&mut ed, 0, 3, b'H' as i32).unwrap(), mark_at_line(12));
        assert_eq!(m_row(&mut ed, 0, 0, b'M' as i32).unwrap(), mark_at_line(21));
        assert_eq!(m_row(&mut ed, 0, 2, b'L' as i32).unwrap(), mark_at_line(31));
    }

    #[test]
    fn z_positions() {
        let mut ed = Editor::new();
        ed.vars.nlines = 100;
        ed.curses.lines = 24;
        let m = mark_at_line(50) + 3;
        assert_eq!(m_z(&mut ed, m, 0, b'\n' as i32).unwrap(), m);
        assert_eq!(ed.vars.topline, 50);
        assert_eq!(m_z(&mut ed, m, 30, b'.' as i32).unwrap(), mark_at_line(30));
        assert_eq!(ed.vars.topline, 18);
        assert_eq!(m_z(&mut ed, m, 5, b'-' as i32).unwrap(), mark_at_line(5));
        assert_eq!(ed.vars.topline, 1);
        assert_eq!(m_z(&mut ed, m, 101, b'\n' as i32).unwrap(), MARK_UNSET);
        assert_eq!(m_z(&mut ed, m, 0, b'x' as i32).unwrap(), MARK_UNSET);
    }
}
