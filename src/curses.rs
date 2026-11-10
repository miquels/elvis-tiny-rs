//! `curses.h` and `curses.c`: a tiny subset of curses.
//!
//! Capabilities come from a small terminfo reader below (IMPLEMENTATION.md
//! section 4) instead of the system termcap emulation.  `tputs` and
//! `tgoto` are reimplemented here to produce the bytes the macOS ncurses
//! produces.

use crate::opts::Opt;
use std::io::{self, IsTerminal};
use std::os::unix::ffi::OsStrExt;

use crate::tio::{put_num, utoa};

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

/// Capability slots in `Curses::caps`, in `starttcap()` order.  The two
/// halves of a `PAIR` are adjacent.
pub const UP: usize = 0;
pub const VB: usize = 1;
pub const CM: usize = 2;
pub const SO: usize = 3;
pub const SE: usize = 4;
pub const TI: usize = 5;
pub const TE: usize = 6;
pub const US: usize = 7;
pub const UE: usize = 8;
pub const MD: usize = 9;
pub const ME: usize = 10;
pub const AS: usize = 11;
pub const AE: usize = 12;
pub const AL: usize = 13;
pub const DL: usize = 14;
pub const CE: usize = 15;
pub const CD: usize = 16;
pub const SR: usize = 17;
pub const IM: usize = 18;
pub const EI: usize = 19;
pub const IC: usize = 20;
pub const DC: usize = 21;
pub const KS: usize = 22;
pub const KE: usize = 23;
pub const KU: usize = 24;
pub const KD: usize = 25;
pub const KL: usize = 26;
pub const KR: usize = 27;
pub const PU: usize = 28;
pub const PD: usize = 29;
pub const HM: usize = 30;
pub const EN: usize = 31;
pub const AEND: usize = 32;
pub const NCAPS: usize = 33;

const fn mask(slots: &[usize]) -> u64 {
    let mut m = 0;
    let mut i = 0;
    while i < slots.len() {
        m |= 1 << slots[i];
        i += 1;
    }
    m
}

/// Slots whose `has_*` macro tests the first byte rather than the pointer.
const STR_TEST: u64 = mask(&[SO, SE, TI, TE, US, UE, MD, ME, AS, AE, IM, EI, IC, KS, KE]);
/// Slots that start as `""` instead of null.
const START_EMPTY: u64 = mask(&[US, UE, MD, ME, IC, AEND]);

/// The termcap name of each slot except `AEND`.
const NAMES: [&[u8; 2]; NCAPS - 1] = [
    b"up", b"vb", b"cm", b"so", b"se", b"ti", b"te", b"us", b"ue", b"md", b"me", b"as", b"ae",
    b"al", b"dl", b"ce", b"cd", b"sr", b"im", b"ei", b"ic", b"dc", b"ks", b"ke", b"ku", b"kd",
    b"kl", b"kr", b"kP", b"kN", b"kh", b"kH",
];
/// The terminfo string index ncurses reads for each termcap name.
const TINFO: [u8; NCAPS - 1] = [
    19, 45, 10, 35, 43, 28, 40, 36, 44, 27, 39, 25, 38, 53, 22, 6, 7, 130, 31, 42, 52, 21, 89, 88,
    87, 61, 79, 83, 82, 81, 76, 80,
];

pub struct Curses {
    pub ospeed: i16,
    pub pc: u8,
    /// `kbuf`; `stdscr` is `kbuf.len()`.
    pub kbuf: Vec<u8>,
    pub lines: i32,
    pub cols: i32,
    pub am: bool,
    pub pt: bool,
    /// Capability strings, indexed by the slot constants above.
    pub caps: [Cap; NCAPS],
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
    /// Terminfo static variables `%PA`..`%PZ` persist across `tgoto()`.
    pub tvars: [i32; 26],
}

