//! `regexp.c` and `regexp.h`: regular expression compiler and matcher.

use crate::fmt::Arg;
use crate::opts::Opt;
use crate::tio::msg;
use crate::types::{ch, cstr, isalnum, isascii, isupper, schar, tolower};
use crate::vars::Editor;

pub const NSUBEXP: usize = 10;

/// `struct regexp`; capture positions are offsets into the subject.
///
/// `program` holds the C program bytes followed by `SHADOW` bytes that stand
/// in for the `struct regexp` bytes C kept just before `program[0]`; see
/// `vbyte`.
#[derive(Debug, Clone, Default)]
pub struct Regexp {
    pub startp: [Option<usize>; NSUBEXP],
    pub endp: [Option<usize>; NSUBEXP],
    pub minlen: i32,
    pub first: u8,
    pub bol: bool,
    pub program: Vec<u8>,
}

#[derive(Default)]
pub struct RegexpState {
    pub previous: Option<Vec<u8>>,
    pub class_cnt: i32,
    pub start_cnt: i32,
    pub end_stk: [i32; NSUBEXP],
    pub end_sp: i32,
    /// Offset into the pattern being compiled.
    pub retext: usize,
}

const META: u8 = 0;

#[inline]
fn base_meta(m: i32) -> u8 {
    (m - 256) as u8
}
#[inline]
fn int_meta(c: i32) -> i32 {
    c + 256
}
#[inline]
fn is_meta(m: i32) -> bool {
    m >= 256
}
#[inline]
fn is_class(m: i32) -> bool {
    m >= m_class(0) && m <= m_class(9)
}
#[inline]
fn is_start(m: i32) -> bool {
    m >= m_start(0) && m <= m_start(9)
}
#[inline]
fn is_closure(m: i32) -> bool {
    (M_SPLAT..=M_QMARK).contains(&m)
}

const M_BEGLINE: i32 = 256;
const M_ENDLINE: i32 = 257;
const M_BEGWORD: i32 = 258;
const M_ENDWORD: i32 = 259;
const M_ANY: i32 = 260;
const M_SPLAT: i32 = 261;
const M_PLUS: i32 = 262;
const M_QMARK: i32 = 263;
const fn m_class(n: i32) -> i32 {
    264 + n
}
const fn m_start(n: i32) -> i32 {
    274 + n
}
const fn m_end(n: i32) -> i32 {
    284 + n
}

type Fail = &'static [u8];

/// In C a class bitmap is indexed with `ch >> 3` on a signed `char`, so bytes
/// 128..255 reach up to 15 bytes before the bitmap: into the previous class,
/// `program[0]`, and for class 0 the tail of `struct regexp` (`bol`, `first`,
/// `minlen`, and the last bytes of `endp[]`).  Those struct bytes are modelled
/// here: `minlen` and `first` exactly, the `endp` bytes and the `bol` byte in
/// a shadow area appended after the program.
const SHADOW: usize = 10;
const SH_BOL: usize = 9;

/// The byte at `program[idx]` as C would see it, `idx >= -15`.
fn vbyte(re: &Regexp, shadow: &[u8], idx: isize) -> u8 {
    match idx {
        0.. => re.program.get(idx as usize).copied().unwrap_or(0),
        -1 => shadow[SH_BOL],
        -2 => re.first,
        -6..=-3 => re.minlen.to_le_bytes()[(idx + 6) as usize],
        _ => shadow[(idx + 15).max(0) as usize],
    }
}

fn vbyte_or(re: &mut Regexp, shadow: &mut [u8; SHADOW], idx: isize, v: u8) {
    match idx {
        0.. => re.program[idx as usize] |= v,
        -1 => shadow[SH_BOL] |= v,
        -2 => re.first |= v,
        -6..=-3 => {
            let mut b = re.minlen.to_le_bytes();
            b[(idx + 6) as usize] |= v;
            re.minlen = i32::from_le_bytes(b);
        }
        _ => shadow[(idx + 15).max(0) as usize] |= v,
    }
}

/// A regexp under construction, plus its shadow bytes.
struct Build {
    re: Regexp,
    shadow: [u8; SHADOW],
}

