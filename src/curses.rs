//! `curses.h` and `curses.c`: a tiny subset of curses.
//!
//! Capabilities come from `terminfo-lean` (IMPLEMENTATION.md section 4)
//! instead of the system termcap emulation.  `tputs` and `tgoto` are
//! reimplemented here to produce the bytes the macOS ncurses produces.

use std::io::{self, IsTerminal};

use terminfo_lean::expand::{ExpandContext, Parameter};
use terminfo_lean::parse::Terminfo;

use crate::os::{self, Termios};
use crate::vars::Editor;

pub const KBSIZ: usize = 6 * 1024;

pub const A_NORMAL: i32 = 0;
pub const A_STANDOUT: i32 = 1;
pub const A_BOLD: i32 = 2;
pub const A_UNDERLINE: i32 = 3;
pub const A_ALTCHARSET: i32 = 4;

/// Capability strings: `None` is a C null pointer, `Some(vec![])` is "".
pub type Cap = Option<Vec<u8>>;

pub struct Curses {
    pub ospeed: i16,
    pub pc: u8,
    /// `kbuf`; `stdscr` is `kbuf.len()`.
    pub kbuf: Vec<u8>,
    pub lines: i32,
    pub cols: i32,
    pub am: bool,
    pub pt: bool,
    pub vb: Cap,
    pub up: Cap,
    pub so: Cap,
    pub se: Cap,
    pub us: Cap,
    pub ue: Cap,
    pub md: Cap,
    pub me: Cap,
    pub as_: Cap,
    pub ae: Cap,
    pub cm: Cap,
    pub ce: Cap,
    pub cd: Cap,
    pub al: Cap,
    pub dl: Cap,
    pub sr: Cap,
    pub ks: Cap,
    pub ke: Cap,
    pub ku: Cap,
    pub kd: Cap,
    pub kl: Cap,
    pub kr: Cap,
    pub hm: Cap,
    pub en: Cap,
    pub pu: Cap,
    pub pd: Cap,
    pub im: Cap,
    pub ic: Cap,
    pub ei: Cap,
    pub dc: Cap,
    pub ti: Cap,
    pub te: Cap,
    pub aend: Cap,
    pub erasekey: u8,
    pub oldtermio: Termios,
    /// Whether `initscr()` has filled `oldtermio` (B4).
    pub have_oldtermio: bool,
    pub newtermio: Termios,
    pub curses_active: bool,
    /// `tgetnum("li")` and `tgetnum("co")` as ncurses computes them at
    /// `tgetent()` time; -1 when absent.
    pub tc_lines: i32,
    pub tc_cols: i32,
    /// Terminfo static variables persist across `tgoto()` calls.
    pub ectx: ExpandContext,
}

impl Default for Curses {
    fn default() -> Curses {
        let empty = || Some(Vec::new());
        Curses {
            ospeed: 0,
            pc: 0,
            kbuf: Vec::with_capacity(KBSIZ),
            lines: 0,
            cols: 0,
            am: false,
            pt: false,
            vb: None,
            up: None,
            so: None,
            se: None,
            us: empty(),
            ue: empty(),
            md: empty(),
            me: empty(),
            as_: None,
            ae: None,
            cm: None,
            ce: None,
            cd: None,
            al: None,
            dl: None,
            sr: None,
            ks: None,
            ke: None,
            ku: None,
            kd: None,
            kl: None,
            kr: None,
            hm: None,
            en: None,
            pu: None,
            pd: None,
            im: None,
            ic: empty(),
            ei: None,
            dc: None,
            ti: None,
            te: None,
            aend: empty(),
            erasekey: 0,
            oldtermio: Termios::zeroed(),
            have_oldtermio: false,
            newtermio: Termios::zeroed(),
            curses_active: false,
            tc_lines: -1,
            tc_cols: -1,
            ectx: ExpandContext::new(),
        }
    }
}

/*------------------------------------------------------------------------*/
/* The termcap emulation: termcap names, tputs, tgoto                      */

/// Termcap name to terminfo name, for every capability `starttcap()` asks
/// for (plus the ones the capability probe reports).
fn tc2ti(id: &str) -> &'static str {
    match id {
        "up" => "cuu1",
        "vb" => "flash",
        "cm" => "cup",
        "so" => "smso",
        "se" => "rmso",
        "ti" => "smcup",
        "te" => "rmcup",
        "us" => "smul",
        "ue" => "rmul",
        "md" => "bold",
        "me" => "sgr0",
        "as" => "smacs",
        "ae" => "rmacs",
        "al" => "il1",
        "dl" => "dl1",
        "ce" => "el",
        "cd" => "ed",
        "sr" => "ri",
        "im" => "smir",
        "ei" => "rmir",
        "ic" => "ich1",
        "dc" => "dch1",
        "ks" => "smkx",
        "ke" => "rmkx",
        "ku" => "kcuu1",
        "kd" => "kcud1",
        "kl" => "kcub1",
        "kr" => "kcuf1",
        "kP" => "kpp",
        "kN" => "knp",
        "kh" => "khome",
        "kH" => "kll",
        "am" => "am",
        "pt" => "OTpt",
        "xo" => "xon",
        "nx" => "nxon",
        "ug" => "UTug",
        "li" => "lines",
        "co" => "cols",
        "pb" => "pb",
        _ => "",
    }
}

