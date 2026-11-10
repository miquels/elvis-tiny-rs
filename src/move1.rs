//! `move1.c`: basic cursor motions.  No module state.

use crate::misc::{fetchline, pfetch};
use crate::tio::{Arg, msg};
use crate::vars::Editor;
use crate::vi_h::LINEMASK;
use crate::vi_h::{
    BLKSIZE, MARK_FIRST, MARK_UNSET, Mark, R, buildmark, ch, default, mark_at_line, mark_last,
    markidx, markline,
};

// C: move1.c:m_updnto
pub fn m_updnto(ed: &mut Editor, m: Mark, cnt: i64, cmd: i32) -> R<Mark> {
    let mut m = m;
    let mut cnt = cnt;
    let nlines = ed.vars.nlines;
    default(&mut cnt, if cmd == b'G' as i32 { nlines } else { 1 });

    // A count too large for a MARK cannot land on a real line.
    let Some(delta) = cnt.checked_mul(BLKSIZE as i64) else {
        if cmd == b'G' as i32 {
            msg(ed, b"Only %ld lines", &[Arg::D(nlines)]);
        }
        return Ok(MARK_UNSET);
    };

    match cmd {
        c if c == (b'P' & 0x1f) as i32 || c == b'-' as i32 || c == b'k' as i32 => {
            m -= delta;
        }
        c if c == b'G' as i32 => {
            if cnt < 1 || cnt > nlines {
                msg(ed, b"Only %ld lines", &[Arg::D(nlines)]);
                return Ok(MARK_UNSET);
            }
            m = delta;
        }
        _ => {
            m = m.saturating_add(delta);
        }
    }

    if m < MARK_FIRST || markline(m) > nlines {
        return Ok(MARK_UNSET);
    }

    Ok(m)
}

// C: move1.c:m_right
pub fn m_right(ed: &mut Editor, m: Mark, cnt: i64) -> R<Mark> {
    let mut cnt = cnt;
    default(&mut cnt, 1);

    pfetch(ed, markline(m));
    let idx = markidx(m) as i64 + cnt;
    if idx < ed.vars.plen as i64 {
        Ok(m + cnt)
    } else {
        Ok(MARK_UNSET)
    }
}

// C: move1.c:m_left
pub fn m_left(_ed: &mut Editor, m: Mark, cnt: i64) -> R<Mark> {
    let mut cnt = cnt;
    default(&mut cnt, 1);

    if markidx(m) as i64 >= cnt {
        Ok(m - cnt)
    } else {
        Ok(MARK_UNSET)
    }
}

// C: move1.c:m_tocol
pub fn m_tocol(ed: &mut Editor, m: Mark, cnt: i64) -> R<Mark> {
    let mut cnt = cnt;
    default(&mut cnt, 1);

    cnt -= 1;

    if cnt == 0 {
        return Ok(m & LINEMASK);
    }

    pfetch(ed, markline(m));
    let tabstop = ed.opts.o_tabstop[0] as i64;
    let list = ed.opts.o_list[0] != 0;
    let text = &ed.vars.ptext;
    let mut col: i64 = 0;
    let mut idx: usize = 0;
    while col < cnt && ch(text, idx) != 0 {
        let c = text[idx];
        if c == b'\t' && !list {
            col += tabstop;
            col -= col % tabstop;
        } else if c < b' ' || c == 0o177 {
            col += 2;
        } else {
            col += 1;
        }
        idx += 1;
    }
    if ch(text, idx) == 0 {
        Ok(MARK_UNSET)
    } else {
        Ok((m & LINEMASK) + idx as Mark)
    }
}

// C: move1.c:m_front
pub fn m_front(ed: &mut Editor, m: Mark, _cnt: i64) -> R<Mark> {
    let mut m = m;
    pfetch(ed, markline(m));
    m &= LINEMASK;
    let mut scan = 0;
    while matches!(ch(&ed.vars.ptext, scan), b' ' | b'\t') {
        scan += 1;
        m += 1;
    }

    Ok(m)
}