// C: regexp.c:makeclass
fn makeclass(
    text: &[u8],
    mut pos: usize,
    mut bmap: Option<(&mut Build, isize)>,
) -> Result<usize, Fail> {
    let mut complement = false;

    if let Some((b, base)) = bmap.as_mut() {
        for i in 0..32 {
            b.re.program[*base as usize + i] = 0;
        }
    }

    if ch(text, pos) == b'^' {
        pos += 1;
        complement = true;
    }

    while ch(text, pos) != 0 && ch(text, pos) != b']' {
        if ch(text, pos + 1) == b'-' && ch(text, pos + 2) != 0 {
            let lo = schar(ch(text, pos));
            let hi = schar(ch(text, pos + 2));
            if lo > hi {
                return Err(b"Backwards span in []");
            }
            if let Some((b, base)) = bmap.as_mut() {
                for i in lo..=hi {
                    vbyte_or(
                        &mut b.re,
                        &mut b.shadow,
                        *base + (i >> 3) as isize,
                        1 << (i & 7),
                    );
                }
            }
            pos += 3;
        } else {
            let i = schar(ch(text, pos));
            pos += 1;
            if let Some((b, base)) = bmap.as_mut() {
                vbyte_or(
                    &mut b.re,
                    &mut b.shadow,
                    *base + (i >> 3) as isize,
                    1 << (i & 7),
                );
            }
        }
    }

    if ch(text, pos) != b']' {
        return Err(b"] missing");
    }
    pos += 1;

    if complement && let Some((b, base)) = bmap.as_mut() {
        for i in 0..32 {
            let p = &mut b.re.program[*base as usize + i];
            *p = !*p;
        }
    }

    Ok(pos)
}

// C: regexp.c:gettoken
fn gettoken(
    st: &mut RegexpState,
    magic: bool,
    text: &[u8],
    sptr: &mut usize,
    re: Option<&mut Build>,
) -> Result<i32, Fail> {
    let mut c = schar(ch(text, *sptr));
    *sptr += 1;
    if c == b'\\' as i32 {
        c = schar(ch(text, *sptr));
        *sptr += 1;
        match c as u8 {
            b'<' => Ok(M_BEGWORD),
            b'>' => Ok(M_ENDWORD),
            b'(' => {
                if st.start_cnt >= NSUBEXP as i32 {
                    return Err(b"Too many \\(s");
                }
                st.end_stk[st.end_sp as usize] = st.start_cnt;
                st.end_sp += 1;
                st.start_cnt += 1;
                Ok(m_start(st.start_cnt - 1))
            }
            b')' => {
                if st.end_sp <= 0 {
                    return Err(b"Mismatched \\)");
                }
                st.end_sp -= 1;
                Ok(m_end(st.end_stk[st.end_sp as usize]))
            }
            b'*' => Ok(if magic { c } else { M_SPLAT }),
            b'.' => Ok(if magic { c } else { M_ANY }),
            b'+' => Ok(M_PLUS),
            b'?' => Ok(M_QMARK),
            _ => Ok(c),
        }
    } else if magic {
        match c as u8 {
            b'^' => Ok(if *sptr == st.retext + 1 { M_BEGLINE } else { c }),
            b'$' => Ok(if ch(text, *sptr) == 0 { M_ENDLINE } else { c }),
            b'.' => Ok(M_ANY),
            b'*' => Ok(M_SPLAT),
            b'[' => {
                if st.class_cnt >= 10 {
                    return Err(b"Too many []s");
                }
                let bmap = re.map(|b| (b, 1 + 32 * st.class_cnt as isize));
                *sptr = makeclass(text, *sptr, bmap)?;
                st.class_cnt += 1;
                Ok(m_class(st.class_cnt - 1))
            }
            _ => Ok(c),
        }
    } else {
        match c as u8 {
            b'^' => Ok(if *sptr == st.retext + 1 { M_BEGLINE } else { c }),
            b'$' => Ok(if ch(text, *sptr) == 0 { M_ENDLINE } else { c }),
            _ => Ok(c),
        }
    }
}

// C: regexp.c:calcsize
fn calcsize(st: &mut RegexpState, magic: bool, text: &[u8]) -> Result<u32, Fail> {
    let mut pos = 0;
    st.retext = 0;
    st.class_cnt = 0;
    st.start_cnt = 1;
    st.end_sp = 0;
    let mut size: u32 = 5;
    loop {
        let token = gettoken(st, magic, text, &mut pos, None)?;
        if token == 0 {
            break;
        }
        if is_class(token) {
            size += 34;
        } else if is_meta(token) {
            size += 2;
        } else {
            size += 1;
        }
    }
    Ok(size)
}