fn tgetstr(ti: &Terminfo<'_>, id: &str) -> Cap {
    let s = ti.strings.get(tc2ti(id))?;
    if id == "me" {
        return Some(trim_sgr0(ti, s));
    }
    Some(s.to_vec())
}

/*------------------------------------------------------------------------*/
/* ncurses `_nc_trim_sgr0()`: `tgetstr("me")` returns `sgr0` without the   */
/* part that leaves the alternate character set.                           */

fn set_attribute_9(sgr: &[u8], flag: i32) -> Option<Vec<u8>> {
    let mut p: Vec<Parameter> = (0..8).map(|_| Parameter::from(0)).collect();
    p.push(Parameter::from(flag));
    let v = ExpandContext::new().expand(&keep_dollars(sgr), &p).ok()?;
    if v.is_empty() { None } else { Some(v) }
}

fn is_csi(s: &[u8]) -> usize {
    if s.first() == Some(&0x9b) {
        1
    } else if s.starts_with(b"\x1b[") {
        2
    } else {
        0
    }
}

fn skip_zero(s: &[u8], i: usize) -> usize {
    if s.get(i) == Some(&b'0') {
        match s.get(i + 1) {
            Some(b';') => return i + 2,
            Some(c) if c.is_ascii_alphabetic() => return i + 1,
            _ => {}
        }
    }
    i
}

fn skip_delay(s: &[u8], mut i: usize) -> usize {
    if s.get(i) == Some(&b'$') && s.get(i + 1) == Some(&b'<') {
        i += 2;
        while i < s.len() && (s[i].is_ascii_digit() || s[i] == b'/') {
            i += 1;
        }
        if s.get(i) == Some(&b'>') {
            i += 1;
        }
    }
    i
}

fn rewrite_sgr(s: &mut [u8], attr: Option<&[u8]>) {
    if let Some(a) = attr
        && !a.is_empty()
        && s.len() > a.len()
        && s.starts_with(a)
    {
        s.rotate_left(a.len());
    }
}

fn strncmp_eq(a: &[u8], b: &[u8], n: usize) -> bool {
    a.iter().take(n).eq(b.iter().take(n))
}

fn similar_sgr(a: &[u8], b: &[u8]) -> bool {
    let csi_a = is_csi(a);
    let csi_b = is_csi(b);
    let (mut ia, mut ib) = (0, 0);
    if csi_a != 0 && csi_b != 0 && csi_a == csi_b {
        ia = csi_a;
        ib = csi_b;
        if a.get(ia) != b.get(ib) {
            ia = skip_zero(a, ia);
            ib = skip_zero(b, ib);
        }
    }
    let (a, b) = (&a[ia..], &b[ib..]);
    if a.is_empty() || b.is_empty() {
        return false;
    }
    strncmp_eq(a, b, a.len().min(b.len()))
}

fn compare_part(part: &[u8], full: &[u8]) -> usize {
    let (mut p, mut f) = (0, 0);
    let mut used_full = 0;
    let mut used_delay = 0;
    while p < part.len() {
        if full.get(f) != Some(&part[p]) {
            used_full = 0;
            break;
        }
        if used_delay != 0 {
            used_full += used_delay;
            used_delay = 0;
        }
        if part[p] == b'$' {
            let np = skip_delay(part, p);
            let nf = skip_delay(full, f);
            if np != p && nf != f {
                used_delay += nf - f;
                f = nf;
                p = np;
                continue;
            }
        }
        used_full += 1;
        p += 1;
        f += 1;
    }
    used_full
}

fn trim_sgr0(ti: &Terminfo<'_>, sgr0: &[u8]) -> Vec<u8> {
    let Some(sgr) = ti.strings.get("sgr") else {
        return sgr0.to_vec();
    };
    let smacs = ti.strings.get("smacs").copied();
    let rmacs = ti.strings.get("rmacs").copied();
    let (Some(mut on), Some(mut off)) = (set_attribute_9(sgr, 1), set_attribute_9(sgr, 0)) else {
        return sgr0.to_vec();
    };
    let mut end = sgr0.to_vec();
    rewrite_sgr(&mut on, smacs);
    rewrite_sgr(&mut off, rmacs);
    rewrite_sgr(&mut end, rmacs);
    if !(similar_sgr(&off, &end) && !similar_sgr(&off, &on)) {
        return sgr0.to_vec();
    }
    let mut found = false;
    if let Some(r) = rmacs {
        let (j, k) = (off.len(), r.len());
        if j > k {
            for i in 0..=(j - k) {
                let k2 = compare_part(r, &off[i..]);
                if k2 != 0 {
                    found = true;
                    off.drain(i..i + k2);
                    break;
                }
            }
        }
    }
    if !found {
        let i = is_csi(&off);
        if i != 0 && off.last() == Some(&b'm') {
            let t = skip_zero(&off, i);
            if off.get(t) == Some(&b'1') && skip_zero(&off, t + 1) != t + 1 {
                let mut i = t;
                if off[i - 1] == b';' {
                    i -= 1;
                }
                let j = skip_zero(&off, t + 1);
                off.drain(i..j);
                found = true;
            }
        }
    }
    let mut result = off;
    if !found
        && let Some(i) = end
            .windows(result.len().max(1))
            .position(|w| w == &result[..])
        && end != result
    {
        let mut tmp = end.clone();
        tmp.truncate(i);
        result = tmp;
    }
    result
}

