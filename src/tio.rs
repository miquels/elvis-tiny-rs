//! `tio.c`: terminal input, messages, and key mapping.

use crate::opts::Opt;
use crate::types::Mode;

use crate::config::{MAXMAPS, ttyread, ttywrite};
use crate::curses;
use crate::curses::{endwin, getsize};
use crate::fmt::{Arg, sprintf};
use crate::os::{self, ReadOutcome};
use crate::redraw::redraw;
use crate::tmp::tmpabort;
use crate::types::{Interrupted, LONGKEY, MARK_UNSET, R, When, ch, cstr, ctrl, schar};
use crate::vars::Editor;

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
    pub when: When,
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
/* tio.c                                                                   */

// C: tio.c:vgets
/// Reads a line into `buf` (cleared first; no NUL).  False if the user
/// cancelled.
pub fn vgets(ed: &mut Editor, prompt: u8, buf: &mut Vec<u8>, bsize: usize) -> R<bool> {
    // Screen width of each byte in `buf`.
    let mut widths: Vec<i32> = Vec::new();
    let mut tab = 0;
    let mut quoted = false;

    buf.clear();

    ed.curses.mv(ed.curses.lines - 1, 0);
    if prompt != 0 {
        ed.curses.addch(prompt);
        tab = 1;
    }
    ed.curses.clrtoeol();
    ed.curses.refresh();

    loop {
        let mut ch = getkey(ed, if quoted { When::RAW } else { When::EX })?;

        if ch == ctrl(b'D') as i32 && buf.is_empty() {
            ch = ctrl(b'[') as i32;
        }

        // The key after ^V is taken literally.  C tests bit 256 of the
        // key, which is also set for negative (non-ASCII) keys.
        if quoted && ch != b'\n' as i32 || ch < 0 {
            quoted = false;
            ed.curses.qaddch(b' ');
            ed.curses.qaddch(b'\x08');
        } else if ch == ctrl(b'V') as i32 {
            ed.curses.qaddch(b'^');
            ed.curses.qaddch(b'\x08');
            quoted = true;
            continue;
        } else if ch == ctrl(b'[') as i32 {
            return Ok(false);
        } else if ch == b'\n' as i32 || ch == b'\r' as i32 {
            ed.curses.clrtoeol();
            break;
        } else if ch == b'\x08' as i32 {
            let Some(w) = widths.pop() else {
                return Ok(false);
            };
            buf.pop();
            for _ in 0..w {
                ed.curses.addch(b'\x08');
            }
            if ed.vars.mode == Mode::Ex {
                ed.curses.clrtoeol();
            }
            tab -= w;
            continue;
        }

        if buf.len() + 1 >= bsize {
            beep(ed);
            continue;
        }
        let tabstop = ed.opts.num(Opt::Tabstop);
        let w = if ch == b'\t' as i32 {
            let w = tabstop - (tab % tabstop);
            ed.curses.addstr(&vec![b' '; w.max(0) as usize]);
            w
        } else if ch > 0 && ch < b' ' as i32 {
            ed.curses.addch(b'^');
            ed.curses.addch((ch + b'@' as i32) as u8);
            2
        } else if ch == 0o177 {
            ed.curses.addch(b'^');
            ed.curses.addch(b'?');
            2
        } else {
            ed.curses.addch(ch as u8);
            1
        };
        tab += w;
        widths.push(w);
        buf.push(ch as u8);
    }
    ed.curses.refresh();
    Ok(true)
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
        if ed.tio.manymsgs && getkey(ed, When::MSG).is_err() {
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
fn countmaps(ed: &Editor, when: When) -> (i32, i32) {
    let t = &ed.tio;
    let kptr = t.next as usize;
    let avail = (t.nkeys - t.next) as usize;
    let keys = &t.keybuf[kptr..t.nkeys as usize];
    let mut j = 0;
    let mut k = -1;
    for (i, km) in t.mapped.iter().enumerate() {
        if km.when.any(when) && km.len > 0 && km.rawin[0] == t.keybuf[kptr] {
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
pub fn getkey(ed: &mut Editor, when: When) -> R<i32> {
    if showmsg(ed) {
        if when == When::MSG {
            ed.curses.qaddstr(b"[More...]");
            ed.curses.refresh();
            ed.tio.getkey_cooked = None;
        } else if when == When::VIINP || when == When::VIREP {
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
        if when.any(When::VICMD) {
            let cursor = ed.vars.cursor;
            redraw(ed, cursor, false);
        }

        // A resize that arrived outside a read: C's handler would have run already.
        if os::take_sigwinch() {
            getsize(ed, false);
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
                        getsize(ed, false);
                    }
                }
                ReadOutcome::Timeout | ReadOutcome::Error(_) => {}
            }

            ed.opts.set_num(Opt::Lines, ed.curses.lines);
            ed.opts.set_num(Opt::Columns, ed.curses.cols);
            if when.any(When::VICMD | When::INPUT) {
                redraw(ed, MARK_UNSET, false);
                let cursor = ed.vars.cursor;
                redraw(ed, cursor, !when.any(When::VICMD));
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
                    getsize(ed, false);
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
        if when.any(When::INPUT) && ed.tio.mapped[k].when.any(When::INMV) {
            ed.tio.getkey_cooked = Some(Cooked { map: k, off: 0 });
            Ok(0)
        } else {
            ed.tio.getkey_cooked = Some(Cooked { map: k, off: 1 });
            Ok(schar(ch(&ed.tio.mapped[k].cooked, 0)))
        }
    } else if schar(ed.tio.keybuf[next]) == schar(ed.curses.erasekey) && when != When::RAW {
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
    mut when: When,
    name: Option<&'static [u8]>,
) {
    let mut cooked = cstr(cooked);
    let rawin = cstr(rawin);

    if cooked.starts_with(b"visual ") {
        when |= When::INMV;
        cooked = &cooked[7..];
    }
    if when.any(When::INMV) {
        when |= When::INPUT;
    }

    // C overflows rawin[LONGKEY] for longer sequences.
    let j = rawin.len().min(LONGKEY);
    let rawin = &rawin[..j];
    let mut i = 0;
    while i < MAXMAPS {
        let m = &ed.tio.mapped[i];
        if m.len == j as i32 && m.rawin[..j] == *rawin && m.when.any(when) {
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
pub fn dumpkey(ed: &mut Editor, when: When) {
    for i in 0..MAXMAPS {
        if ed.tio.mapped[i].len <= 0 || !ed.tio.mapped[i].when.any(when) {
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

        if m.when.any(When::INMV) && when.any(When::INPUT) {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(ed: &mut Editor, keys: &[u8]) {
        ed.tio.keybuf[..keys.len()].copy_from_slice(keys);
        ed.tio.nkeys = keys.len() as i32;
        ed.tio.next = 0;
    }

    #[test]
    fn getkey_maps_and_translates() {
        let mut ed = Editor::new();
        ed.curses.erasekey = 0x08;
        mapkey(&mut ed, b"ab", b"xyz", When::VICMD, None);
        mapkey(&mut ed, b"\x1b[A", b"visual k", When::VICMD, Some(b"<Up>"));
        feed(&mut ed, b"ab\x08\0\xe9\x1b[Aq");
        let mut got = Vec::new();
        for _ in 0..8 {
            got.push(getkey(&mut ed, When::VICMD).unwrap());
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
        assert_eq!(getkey(&mut ed, When::VIINP).unwrap(), 0);
        assert_eq!(getkey(&mut ed, When::RAW).unwrap(), b'k' as i32);
        // when == 0 disables the erase-key translation
        feed(&mut ed, b"\x08");
        assert_eq!(getkey(&mut ed, When::RAW).unwrap(), 8);
    }

    #[test]
    fn mapkey_flags_slots_and_unmap() {
        let mut ed = Editor::new();
        mapkey(&mut ed, b"v", b"visual x", When::VICMD, None);
        assert_eq!(
            ed.tio.mapped[0].when,
            When::VICMD | When::INMV | When::INPUT
        );
        assert_eq!(ed.tio.mapped[0].cooked, b"x");
        mapkey(&mut ed, b"v", b"", When::VICMD, None);
        assert_eq!(ed.tio.mapped[0].len, 0);
        ed.vars.mode = Mode::Ex;
        for i in 0..MAXMAPS {
            mapkey(&mut ed, &[b'a' + i as u8], b"z", When::VICMD, None);
        }
        ed.curses.kbuf.clear();
        ed.opts.set(Opt::Exrefresh, false);
        mapkey(&mut ed, b"Q", b"z", When::VICMD, None);
        assert_eq!(ed.curses.kbuf, b"No room left in the key map table\n\r");
    }

    #[test]
    fn dumpkey_format() {
        let mut ed = Editor::new();
        ed.vars.mode = Mode::Ex;
        ed.opts.set(Opt::Exrefresh, false);
        mapkey(
            &mut ed,
            b"\x1b[A",
            b"k",
            When::VICMD | When::INMV,
            Some(b"<Up>"),
        );
        mapkey(&mut ed, b"q", b"dd\x01", When::VICMD, None);
        mapkey(&mut ed, b"#", b"visual x", When::INPUT, None);
        dumpkey(&mut ed, When::VICMD);
        assert_eq!(
            ed.curses.kbuf,
            b"<Up>     ^[[A    k\n\r         q       dd^A\n\r".to_vec()
        );
        ed.curses.kbuf.clear();
        dumpkey(&mut ed, When::VIINP);
        assert_eq!(
            ed.curses.kbuf,
            b"<Up>     ^[[A    visual k\n\r         #       visual x\n\r".to_vec()
        );
    }

    #[test]
    fn vgets_edits_line() {
        let mut ed = Editor::new();
        ed.vars.mode = Mode::Ex;
        feed(&mut ed, b"ab\tc\x08\x16\x1bd\r");
        let mut buf = Vec::new();
        assert!(vgets(&mut ed, b':', &mut buf, 80).unwrap());
        assert_eq!(buf, b"ab\t\x1bd");
        assert!(ed.curses.kbuf.is_empty());

        feed(&mut ed, b"x\x1b");
        assert!(!vgets(&mut ed, 0, &mut buf, 80).unwrap());
        feed(&mut ed, b"\x04");
        assert!(!vgets(&mut ed, 0, &mut buf, 80).unwrap());
        feed(&mut ed, b"abc\r");
        assert!(vgets(&mut ed, 0, &mut buf, 3).unwrap());
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