fn add_meta(b: &mut Build, m: i32) {
    b.re.program.push(META);
    b.re.program.push(base_meta(m));
}

fn compile(st: &mut RegexpState, magic: bool, text: &[u8]) -> Result<Regexp, Fail> {
    st.class_cnt = 0;
    st.start_cnt = 1;
    st.end_sp = 0;
    st.retext = 0;
    let size = calcsize(st, magic, text)?;

    let mut b = Build {
        re: Regexp::default(),
        shadow: [0; SHADOW],
    };
    b.re.program = Vec::with_capacity(size as usize + SHADOW);
    b.re.program.resize(1 + 32 * st.class_cnt as usize, 0);
    b.re.program[0] = st.class_cnt as u8;
    let mut needfirst = true;
    st.class_cnt = 0;
    st.start_cnt = 1;
    st.end_sp = 0;
    st.retext = 0;
    let mut pos = 0;
    let mut token = m_start(0);
    let mut peek = gettoken(st, magic, text, &mut pos, Some(&mut b))?;
    while token != 0 {
        if is_closure(peek) {
            if is_start(token) {
                return Err(b"* or \\+ or \\? follows nothing");
            } else if is_meta(token) && token != M_ANY && !is_class(token) {
                return Err(b"* or \\+ or \\? can only follow a normal character or . or []");
            }

            add_meta(&mut b, peek);

            if needfirst && peek == M_PLUS && !is_meta(token) {
                b.re.first = token as u8;
            }
            needfirst = false;

            peek = gettoken(st, magic, text, &mut pos, Some(&mut b))?;
            if is_closure(peek) {
                return Err(b"* or \\+ or \\? doubled up");
            }
        } else if !is_meta(token) {
            if needfirst {
                b.re.first = token as u8;
                needfirst = false;
            }
            b.re.minlen = b.re.minlen.wrapping_add(1);
        } else if token == M_ANY || is_class(token) {
            needfirst = false;
            b.re.minlen = b.re.minlen.wrapping_add(1);
        }

        if token == M_BEGLINE {
            b.shadow[SH_BOL] = 1;
        } else if is_meta(token) {
            add_meta(&mut b, token);
        } else {
            b.re.program.push(token as u8);
        }

        token = peek;
        peek = gettoken(st, magic, text, &mut pos, Some(&mut b))?;
    }

    add_meta(&mut b, m_end(0));
    if st.end_sp > 0 {
        return Err(b"Not enough \\)s");
    }

    b.re.bol = b.shadow[SH_BOL] != 0;
    b.re.program.extend_from_slice(&b.shadow);
    Ok(b.re)
}

// C: regexp.c:regcomp
/// Calls `regerror()` and returns `None` where C longjmp'd.
pub fn regcomp(ed: &mut Editor, exp: &[u8]) -> Option<Regexp> {
    let exp = cstr(exp);
    let text = if exp.is_empty() {
        match &ed.regexp.previous {
            Some(p) => p.clone(),
            None => {
                regerror(ed, b"No previous RE");
                return None;
            }
        }
    } else {
        ed.regexp.previous = Some(exp.to_vec());
        exp.to_vec()
    };

    let magic = ed.opts.on(Opt::Magic);
    match compile(&mut ed.regexp, magic, &text) {
        Ok(re) => Some(re),
        Err(why) => {
            regerror(ed, why);
            None
        }
    }
}

/*---------------------------------------------------------------------------*/

fn shadow_of(re: &Regexp) -> [u8; SHADOW] {
    let mut s = [0; SHADOW];
    let n = re.program.len();
    if n >= SHADOW {
        s.copy_from_slice(&re.program[n - SHADOW..]);
    }
    s
}

// C: regexp.c:match1
/// Whether `chr` matches the single-character `token`.
fn match1(re: &Regexp, ic: bool, chr: u8, token: i32) -> bool {
    let c = schar(chr);
    if c == 0 {
        return false;
    }
    if token == M_ANY {
        return true;
    } else if is_class(token) {
        let idx = 1 + 32 * (token - m_class(0)) as isize + (c >> 3) as isize;
        if vbyte(re, &shadow_of(re), idx) & (1 << (c & 7)) != 0 {
            return true;
        }
    } else if c == token || (ic && isupper(c) && tolower(c) == token) {
        return true;
    }
    false
}