fn tgetflag(ti: &Terminfo<'_>, id: &str) -> bool {
    ti.booleans.contains(tc2ti(id))
}

fn tgetnum(ti: &Terminfo<'_>, id: &str) -> i32 {
    ti.numbers.get(tc2ti(id)).copied().unwrap_or(-1)
}

/// `_nc_getenv_num()`: a whole `strtol(s, &end, 0)` number, else -1.
fn getenv_num(name: &str) -> i32 {
    let Some(v) = std::env::var_os(name) else {
        return -1;
    };
    let s = v.to_string_lossy();
    let (digits, radix) = if let Some(h) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        (h, 16)
    } else if s.len() > 1 && s.starts_with('0') {
        (&s[1..], 8)
    } else {
        (&s[..], 10)
    };
    match i64::from_str_radix(digits, radix) {
        Ok(n) if (0..=i32::MAX as i64).contains(&n) && !digits.starts_with(['+', '-']) => n as i32,
        _ => -1,
    }
}

/// The screen size ncurses `tgetent()` stores for `li`/`co`: `LINES` and
/// `COLUMNS`, else `TIOCGWINSZ` on fd 1, else the database, else 24x80.
fn tc_screensize(ti: &Terminfo<'_>) -> (i32, i32) {
    let mut lines = getenv_num("LINES");
    let mut cols = getenv_num("COLUMNS");
    if (lines <= 0 || cols <= 0)
        && io::stdout().is_terminal()
        && let Some((r, c)) = os::window_size(io::stdout())
    {
        lines = r as i32;
        cols = c as i32;
    }
    if lines <= 0 {
        lines = tgetnum(ti, "li");
    }
    if cols <= 0 {
        cols = tgetnum(ti, "co");
    }
    if lines <= 0 {
        lines = 24;
    }
    if cols <= 0 {
        cols = 80;
    }
    (lines, cols)
}

/// `tputs(cap, affcnt, faddch)` as the macOS ncurses does it: well-formed
/// `$<…>` delays are removed without emitting pad characters, anything
/// else is output as is.
pub fn tputs_into(cap: &[u8], _affcnt: i32, out: &mut Vec<u8>) {
    let n = cap.len();
    let mut i = 0;
    while i < n {
        if cap[i] != b'$' {
            addch_into(out, cap[i]);
        } else {
            i += 1;
            if i >= n || cap[i] != b'<' {
                addch_into(out, b'$');
                if i < n {
                    addch_into(out, cap[i]);
                }
            } else {
                i += 1;
                if i >= n
                    || (!cap[i].is_ascii_digit() && cap[i] != b'.')
                    || !cap[i..].contains(&b'>')
                {
                    addch_into(out, b'$');
                    addch_into(out, b'<');
                    continue;
                }
                while i < n && cap[i].is_ascii_digit() {
                    i += 1;
                }
                if i < n && cap[i] == b'.' {
                    i += 1;
                    while i < n && cap[i].is_ascii_digit() {
                        i += 1;
                    }
                }
                while i < n && (cap[i] == b'*' || cap[i] == b'/') {
                    i += 1;
                }
            }
        }
        if i >= n {
            break;
        }
        i += 1;
    }
}

/// `tgoto(cm, col, row)`: ncurses calls `tparm(cm, row, col)`, which copies
/// `$<…>` through for `tputs` to handle.  The `terminfo-lean` expander
/// drops delays, so every literal `$` is re-encoded as `%{36}%c` first.
/// Returns `None` if the expansion fails (C would output "OOPS").
pub fn tgoto(ectx: &mut ExpandContext, cm: &[u8], col: i32, row: i32) -> Option<Vec<u8>> {
    ectx.expand(
        &keep_dollars(cm),
        &[Parameter::from(row), Parameter::from(col)],
    )
    .ok()
}

/// Rewrites every literal `$` as `%{36}%c` so the expander keeps delays.
fn keep_dollars(cm: &[u8]) -> Vec<u8> {
    let mut fmt = Vec::with_capacity(cm.len() + 16);
    let mut i = 0;
    while i < cm.len() {
        match cm[i] {
            b'%' if cm.get(i + 1) == Some(&b'\'') => {
                let end = (i + 4).min(cm.len());
                fmt.extend_from_slice(&cm[i..end]);
                i = end;
            }
            b'%' if i + 1 < cm.len() => {
                fmt.extend_from_slice(&cm[i..i + 2]);
                i += 2;
            }
            b'$' => {
                fmt.extend_from_slice(b"%{36}%c");
                i += 1;
            }
            b => {
                fmt.push(b);
                i += 1;
            }
        }
    }
    fmt
}

