//! `tio.c`: terminal input, messages, and key mapping.

use crate::opts::Opt;
use crate::vi_h::Mode;
use std::fs::File;

use crate::config::{MAXMAPS, ttyread, ttywrite, twrite};
use crate::curses;
use crate::curses::{endwin, getsize};
use crate::os::{self, ReadOutcome};
use crate::redraw::redraw;
use crate::tmp::tmpabort;
use crate::vars::Editor;
use crate::vi_h::{
    Interrupted, LONGKEY, MARK_UNSET, R, WHEN_EX, WHEN_INMV, WHEN_MSG, WHEN_VICMD, WHEN_VIINP,
    WHEN_VIREP, ch, cstr, ctrl, schar,
};

/// Capacity of `pmsg[80]`.
pub const PMSG_SIZE: usize = 80;
/// Capacity of `keybuf[100]`.
pub const KEYBUF_SIZE: usize = 100;
/// Capacity of `struct _keymap.cooked[80]`.
pub const COOKED_SIZE: usize = 80;

/// `struct _keymap`
#[derive(Default, Clone)]
pub struct KeyMap {
    pub name: Option<&'static [u8]>,
    pub rawin: [u8; LONGKEY],
    pub cooked: Vec<u8>,
    pub len: i32,
    pub when: i32,
}

/// Where `getkey()`'s static `cooked` pointer points: an offset into
/// `mapped[map].cooked`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cooked {
    pub map: usize,
    pub off: usize,
}

pub struct Tio {
    pub manymsgs: bool,
    pub pmsg: Vec<u8>,
    pub keybuf: [u8; KEYBUF_SIZE],
    pub nkeys: i32,
    pub next: i32,
    pub mapped: [KeyMap; MAXMAPS],
    pub getkey_cooked: Option<Cooked>,
}

impl Default for Tio {
    fn default() -> Tio {
        Tio {
            manymsgs: false,
            pmsg: Vec::new(),
            keybuf: [0; KEYBUF_SIZE],
            nkeys: 0,
            next: 0,
            mapped: Default::default(),
            getkey_cooked: None,
        }
    }
}

/*------------------------------------------------------------------------*/
/* printf subset used by msg() and sprintf() call sites                    */

/// One argument of a `printf`-style call.
#[derive(Debug, Clone, Copy)]
pub enum Arg<'a> {
    /// `%s`
    S(&'a [u8]),
    /// `%d`, `%ld`, `%x`, `%lx`, `%u`, and `*` widths
    D(i64),
    /// `%c`
    C(u8),
}

/// Digits of `v` in `radix` (8, 10 or 16), like the printf conversions.
pub fn utoa(mut v: u64, radix: u64, upper: bool) -> Vec<u8> {
    let digits: &[u8; 16] = if upper {
        b"0123456789ABCDEF"
    } else {
        b"0123456789abcdef"
    };
    let mut s = Vec::new();
    loop {
        s.push(digits[(v % radix) as usize]);
        v /= radix;
        if v == 0 {
            break;
        }
    }
    s.reverse();
    s
}
/// Outputs one printf conversion: `prec` zero-extends `digits`, then
/// `width` pads with spaces, or with zeros after `prefix` when `zero` is
/// set and there is no precision.
pub fn put_num(
    out: &mut Vec<u8>,
    prefix: &[u8],
    mut digits: Vec<u8>,
    prec: Option<usize>,
    width: usize,
    left: bool,
    zero: bool,
) {
    if let Some(p) = prec
        && digits.len() < p
    {
        digits.splice(0..0, std::iter::repeat_n(b'0', p - digits.len()));
    }
    let pad = width.saturating_sub(prefix.len() + digits.len());
    if !left && !(zero && prec.is_none()) {
        out.extend(std::iter::repeat_n(b' ', pad));
    }
    out.extend_from_slice(prefix);
    if !left && zero && prec.is_none() {
        out.extend(std::iter::repeat_n(b'0', pad));
    }
    out.extend_from_slice(&digits);
    if left {
        out.extend(std::iter::repeat_n(b' ', pad));
    }
}

