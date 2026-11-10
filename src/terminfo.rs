//! A minimal terminfo reader and the parts of the ncurses termcap
//! emulation elvis uses: `tgetent`, `tgetflag`, `tgetnum`, `tgetstr`'s
//! `sgr0` trimming, `tparm`, `tgoto` and `tputs` (IMPLEMENTATION.md
//! section 4).

use std::io::{self, IsTerminal};
use std::os::unix::ffi::OsStrExt;

use crate::curses::addch_into;
use crate::fmt::{put_num, utoa};
use crate::os;

/// The standard part of a compiled terminfo entry, in `term.h` order.
pub struct Terminfo {
    bools: Vec<bool>,
    /// -1 for absent or cancelled numbers.
    nums: Vec<i32>,
    strs: Vec<Option<Vec<u8>>>,
}

// The standard capability counts; entries claiming more are rejected.
const NBOOLS: usize = 44;
const NNUMS: usize = 39;
const NSTRS: usize = 414;

// String capability indexes used outside `tgetstr`.
const TI_SMACS: usize = 25;
const TI_RMACS: usize = 38;
const TI_SGR: usize = 131;

impl Terminfo {
    /// `tgetent()`: the entry if it is readable and parseable.  C also has a
    /// -1 result ("Can't read /etc/termcap", exit 2), but the macOS ncurses
    /// never returns it, so every failure is the 0 case.
    pub fn load(term: &std::ffi::OsStr) -> Option<Terminfo> {
        Terminfo::parse(&read_terminfo(term.as_bytes())?)
    }

    /// Parses the header, booleans, numbers and strings of a compiled entry in
    /// the 16-bit (0x011a) or 32-bit (0x021e) number format.  The extended
    /// (user-defined) section is ignored: none of the capabilities read here
    /// lives there.
    pub fn parse(b: &[u8]) -> Option<Terminfo> {
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

    pub fn str(&self, i: usize) -> Option<&[u8]> {
        self.strs.get(i)?.as_deref()
    }

    /// `tgetflag(id)`; `pt` is the obsolete `OTpt`.
    pub fn flag(&self, id: &str) -> bool {
        let i = match id {
            "am" => 1,
            "xo" => 20, // xon
            "nx" => 21, // nxon
            "pt" => 42, // OTpt
            _ => return false,
        };
        self.bools.get(i) == Some(&true)
    }

    /// `tgetnum(id)`, -1 when absent; `ug` is `UTug`.
    pub fn num(&self, id: &str) -> i32 {
        let i = match id {
            "co" => 0, // cols
            "li" => 2, // lines
            "pb" => 5,
            "ug" => 33, // UTug
            _ => return -1,
        };
        self.nums.get(i).copied().unwrap_or(-1)
    }

    /// The screen size ncurses `tgetent()` stores for `li`/`co`: `LINES` and
    /// `COLUMNS`, else `TIOCGWINSZ` on fd 1, else the database, else 24x80.
    pub fn screensize(&self) -> (i32, i32) {
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
            lines = self.num("li");
        }
        if cols <= 0 {
            cols = self.num("co");
        }
        if lines <= 0 {
            lines = 24;
        }
        if cols <= 0 {
            cols = 80;
        }
        (lines, cols)
    }

    /// ncurses `_nc_trim_sgr0()`: `tgetstr("me")` returns `sgr0` without the
    /// part that leaves the alternate character set.
    pub fn trim_sgr0(&self, sgr0: &[u8]) -> Vec<u8> {
        let Some(sgr) = self.str(TI_SGR) else {
            return sgr0.to_vec();
        };
        let smacs = self.str(TI_SMACS);
        let rmacs = self.str(TI_RMACS);
        let (Some(mut on), Some(mut off)) = (set_attribute_9(sgr, 1), set_attribute_9(sgr, 0))
        else {
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

/*------------------------------------------------------------------------*/
/* Helpers for `trim_sgr0`                                                 */

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

/*------------------------------------------------------------------------*/
/* tputs, tgoto, tparm                                                     */

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
        let ti = Terminfo::parse(&f).unwrap();
        assert_eq!(ti.bools, [true, false, true]);
        assert_eq!(ti.nums, [80, -1, 25]);
        assert_eq!(ti.strs, [None, Some(b"ab".to_vec()), None]);
        assert!(Terminfo::parse(&f[..f.len() - 1]).is_none());
        let mut bad = f.clone();
        bad[0] = 0x1b;
        assert!(Terminfo::parse(&bad).is_none());
        let mut bad = f.clone();
        bad[14] = 2;
        assert!(Terminfo::parse(&bad).is_none());
    }
    #[test]
    fn env_num() {
        assert_eq!(getenv_num("ELVIS_TEST_UNSET_VARIABLE"), -1);
    }
}
