//! `regsub.c`: substitution of matched text.

use crate::opts::Opt;
use crate::regexp::Regexp;
use crate::types::{ch, cstr};
use crate::vars::Editor;

#[derive(Default)]
pub struct Regsub {
    pub previous: Option<Vec<u8>>,
}

fn copy_sub(re: &Regexp, subject: &[u8], n: usize, dst: &mut Vec<u8>) {
    if let (Some(s), Some(e)) = (re.startp[n], re.endp[n]) {
        let e = e.min(subject.len());
        if s < e {
            dst.extend_from_slice(&subject[s..e]);
        }
    }
}

// C: regsub.c:regsub
/// `subject` is the string `re` last matched; output is appended to `dst`.
/// C wrote into the caller's buffer without a bound; callers check the size.
pub fn regsub(ed: &mut Editor, re: &Regexp, subject: &[u8], src: &[u8], dst: &mut Vec<u8>) {
    let magic = ed.opts.on(Opt::Magic);
    let src = cstr(src);
    let mut i = 0;
    loop {
        let mut c = ch(src, i);
        i += 1;
        if c == 0 {
            break;
        }
        if c == b'&' && magic {
            copy_sub(re, subject, 0, dst);
        } else if c == b'~' && magic {
            if let Some(p) = &ed.regsub.previous {
                dst.extend_from_slice(p);
            }
        } else if c == b'\\' {
            c = ch(src, i);
            i += 1;
            match c {
                b'0'..=b'9' => copy_sub(re, subject, (c - b'0') as usize, dst),
                b'&' => {
                    if magic {
                        dst.push(c);
                    } else {
                        copy_sub(re, subject, 0, dst);
                    }
                }
                b'~' => {
                    if magic {
                        dst.push(c);
                    } else if let Some(p) = &ed.regsub.previous {
                        dst.extend_from_slice(p);
                    }
                }
                // A trailing backslash: C stores a NUL (ending the result)
                // and then reads past the end of `src`.
                0 => break,
                _ => dst.push(c),
            }
        } else {
            dst.push(c);
        }
    }

    if !(ch(src, 0) == b'~' && magic) {
        ed.regsub.previous = Some(src.to_vec());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn re_with(caps: &[(usize, usize)]) -> Regexp {
        let mut re = Regexp::default();
        for (i, &(s, e)) in caps.iter().enumerate() {
            re.startp[i] = Some(s);
            re.endp[i] = Some(e);
        }
        re
    }

    #[test]
    fn substitutions() {
        let mut ed = Editor::new();
        let re = re_with(&[(1, 4), (2, 3)]);
        let mut d = Vec::new();
        regsub(&mut ed, &re, b"xabcx", b"[&|\\1|\\&|\\q]", &mut d);
        assert_eq!(d, b"[abc|b|&|q]");
        assert_eq!(ed.regsub.previous.as_deref(), Some(&b"[&|\\1|\\&|\\q]"[..]));

        let mut d = Vec::new();
        regsub(&mut ed, &re, b"xabcx", b"~!", &mut d);
        assert_eq!(d, b"[&|\\1|\\&|\\q]!");
        // a pattern starting with ~ leaves `previous` alone
        assert_eq!(ed.regsub.previous.as_deref(), Some(&b"[&|\\1|\\&|\\q]"[..]));

        let mut d = Vec::new();
        regsub(&mut ed, &re, b"xabcx", b"a\\", &mut d);
        assert_eq!(d, b"a");

        ed.opts.set(Opt::Magic, false);
        let mut d = Vec::new();
        regsub(&mut ed, &re, b"xabcx", b"&\\&\\~", &mut d);
        assert_eq!(d, b"&abca\\");
        // unset subexpression copies nothing
        let mut d = Vec::new();
        regsub(&mut ed, &re, b"xabcx", b"<\\5>", &mut d);
        assert_eq!(d, b"<>");
    }
}