/*------------------------------------------------------------------------*/
/* Output buffer: qaddch, addch, and the do_* / has_* macros               */

#[inline]
fn addch_into(out: &mut Vec<u8>, ch: u8) {
    out.push(ch);
    if ch == b'\n' {
        out.push(b'\r');
    }
}

fn first_byte(c: &Cap) -> u8 {
    c.as_ref().and_then(|s| s.first().copied()).unwrap_or(0)
}

macro_rules! do_cap {
    ($name:ident, $field:ident, $aff:expr) => {
        pub fn $name(&mut self) {
            let aff = $aff(&*self);
            if let Some(s) = &self.$field {
                tputs_into(s, aff, &mut self.kbuf);
            }
        }
    };
}

impl Curses {
    /// `qaddch(ch)`; returns `ch` like the C macro.
    #[inline]
    pub fn qaddch(&mut self, ch: u8) -> u8 {
        self.kbuf.push(ch);
        ch
    }

    /// `addch(ch)`
    #[inline]
    pub fn addch(&mut self, ch: u8) {
        addch_into(&mut self.kbuf, ch);
    }

    /// `faddch(ch)`, the output function passed to `tputs()`.
    pub fn faddch(&mut self, ch: u8) -> i32 {
        self.addch(ch);
        0
    }

    /// `qaddstr(str)`
    pub fn qaddstr(&mut self, s: &[u8]) {
        self.kbuf.extend_from_slice(s);
    }

    /// `addstr(str)`: `qaddstr` then `_addCR`.
    pub fn addstr(&mut self, s: &[u8]) {
        self.qaddstr(s);
        if self.kbuf.last() == Some(&b'\n') {
            self.qaddch(b'\r');
        }
    }

    /// `move(y, x)`: `tputs(tgoto(CM, x, y), 1, faddch)`.
    pub fn mv(&mut self, y: i32, x: i32) {
        if let Some(cm) = &self.cm
            && let Some(s) = tgoto(&mut self.ectx, cm, x, y)
        {
            tputs_into(&s, 1, &mut self.kbuf);
        }
    }

    /// `mvaddch(y, x, ch)`
    pub fn mvaddch(&mut self, y: i32, x: i32, ch: u8) {
        self.mv(y, x);
        self.addch(ch);
    }

    /// `refresh()` / `wrefresh(stdscr)`
    pub fn refresh(&mut self) {
        if !self.kbuf.is_empty() {
            let _ = os::tty_write(&self.kbuf);
            self.kbuf.clear();
        }
    }

    /// `wqrefresh(stdscr)`
    pub fn wqrefresh(&mut self) {
        if self.kbuf.len() > 2000 {
            let _ = os::tty_write(&self.kbuf);
            self.kbuf.clear();
        }
    }

    do_cap!(do_vb, vb, |_: &Curses| 1);
    do_cap!(do_up, up, |_: &Curses| 1);
    do_cap!(do_so, so, |_: &Curses| 1);
    do_cap!(do_se, se, |_: &Curses| 1);
    do_cap!(do_us, us, |_: &Curses| 1);
    do_cap!(do_ue, ue, |_: &Curses| 1);
    do_cap!(do_md, md, |_: &Curses| 1);
    do_cap!(do_me, me, |_: &Curses| 1);
    do_cap!(do_as, as_, |_: &Curses| 1);
    do_cap!(do_ae, ae, |_: &Curses| 1);
    do_cap!(do_ce, ce, |_: &Curses| 1);
    do_cap!(do_cd, cd, |_: &Curses| 1);
    do_cap!(do_al, al, |c: &Curses| c.lines);
    do_cap!(do_dl, dl, |c: &Curses| c.lines);
    do_cap!(do_sr, sr, |_: &Curses| 1);
    do_cap!(do_ks, ks, |_: &Curses| 1);
    do_cap!(do_ke, ke, |_: &Curses| 1);
    do_cap!(do_im, im, |_: &Curses| 1);
    do_cap!(do_ic, ic, |_: &Curses| 1);
    do_cap!(do_ei, ei, |_: &Curses| 1);
    do_cap!(do_dc, dc, |c: &Curses| c.cols);
    do_cap!(do_aend, aend, |_: &Curses| 1);

    /// `do_TI()`: written raw, so delays are output literally.
    pub fn do_ti(&mut self) {
        if let Some(s) = &self.ti {
            let _ = os::tty_write(s);
        }
    }

    /// `do_TE()`
    pub fn do_te(&mut self) {
        if let Some(s) = &self.te {
            let _ = os::tty_write(s);
        }
    }