// C: move1.c:m_rear
pub fn m_rear(_ed: &mut Editor, m: Mark, _cnt: i64) -> R<Mark> {
    // Deliberately past the end of the line; adjmove() in vi.rs fixes it.
    Ok(m | (BLKSIZE as Mark - 1))
}

// C: move1.c:m_fparagraph
pub fn m_fparagraph(ed: &mut Editor, m: Mark, cnt: i64) -> R<Mark> {
    let mut cnt = cnt;
    default(&mut cnt, 1);

    let nlines = ed.vars.nlines;
    let mut l = markline(m);
    while cnt > 0 && {
        let c = l < nlines;
        l += 1;
        c
    } {
        let text = fetchline(ed, l);
        if ch(&text, 0) == 0 {
            cnt -= 1;
        }
    }
    if l <= nlines {
        Ok(mark_at_line(l))
    } else {
        Ok(mark_last(nlines))
    }
}

// C: move1.c:m_bparagraph
pub fn m_bparagraph(ed: &mut Editor, m: Mark, cnt: i64) -> R<Mark> {
    let mut cnt = cnt;
    default(&mut cnt, 1);

    let mut l = markline(m);
    while cnt > 0 && {
        let c = l > 1;
        l -= 1;
        c
    } {
        let text = fetchline(ed, l);
        if ch(&text, 0) == 0 {
            cnt -= 1;
        }
    }
    if l >= 1 {
        Ok(mark_at_line(l))
    } else {
        Ok(MARK_FIRST)
    }
}

// C: move1.c:m_fsection
pub fn m_fsection(ed: &mut Editor, m: Mark, _cnt: i64, key: i32) -> R<Mark> {
    if key != b']' as i32 {
        return Ok(MARK_UNSET);
    }

    let nlines = ed.vars.nlines;
    let mut l = markline(m);
    while {
        let c = l < nlines;
        l += 1;
        c
    } {
        let text = fetchline(ed, l);
        if ch(&text, 0) == b'{' {
            break;
        }
    }
    if l <= nlines {
        Ok(mark_at_line(l))
    } else {
        Ok(mark_last(nlines))
    }
}

// C: move1.c:m_bsection
pub fn m_bsection(ed: &mut Editor, m: Mark, _cnt: i64, key: i32) -> R<Mark> {
    if key != b'[' as i32 {
        return Ok(MARK_UNSET);
    }

    let mut l = markline(m);
    while {
        let c = l > 1;
        l -= 1;
        c
    } {
        let text = fetchline(ed, l);
        if ch(&text, 0) == b'{' {
            break;
        }
    }
    if l >= 1 {
        Ok(mark_at_line(l))
    } else {
        Ok(MARK_FIRST)
    }
}

// C: move1.c:m_match
pub fn m_match(ed: &mut Editor, m: Mark, _cnt: i64) -> R<Mark> {
    let mut l = markline(m);
    pfetch(ed, l);
    let mut text = markidx(m);

    let mut matched: u8 = 0;
    let mut nest: u8 = 0;
    while matched == 0 && ch(&ed.vars.ptext, text) != 0 {
        // tricky way to recognize 'em in ASCII
        nest = ed.vars.ptext[text];
        if (nest & 0xdf) == b']' || (nest & 0xdf) == b'[' {
            matched = nest ^ (b'[' ^ b']');
        } else if (nest & 0xfe) == b'(' {
            matched = nest ^ (b'(' ^ b')');
        } else {
            matched = 0;
        }
        text += 1;
    }
    if matched == 0 {
        return Ok(MARK_UNSET);
    }
    text -= 1;

    if matched == b'(' || matched == b'[' || matched == b'{' {
        let mut count = 1;
        while count > 0 {
            if text == 0 {
                loop {
                    if l <= 1 {
                        return Ok(MARK_UNSET);
                    }
                    l -= 1;
                    pfetch(ed, l);
                    if ch(&ed.vars.ptext, 0) != 0 {
                        break;
                    }
                }
                text = ed.vars.plen as usize - 1;
            } else {
                text -= 1;
            }

            let c = ch(&ed.vars.ptext, text);
            if c == matched {
                count -= 1;
            } else if c == nest {
                count += 1;
            }
        }
    } else {
        let mut count = 1;
        while count > 0 {
            if ch(&ed.vars.ptext, text) == 0 {
                if l >= ed.vars.nlines {
                    return Ok(MARK_UNSET);
                }
                l += 1;
                pfetch(ed, l);
                text = 0;
            } else {
                text += 1;
            }

            let c = ch(&ed.vars.ptext, text);
            if c == matched {
                count -= 1;
            } else if c == nest {
                count += 1;
            }
        }
    }

    Ok(buildmark(ed.vars.pline, text))
}