impl Default for Curses {
    fn default() -> Curses {
        Curses {
            ospeed: 0,
            pc: 0,
            kbuf: Vec::with_capacity(KBSIZ),
            lines: 0,
            cols: 0,
            am: false,
            pt: false,
            caps: std::array::from_fn(|i| (START_EMPTY & (1 << i) != 0).then(Vec::new)),
            erasekey: 0,
            oldtermio: Termios::zeroed(),
            have_oldtermio: false,
            newtermio: Termios::zeroed(),
            curses_active: false,
            tc_lines: -1,
            tc_cols: -1,
            tvars: [0; 26],
        }
    }
}

/*------------------------------------------------------------------------*/
/* The termcap emulation: termcap names, tputs, tgoto                      */

/// `tgetstr(id)` for the termcap name of `slot`, read from the terminfo
/// string capability ncurses uses for it.
fn tgetstr(ti: &Terminfo, slot: usize) -> Cap {
    let s = ti.str(TINFO[slot] as usize)?;
    if slot == ME {
        return Some(trim_sgr0(ti, s));
    }
    Some(s.to_vec())
}
/*------------------------------------------------------------------------*/
/* ncurses `_nc_trim_sgr0()`: `tgetstr("me")` returns `sgr0` without the   */
/* part that leaves the alternate character set.                           */