    pub fn has_am(&self) -> bool {
        self.am
    }
    pub fn has_pt(&self) -> bool {
        self.pt
    }
    pub fn has_vb(&self) -> bool {
        self.vb.is_some()
    }
    pub fn has_up(&self) -> bool {
        self.up.is_some()
    }
    pub fn has_so(&self) -> bool {
        first_byte(&self.so) != 0
    }
    pub fn has_se(&self) -> bool {
        first_byte(&self.se) != 0
    }
    pub fn has_us(&self) -> bool {
        first_byte(&self.us) != 0
    }
    pub fn has_ue(&self) -> bool {
        first_byte(&self.ue) != 0
    }
    pub fn has_md(&self) -> bool {
        first_byte(&self.md) != 0
    }
    pub fn has_me(&self) -> bool {
        first_byte(&self.me) != 0
    }
    pub fn has_as(&self) -> bool {
        first_byte(&self.as_) != 0
    }
    pub fn has_ae(&self) -> bool {
        first_byte(&self.ae) != 0
    }
    pub fn has_cb(&self) -> bool {
        false
    }
    pub fn has_cs(&self) -> bool {
        false
    }
    pub fn has_ce(&self) -> bool {
        self.ce.is_some()
    }
    pub fn has_cd(&self) -> bool {
        self.cd.is_some()
    }
    pub fn has_al(&self) -> bool {
        self.al.is_some()
    }
    pub fn has_dl(&self) -> bool {
        self.dl.is_some()
    }
    pub fn has_sr(&self) -> bool {
        self.sr.is_some()
    }
    pub fn has_ks(&self) -> bool {
        first_byte(&self.ks) != 0
    }
    pub fn has_ke(&self) -> bool {
        first_byte(&self.ke) != 0
    }
    pub fn has_ku(&self) -> bool {
        self.ku.is_some()
    }
    pub fn has_kd(&self) -> bool {
        self.kd.is_some()
    }
    pub fn has_kl(&self) -> bool {
        self.kl.is_some()
    }
    pub fn has_kr(&self) -> bool {
        self.kr.is_some()
    }
    pub fn has_hm(&self) -> bool {
        self.hm.is_some()
    }
    pub fn has_en(&self) -> bool {
        self.en.is_some()
    }
    pub fn has_pu(&self) -> bool {
        self.pu.is_some()
    }
    pub fn has_pd(&self) -> bool {
        self.pd.is_some()
    }
    pub fn has_im(&self) -> bool {
        first_byte(&self.im) != 0
    }
    pub fn has_ic(&self) -> bool {
        first_byte(&self.ic) != 0
    }
    pub fn has_ei(&self) -> bool {
        first_byte(&self.ei) != 0
    }
    pub fn has_dc(&self) -> bool {
        self.dc.is_some()
    }
    pub fn has_ti(&self) -> bool {
        first_byte(&self.ti) != 0
    }
    pub fn has_te(&self) -> bool {
        first_byte(&self.te) != 0
    }

    /// `standout()` and friends.
    pub fn standout(&mut self) {
        self.do_so();
    }
    pub fn standend(&mut self) {
        self.do_se();
    }
    pub fn clrtoeol(&mut self) {
        self.do_ce();
    }
    pub fn clrtobot(&mut self) {
        self.do_cd();
    }
    pub fn insertln(&mut self) {
        self.do_al();
    }
    pub fn deleteln(&mut self) {
        self.do_dl();
    }
    pub fn delch(&mut self) {
        self.do_dc();
    }

    // C: curses.c:attrset
    pub fn attrset(&mut self, a: i32) {
        self.do_aend();
        if a == A_BOLD {
            self.do_md();
            self.aend = self.me.clone();
        } else if a == A_UNDERLINE {
            self.do_us();
            self.aend = self.ue.clone();
        } else if a == A_ALTCHARSET {
            self.do_as();
            self.aend = self.ae.clone();
        } else {
            self.aend = Some(Vec::new());
        }
    }

    // C: curses.c:insch
    pub fn insch(&mut self, ch: u8) {
        if self.has_im() {
            self.do_im();
        }
        self.do_ic();
        self.qaddch(ch);
        if self.has_ei() {
            self.do_ei();
        }
    }
}

/*------------------------------------------------------------------------*/
/* curses.c                                                                */

// C: curses.c:initscr
pub fn initscr(ed: &mut Editor) {
    if std::env::var_os("TERM").is_none() {
        let _ = os::err_write(b"Environment variable TERM must be set\n");
        os::exit(1);
    } else {
        starttcap(ed);
    }

    ed.curses.kbuf.clear();

    // C ignores the tcgetattr() result and keeps the zeroed static.
    if let Ok(t) = os::tcgetattr(io::stderr()) {
        ed.curses.oldtermio = t;
    }
    ed.curses.have_oldtermio = true;

    resume_curses(ed, true);
}

// C: curses.c:endwin
pub fn endwin(ed: &mut Editor) {
    suspend_curses(ed);
}

// C: curses.c:suspend_curses
pub fn suspend_curses(ed: &mut Editor) {
    let c = &mut ed.curses;
    if c.has_te() {
        c.do_te();
    }
    if c.has_ke() {
        c.do_ke();
    }
    c.refresh();

    // Before initscr() the terminal is still untouched (B4).
    if c.have_oldtermio {
        let _ = os::tcsetattr_now(io::stderr(), &c.oldtermio);
    }
    os::clear_restore_state();
    c.curses_active = false;
}