/// `sprintf(fmt, ...)` for the conversions elvis uses: `%s %c %d %ld %u
/// %x %lx %o %%`, with the `-` and `0` flags, a width (digits or `*`) and
/// a precision (digits or `*`).
pub fn sprintf(fmt: &[u8], args: &[Arg<'_>]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut ai = 0;
    let mut next = || {
        let a = args.get(ai).copied();
        ai += 1;
        a
    };
    let mut i = 0;
    while i < fmt.len() {
        let c = fmt[i];
        i += 1;
        if c != b'%' {
            out.push(c);
            continue;
        }
        let mut left = false;
        let mut zero = false;
        while i < fmt.len() && (fmt[i] == b'-' || fmt[i] == b'0') {
            if fmt[i] == b'-' {
                left = true;
            } else {
                zero = true;
            }
            i += 1;
        }
        let mut width: i64 = 0;
        if i < fmt.len() && fmt[i] == b'*' {
            i += 1;
            if let Some(Arg::D(w)) = next() {
                if w < 0 {
                    left = true;
                }
                width = w.abs();
            }
        } else {
            while i < fmt.len() && fmt[i].is_ascii_digit() {
                width = width * 10 + (fmt[i] - b'0') as i64;
                i += 1;
            }
        }
        let mut prec: Option<i64> = None;
        if i < fmt.len() && fmt[i] == b'.' {
            i += 1;
            if i < fmt.len() && fmt[i] == b'*' {
                i += 1;
                if let Some(Arg::D(p)) = next() {
                    prec = if p < 0 { None } else { Some(p) };
                }
            } else {
                let mut p = 0;
                while i < fmt.len() && fmt[i].is_ascii_digit() {
                    p = p * 10 + (fmt[i] - b'0') as i64;
                    i += 1;
                }
                prec = Some(p);
            }
        }
        let mut long = false;
        while i < fmt.len() && fmt[i] == b'l' {
            long = true;
            i += 1;
        }
        let Some(&conv) = fmt.get(i) else { break };
        i += 1;
        let width = width as usize;
        let body: Vec<u8> = match conv {
            b'%' => {
                out.push(b'%');
                continue;
            }
            b's' => {
                let s = match next() {
                    Some(Arg::S(s)) => s,
                    _ => &b"(null)"[..],
                };
                let n = prec.map_or(s.len(), |p| (p as usize).min(s.len()));
                s[..n].to_vec()
            }
            b'c' => match next() {
                Some(Arg::C(c)) => vec![c],
                Some(Arg::D(d)) => vec![d as u8],
                _ => vec![0],
            },
            b'd' | b'i' | b'u' | b'x' | b'X' | b'o' => {
                let v = match next() {
                    Some(Arg::D(d)) => d,
                    Some(Arg::C(c)) => c as i8 as i64,
                    _ => 0,
                };
                let v = if long { v } else { v as i32 as i64 };
                let u = if long { v as u64 } else { v as u32 as u64 };
                let (prefix, digits): (&[u8], _) = match conv {
                    b'd' | b'i' if v < 0 => (b"-", utoa(v.unsigned_abs(), 10, false)),
                    b'd' | b'i' | b'u' => (b"", utoa(u, 10, false)),
                    b'x' => (b"", utoa(u, 16, false)),
                    b'X' => (b"", utoa(u, 16, true)),
                    _ => (b"", utoa(u, 8, false)),
                };
                put_num(
                    &mut out,
                    prefix,
                    digits,
                    prec.map(|p| p as usize),
                    width,
                    left,
                    zero,
                );
                continue;
            }
            other => {
                out.push(b'%');
                out.push(other);
                continue;
            }
        };
        put_num(&mut out, b"", body, None, width, left, false);
    }
    out
}

/*------------------------------------------------------------------------*/
/* tio.c                                                                   */

// C: tio.c:vgets
/// Reads a line into `buf` (cleared first; no NUL).  Returns its length,
/// or -1 if the user cancelled.
pub fn vgets(ed: &mut Editor, prompt: u8, buf: &mut Vec<u8>, bsize: usize) -> R<i32> {
    let mut len: usize;
    let mut ch: i32;
    let mut quoted: bool;
    let mut tab: i32;
    // C has `char widths[132]`; callers never pass a larger bsize.
    let mut widths: Vec<i32> = vec![0; bsize.max(1)];

    buf.clear();

    ed.curses.mv(ed.curses.lines - 1, 0);
    tab = 0;
    if prompt != 0 {
        ed.curses.addch(prompt);
        tab = 1;
    }
    ed.curses.clrtoeol();
    ed.curses.refresh();

    quoted = false;
    len = 0;
    loop {
        ch = getkey(ed, if quoted { 0 } else { WHEN_EX })?;

        if ch == ctrl(b'D') as i32 && len == 0 {
            ch = ctrl(b'[') as i32;
        }

        if quoted && ch != b'\n' as i32 {
            ch |= 256;
        }

        if ch == ctrl(b'V') as i32 {
            ed.curses.qaddch(b'^');
            ed.curses.qaddch(b'\x08');
            quoted = true;
        } else if ch == ctrl(b'[') as i32 {
            return Ok(-1);
        } else if ch == b'\n' as i32 || ch == b'\r' as i32 {
            ed.curses.clrtoeol();
            break;
        } else if ch == b'\x08' as i32 {
            if len > 0 {
                len -= 1;
                buf.truncate(len);
                let mut c = widths[len];
                while c > 0 {
                    ed.curses.addch(b'\x08');
                    c -= 1;
                }
                if ed.vars.mode == Mode::Ex {
                    ed.curses.clrtoeol();
                }
                tab -= widths[len];
            } else {
                return Ok(-1);
            }
        } else {
            if ch & 256 != 0 {
                ch &= !256;
                quoted = false;
                ed.curses.qaddch(b' ');
                ed.curses.qaddch(b'\x08');
            }
            if len + 1 < bsize {
                let tabstop = ed.opts.num(Opt::Tabstop);
                if ch == b'\t' as i32 {
                    widths[len] = tabstop - (tab % tabstop);
                    let spaces = vec![b' '; widths[len].max(0) as usize];
                    ed.curses.addstr(&spaces);
                    tab += widths[len];
                } else if ch > 0 && ch < b' ' as i32 {
                    ed.curses.addch(b'^');
                    ed.curses.addch((ch + b'@' as i32) as u8);
                    widths[len] = 2;
                    tab += 2;
                } else if ch == 0o177 {
                    ed.curses.addch(b'^');
                    ed.curses.addch(b'?');
                    widths[len] = 2;
                    tab += 2;
                } else {
                    ed.curses.addch(ch as u8);
                    widths[len] = 1;
                    tab += 1;
                }
                buf.push(ch as u8);
                len += 1;
            } else {
                beep(ed);
            }
        }
    }
    ed.curses.refresh();
    Ok(len as i32)
}

// C: tio.c:beep
pub fn beep(ed: &mut Editor) {
    if ed.opts.on(Opt::Vbell) {
        ed.curses.put(curses::VB);
        ed.curses.refresh();
    } else if ed.opts.on(Opt::Errorbells) {
        let _ = ttywrite(b"\x07");
    }
}

// C: tio.c:showmsg
fn showmsg(ed: &mut Editor) -> bool {
    if !ed.tio.manymsgs {
        return false;
    }

    ed.curses.mv(ed.curses.lines - 1, 0);
    if !ed.tio.pmsg.is_empty() {
        ed.curses.standout();
        ed.curses.qaddch(b' ');
        let pmsg = std::mem::take(&mut ed.tio.pmsg);
        ed.curses.qaddstr(&pmsg);
        ed.tio.pmsg = pmsg;
        ed.curses.qaddch(b' ');
        ed.curses.standend();
    }
    ed.curses.clrtoeol();

    ed.tio.manymsgs = false;
    true
}

// C: tio.c:endmsgs
pub fn endmsgs(ed: &mut Editor) {
    if ed.tio.manymsgs {
        showmsg(ed);
        ed.curses.addch(b'\n');
    }
}

/// `vsprintf(pmsg, fmt, args)`: the string ends at the first NUL, and is
/// cut to fit `pmsg[80]` (C overflows the buffer instead).
fn format_pmsg(fmt: &[u8], args: &[Arg<'_>]) -> Vec<u8> {
    let mut s = sprintf(fmt, args);
    if let Some(n) = s.iter().position(|&b| b == 0) {
        s.truncate(n);
    }
    s.truncate(PMSG_SIZE - 1);
    s
}

// C: tio.c:msg
/// `msg(fmt, ...)`.  If the `[More...]` wait is interrupted, the interrupt
/// is re-armed (`os::rearm_sigint`) for the next checkpoint.
pub fn msg(ed: &mut Editor, fmt: &[u8], args: &[Arg<'_>]) {
    if ed.vars.mode != Mode::Vi {
        ed.tio.pmsg = format_pmsg(fmt, args);
        let pmsg = std::mem::take(&mut ed.tio.pmsg);
        ed.curses.qaddstr(&pmsg);
        ed.tio.pmsg = pmsg;
        ed.curses.addch(b'\n');
        exrefresh(ed);
    } else {
        if ed.tio.manymsgs && getkey(ed, WHEN_MSG).is_err() {
            os::rearm_sigint();
        }

        ed.tio.pmsg = format_pmsg(fmt, args);
        ed.tio.manymsgs = true;
    }
}

// C: tio.c:exrefresh
pub fn exrefresh(ed: &mut Editor) {
    if ed.curses.kbuf.contains(&b'\n') {
        ed.vars.exwrote = true;
    }

    if ed.opts.on(Opt::Exrefresh) {
        ed.curses.refresh();
    } else {
        ed.curses.wqrefresh();
    }
    if ed.vars.mode != Mode::Vi {
        ed.tio.manymsgs = false;
    }
}

/// The two counting loops of `getkey()`: `j` is how many maps could match
/// the keys at `keybuf[next..nkeys]`, `k` the last map that matches fully.
fn countmaps(ed: &Editor, when: i32) -> (i32, i32) {
    let t = &ed.tio;
    let kptr = t.next as usize;
    let avail = (t.nkeys - t.next) as usize;
    let keys = &t.keybuf[kptr..t.nkeys as usize];
    let mut j = 0;
    let mut k = -1;
    for (i, km) in t.mapped.iter().enumerate() {
        if (km.when & when) != 0 && km.len > 0 && km.rawin[0] == t.keybuf[kptr] {
            let len = km.len as usize;
            if len > avail {
                if km.rawin[..avail] == keys[..avail] {
                    j += 1;
                }
            } else if km.rawin[..len] == keys[..len] {
                j += 1;
                k = i as i32;
            }
        }
    }
    (j, k)
}

// C: tio.c:getkey
pub fn getkey(ed: &mut Editor, when: i32) -> R<i32> {
    if showmsg(ed) {
        if when == WHEN_MSG {
            ed.curses.qaddstr(b"[More...]");
            ed.curses.refresh();
            ed.tio.getkey_cooked = None;
        } else if when == WHEN_VIINP || when == WHEN_VIREP {
            let cursor = ed.vars.cursor;
            redraw(ed, cursor, true);
        }
    }

    if let Some(c) = ed.tio.getkey_cooked {
        let b = ch(&ed.tio.mapped[c.map].cooked, c.off);
        if b != 0 {
            ed.tio.getkey_cooked = Some(Cooked {
                map: c.map,
                off: c.off + 1,
            });
            return Ok(schar(b));
        }
    }

    if ed.tio.next == ed.tio.nkeys {
        if when & WHEN_VICMD != 0 {
            let cursor = ed.vars.cursor;
            redraw(ed, cursor, false);
        }

        // A resize that arrived outside a read: C's handler would have run already.
        if os::take_sigwinch() {
            getsize(ed, os::SIGWINCH);
        }

        ed.curses.refresh();
        loop {
            match ttyread(&mut ed.tio.keybuf) {
                ReadOutcome::Data(n) => {
                    ed.tio.nkeys = n as i32;
                    break;
                }
                ReadOutcome::Eof => {
                    ed.tio.nkeys = 0;
                    break;
                }
                ReadOutcome::Interrupted => {
                    if os::take_sigint() {
                        return Err(Interrupted);
                    }
                    if os::take_sigwinch() {
                        getsize(ed, os::SIGWINCH);
                    }
                }
                ReadOutcome::Timeout | ReadOutcome::Error(_) => {}
            }

            ed.opts.set_num(Opt::Lines, ed.curses.lines);
            ed.opts.set_num(Opt::Columns, ed.curses.cols);
            if when & (WHEN_VICMD | WHEN_VIINP | WHEN_VIREP) != 0 {
                redraw(ed, MARK_UNSET, false);
                let cursor = ed.vars.cursor;
                redraw(ed, cursor, (when & WHEN_VICMD) == 0);
                ed.curses.refresh();
            }
        }
        ed.tio.next = 0;

        if ed.tio.nkeys == 0 {
            tmpabort(ed, true);
            ed.curses.clear_last_line();
            ed.curses.refresh();
            endwin(ed);
            os::exit(1);
        }
    }

    let (mut j, mut k) = countmaps(ed, when);

    while j > 1 {
        let i = ed.tio.nkeys;
        let keytime = ed.opts.num(Opt::Keytime) as u32;
        let start = ed.tio.nkeys as usize;
        match os::tty_read_timeout(&mut ed.tio.keybuf[start..], keytime) {
            ReadOutcome::Data(n) => ed.tio.nkeys += n as i32,
            ReadOutcome::Interrupted => {
                if os::take_sigint() {
                    return Err(Interrupted);
                }
                if os::take_sigwinch() {
                    getsize(ed, os::SIGWINCH);
                }
            }
            ReadOutcome::Eof | ReadOutcome::Timeout | ReadOutcome::Error(_) => {}
        }

        if i == ed.tio.nkeys {
            j = 0;
        } else {
            (j, k) = countmaps(ed, when);
        }
    }

    let next = ed.tio.next as usize;
    if j == 1 && k >= 0 {
        let k = k as usize;
        ed.tio.next += ed.tio.mapped[k].len;
        if (when & (WHEN_VIINP | WHEN_VIREP)) != 0 && (ed.tio.mapped[k].when & WHEN_INMV) != 0 {
            ed.tio.getkey_cooked = Some(Cooked { map: k, off: 0 });
            Ok(0)
        } else {
            ed.tio.getkey_cooked = Some(Cooked { map: k, off: 1 });
            Ok(schar(ch(&ed.tio.mapped[k].cooked, 0)))
        }
    } else if schar(ed.tio.keybuf[next]) == schar(ed.curses.erasekey) && when != 0 {
        ed.tio.next += 1;
        Ok(b'\x08' as i32)
    } else if ed.tio.keybuf[next] == 0 {
        ed.tio.next += 1;
        Ok((b'A' & 0x1f) as i32)
    } else {
        ed.tio.next += 1;
        Ok(schar(ed.tio.keybuf[next]))
    }
}

// C: tio.c:mapkey
/// `cooked` empty means unmap.
pub fn mapkey(
    ed: &mut Editor,
    rawin: &[u8],
    cooked: &[u8],
    when: i16,
    name: Option<&'static [u8]>,
) {
    let mut when = when as i32;
    let mut cooked = cstr(cooked);
    let rawin = cstr(rawin);

    if cooked.starts_with(b"visual ") {
        when |= WHEN_INMV;
        cooked = &cooked[7..];
    }
    if when & WHEN_INMV != 0 {
        when |= WHEN_VIINP | WHEN_VIREP;
    }
    // C's `short when` parameter.
    let when = when as i16 as i32;

    // C overflows rawin[LONGKEY] for longer sequences.
    let j = rawin.len().min(LONGKEY);
    let rawin = &rawin[..j];
    let mut i = 0;
    while i < MAXMAPS {
        let m = &ed.tio.mapped[i];
        if m.len == j as i32 && m.rawin[..j] == *rawin && (m.when & when) != 0 {
            break;
        }
        i += 1;
    }

    if i == MAXMAPS {
        i = 0;
        while i < MAXMAPS && ed.tio.mapped[i].len > 0 {
            i += 1;
        }
    }

    if i == MAXMAPS {
        msg(ed, b"No room left in the key map table", &[]);
        return;
    }

    let m = &mut ed.tio.mapped[i];
    if !cooked.is_empty() {
        m.len = j as i32;
        m.rawin[..j].copy_from_slice(rawin);
        // C overflows cooked[80] for longer text.
        m.cooked = cooked[..cooked.len().min(COOKED_SIZE - 1)].to_vec();
        m.when = when;
        m.name = name;
    } else {
        m.len = 0;
    }
}

// C: tio.c:dumpkey
pub fn dumpkey(ed: &mut Editor, when: i32) {
    for i in 0..MAXMAPS {
        if ed.tio.mapped[i].len <= 0 || (ed.tio.mapped[i].when & when) == 0 {
            continue;
        }
        let m = ed.tio.mapped[i].clone();

        let mut len: i32 = 8;
        if let Some(name) = m.name {
            ed.curses.qaddstr(name);
            len -= name.len() as i32;
        }
        loop {
            ed.curses.qaddch(b' ');
            let more = len > 0;
            len -= 1;
            if !more {
                break;
            }
        }

        len = 0;
        for &c in &m.rawin[..m.len as usize] {
            if c < b' ' || c == 0o177 {
                ed.curses.qaddctl(c);
                len += 2;
            } else {
                ed.curses.qaddch(c);
                len += 1;
            }
        }
        loop {
            ed.curses.qaddch(b' ');
            len += 1;
            if len >= 8 {
                break;
            }
        }

        if (m.when & WHEN_INMV) != 0 && (when & (WHEN_VIINP | WHEN_VIREP)) != 0 {
            ed.curses.qaddstr(b"visual ");
        }
        for &c in cstr(&m.cooked) {
            if c < b' ' || c == 0o177 {
                ed.curses.qaddctl(c);
            } else {
                ed.curses.qaddch(c);
            }
        }

        ed.curses.addch(b'\n');
        exrefresh(ed);
    }
}

// C: tio.c:savekeys
pub fn savekeys(ed: &mut Editor, fd: &mut File) {
    for i in 0..MAXMAPS {
        let m = &ed.tio.mapped[i];
        if m.name.is_some() {
            continue;
        }

        if m.len <= 0 {
            continue;
        }

        let raw = &m.rawin[..];
        if m.when & WHEN_INMV != 0 {
            let buf = sprintf(
                b"map%s %.*s visual %s\n",
                &[
                    Arg::S(if m.when & WHEN_VICMD != 0 { b"" } else { b"!" }),
                    Arg::D(m.len as i64),
                    Arg::S(raw),
                    Arg::S(&m.cooked),
                ],
            );
            twrite(fd, &buf);
        } else {
            if m.when & WHEN_VICMD != 0 {
                let buf = sprintf(
                    b"map %.*s %s\n",
                    &[Arg::D(m.len as i64), Arg::S(raw), Arg::S(&m.cooked)],
                );
                twrite(fd, &buf);
            }
            if m.when & (WHEN_VIINP | WHEN_VIREP) != 0 {
                let buf = sprintf(
                    b"map! %.*s %s\n",
                    &[Arg::D(m.len as i64), Arg::S(raw), Arg::S(&m.cooked)],
                );
                twrite(fd, &buf);
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sprintf_matches_c() {
        let s = |f: &[u8], a: &[Arg<'_>]| String::from_utf8(sprintf(f, a)).unwrap();
        assert_eq!(
            s(b"%s %d %ld", &[Arg::S(b"x"), Arg::D(-5), Arg::D(1 << 40)]),
            "x -5 1099511627776"
        );
        assert_eq!(s(b"%c%%", &[Arg::C(b'q')]), "q%");
        assert_eq!(
            s(b"map %.*s %s\n", &[Arg::D(2), Arg::S(b"abc"), Arg::S(b"z")]),
            "map ab z\n"
        );
        assert_eq!(
            s(b"[%5d][%-5d][%05d]", &[Arg::D(42), Arg::D(42), Arg::D(-42)]),
            "[   42][42   ][-0042]"
        );
        assert_eq!(s(b"%x %lx", &[Arg::D(-1), Arg::D(255)]), "ffffffff ff");
        assert_eq!(s(b"%-10s|", &[Arg::S(b"ab")]), "ab        |");
        assert_eq!(s(b"%3s|%.1s", &[Arg::S(b"abcd"), Arg::S(b"xyz")]), "abcd|x");
    }

    fn feed(ed: &mut Editor, keys: &[u8]) {
        ed.tio.keybuf[..keys.len()].copy_from_slice(keys);
        ed.tio.nkeys = keys.len() as i32;
        ed.tio.next = 0;
    }

    #[test]
    fn getkey_maps_and_translates() {
        let mut ed = Editor::new();
        ed.curses.erasekey = 0x08;
        mapkey(&mut ed, b"ab", b"xyz", WHEN_VICMD as i16, None);
        mapkey(
            &mut ed,
            b"\x1b[A",
            b"visual k",
            WHEN_VICMD as i16,
            Some(b"<Up>"),
        );
        feed(&mut ed, b"ab\x08\0\xe9\x1b[Aq");
        let mut got = Vec::new();
        for _ in 0..8 {
            got.push(getkey(&mut ed, WHEN_VICMD).unwrap());
        }
        assert_eq!(
            got,
            vec![
                b'x' as i32,
                b'y' as i32,
                b'z' as i32,
                8,
                1,
                -23,
                b'k' as i32,
                b'q' as i32
            ]
        );

        // INMV maps return 0 first in input mode, then the cooked text.
        feed(&mut ed, b"\x1b[A");
        assert_eq!(getkey(&mut ed, WHEN_VIINP).unwrap(), 0);
        assert_eq!(getkey(&mut ed, 0).unwrap(), b'k' as i32);
        // when == 0 disables the erase-key translation
        feed(&mut ed, b"\x08");
        assert_eq!(getkey(&mut ed, 0).unwrap(), 8);
    }

    #[test]
    fn mapkey_flags_slots_and_unmap() {
        let mut ed = Editor::new();
        mapkey(&mut ed, b"v", b"visual x", WHEN_VICMD as i16, None);
        assert_eq!(
            ed.tio.mapped[0].when,
            WHEN_VICMD | WHEN_INMV | WHEN_VIINP | WHEN_VIREP
        );
        assert_eq!(ed.tio.mapped[0].cooked, b"x");
        mapkey(&mut ed, b"v", b"", WHEN_VICMD as i16, None);
        assert_eq!(ed.tio.mapped[0].len, 0);
        ed.vars.mode = Mode::Ex;
        for i in 0..MAXMAPS {
            mapkey(&mut ed, &[b'a' + i as u8], b"z", WHEN_VICMD as i16, None);
        }
        ed.curses.kbuf.clear();
        ed.opts.set(Opt::Exrefresh, false);
        mapkey(&mut ed, b"Q", b"z", WHEN_VICMD as i16, None);
        assert_eq!(ed.curses.kbuf, b"No room left in the key map table\n\r");
    }

    #[test]
    fn dumpkey_and_savekeys_format() {
        let mut ed = Editor::new();
        ed.vars.mode = Mode::Ex;
        ed.opts.set(Opt::Exrefresh, false);
        mapkey(
            &mut ed,
            b"\x1b[A",
            b"k",
            (WHEN_VICMD | WHEN_INMV) as i16,
            Some(b"<Up>"),
        );
        mapkey(&mut ed, b"q", b"dd\x01", WHEN_VICMD as i16, None);
        mapkey(
            &mut ed,
            b"#",
            b"visual x",
            (WHEN_VIINP | WHEN_VIREP) as i16,
            None,
        );
        dumpkey(&mut ed, WHEN_VICMD);
        assert_eq!(
            ed.curses.kbuf,
            b"<Up>     ^[[A    k\n\r         q       dd^A\n\r".to_vec()
        );
        ed.curses.kbuf.clear();
        dumpkey(&mut ed, WHEN_VIINP);
        assert_eq!(
            ed.curses.kbuf,
            b"<Up>     ^[[A    visual k\n\r         #       visual x\n\r".to_vec()
        );

        let path = std::env::temp_dir().join(format!("elvis-savekeys-{}", std::process::id()));
        let mut f = File::create(&path).unwrap();
        savekeys(&mut ed, &mut f);
        drop(f);
        let out = std::fs::read(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(out, b"map q dd\x01\nmap! # visual x\n".to_vec());
    }

    #[test]
    fn vgets_edits_line() {
        let mut ed = Editor::new();
        ed.vars.mode = Mode::Ex;
        feed(&mut ed, b"ab\tc\x08\x16\x1bd\r");
        let mut buf = Vec::new();
        assert_eq!(vgets(&mut ed, b':', &mut buf, 80).unwrap(), 5);
        assert_eq!(buf, b"ab\t\x1bd");
        assert!(ed.curses.kbuf.is_empty());

        feed(&mut ed, b"x\x1b");
        assert_eq!(vgets(&mut ed, 0, &mut buf, 80).unwrap(), -1);
        feed(&mut ed, b"\x04");
        assert_eq!(vgets(&mut ed, 0, &mut buf, 80).unwrap(), -1);
        feed(&mut ed, b"abc\r");
        assert_eq!(vgets(&mut ed, 0, &mut buf, 3).unwrap(), 2);
        assert_eq!(buf, b"ab");
    }

    #[test]
    fn msg_truncates_and_queues() {
        let mut ed = Editor::new();
        ed.vars.mode = Mode::Vi;
        msg(&mut ed, b"%s", &[Arg::S(&[b'x'; 200])]);
        assert!(ed.tio.manymsgs);
        assert_eq!(ed.tio.pmsg.len(), PMSG_SIZE - 1);
        endmsgs(&mut ed);
        assert!(!ed.tio.manymsgs);
        assert!(ed.curses.kbuf.ends_with(b" \n\r"));
    }
}