// C: move1.c:m_tomark
pub fn m_tomark(ed: &mut Editor, _m: Mark, _cnt: i64, key: i32) -> R<Mark> {
    if key == b'\'' as i32 || key == b'`' as i32 {
        if ed.vars.mark[26] == MARK_UNSET {
            return Ok(MARK_FIRST);
        } else {
            return Ok(ed.vars.mark[26]);
        }
    }

    if key < b'a' as i32 || key > b'z' as i32 {
        return Ok(MARK_UNSET);
    }

    let idx = (key - b'a' as i32) as usize;
    if ed.vars.mark[idx] == 0 {
        msg(ed, b"mark '%c is unset", &[Arg::C(key as u8)]);
    }
    Ok(ed.vars.mark[idx])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn left_and_rear() {
        let mut ed = Editor::new();
        let m = mark_at_line(3) + 5;
        assert_eq!(m_left(&mut ed, m, 0).unwrap(), m - 1);
        assert_eq!(m_left(&mut ed, m, 5).unwrap(), m - 5);
        assert_eq!(m_left(&mut ed, m, 6).unwrap(), MARK_UNSET);
        assert_eq!(m_rear(&mut ed, m, 0).unwrap(), mark_at_line(3) + 1023);
    }

    #[test]
    fn updnto() {
        let mut ed = Editor::new();
        ed.vars.nlines = 10;
        let m = mark_at_line(3) + 2;
        assert_eq!(
            m_updnto(&mut ed, m, 2, b'j' as i32).unwrap(),
            mark_at_line(5) + 2
        );
        assert_eq!(
            m_updnto(&mut ed, m, 2, b'k' as i32).unwrap(),
            mark_at_line(1) + 2
        );
        assert_eq!(m_updnto(&mut ed, m, 3, b'k' as i32).unwrap(), MARK_UNSET);
        assert_eq!(m_updnto(&mut ed, m, 8, b'j' as i32).unwrap(), MARK_UNSET);
        assert_eq!(
            m_updnto(&mut ed, m, 0, b'G' as i32).unwrap(),
            mark_at_line(10)
        );
        assert_eq!(
            m_updnto(&mut ed, m, 4, b'G' as i32).unwrap(),
            mark_at_line(4)
        );
        assert_eq!(
            m_updnto(&mut ed, m, i64::MAX, b'j' as i32).unwrap(),
            MARK_UNSET
        );
    }

    #[test]
    fn tomark() {
        let mut ed = Editor::new();
        assert_eq!(m_tomark(&mut ed, 0, 0, b'\'' as i32).unwrap(), MARK_FIRST);
        ed.vars.mark[26] = 5000;
        assert_eq!(m_tomark(&mut ed, 0, 0, b'`' as i32).unwrap(), 5000);
        ed.vars.mark[1] = 7000;
        assert_eq!(m_tomark(&mut ed, 0, 0, b'b' as i32).unwrap(), 7000);
        assert_eq!(m_tomark(&mut ed, 0, 0, b'A' as i32).unwrap(), MARK_UNSET);
    }
}
