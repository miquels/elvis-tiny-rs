//! The `printf` subset used by `msg()` and the `sprintf()` call sites.

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
    if let Some(p) = prec.filter(|&p| digits.len() < p) {
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
}
