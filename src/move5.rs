//! `move5.c`: word motions.  No module state.

use crate::misc::pfetch;
use crate::vars::Editor;
use crate::vi_h::{
    MARK_UNSET, Mark, R, buildmark, ch, default, isalnum, isascii, isspace, isword, markidx,
    markline, schar,
};

/// `*text` as a signed C `char`, where `text` indexes `ptext`.  Bytes past
/// the end read as NUL.
fn at(ed: &Editor, text: isize) -> i32 {
    if text < 0 {
        0
    } else {
        schar(ch(&ed.vars.ptext, text as usize))
    }
}

// C: move5.c:m_fword
pub fn m_fword(ed: &mut Editor, m: Mark, cnt: i64, cmd: i32) -> R<Mark> {
    let mut cnt = cnt;
    default(&mut cnt, 1);

    let mut l = markline(m);
    pfetch(ed, l);
    let mut text = markidx(m) as isize;
    while cnt > 0 {
        cnt -= 1;
        let mut i = at(ed, text);
        text += 1;

        if cmd == b'W' as i32 {
            while i != 0 && !isspace(i) {
                i = at(ed, text);
                text += 1;
            }
        } else if isword(i) {
            while i != 0 && isword(i) {
                i = at(ed, text);
                text += 1;
            }
        } else {
            while i != 0 && isascii(i) && !isalnum(i) && !isspace(i) {
                i = at(ed, text);
                text += 1;
            }
        }

        // include trailing whitespace
        while i == 0 || isspace(i) {
            if i == 0 {
                l += 1;
                if l > ed.vars.nlines {
                    return Ok(MARK_UNSET);
                }
                pfetch(ed, l);
                text = 0;
            }

            i = at(ed, text);
            text += 1;
        }
        text -= 1;
    }

    Ok(buildmark(ed.vars.pline, text as usize))
}

// C: move5.c:m_bword
pub fn m_bword(ed: &mut Editor, m: Mark, cnt: i64, cmd: i32) -> R<Mark> {
    let mut cnt = cnt;
    default(&mut cnt, 1);

    let mut l = markline(m);
    pfetch(ed, l);
    let mut text = markidx(m) as isize;
    while cnt > 0 {
        cnt -= 1;
        text -= 1;

        // include preceding whitespace
        while text < 0 || isspace(at(ed, text)) {
            if text < 0 {
                l -= 1;
                if l <= 0 {
                    return Ok(MARK_UNSET);
                }
                pfetch(ed, l);
                text = ed.vars.plen as isize - 1;
            } else {
                text -= 1;
            }
        }

        if cmd == b'B' as i32 {
            while text >= 0 && !isspace(at(ed, text)) {
                text -= 1;
            }
        } else if isword(at(ed, text)) {
            while text >= 0 && isword(at(ed, text)) {
                text -= 1;
            }
        } else {
            while text >= 0
                && isascii(at(ed, text))
                && !isalnum(at(ed, text))
                && !isspace(at(ed, text))
            {
                text -= 1;
            }
        }
        text += 1;
    }

    Ok(buildmark(ed.vars.pline, text as usize))
}

// C: move5.c:m_eword
pub fn m_eword(ed: &mut Editor, m: Mark, cnt: i64, cmd: i32) -> R<Mark> {
    let mut cnt = cnt;
    default(&mut cnt, 1);

    let mut l = markline(m);
    pfetch(ed, l);
    let mut text = markidx(m) as isize;
    while cnt > 0 {
        cnt -= 1;
        text += 1;
        let mut i = at(ed, text);
        text += 1;

        // include preceding whitespace
        while i == 0 || isspace(i) {
            if i == 0 {
                l += 1;
                if l > ed.vars.nlines {
                    return Ok(MARK_UNSET);
                }
                pfetch(ed, l);
                text = 0;
            }

            i = at(ed, text);
            text += 1;
        }

        if cmd == b'E' as i32 {
            while i != 0 && !isspace(i) {
                i = at(ed, text);
                text += 1;
            }
        } else if isword(i) {
            while i != 0 && isword(i) {
                i = at(ed, text);
                text += 1;
            }
        } else {
            while i != 0 && isascii(i) && !isalnum(i) && !isspace(i) {
                i = at(ed, text);
                text += 1;
            }
        }
        text -= 2;
    }

    Ok(buildmark(ed.vars.pline, text as usize))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn at_reads_nul_outside_line() {
        let mut ed = Editor::new();
        ed.vars.ptext = b"a\xe9".to_vec();
        assert_eq!(at(&ed, -1), 0);
        assert_eq!(at(&ed, 0), b'a' as i32);
        assert_eq!(at(&ed, 1), -23);
        assert_eq!(at(&ed, 2), 0);
    }
}
