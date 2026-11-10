//! `opts.c`: run-time options.  Option values keep the C `char` array
//! storage: numbers and booleans are `i8` (so a maximum of 255 is stored
//! as -1), strings are NUL-terminated byte arrays of the C size.

use crate::config::{KEYWORDPRG, NEEDSYNC, SHELL, TMPDIR};

/// A NUL-terminated C string literal copied into a fixed array.
const fn cbuf<const N: usize>(s: &[u8]) -> [u8; N] {
    let mut a = [0u8; N];
    let mut i = 0;
    while i < s.len() {
        a[i] = s[i];
        i += 1;
    }
    a
}

pub struct Opts {
    pub o_autoindent: [i8; 1],
    pub o_autoprint: [i8; 1],
    pub o_autowrite: [i8; 1],
    pub o_columns: [i8; 3],
    pub o_directory: [u8; 30],
    pub o_edcompatible: [i8; 1],
    pub o_errorbells: [i8; 1],
    pub o_exrefresh: [i8; 1],
    pub o_ignorecase: [i8; 1],
    pub o_inputmode: [i8; 1],
    pub o_keytime: [i8; 3],
    pub o_keywordprg: [u8; 80],
    pub o_lines: [i8; 3],
    pub o_list: [i8; 1],
    pub o_magic: [i8; 1],
    pub o_readonly: [i8; 1],
    pub o_report: [i8; 3],
    pub o_scroll: [i8; 3],
    pub o_shell: [u8; 60],
    pub o_shiftwidth: [i8; 3],
    pub o_showmatch: [i8; 1],
    pub o_sidescroll: [i8; 3],
    pub o_sync: [i8; 1],
    pub o_tabstop: [i8; 3],
    pub o_term: [u8; 30],
    pub o_vbell: [i8; 1],
    pub o_warn: [i8; 1],
    pub o_wrapmargin: [i8; 3],
    pub o_wrapscan: [i8; 1],
}

impl Default for Opts {
    fn default() -> Opts {
        Opts {
            o_autoindent: [0],
            o_autoprint: [1],
            o_autowrite: [0],
            o_columns: [80, 32, 255u8 as i8],
            o_directory: cbuf(TMPDIR),
            o_edcompatible: [0],
            o_errorbells: [1],
            o_exrefresh: [1],
            o_ignorecase: [0],
            o_inputmode: [0],
            o_keytime: [2, 0, 5],
            o_keywordprg: cbuf(KEYWORDPRG),
            o_lines: [25, 2, 50],
            o_list: [0],
            o_magic: [1],
            o_readonly: [0],
            o_report: [5, 1, 127],
            o_scroll: [12, 1, 127],
            o_shell: cbuf(SHELL),
            o_shiftwidth: [8, 1, 255u8 as i8],
            o_showmatch: [0],
            o_sidescroll: [8, 1, 40],
            o_sync: [NEEDSYNC as i8],
            o_tabstop: [8, 1, 40],
            o_term: cbuf(b"?"),
            o_vbell: [1],
            o_warn: [1],
            o_wrapmargin: [0, 0, 255u8 as i8],
            o_wrapscan: [1],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vi_h::cstr;

    #[test]
    fn defaults_match_c() {
        let o = Opts::default();
        assert_eq!(cstr(&o.o_directory), b"/tmp");
        assert_eq!(cstr(&o.o_shell), b"/bin/sh");
        assert_eq!(cstr(&o.o_keywordprg), b"ref");
        assert_eq!(cstr(&o.o_term), b"?");
        // {80, 32, 255} in a signed char array: the maximum reads as -1,
        // and C compares it with `& 0xff`.
        assert_eq!(o.o_columns, [80, 32, -1]);
        assert_eq!(o.o_columns[2] as u8, 255);
        assert_eq!(o.o_shiftwidth[2], -1);
        assert_eq!(o.o_wrapmargin, [0, 0, -1]);
        assert_eq!(o.o_keytime, [2, 0, 5]);
        assert_eq!(o.o_lines, [25, 2, 50]);
        assert_eq!(o.o_report, [5, 1, 127]);
        assert_eq!(o.o_scroll, [12, 1, 127]);
        assert_eq!(o.o_sidescroll, [8, 1, 40]);
        assert_eq!(o.o_tabstop, [8, 1, 40]);
        let on = [
            o.o_autoprint,
            o.o_errorbells,
            o.o_exrefresh,
            o.o_magic,
            o.o_vbell,
            o.o_warn,
            o.o_wrapscan,
        ];
        assert!(on.iter().all(|v| v[0] == 1));
        let off = [
            o.o_autoindent,
            o.o_autowrite,
            o.o_edcompatible,
            o.o_ignorecase,
            o.o_inputmode,
            o.o_list,
            o.o_readonly,
            o.o_showmatch,
            o.o_sync,
        ];
        assert!(off.iter().all(|v| v[0] == 0));
    }
}
