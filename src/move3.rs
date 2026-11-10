//! `move3.c`: character searches (`f`, `F`, `t`, `T`, `;`, `,`).

use crate::misc::pfetch;
use crate::tio::msg;
use crate::vars::Editor;
use crate::vi_h::{MARK_UNSET, Mark, R, ch, default, markidx, markline, schar};

/// Stands in for the C function pointers `prevfwdfn`/`prevrevfn`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CharSearch {
    Fch,
    #[allow(non_camel_case_types)]
    F_ch,
    Tch,
    #[allow(non_camel_case_types)]
    T_ch,
}

#[derive(Default)]
pub struct Move3 {
    pub prevfwdfn: Option<CharSearch>,
    pub prevrevfn: Option<CharSearch>,
    pub prev_key: i32,
}

/// Calls the function a `CharSearch` value stands for.
fn call(ed: &mut Editor, f: CharSearch, m: Mark, cnt: i64, key: i32) -> R<Mark> {
    match f {
        CharSearch::Fch => m_fch(ed, m, cnt, key),
        CharSearch::F_ch => m_Fch(ed, m, cnt, key),
        CharSearch::Tch => m_tch(ed, m, cnt, key),
        CharSearch::T_ch => m_Tch(ed, m, cnt, key),
    }
}

// C: move3.c:m__ch
#[allow(non_snake_case)]
pub fn m__ch(ed: &mut Editor, m: Mark, cnt: i64, cmd: i32) -> R<Mark> {
    let Some(fwd) = ed.move3.prevfwdfn else {
        msg(ed, b"No previous f, F, t, or T command", &[]);
        return Ok(MARK_UNSET);
    };

    if cmd == b',' as i32 {
        let rev = ed.move3.prevrevfn.unwrap_or(fwd);
        let key = ed.move3.prev_key;
        let m = call(ed, rev, m, cnt, key)?;

        // the call above reset prev*fn; swap them back
        std::mem::swap(&mut ed.move3.prevfwdfn, &mut ed.move3.prevrevfn);

        Ok(m)
    } else {
        let key = ed.move3.prev_key;
        call(ed, fwd, m, cnt, key)
    }
}

// C: move3.c:m_fch
pub fn m_fch(ed: &mut Editor, m: Mark, cnt: i64, key: i32) -> R<Mark> {
    let mut m = m;
    let mut cnt = cnt;
    default(&mut cnt, 1);

    ed.move3.prevfwdfn = Some(CharSearch::Fch);
    ed.move3.prevrevfn = Some(CharSearch::F_ch);
    ed.move3.prev_key = key;

    pfetch(ed, markline(m));
    let text = &ed.vars.ptext;
    let mut i = markidx(m);
    while cnt > 0 {
        cnt -= 1;
        loop {
            m += 1;
            i += 1;
            let c = ch(text, i);
            if c == 0 || schar(c) == key {
                break;
            }
        }
    }
    if ch(text, i) == 0 {
        return Ok(MARK_UNSET);
    }
    Ok(m)
}

// C: move3.c:m_Fch
#[allow(non_snake_case)]
pub fn m_Fch(ed: &mut Editor, m: Mark, cnt: i64, key: i32) -> R<Mark> {
    let mut m = m;
    let mut cnt = cnt;
    default(&mut cnt, 1);

    ed.move3.prevfwdfn = Some(CharSearch::F_ch);
    ed.move3.prevrevfn = Some(CharSearch::Fch);
    ed.move3.prev_key = key;

    pfetch(ed, markline(m));
    let text = &ed.vars.ptext;
    let mut i = markidx(m) as isize;
    while cnt > 0 {
        cnt -= 1;
        loop {
            m -= 1;
            i -= 1;
            if i < 0 || schar(ch(text, i as usize)) == key {
                break;
            }
        }
    }
    if i < 0 {
        return Ok(MARK_UNSET);
    }
    Ok(m)
}

// C: move3.c:m_tch
pub fn m_tch(ed: &mut Editor, m: Mark, cnt: i64, key: i32) -> R<Mark> {
    let mut m = m;
    pfetch(ed, markline(m));
    if ed.vars.plen as i64 <= markidx(m) as i64 {
        return Ok(MARK_UNSET);
    }
    m += 1;

    m = m_fch(ed, m, cnt, key)?;
    if m == MARK_UNSET {
        return Ok(MARK_UNSET);
    }

    ed.move3.prevfwdfn = Some(CharSearch::Tch);
    ed.move3.prevrevfn = Some(CharSearch::T_ch);

    Ok(m - 1)
}

// C: move3.c:m_Tch
#[allow(non_snake_case)]
pub fn m_Tch(ed: &mut Editor, m: Mark, cnt: i64, key: i32) -> R<Mark> {
    let mut m = m;
    if markidx(m) == 0 {
        return Ok(MARK_UNSET);
    }
    m -= 1;

    m = m_Fch(ed, m, cnt, key)?;
    if m == MARK_UNSET {
        return Ok(MARK_UNSET);
    }

    ed.move3.prevfwdfn = Some(CharSearch::T_ch);
    ed.move3.prevrevfn = Some(CharSearch::Tch);

    Ok(m + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dispatch_swaps_back_after_comma() {
        let mut ed = Editor::new();
        ed.move3.prevfwdfn = Some(CharSearch::Tch);
        ed.move3.prevrevfn = Some(CharSearch::T_ch);
        // m_Tch at index 0 fails without touching prev*fn
        let r = m__ch(&mut ed, crate::vi_h::mark_at_line(1), 1, b',' as i32).unwrap();
        assert_eq!(r, MARK_UNSET);
        assert_eq!(ed.move3.prevfwdfn, Some(CharSearch::T_ch));
        assert_eq!(ed.move3.prevrevfn, Some(CharSearch::Tch));
    }
}
