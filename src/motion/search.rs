//! `move2.c`: searching (`/`, `?`, `n`, `N`).

use crate::blk::{fetchline, pfetch};
use crate::ex::parseptrn;
use crate::motion::m_front;
use crate::opts::Opt;
use crate::os::take_sigint;
use crate::regexp::{Regexp, regcomp, regexec};
use crate::tio::msg;
use crate::types::{
    BLKSIZE, Interrupted, MARK_UNSET, Mark, R, atol, ch, cstr, mark_at_line, markidx, markline,
};
use crate::vars::Editor;

/// The last search pattern and direction, for `n` and `N`.
#[derive(Default)]
pub struct Search {
    pub re: Option<Regexp>,
    /// C's `prevsf`.
    pub forward: bool,
}

/// Runs the saved pattern against `subject`; returns the match bounds.
fn exec_re(ed: &mut Editor, subject: &[u8], bol: bool) -> Option<(usize, usize)> {
    let mut re = ed.motion.search.re.take()?;
    let found = regexec(ed, &mut re, subject, bol);
    let span = (re.startp[0].unwrap_or(0), re.endp[0].unwrap_or(0));
    ed.motion.search.re = Some(re);
    if found { Some(span) } else { None }
}

/// Parses and compiles `ptrn` (which starts at the delimiter) into
/// `ed.motion.search.re`.  Returns the text after the closing delimiter, or `None`
/// where C returned MARK_UNSET.
fn newpattern(ed: &mut Editor, ptrn: &[u8]) -> Option<Vec<u8>> {
    let mut buf = ptrn.to_vec();
    buf.push(0);
    let line = parseptrn(&mut buf);
    let rest = cstr(buf.get(line..).unwrap_or(&[])).to_vec();
    ed.motion.search.re = None;
    ed.motion.search.re = regcomp(ed, cstr(&buf[1..]));
    ed.motion.search.re.as_ref()?;
    Some(rest)
}

// C: move2.c:m_nsrch
pub fn m_nsrch(ed: &mut Editor, m: Mark) -> R<Mark> {
    let m = if ed.motion.search.forward {
        let m = m_fsrch(ed, m, b"")?;
        ed.motion.search.forward = true;
        m
    } else {
        let m = m_bsrch(ed, m, b"")?;
        ed.motion.search.forward = false;
        m
    };
    Ok(m)
}

// C: move2.c:m_Nsrch
#[allow(non_snake_case)]
pub fn m_Nsrch(ed: &mut Editor, m: Mark) -> R<Mark> {
    let m = if ed.motion.search.forward {
        let m = m_bsrch(ed, m, b"")?;
        ed.motion.search.forward = true;
        m
    } else {
        let m = m_fsrch(ed, m, b"")?;
        ed.motion.search.forward = false;
        m
    };
    Ok(m)
}

// C: move2.c:m_fsrch
/// `ptrn` starts at the `/` delimiter; empty means "reuse the previous RE".
pub fn m_fsrch(ed: &mut Editor, m: Mark, ptrn: &[u8]) -> R<Mark> {
    let mut m = m;
    ed.motion.search.forward = true;

    let mut delta: i64 = 0;
    if ch(ptrn, 0) != 0 {
        let Some(line) = newpattern(ed, ptrn) else {
            return Ok(MARK_UNSET);
        };
        if ch(&line, 0) != 0 {
            delta = atol(&line);
        }
    } else if ed.motion.search.re.is_none() {
        msg(ed, b"No previous expression", &[]);
        return Ok(MARK_UNSET);
    }

    let mut pos = markidx(m) + 1;
    pfetch(ed, markline(m));
    if pos as i64 >= ed.vars.plen as i64 {
        pos = 0;
        m = (m | (BLKSIZE as Mark - 1)) + 1;
    }
    let mut wrapped = false;
    let mut l = markline(m);
    while l != markline(m) + 1 || !wrapped {
        if take_sigint() {
            return Err(Interrupted);
        }

        if l > ed.vars.nlines {
            if wrapped {
                break;
            }

            if ed.opts.on(Opt::Wrapscan) {
                l = 0;
                wrapped = true;
                l += 1;
                continue;
            } else {
                break;
            }
        }

        let line = fetchline(ed, l);

        if let Some((start, _)) = exec_re(ed, line.get(pos..).unwrap_or(&[]), pos == 0) {
            if wrapped && ed.opts.on(Opt::Warn) {
                msg(ed, b"(wrapped)", &[]);
            }
            if delta != 0 {
                l = l.saturating_add(delta);
                if l < 1 || l > ed.vars.nlines {
                    msg(ed, b"search offset too big", &[]);
                    return Ok(MARK_UNSET);
                }
                return m_front(ed, mark_at_line(l));
            }
            return Ok(mark_at_line(l) + (pos + start) as Mark);
        }
        pos = 0;
        l += 1;
    }

    if ed.opts.on(Opt::Wrapscan) {
        msg(ed, b"Not found", &[]);
    } else {
        msg(ed, b"Hit bottom without finding RE", &[]);
    }
    Ok(MARK_UNSET)
}

// C: move2.c:m_bsrch
/// `ptrn` starts at the `?` delimiter; empty means "reuse the previous RE".
pub fn m_bsrch(ed: &mut Editor, m: Mark, ptrn: &[u8]) -> R<Mark> {
    ed.motion.search.forward = false;

    if ch(ptrn, 0) != 0 {
        if newpattern(ed, ptrn).is_none() {
            return Ok(MARK_UNSET);
        }
    } else if ed.motion.search.re.is_none() {
        msg(ed, b"No previous expression", &[]);
        return Ok(MARK_UNSET);
    }

    let mut pos = markidx(m);
    let mut wrapped = false;
    let mut l = markline(m);
    while l != markline(m) - 1 || !wrapped {
        if take_sigint() {
            return Err(Interrupted);
        }

        if l < 1 {
            if ed.opts.on(Opt::Wrapscan) {
                l = ed.vars.nlines + 1;
                wrapped = true;
                l -= 1;
                continue;
            } else {
                break;
            }
        }

        let line = fetchline(ed, l);

        if let Some((mut start, mut end)) = exec_re(ed, &line, true).filter(|&(s, _)| s < pos) {
            // find the last acceptable match in this line
            let mut last;
            loop {
                if take_sigint() {
                    return Err(Interrupted);
                }
                last = start;
                let tr = end;
                if tr == 0 {
                    break;
                }
                match exec_re(ed, line.get(tr..).unwrap_or(&[]), false) {
                    Some((s, e)) if tr + s < pos => {
                        start = tr + s;
                        end = tr + e;
                    }
                    _ => break,
                }
            }

            if wrapped && ed.opts.on(Opt::Warn) {
                msg(ed, b"(wrapped)", &[]);
            }
            return Ok(mark_at_line(l) + last as Mark);
        }
        pos = BLKSIZE;
        l -= 1;
    }

    if ed.opts.on(Opt::Wrapscan) {
        msg(ed, b"Not found", &[]);
    } else {
        msg(ed, b"Hit top without finding RE", &[]);
    }
    Ok(MARK_UNSET)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atol_matches_c() {
        assert_eq!(atol(b"+3"), 3);
        assert_eq!(atol(b"  -12x"), -12);
        assert_eq!(atol(b""), 0);
        assert_eq!(atol(b"abc"), 0);
        assert_eq!(atol(b"99999999999999999999999"), i64::MAX);
        assert_eq!(atol(b"-99999999999999999999999"), i64::MIN);
    }
}