fn set_attribute_9(sgr: &[u8], flag: i32) -> Option<Vec<u8>> {
    let v = tparm(&mut [0; 26], sgr, &[0, 0, 0, 0, 0, 0, 0, 0, flag])?;
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

fn trim_sgr0(ti: &Terminfo, sgr0: &[u8]) -> Vec<u8> {
    let Some(sgr) = ti.str(TI_SGR) else {
        return sgr0.to_vec();
    };
    let smacs = ti.str(TI_SMACS);
    let rmacs = ti.str(TI_RMACS);
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

/// `tgetflag(id)`; `pt` is the obsolete `OTpt`.
fn tgetflag(ti: &Terminfo, id: &str) -> bool {
    let i = match id {
        "am" => 1,
        "xo" => 20, // xon
        "nx" => 21, // nxon
        "pt" => 42, // OTpt
        _ => return false,
    };
    ti.bools.get(i) == Some(&true)
}

/// `tgetnum(id)`, -1 when absent; `ug` is `UTug`.
fn tgetnum(ti: &Terminfo, id: &str) -> i32 {
    let i = match id {
        "co" => 0, // cols
        "li" => 2, // lines
        "pb" => 5,
        "ug" => 33, // UTug
        _ => return -1,
    };
    ti.nums.get(i).copied().unwrap_or(-1)
}

/// `_nc_getenv_num()`: a whole `strtol(s, &end, 0)` number, else -1.
fn getenv_num(name: &str) -> i32 {
    let Some(v) = crate::os::getenv(name) else {
        return -1;
    };
    let s = v.as_bytes();
    let (d, radix): (&[u8], u32) = if s.len() > 1 && s[0] == b'0' && (s[1] | 0x20) == b'x' {
        (&s[2..], 16)
    } else if s.len() > 1 && s[0] == b'0' {
        (&s[1..], 8)
    } else {
        (s, 10)
    };
    if d.is_empty() {
        return -1;
    }
    let mut n: i64 = 0;
    for &c in d {
        let Some(x) = (c as char).to_digit(radix) else {
            return -1;
        };
        n = n * radix as i64 + x as i64;
        if n > i32::MAX as i64 {
            return -1;
        }
    }
    n as i32
}

/// The screen size ncurses `tgetent()` stores for `li`/`co`: `LINES` and
/// `COLUMNS`, else `TIOCGWINSZ` on fd 1, else the database, else 24x80.
fn tc_screensize(ti: &Terminfo) -> (i32, i32) {
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

/// `tgoto(cm, col, row)`: ncurses calls `tparm(cm, row, col)`.
pub fn tgoto(vars: &mut [i32; 26], cm: &[u8], col: i32, row: i32) -> Option<Vec<u8>> {
    tparm(vars, cm, &[row, col])
}

/*------------------------------------------------------------------------*/
/* A minimal terminfo reader                                               */

/// The standard part of a compiled terminfo entry, in `term.h` order.
pub struct Terminfo {
    bools: Vec<bool>,
    /// -1 for absent or cancelled numbers.
    nums: Vec<i32>,
    strs: Vec<Option<Vec<u8>>>,
}

impl Terminfo {
    fn str(&self, i: usize) -> Option<&[u8]> {
        self.strs.get(i)?.as_deref()
    }
}

// The standard capability counts; entries claiming more are rejected.
const NBOOLS: usize = 44;
const NNUMS: usize = 39;
const NSTRS: usize = 414;

// String capability indexes used outside `tgetstr`.
const TI_SMACS: usize = 25;
const TI_RMACS: usize = 38;
const TI_SGR: usize = 131;

/// Parses the header, booleans, numbers and strings of a compiled entry in
/// the 16-bit (0x011a) or 32-bit (0x021e) number format.  The extended
/// (user-defined) section is ignored: none of the capabilities read here
/// lives there.
fn parse_terminfo(b: &[u8]) -> Option<Terminfo> {
    let h = |i: usize| -> Option<usize> {
        Some(u16::from_le_bytes([*b.get(2 * i)?, *b.get(2 * i + 1)?]) as usize)
    };
    let numsize = match h(0)? {
        0x011a => 2,
        0x021e => 4,
        _ => return None,
    };
    let (names, nb, nn, ns, size) = (h(1)?, h(2)?, h(3)?, h(4)?, h(5)?);
    if nb > NBOOLS || nn > NNUMS || ns > NSTRS {
        return None;
    }
    let mut at = 12 + names;
    let mut bools = Vec::with_capacity(nb);
    for &v in b.get(at..at + nb)? {
        match v {
            0 => bools.push(false),
            1 => bools.push(true),
            _ => return None,
        }
    }
    at += nb;
    at += at & 1;
    let nums = b
        .get(at..at + nn * numsize)?
        .chunks(numsize)
        .map(|c| {
            let v = if numsize == 2 {
                i16::from_le_bytes([c[0], c[1]]) as i32
            } else {
                i32::from_le_bytes([c[0], c[1], c[2], c[3]])
            };
            if v > 0 { v } else { -1 }
        })
        .collect();
    at += nn * numsize;
    let offsets = b.get(at..at + 2 * ns)?;
    let table = b.get(at + 2 * ns..at + 2 * ns + size)?;
    let mut strs = Vec::with_capacity(ns);
    for c in offsets.chunks(2) {
        let off = u16::from_le_bytes([c[0], c[1]]);
        // 0xffff is absent, 0xfffe cancelled
        if off >= 0xfffe {
            strs.push(None);
            continue;
        }
        let s = table.get(off as usize..)?;
        let n = s.iter().position(|&c| c == 0)?;
        strs.push(Some(s[..n].to_vec()));
    }
    Some(Terminfo { bools, nums, strs })
}

/// Reads the compiled entry for `term`, searching `$TERMINFO`,
/// `$HOME/.terminfo`, `$TERMINFO_DIRS` (an empty element means the system
/// directories) and then the system directories.  Each directory is tried
/// with both the `x/xterm` and the `78/xterm` layouts.
fn read_terminfo(term: &[u8]) -> Option<Vec<u8>> {
    const SYSDIRS: [&[u8]; 5] = [
        b"/etc/terminfo",
        b"/lib/terminfo",
        b"/usr/share/terminfo",
        b"/usr/lib/terminfo",
        b"/boot/system/data/terminfo",
    ];
    let first = *term.first()?;
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let hex = [HEX[(first >> 4) as usize], HEX[(first & 15) as usize]];
    let try_dir = |dir: &[u8], suffix: &[u8]| {
        for sub in [&[first][..], &hex[..]] {
            let mut path = dir.to_vec();
            path.extend_from_slice(suffix);
            path.push(b'/');
            path.extend_from_slice(sub);
            path.push(b'/');
            path.extend_from_slice(term);
            if let Ok(buf) = std::fs::read(std::ffi::OsStr::from_bytes(&path)) {
                return Some(buf);
            }
        }
        None
    };
    let try_sys = || SYSDIRS.iter().find_map(|d| try_dir(d, b""));
    if let Some(d) = os::getenv("TERMINFO")
        && let Some(buf) = try_dir(d.as_bytes(), b"")
    {
        return Some(buf);
    }
    if let Some(d) = os::getenv("HOME")
        && let Some(buf) = try_dir(d.as_bytes(), b"/.terminfo")
    {
        return Some(buf);
    }
    let mut sys_tried = false;
    if let Some(list) = os::getenv("TERMINFO_DIRS") {
        for d in list.as_bytes().split(|&c| c == b':') {
            let found = if !d.is_empty() {
                try_dir(d, b"")
            } else if !sys_tried {
                sys_tried = true;
                try_sys()
            } else {
                None
            };
            if found.is_some() {
                return found;
            }
        }
    }
    if sys_tried { None } else { try_sys() }
}

/// `tparm(cap, p1, ..., p9)` with numeric parameters, expanded the way
/// ncurses does: `$<…>` delays are copied through for `tputs`.  `vars` holds
/// the static variables `%PA`..`%PZ`, which persist between calls.  Returns
/// `None` for a malformed string (C's `tgoto` would return "OOPS").
fn tparm(vars: &mut [i32; 26], cap: &[u8], params: &[i32]) -> Option<Vec<u8>> {
    let mut p = [0i32; 9];
    p[..params.len()].copy_from_slice(params);
    let mut dynamic = [0i32; 26];
    let mut stack: Vec<i32> = Vec::new();
    let mut out = Vec::with_capacity(cap.len());
    let mut incremented = false;
    let mut i = 0;
    while i < cap.len() {
        let c = cap[i];
        i += 1;
        if c != b'%' {
            out.push(c);
            continue;
        }
        let Some(&op) = cap.get(i) else { break };
        i += 1;
        match op {
            b'%' => out.push(b'%'),
            b'c' => {
                let v = stack.pop()?;
                out.push(if v == 0 { 0o200 } else { v as u8 });
            }
            b'p' => {
                let d = *cap.get(i)?;
                i += 1;
                if !(b'1'..=b'9').contains(&d) {
                    return None;
                }
                stack.push(p[(d - b'1') as usize]);
            }
            b'P' | b'g' => {
                let v = *cap.get(i)?;
                i += 1;
                let slot = match v {
                    b'A'..=b'Z' => &mut vars[(v - b'A') as usize],
                    b'a'..=b'z' => &mut dynamic[(v - b'a') as usize],
                    _ => return None,
                };
                if op == b'P' {
                    *slot = stack.pop()?;
                } else {
                    stack.push(*slot);
                }
            }
            b'\'' => {
                let ch = *cap.get(i)?;
                if cap.get(i + 1) != Some(&b'\'') {
                    return None;
                }
                i += 2;
                stack.push(ch as i32);
            }
            b'{' => {
                let mut n: i32 = 0;
                loop {
                    let d = *cap.get(i)?;
                    i += 1;
                    if d == b'}' {
                        break;
                    }
                    if !d.is_ascii_digit() {
                        return None;
                    }
                    n = n.checked_mul(10)?.checked_add((d - b'0') as i32)?;
                }
                stack.push(n);
            }
            b'+' | b'-' | b'*' | b'/' | b'm' | b'|' | b'&' | b'^' | b'=' | b'<' | b'>' | b'A'
            | b'O' => {
                let y = stack.pop()?;
                let x = stack.pop()?;
                stack.push(match op {
                    b'+' => x.wrapping_add(y),
                    b'-' => x.wrapping_sub(y),
                    b'*' => x.wrapping_mul(y),
                    b'/' => x.checked_div(y).unwrap_or(0),
                    b'm' => x.checked_rem(y).unwrap_or(0),
                    b'|' => x | y,
                    b'&' => x & y,
                    b'^' => x ^ y,
                    b'=' => (x == y) as i32,
                    b'<' => (x < y) as i32,
                    b'>' => (x > y) as i32,
                    b'A' => (x != 0 && y != 0) as i32,
                    _ => (x != 0 || y != 0) as i32,
                });
            }
            b'!' => {
                let x = stack.pop()?;
                stack.push((x == 0) as i32);
            }
            b'~' => {
                let x = stack.pop()?;
                stack.push(!x);
            }
            b'i' => {
                if !incremented {
                    p[0] = p[0].wrapping_add(1);
                    p[1] = p[1].wrapping_add(1);
                    incremented = true;
                }
            }
            b'?' | b';' => {}
            b't' => {
                if stack.pop()? == 0 {
                    i = skip_cond(cap, i, true);
                }
            }
            b'e' => i = skip_cond(cap, i, false),
            b'd' | b'o' | b'x' | b'X' | b':' | b'#' | b' ' | b'.' | b'0'..=b'9' => {
                i = tparm_format(cap, i - 1, stack.pop()?, &mut out)?;
            }
            _ => return None,
        }
    }
    Some(out)
}

/// Skips past the `%e` (if `to_else`) or `%;` that ends the current branch.
fn skip_cond(cap: &[u8], mut i: usize, to_else: bool) -> usize {
    let mut level = 0;
    while i < cap.len() {
        if cap[i] != b'%' || i + 1 >= cap.len() {
            i += 1;
            continue;
        }
        i += 2;
        match cap[i - 1] {
            b'?' => level += 1,
            b';' if level == 0 => return i,
            b';' => level -= 1,
            b'e' if to_else && level == 0 => return i,
            _ => {}
        }
    }
    i
}

/// One printf-style `%[:][flags][width][.precision]{d,o,x,X}` conversion
/// starting at `cap[i]` (just after the `%`).  Returns the index after it.
fn tparm_format(cap: &[u8], mut i: usize, v: i32, out: &mut Vec<u8>) -> Option<usize> {
    let (mut left, mut plus, mut space, mut alt, mut zero) = (false, false, false, false, false);
    if matches!(cap[i], b':' | b'#' | b' ') {
        if cap[i] == b':' {
            i += 1;
        }
        while let Some(&c) = cap.get(i) {
            match c {
                b'-' => left = true,
                b'+' => plus = true,
                b' ' => space = true,
                b'#' => alt = true,
                b'0' => zero = true,
                _ => break,
            }
            i += 1;
        }
    }
    if cap.get(i) == Some(&b'0') {
        zero = true;
    }
    let mut width = 0usize;
    while let Some(&d) = cap.get(i).filter(|d| d.is_ascii_digit()) {
        width = width.checked_mul(10)?.checked_add((d - b'0') as usize)?;
        i += 1;
    }
    let mut prec: Option<usize> = None;
    if cap.get(i) == Some(&b'.') {
        i += 1;
        let mut n = 0usize;
        while let Some(&d) = cap.get(i).filter(|d| d.is_ascii_digit()) {
            n = n.checked_mul(10)?.checked_add((d - b'0') as usize)?;
            i += 1;
        }
        prec = Some(n);
    }
    let conv = *cap.get(i)?;
    let u = v as u32 as u64;
    let (prefix, mut digits): (&[u8], Vec<u8>) = match conv {
        b'd' => (
            if v < 0 {
                b"-"
            } else if plus {
                b"+"
            } else if space {
                b" "
            } else {
                b""
            },
            utoa(v.unsigned_abs() as u64, 10, false),
        ),
        b'o' => (b"", utoa(u, 8, false)),
        b'x' => (if alt && v != 0 { b"0x" } else { b"" }, utoa(u, 16, false)),
        b'X' => (if alt && v != 0 { b"0X" } else { b"" }, utoa(u, 16, true)),
        _ => return None,
    };
    // Before the precision padding, which gives the same result as after.
    if conv == b'o' && alt && digits[0] != b'0' {
        digits.insert(0, b'0');
    }
    put_num(out, prefix, digits, prec, width, left, zero);
    Some(i + 1)
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
        if let Some(cm) = &self.caps[CM]
            && let Some(s) = tgoto(&mut self.tvars, cm, x, y)
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

    /// The `do_*` macros: output a capability through `tputs()`.
    pub fn put(&mut self, slot: usize) {
        let aff = match slot {
            AL | DL => self.lines,
            DC => self.cols,
            _ => 1,
        };
        if let Some(s) = &self.caps[slot] {
            tputs_into(s, aff, &mut self.kbuf);
        }
    }

    /// The `has_*` macros.
    pub fn has(&self, slot: usize) -> bool {
        if STR_TEST & (1 << slot) != 0 {
            first_byte(&self.caps[slot]) != 0
        } else {
            self.caps[slot].is_some()
        }
    }

    /// `do_TI()`: written raw, so delays are output literally.
    pub fn do_ti(&mut self) {
        if let Some(s) = &self.caps[TI] {
            let _ = os::tty_write(s);
        }
    }

    /// `do_TE()`
    pub fn do_te(&mut self) {
        if let Some(s) = &self.caps[TE] {
            let _ = os::tty_write(s);
        }
    }

    pub fn has_am(&self) -> bool {
        self.am
    }
    pub fn has_pt(&self) -> bool {
        self.pt
    }

    /// `standout()` and friends.
    pub fn standout(&mut self) {
        self.put(SO);
    }
    pub fn standend(&mut self) {
        self.put(SE);
    }
    pub fn clrtoeol(&mut self) {
        self.put(CE);
    }
    /// Queues a control character as `^X`.
    pub fn qaddctl(&mut self, c: u8) {
        self.qaddch(b'^');
        self.qaddch(c ^ b'@');
    }
    /// Moves to the start of the bottom line and clears it.
    pub fn clear_last_line(&mut self) {
        self.mv(self.lines - 1, 0);
        self.clrtoeol();
    }
    pub fn clrtobot(&mut self) {
        self.put(CD);
    }
    pub fn insertln(&mut self) {
        self.put(AL);
    }
    pub fn deleteln(&mut self) {
        self.put(DL);
    }
    pub fn delch(&mut self) {
        self.put(DC);
    }

    // C: curses.c:attrset
    pub fn attrset(&mut self, a: i32) {
        self.put(AEND);
        if a == A_BOLD {
            self.put(MD);
            self.caps[AEND] = self.caps[ME].clone();
        } else if a == A_UNDERLINE {
            self.put(US);
            self.caps[AEND] = self.caps[UE].clone();
        } else if a == A_ALTCHARSET {
            self.put(AS);
            self.caps[AEND] = self.caps[AE].clone();
        } else {
            self.caps[AEND] = Some(Vec::new());
        }
    }

    // C: curses.c:insch
    pub fn insch(&mut self, ch: u8) {
        if self.has(IM) {
            self.put(IM);
        }
        self.put(IC);
        self.qaddch(ch);
        if self.has(EI) {
            self.put(EI);
        }
    }
}

/*------------------------------------------------------------------------*/
/* curses.c                                                                */

// C: curses.c:initscr
// Out of line: inlining this into `main` makes the binary larger.
#[inline(never)]
pub fn initscr(ed: &mut Editor) {
    if crate::os::getenv("TERM").is_none() {
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
    if c.has(TE) {
        c.do_te();
    }
    if c.has(KE) {
        c.put(KE);
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

        if c.has(TI) {
            c.do_ti();
        }
        if c.has(KS) {
            c.put(KS);
        }

        let mut restore = Vec::new();
        if c.has(TE) {
            restore.extend_from_slice(c.caps[TE].as_deref().unwrap_or_default());
        }
        if c.has(KE) {
            tputs_into(c.caps[KE].as_deref().unwrap_or_default(), 1, &mut restore);
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
    c.put(SO);
    c.qaddstr(b"[Press <RETURN> to continue]");
    c.put(SE);
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
fn lacking(s: &[u8; 2]) -> ! {
    let _ = os::err_write(b"This termcap entry lacks the :");
    let _ = os::err_write(s);
    let _ = os::err_write(b"=: capability\n");
    os::exit(1);
}

/// `tgetent()`: the entry if it is readable and parseable.  C also has a
/// -1 result ("Can't read /etc/termcap", exit 2), but the macOS ncurses
/// never returns it, so every failure is the 0 case.
fn tgetent(term: &std::ffi::OsStr) -> Option<Terminfo> {
    parse_terminfo(&read_terminfo(term.as_bytes())?)
}

// The C `MUSTHAVE`, `MAYHAVE` and `PAIR` macros; `PAIR` also loads `slot + 1`.
const MUST: u8 = 0;
const MAY: u8 = 1;
const PAIR: u8 = 2;

fn load(ed: &mut Editor, ti: &Terminfo, steps: &[(u8, usize)]) {
    let caps = &mut ed.curses.caps;
    for &(rule, s) in steps {
        let v = tgetstr(ti, s);
        match rule {
            MUST => {
                if v.is_none() {
                    lacking(NAMES[s]);
                }
                caps[s] = v;
            }
            MAY => {
                if v.is_some() {
                    caps[s] = v;
                }
            }
            _ => {
                let w = tgetstr(ti, s + 1);
                if v.is_none() || w.is_none() {
                    caps[s] = Some(Vec::new());
                    caps[s + 1] = Some(Vec::new());
                } else {
                    caps[s] = v;
                    caps[s + 1] = w;
                }
            }
        }
    }
}

// C: curses.c:starttcap
fn starttcap(ed: &mut Editor) {
    let term = crate::os::getenv("TERM").unwrap_or_default();
    let Some(ti) = tgetent(&term) else {
        let _ = os::err_write(b"Unrecognized TERM type\n");
        os::exit(3);
    };

    let (li, co) = tc_screensize(&ti);
    ed.curses.tc_lines = li;
    ed.curses.tc_cols = co;

    load(
        ed,
        &ti,
        &[(MUST, UP), (MAY, VB), (MUST, CM), (PAIR, SO), (PAIR, TI)],
    );
    if tgetnum(&ti, "ug") <= 0 {
        load(ed, &ti, &[(PAIR, US), (PAIR, MD), (PAIR, AS)]);
        let c = &mut ed.curses.caps;
        if first_byte(&c[AS]) == 0 {
            c[AS] = c[US].clone();
            c[AE] = c[UE].clone();
        }
    }
    #[rustfmt::skip]
    load(ed, &ti, &[
        (MAY, AL), (MAY, DL), (MUST, CE), (MAY, CD), (MAY, SR), (PAIR, IM), (MAY, IC), (MAY, DC),
    ]);

    ed.curses.am = tgetflag(&ti, "am");
    ed.curses.pt = tgetflag(&ti, "pt");
    getsize(ed, 0);

    #[rustfmt::skip]
    load(ed, &ti, &[
        (PAIR, KS), (MAY, KU), (MAY, KD), (MAY, KL), (MAY, KR), (MAY, PU), (MAY, PD), (MAY, HM),
        (MAY, EN),
    ]);
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

    ed.opts.set_num(Opt::Columns, ed.curses.cols);
    ed.opts.set_num(Opt::Lines, ed.curses.lines);
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
        let mut e = [0; 26];
        let cup = b"\x1b[%i%p1%d;%p2%dH$<5>";
        let s = tgoto(&mut e, cup, 7, 9).unwrap();
        assert_eq!(s, b"\x1b[10;8H$<5>");
        assert_eq!(tp(&s, 1), b"\x1b[10;8H");
        let asym = b"\x1b[%p1%{3}%+%d;%p2%{50}%+%dH";
        assert_eq!(tgoto(&mut e, asym, 1, 0).unwrap(), b"\x1b[3;51H");
        assert_eq!(tgoto(&mut e, b"%'$'%c%p1%d", 0, 4).unwrap(), b"$4");
    }

    #[test]
    fn tparm_expansion() {
        let t = |cap: &[u8], p: &[i32]| tparm(&mut [0; 26], cap, p);
        let setaf = b"\x1b[%?%p1%{8}%<%t3%p1%d%e%p1%{16}%<%t9%p1%{8}%-%d%e38;5;%p1%d%;m";
        assert_eq!(t(setaf, &[1]).unwrap(), b"\x1b[31m");
        assert_eq!(t(setaf, &[9]).unwrap(), b"\x1b[91m");
        assert_eq!(t(setaf, &[100]).unwrap(), b"\x1b[38;5;100m");
        assert_eq!(t(b"%p1%03d|%p1%:-3d|%p1%3d", &[7]).unwrap(), b"007|7  |  7");
        assert_eq!(
            t(b"%p1%#x %p1%X %p1%o %p1%#o", &[255]).unwrap(),
            b"0xff FF 377 0377"
        );
        assert_eq!(t(b"%p1%.3d %p1%: +d", &[-5]).unwrap(), b"-005 -5");
        assert_eq!(t(b"%p1%{0}%/%d%p1%!%d%p1%~%d", &[3]).unwrap(), b"00-4");
        assert_eq!(t(b"%p1%c%p2%c", &[0, 65]).unwrap(), b"\x80A");
        assert_eq!(t(b"a%%$<2>", &[]).unwrap(), b"a%$<2>");
        assert_eq!(t(b"%?%p1%t%?%p2%tA%eB%;%eC%;", &[1, 0]).unwrap(), b"B");
        assert_eq!(t(b"%?%p1%t%?%p2%tA%eB%;%eC%;", &[0, 1]).unwrap(), b"C");
        assert!(t(b"%d", &[]).is_none());
        assert!(t(b"%+", &[]).is_none());
        assert!(t(b"%p0", &[]).is_none());
        assert!(t(b"%z", &[]).is_none());
        let mut vars = [0; 26];
        assert_eq!(tparm(&mut vars, b"%p1%PA", &[42]).unwrap(), b"");
        assert_eq!(tparm(&mut vars, b"%gA%d%ga%d", &[]).unwrap(), b"420");
    }

    #[test]
    fn parse_terminfo_formats() {
        // names "t\0", booleans [1, 0, 1], numbers [80, -1, 25], strings
        // [absent, "ab", cancelled]
        let mut f: Vec<u8> = Vec::new();
        for v in [0x011au16, 2, 3, 3, 3, 3] {
            f.extend_from_slice(&v.to_le_bytes());
        }
        f.extend_from_slice(b"t\0\x01\x00\x01\x00");
        for v in [80i16, -1, 25] {
            f.extend_from_slice(&v.to_le_bytes());
        }
        for v in [0xffffu16, 0, 0xfffe] {
            f.extend_from_slice(&v.to_le_bytes());
        }
        f.extend_from_slice(b"ab\0");
        let ti = parse_terminfo(&f).unwrap();
        assert_eq!(ti.bools, [true, false, true]);
        assert_eq!(ti.nums, [80, -1, 25]);
        assert_eq!(ti.strs, [None, Some(b"ab".to_vec()), None]);
        assert!(parse_terminfo(&f[..f.len() - 1]).is_none());
        let mut bad = f.clone();
        bad[0] = 0x1b;
        assert!(parse_terminfo(&bad).is_none());
        let mut bad = f.clone();
        bad[14] = 2;
        assert!(parse_terminfo(&bad).is_none());
    }
    #[test]
    fn env_num() {
        assert_eq!(getenv_num("ELVIS_TEST_UNSET_VARIABLE"), -1);
    }

    fn slot(name: &str) -> usize {
        NAMES
            .iter()
            .position(|n| &n[..] == name.as_bytes())
            .unwrap()
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
        let ti = parse_terminfo(file).unwrap();
        let mut o = String::from("tgetent=1\nospeed=0 PC=0\n");
        for s in STRS {
            o.push_str(&format!("str {s}="));
            match tgetstr(&ti, slot(s)) {
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
        if let Some(cm) = tgetstr(&ti, CM) {
            let mut e = [0; 26];
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
            let Some(v) = tgetstr(&ti, slot(s)) else {
                continue;
            };
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
        if crate::os::getenv("ELVIS_EXIT_PATH_CHILD").is_none() {
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