// C: curses.c:resume_curses
pub fn resume_curses(ed: &mut Editor, quietly: bool) {
    let c = &mut ed.curses;
    if !c.curses_active {
        c.ospeed = os::ospeed_short(&c.oldtermio);
        c.erasekey = os::erase_char(&c.oldtermio);
        c.newtermio = os::elvis_mode(&c.oldtermio);
        let _ = os::tcsetattr_now(io::stderr(), &c.newtermio);

        if c.has_ti() {
            c.do_ti();
        }
        if c.has_ks() {
            c.do_ks();
        }

        let mut restore = Vec::new();
        if c.has_te() {
            restore.extend_from_slice(c.te.as_deref().unwrap_or_default());
        }
        if c.has_ke() {
            tputs_into(c.ke.as_deref().unwrap_or_default(), 1, &mut restore);
        }
        os::save_restore_state(&c.oldtermio, &restore);

        c.curses_active = true;
    }

    if quietly {
        return;
    }

    resume_curses_prompt(ed);
}

/// The "[Press <RETURN> to continue]" half of `resume_curses()`.
fn resume_curses_prompt(ed: &mut Editor) {
    os::ignore_sigint();

    let c = &mut ed.curses;
    c.mv(c.lines - 1, 0);
    c.do_so();
    c.qaddstr(b"[Press <RETURN> to continue]");
    c.do_se();
    c.refresh();
    // C reads into kbuf, which refresh() has just emptied.
    let mut buf = [0u8; 20];
    let _ = crate::config::ttyread(&mut buf);
    if buf[0] == b':' {
        ed.vars.mode = crate::vi_h::MODE_COLON;
        ed.curses.addch(b'\n');
        ed.curses.refresh();
    } else {
        ed.vars.mode = crate::vi_h::MODE_VI;
        crate::redraw::redraw(ed, crate::vi_h::MARK_UNSET, false);
    }
    ed.vars.exwrote = false;

    os::trap_sigint();
}

// C: curses.c:lacking
fn lacking(s: &str) -> ! {
    let _ = os::err_write(b"This termcap entry lacks the :");
    let _ = os::err_write(&s.as_bytes()[..2]);
    let _ = os::err_write(b"=: capability\n");
    os::exit(1);
}

/// `tgetent()`: `Ok(buffer)` for a readable, parseable entry.  C also has
/// a -1 result ("Can't read /etc/termcap", exit 2), but the macOS ncurses
/// never returns it, so every failure is the 0 case.
fn tgetent(term: &std::ffi::OsStr) -> Option<Vec<u8>> {
    let path = terminfo_lean::locate::locate(term).ok()?;
    let buf = std::fs::read(path).ok()?;
    terminfo_lean::parse::parse(&buf).ok()?;
    Some(buf)
}

// C: curses.c:starttcap
fn starttcap(ed: &mut Editor) {
    let term = std::env::var_os("TERM").unwrap_or_default();
    let Some(buf) = tgetent(&term) else {
        let _ = os::err_write(b"Unrecognized TERM type\n");
        os::exit(3);
    };
    let Ok(ti) = terminfo_lean::parse::parse(&buf) else {
        unreachable!("tgetent() parsed this buffer")
    };

    macro_rules! musthave {
        ($t:ident, $s:expr) => {
            ed.curses.$t = tgetstr(&ti, $s);
            if ed.curses.$t.is_none() {
                lacking($s);
            }
        };
    }
    macro_rules! mayhave {
        ($t:ident, $s:expr) => {
            if let Some(str) = tgetstr(&ti, $s) {
                ed.curses.$t = Some(str);
            }
        };
    }
    macro_rules! pair {
        ($t:ident, $u:ident, $st:expr, $su:expr) => {
            ed.curses.$t = tgetstr(&ti, $st);
            ed.curses.$u = tgetstr(&ti, $su);
            if ed.curses.$t.is_none() || ed.curses.$u.is_none() {
                ed.curses.$t = Some(Vec::new());
                ed.curses.$u = Some(Vec::new());
            }
        };
    }

    let (li, co) = tc_screensize(&ti);
    ed.curses.tc_lines = li;
    ed.curses.tc_cols = co;

    musthave!(up, "up");
    mayhave!(vb, "vb");
    musthave!(cm, "cm");
    pair!(so, se, "so", "se");
    pair!(ti, te, "ti", "te");
    if tgetnum(&ti, "ug") <= 0 {
        pair!(us, ue, "us", "ue");
        pair!(md, me, "md", "me");

        pair!(as_, ae, "as", "ae");
        if first_byte(&ed.curses.as_) == 0 {
            ed.curses.as_ = ed.curses.us.clone();
            ed.curses.ae = ed.curses.ue.clone();
        }
    }
    mayhave!(al, "al");
    mayhave!(dl, "dl");
    musthave!(ce, "ce");
    mayhave!(cd, "cd");
    mayhave!(sr, "sr");
    pair!(im, ei, "im", "ei");
    mayhave!(ic, "ic");
    mayhave!(dc, "dc");

    ed.curses.am = tgetflag(&ti, "am");
    ed.curses.pt = tgetflag(&ti, "pt");
    getsize(ed, 0);

    pair!(ks, ke, "ks", "ke");
    mayhave!(ku, "ku");
    mayhave!(kd, "kd");
    mayhave!(kl, "kl");
    mayhave!(kr, "kr");
    mayhave!(pu, "kP");
    mayhave!(pd, "kN");
    mayhave!(hm, "kh");
    mayhave!(en, "kH");
}

