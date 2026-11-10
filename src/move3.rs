//! `move3.c`: character searches (`f`, `F`, `t`, `T`, `;`, `,`).

use crate::misc::pfetch;
use crate::tio::msg;
use crate::vars::Editor;
use crate::vi_h::{MARK_UNSET, Mark, R, ch, default, markidx, markline, schar};

/// One of the `f`, `F`, `t` and `T` commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CharSearch {
    pub forward: bool,
    /// `t`/`T`: stop next to the character instead of on it.
    pub till: bool,
}

impl CharSearch {
    fn reverse(self) -> CharSearch {
        CharSearch {
            forward: !self.forward,
            till: self.till,
        }
    }
}

#[derive(Default)]
pub struct Move3 {
    /// The last search, for `;`; `,` uses its reverse.  C kept this as the
    /// pair of function pointers `prevfwdfn`/`prevrevfn`.
    pub prev: Option<CharSearch>,
    pub prev_key: i32,
}

// C: move3.c:m__ch
#[allow(non_snake_case)]
pub fn m__ch(ed: &mut Editor, m: Mark, cnt: i64, cmd: i32) -> R<Mark> {
    let Some(prev) = ed.move3.prev else {
        msg(ed, b"No previous f, F, t, or T command", &[]);
        return Ok(MARK_UNSET);
    };
    let key = ed.move3.prev_key;

    if cmd == b',' as i32 {
        let m = charsearch(ed, m, cnt, key, prev.reverse())?;

        // the search above replaced `prev`; reverse it back
        ed.move3.prev = ed.move3.prev.map(CharSearch::reverse);

        Ok(m)
    } else {
        charsearch(ed, m, cnt, key, prev)
    }
}

// C: move3.c:m_fch, m_Fch, m_tch, m_Tch
fn charsearch(ed: &mut Editor, m: Mark, cnt: i64, key: i32, how: CharSearch) -> R<Mark> {
    let mut m = m;
    let mut cnt = cnt;
    if how.till && !how.forward {
        if markidx(m) == 0 {
            return Ok(MARK_UNSET);
        }
        m -= 1;
    }

    pfetch(ed, markline(m));

    if how.till && how.forward {
        if ed.vars.plen as i64 <= markidx(m) as i64 {
            return Ok(MARK_UNSET);
        }
        m += 1;
    }

    default(&mut cnt, 1);

    // A `t`/`T` that finds nothing is remembered as `f`/`F`, as in C.
    ed.move3.prev = Some(CharSearch {
        forward: how.forward,
        till: false,
    });
    ed.move3.prev_key = key;

    let text = &ed.vars.ptext;
    let mut i = markidx(m) as isize;
    let step: isize = if how.forward { 1 } else { -1 };
    while cnt > 0 {
        cnt -= 1;
        loop {
            m += step as Mark;
            i += step;
            if i < 0 {
                break;
            }
            let c = ch(text, i as usize);
            if (how.forward && c == 0) || schar(c) == key {
                break;
            }
        }
    }
    if i < 0 || (how.forward && ch(text, i as usize) == 0) {
        return Ok(MARK_UNSET);
    }

    if how.till {
        ed.move3.prev = Some(how);
        m -= step as Mark;
    }
    Ok(m)
}

// C: move3.c:m_fch
pub fn m_fch(ed: &mut Editor, m: Mark, cnt: i64, key: i32) -> R<Mark> {
    let how = CharSearch {
        forward: true,
        till: false,
    };
    charsearch(ed, m, cnt, key, how)
}

// C: move3.c:m_Fch
#[allow(non_snake_case)]
pub fn m_Fch(ed: &mut Editor, m: Mark, cnt: i64, key: i32) -> R<Mark> {
    let how = CharSearch {
        forward: false,
        till: false,
    };
    charsearch(ed, m, cnt, key, how)
}

// C: move3.c:m_tch
pub fn m_tch(ed: &mut Editor, m: Mark, cnt: i64, key: i32) -> R<Mark> {
    let how = CharSearch {
        forward: true,
        till: true,
    };
    charsearch(ed, m, cnt, key, how)
}

// C: move3.c:m_Tch
#[allow(non_snake_case)]
pub fn m_Tch(ed: &mut Editor, m: Mark, cnt: i64, key: i32) -> R<Mark> {
    let how = CharSearch {
        forward: false,
        till: true,
    };
    charsearch(ed, m, cnt, key, how)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dispatch_swaps_back_after_comma() {
        let mut ed = Editor::new();
        let t = CharSearch {
            forward: true,
            till: true,
        };
        ed.move3.prev = Some(t);
        // `T` at index 0 fails without touching `prev`
        let r = m__ch(&mut ed, crate::vi_h::mark_at_line(1), 1, b',' as i32).unwrap();
        assert_eq!(r, MARK_UNSET);
        assert_eq!(ed.move3.prev, Some(t.reverse()));
    }
}