fn get_meta(program: &[u8], p: &mut usize) -> i32 {
    if ch(program, *p) == META {
        *p += 1;
        int_meta(ch(program, *p) as i32)
    } else {
        schar(program[*p])
    }
}

// C: regexp.c:match
/// Whether the program at `prog` matches `s` at `here`.
fn match_(re: &mut Regexp, ic: bool, s: &[u8], mut prog: usize, mut here: isize) -> bool {
    let mut token = get_meta(&re.program, &mut prog);
    while !is_closure(token) {
        match token {
            M_ENDLINE => {
                if ch(s, here as usize) != 0 {
                    return false;
                }
            }
            M_BEGWORD => {
                if here != 0 {
                    let p = schar(ch(s, here as usize - 1));
                    if p == b'_' as i32 || (isascii(p) && isalnum(p)) {
                        return false;
                    }
                }
            }
            M_ENDWORD => {
                let p = schar(ch(s, here as usize));
                if p == b'_' as i32 || (isascii(p) && isalnum(p)) {
                    return false;
                }
            }
            t if (m_start(0)..=m_start(9)).contains(&t) => {
                re.startp[(t - m_start(0)) as usize] = Some(here as usize);
            }
            t if (m_end(0)..=m_end(9)).contains(&t) => {
                re.endp[(t - m_end(0)) as usize] = Some(here as usize);
                if t == m_end(0) {
                    return true;
                }
            }
            _ => {
                if !match1(re, ic, ch(s, here as usize), token) {
                    return false;
                }
                here += 1;
            }
        }
        prog += 1;
        token = get_meta(&re.program, &mut prog);
    }

    let closure = token;
    prog += 1;
    token = get_meta(&re.program, &mut prog);
    prog += 1;

    let mut nmatched: i32 = 0;
    while (closure != M_QMARK || nmatched < 1)
        && ch(s, here as usize) != 0
        && match1(re, ic, ch(s, here as usize), token)
    {
        nmatched += 1;
        here += 1;
    }

    while nmatched >= 0 && !match_(re, ic, s, prog, here) {
        nmatched -= 1;
        here -= 1;
    }

    nmatched >= if closure == M_PLUS { 1 } else { 0 }
}

// C: regexp.c:regexec
/// `string` has no NUL terminator; capture offsets index into it.
pub fn regexec(ed: &mut Editor, prog: &mut Regexp, string: &[u8], bolflag: bool) -> bool {
    let re = prog;
    if re.bol && !bolflag {
        return false;
    }
    if re.program.len() <= SHADOW {
        return false;
    }
    let ic = ed.opts.on(Opt::Ignorecase);

    let slen = cstr(string).len();
    let mut len = slen as i64;
    let entry = 1 + 32 * schar(re.program[0]) as isize;
    if entry < 0 || entry as usize >= re.program.len() - SHADOW {
        return false;
    }
    let entry = entry as usize;

    if re.bol {
        if (re.first != 0 && !match1(re, ic, ch(string, 0), schar(re.first)))
            || len < re.minlen as i64
            || !match_(re, ic, string, entry, 0)
        {
            return false;
        }
    } else {
        let mut here: usize = 0;
        while (re.first != 0 && !match1(re, ic, ch(string, here), schar(re.first)))
            || !match_(re, ic, string, entry, here as isize)
        {
            if len < re.minlen as i64 {
                return false;
            }
            here += 1;
            len -= 1;
            // C goes on to look at the byte after the terminating NUL.
            if here > slen {
                return false;
            }
        }
    }

    true
}