// C: curses.c:getsize
/// `signo` is 0 at startup and `os::SIGWINCH` after a resize.
pub fn getsize(ed: &mut Editor, signo: i32) {
    if signo == 0 {
        os::trap_sigwinch();
    }

    let mut lines = 0;
    let mut cols = 0;
    if let Some((r, c)) = os::window_size(io::stderr()) {
        lines = r as i32;
        cols = c as i32;
    }
    let c = &mut ed.curses;
    if (lines == 0 || cols == 0) && signo == 0 {
        c.lines = c.tc_lines;
        c.cols = c.tc_cols;
    }
    if lines >= 2 && cols >= 30 {
        c.lines = lines;
        c.cols = cols;
    }

    if c.lines < 2 || c.cols < 30 {
        let _ = os::err_write(b"Screen too small\n");
        endwin(ed);
        os::exit(2);
    }

    ed.opts.o_columns[0] = ed.curses.cols as i8;
    ed.opts.o_lines[0] = ed.curses.lines as i8;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tp(cap: &[u8], aff: i32) -> Vec<u8> {
        let mut v = Vec::new();
        tputs_into(cap, aff, &mut v);
        v
    }

    #[test]
    fn tputs_padding() {
        assert_eq!(tp(b"\x1b[K", 1), b"\x1b[K");
        assert_eq!(tp(b"\x1b[K$<3>", 1), b"\x1b[K");
        assert_eq!(tp(b"\x1b[P$<1.5*>", 24), b"\x1b[P");
        assert_eq!(tp(b"\x1b[L$<2*>", 1), b"\x1b[L");
        assert_eq!(tp(b"\x1b[M$<2*/>", 24), b"\x1b[M");
        assert_eq!(tp(b"\x1b[?5h$<100/>\x1b[?5l", 1), b"\x1b[?5h\x1b[?5l");
        assert_eq!(tp(b"\x1bM$<5", 1), b"\x1bM$<5");
        assert_eq!(tp(b"a$b", 1), b"a$b");
        assert_eq!(tp(b"a$", 1), b"a$");
        assert_eq!(tp(b"$<x>", 1), b"$<x>");
        assert_eq!(tp(b"$<.5>z", 1), b"z");
        assert_eq!(tp(b"a\nb", 1), b"a\n\rb");
    }

    #[test]
    fn tgoto_order_and_padding() {
        let mut e = ExpandContext::new();
        let cup = b"\x1b[%i%p1%d;%p2%dH$<5>";
        let s = tgoto(&mut e, cup, 7, 9).unwrap();
        assert_eq!(s, b"\x1b[10;8H$<5>");
        assert_eq!(tp(&s, 1), b"\x1b[10;8H");
        let asym = b"\x1b[%p1%{3}%+%d;%p2%{50}%+%dH";
        assert_eq!(tgoto(&mut e, asym, 1, 0).unwrap(), b"\x1b[3;51H");
        assert_eq!(tgoto(&mut e, b"%'$'%c%p1%d", 0, 4).unwrap(), b"$4");
    }

    #[test]
    fn env_num() {
        assert_eq!(getenv_num("ELVIS_TEST_UNSET_VARIABLE"), -1);
    }

    /// The `capprobe.c` report, produced from the Rust side.
    fn report(file: &[u8]) -> String {
        fn esc(out: &mut String, s: &[u8]) {
            for &c in s {
                if (0x20..0x7f).contains(&c) && c != b'\\' {
                    out.push(c as char);
                } else {
                    out.push_str(&format!("\\x{c:02x}"));
                }
            }
        }
        const STRS: [&str; 32] = [
            "up", "vb", "cm", "so", "se", "ti", "te", "us", "ue", "md", "me", "as", "ae", "al",
            "dl", "ce", "cd", "sr", "im", "ei", "ic", "dc", "ks", "ke", "ku", "kd", "kl", "kr",
            "kP", "kN", "kh", "kH",
        ];
        let ti = terminfo_lean::parse::parse(file).unwrap();
        let mut o = String::from("tgetent=1\nospeed=0 PC=0\n");
        for s in STRS {
            o.push_str(&format!("str {s}="));
            match tgetstr(&ti, s) {
                None => o.push_str("(null)"),
                Some(v) => {
                    o.push('"');
                    esc(&mut o, &v);
                    o.push('"');
                }
            }
            o.push('\n');
        }
        for f in ["am", "pt", "xo", "nx"] {
            o.push_str(&format!("flag {f}={}\n", tgetflag(&ti, f) as i32));
        }
        for n in ["ug", "li", "co", "pb"] {
            // The probe ran on a 24x80 pty with no LINES/COLUMNS.
            let v = match (n, tgetnum(&ti, n)) {
                ("li", v) if v <= 0 => 24,
                ("co", v) if v <= 0 => 80,
                (_, v) => v,
            };
            o.push_str(&format!("num {n}={v}\n"));
        }
        if let Some(cm) = tgetstr(&ti, "cm") {
            let mut e = ExpandContext::new();
            for r in [0, 1, 9, 10, 23, 99] {
                for c in [0, 1, 7, 79, 132] {
                    let mut out = Vec::new();
                    if let Some(s) = tgoto(&mut e, &cm, c, r) {
                        tputs_into(&s, 1, &mut out);
                    }
                    o.push_str(&format!("cm row={r} col={c} \""));
                    esc(&mut o, &out);
                    o.push_str("\"\n");
                }
            }
        }
        for s in STRS {
            let Some(v) = tgetstr(&ti, s) else { continue };
            for aff in [1, 24] {
                o.push_str(&format!("tputs {s} aff={aff} \""));
                esc(&mut o, &tp(&v, aff));
                o.push_str("\"\n");
            }
        }
        o
    }

    fn root() -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
    }

    fn fixture_file(name: &str) -> Option<Vec<u8>> {
        let dir = root().join("target/terminfo-fixtures");
        let first = format!("{:x}", name.as_bytes()[0]);
        std::fs::read(dir.join(first).join(name)).ok()
    }

    /// ncurses rewrites `%{50}` as `%'2'` when returning `cm` through
    /// `tgetstr`; elvis only ever expands `cm`, so the stored spelling is
    /// not observable.
    fn normalize(s: &str) -> String {
        s.replace("%'2'", "%{50}")
    }

    #[test]
    fn capprobe_golden() {
        let golden = root().join("tests/golden/macos/capprobe");
        let mut checked = 0;
        let mut diffs = Vec::new();
        for t in [
            "xterm",
            "vt100",
            "linux",
            "screen",
            "missing-cup",
            "missing-el",
            "asym-cup",
            "pad-cup",
        ] {
            let Some(file) = fixture_file(t) else {
                eprintln!("skipping {t}: run tests/tools/build-terminfo.sh");
                continue;
            };
            let got = report(&file);
            for speed in ["1200", "38400"] {
                let want = std::fs::read_to_string(golden.join(format!("{t}.{speed}.txt")))
                    .expect("golden capprobe report");
                let (g, w) = (normalize(&got), normalize(&want));
                for (a, b) in g.lines().zip(w.lines()) {
                    if a != b {
                        diffs.push(format!("{t}.{speed}: got {a} want {b}"));
                    }
                }
                if g.lines().count() != w.lines().count() {
                    diffs.push(format!("{t}.{speed}: line counts differ"));
                }
                checked += 1;
            }
        }
        if checked == 0 {
            eprintln!("capprobe_golden: no fixtures compiled");
        }
        assert!(diffs.is_empty(), "{}", diffs.join("\n"));
    }

    /// Child half of `exit_paths`: runs `initscr()` in a subprocess.
    #[test]
    fn exit_path_child() {
        if std::env::var_os("ELVIS_EXIT_PATH_CHILD").is_none() {
            return;
        }
        let mut ed = Editor::new();
        initscr(&mut ed);
        endwin(&mut ed);
        os::exit(0);
    }

    #[test]
    fn exit_paths() {
        let dir = root().join("target/terminfo-fixtures");
        if !dir.exists() {
            eprintln!("skipping: run tests/tools/build-terminfo.sh");
            return;
        }
        let exe = std::env::current_exe().unwrap();
        let run = |term: Option<&str>, lines: &str, cols: &str| {
            let mut cmd = std::process::Command::new(&exe);
            cmd.args(["--exact", "curses::tests::exit_path_child", "--nocapture"])
                .env("ELVIS_EXIT_PATH_CHILD", "1")
                .env("TERMINFO", &dir)
                .env("LINES", lines)
                .env("COLUMNS", cols)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::piped());
            match term {
                Some(t) => cmd.env("TERM", t),
                None => cmd.env_remove("TERM"),
            };
            let out = cmd.output().unwrap();
            let err = String::from_utf8_lossy(&out.stderr).into_owned();
            (out.status.code(), err)
        };
        let (code, err) = run(None, "24", "80");
        assert_eq!(code, Some(1));
        assert!(err.contains("Environment variable TERM must be set\n"));
        let (code, err) = run(Some("no-such-terminal"), "24", "80");
        assert_eq!(code, Some(3));
        assert!(err.contains("Unrecognized TERM type\n"));
        let (code, err) = run(Some("missing-cup"), "24", "80");
        assert_eq!(code, Some(1));
        assert!(err.contains("This termcap entry lacks the :cm=: capability\n"));
        let (code, err) = run(Some("missing-el"), "24", "80");
        assert_eq!(code, Some(1));
        assert!(err.contains("This termcap entry lacks the :ce=: capability\n"));
        let (code, err) = run(Some("vt100"), "1", "80");
        assert_eq!(code, Some(2));
        assert!(err.contains("Screen too small\n"));
        let (code, err) = run(Some("vt100"), "24", "29");
        assert_eq!(code, Some(2));
        assert!(err.contains("Screen too small\n"));
        let (code, _) = run(Some("vt100"), "24", "80");
        assert_eq!(code, Some(0));
    }
}