// C: misc.c:regerror
pub fn regerror(ed: &mut Editor, txt: &[u8]) {
    msg(ed, b"RE error: %s", &[Arg::S(txt)]);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn comp(ed: &mut Editor, p: &[u8]) -> Regexp {
        compile(&mut ed.regexp, ed.opts.on(Opt::Magic), p).expect("compiles")
    }

    fn find(ed: &mut Editor, p: &[u8], s: &[u8]) -> Option<(usize, usize)> {
        let mut re = comp(ed, p);
        if regexec(ed, &mut re, s, true) {
            Some((re.startp[0].unwrap(), re.endp[0].unwrap()))
        } else {
            None
        }
    }

    #[test]
    fn program_layout_and_size() {
        let mut ed = Editor::new();
        let re = comp(&mut ed, b"a[bc]*$");
        let n = re.program.len() - SHADOW;
        // class count, one bitmap, M_START(0), 'a', M_SPLAT, M_CLASS(0), M_ENDLINE, M_END(0)
        assert_eq!(n, 1 + 32 + 2 + 1 + 2 + 2 + 2 + 2);
        assert_eq!(re.program[0], 1);
        assert_eq!(&re.program[33..36], &[0, 18, b'a']);
        assert_eq!(re.first, b'a');
        assert_eq!(re.minlen, 1);
        let size = calcsize(&mut ed.regexp, true, b"a[bc]*$").unwrap();
        assert_eq!(size, 5 + 1 + 34 + 2 + 2);
    }

    #[test]
    fn matching() {
        let mut ed = Editor::new();
        assert_eq!(find(&mut ed, b"b+c", b"abbc"), None);
        assert_eq!(find(&mut ed, b"b\\+c", b"abbc"), Some((1, 4)));
        assert_eq!(find(&mut ed, b"^ab", b"xab"), None);
        assert_eq!(find(&mut ed, b"x*", b"abc"), Some((0, 0)));
        assert_eq!(find(&mut ed, b"c$", b"abc"), Some((2, 3)));
        assert_eq!(find(&mut ed, b"\\<b", b"ab b"), Some((3, 4)));
        assert_eq!(find(&mut ed, b"a\\>", b"ab a"), Some((3, 4)));
        assert_eq!(find(&mut ed, b"[^a-c]", b"abcd"), Some((3, 4)));
        assert_eq!(find(&mut ed, b"a.c", b"xxabc"), Some((2, 5)));
        assert_eq!(find(&mut ed, b"ab\\?c", b"ac"), Some((0, 2)));
        let mut re = comp(&mut ed, b"\\(a*\\)\\(b\\)");
        assert!(regexec(&mut ed, &mut re, b"xaab", true));
        assert_eq!((re.startp[1], re.endp[1]), (Some(1), Some(3)));
        assert_eq!((re.startp[2], re.endp[2]), (Some(3), Some(4)));
        let mut re = comp(&mut ed, b"^a");
        assert!(!regexec(&mut ed, &mut re, b"a", false));
        ed.opts.set(Opt::Ignorecase, true);
        assert_eq!(find(&mut ed, b"abc", b"xABC"), Some((1, 4)));
        ed.opts.set(Opt::Ignorecase, false);
        ed.opts.set(Opt::Magic, false);
        assert_eq!(find(&mut ed, b"a.c", b"abc a.c"), Some((4, 7)));
        assert_eq!(find(&mut ed, b"a\\.c", b"xabc"), Some((1, 4)));
    }

    #[test]
    fn errors() {
        let mut ed = Editor::new();
        let e = |ed: &mut Editor, p: &[u8]| compile(&mut ed.regexp, true, p).unwrap_err();
        assert_eq!(e(&mut ed, b"*a"), b"* or \\+ or \\? follows nothing");
        assert_eq!(e(&mut ed, b"[z-a]"), b"Backwards span in []");
        assert_eq!(e(&mut ed, b"[ab"), b"] missing");
        assert_eq!(e(&mut ed, b"\\(a"), b"Not enough \\)s");
        assert_eq!(e(&mut ed, b"a\\)"), b"Mismatched \\)");
        assert_eq!(e(&mut ed, b"a**"), b"* or \\+ or \\? doubled up");
        assert_eq!(
            e(&mut ed, b"\\<*"),
            b"* or \\+ or \\? can only follow a normal character or . or []"
        );
        assert_eq!(
            e(&mut ed, b"[a][a][a][a][a][a][a][a][a][a][a]"),
            b"Too many []s"
        );
    }

    #[test]
    fn high_bytes_in_class_use_c_layout() {
        let mut ed = Editor::new();
        // 0xc3 -> ch>>3 == -8 -> program[-7] (an `endp` byte in C)
        assert_eq!(find(&mut ed, b"[\xc3]", b"a\xc3"), Some((1, 2)));
        assert_eq!(find(&mut ed, b"[^a]", b"a\xc3"), None);
        // 0xd0 -> program[-5], a byte of `minlen`: the RE needs 257+ bytes.
        let re = comp(&mut ed, b"[\xd0]");
        assert_eq!(re.minlen, 1 + 256);
    }
}
