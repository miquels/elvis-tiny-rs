//! `opts.c`: run-time options.  Option values keep the C `char` array
//! storage: numbers and booleans are `i8` (so a maximum of 255 is stored
//! as -1), strings are NUL-terminated byte arrays of the C size.

use crate::curses;
use crate::vars::Editor;

use std::os::unix::ffi::OsStrExt;

use crate::config::{EXRC, KEYWORDPRG, NEEDSYNC, SHELL, TMPDIR};
use crate::tio::{Arg, exrefresh, msg, sprintf};
use crate::vi_h::{READONLY, atoi, ch, cstr};

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
    /// `opts[i].flags`, which C changes at run time.
    pub flags: [u8; NOPTS],
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
            flags: default_flags(),
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

/* maximum width to permit for strings, including ="" */
const MAXWIDTH: i32 = 20;

const BOOL: u8 = 0;
const NUM: u8 = 1;
const STR: u8 = 2;
const SET: u8 = 0x01;
const CANSET: u8 = 0x02;
const RCSET: u8 = 0x06;
const MR: u8 = 0x40;

/// Which `o_*` array an `opts[]` entry points at.
#[derive(Clone, Copy, PartialEq, Eq)]
enum V {
    Autoindent,
    Autoprint,
    Autowrite,
    Columns,
    Directory,
    Edcompatible,
    Errorbells,
    Exrefresh,
    Ignorecase,
    Inputmode,
    Keytime,
    Keywordprg,
    Lines,
    List,
    Magic,
    Readonly,
    Report,
    Scroll,
    Shell,
    Showmatch,
    Shiftwidth,
    Sidescroll,
    Sync,
    Tabstop,
    Term,
    Vbell,
    Warn,
    Wrapmargin,
    Wrapscan,
}

struct OptDesc {
    name: &'static [u8],
    nm: &'static [u8],
    typ: u8,
    flags: u8,
    value: V,
}

const fn o(name: &'static [u8], nm: &'static [u8], typ: u8, flags: u8, value: V) -> OptDesc {
    OptDesc {
        name,
        nm,
        typ,
        flags,
        value,
    }
}

pub const NOPTS: usize = 29;

static OPTS: [OptDesc; NOPTS] = [
    o(b"autoindent", b"ai", BOOL, CANSET, V::Autoindent),
    o(b"autoprint", b"ap", BOOL, CANSET, V::Autoprint),
    o(b"autowrite", b"aw", BOOL, CANSET, V::Autowrite),
    o(b"columns", b"co", NUM, SET, V::Columns),
    o(b"directory", b"dir", STR, RCSET, V::Directory),
    o(b"edcompatible", b"ed", BOOL, CANSET, V::Edcompatible),
    o(b"errorbells", b"eb", BOOL, CANSET, V::Errorbells),
    o(b"exrefresh", b"er", BOOL, CANSET, V::Exrefresh),
    o(b"ignorecase", b"ic", BOOL, CANSET, V::Ignorecase),
    o(b"inputmode", b"im", BOOL, CANSET, V::Inputmode),
    o(b"keytime", b"kt", NUM, CANSET, V::Keytime),
    o(b"keywordprg", b"kp", STR, CANSET, V::Keywordprg),
    o(b"lines", b"ls", NUM, SET, V::Lines),
    o(b"list", b"li", BOOL, CANSET | MR, V::List),
    o(b"magic", b"ma", BOOL, CANSET, V::Magic),
    o(b"readonly", b"ro", BOOL, CANSET, V::Readonly),
    o(b"report", b"re", NUM, CANSET, V::Report),
    o(b"scroll", b"sc", NUM, CANSET, V::Scroll),
    o(b"shell", b"sh", STR, CANSET, V::Shell),
    o(b"showmatch", b"sm", BOOL, CANSET, V::Showmatch),
    o(b"shiftwidth", b"sw", NUM, CANSET, V::Shiftwidth),
    o(b"sidescroll", b"ss", NUM, CANSET, V::Sidescroll),
    o(b"sync", b"sy", BOOL, CANSET, V::Sync),
    o(b"tabstop", b"ts", NUM, CANSET | MR, V::Tabstop),
    o(b"term", b"te", STR, SET, V::Term),
    o(b"vbell", b"vb", BOOL, CANSET, V::Vbell),
    o(b"warn", b"wa", BOOL, CANSET, V::Warn),
    o(b"wrapmargin", b"wm", NUM, CANSET, V::Wrapmargin),
    o(b"wrapscan", b"ws", BOOL, CANSET, V::Wrapscan),
];

fn default_flags() -> [u8; NOPTS] {
    let mut f = [0; NOPTS];
    for (i, d) in OPTS.iter().enumerate() {
        f[i] = d.flags;
    }
    f
}

impl Opts {
    /// The value array of a BOOL or NUM option.
    fn num(&mut self, v: V) -> &mut [i8] {
        match v {
            V::Autoindent => &mut self.o_autoindent,
            V::Autoprint => &mut self.o_autoprint,
            V::Autowrite => &mut self.o_autowrite,
            V::Columns => &mut self.o_columns,
            V::Edcompatible => &mut self.o_edcompatible,
            V::Errorbells => &mut self.o_errorbells,
            V::Exrefresh => &mut self.o_exrefresh,
            V::Ignorecase => &mut self.o_ignorecase,
            V::Inputmode => &mut self.o_inputmode,
            V::Keytime => &mut self.o_keytime,
            V::Lines => &mut self.o_lines,
            V::List => &mut self.o_list,
            V::Magic => &mut self.o_magic,
            V::Readonly => &mut self.o_readonly,
            V::Report => &mut self.o_report,
            V::Scroll => &mut self.o_scroll,
            V::Showmatch => &mut self.o_showmatch,
            V::Shiftwidth => &mut self.o_shiftwidth,
            V::Sidescroll => &mut self.o_sidescroll,
            V::Sync => &mut self.o_sync,
            V::Tabstop => &mut self.o_tabstop,
            V::Vbell => &mut self.o_vbell,
            V::Warn => &mut self.o_warn,
            V::Wrapmargin => &mut self.o_wrapmargin,
            V::Wrapscan => &mut self.o_wrapscan,
            V::Directory | V::Keywordprg | V::Shell | V::Term => {
                unreachable!("string option")
            }
        }
    }

    /// The NUL-terminated buffer of a STR option.
    fn string(&mut self, v: V) -> &mut [u8] {
        match v {
            V::Directory => &mut self.o_directory,
            V::Keywordprg => &mut self.o_keywordprg,
            V::Shell => &mut self.o_shell,
            V::Term => &mut self.o_term,
            _ => unreachable!("not a string option"),
        }
    }
}

/// `strcpy` into an option buffer.  A value that does not fit (with its NUL)
/// is refused, where C would overflow into the next option.
fn setstr(buf: &mut [u8], val: &[u8]) -> bool {
    if val.len() >= buf.len() {
        return false;
    }
    buf[..val.len()].copy_from_slice(val);
    buf[val.len()] = 0;
    true
}

// C: opts.c:initopts
// Out of line: inlining this into `main` makes the binary larger.
#[inline(never)]
pub fn initopts(ed: &mut Editor) {
    if let Some(val) = crate::os::getenv("SHELL") {
        setstr(&mut ed.opts.o_shell, val.as_bytes());
    }

    if let Some(val) = crate::os::getenv("TERM") {
        setstr(&mut ed.opts.o_term, val.as_bytes());
    }

    ed.opts.o_scroll[0] = (ed.curses.lines / 2 - 1) as i8;

    if !ed.curses.has(curses::VB) {
        let mut i = 0;
        while OPTS[i].value != V::Vbell {
            i += 1;
        }
        ed.opts.flags[i] &= !CANSET;
        ed.opts.o_vbell[0] = 0;
    }
}

// C: opts.c:dumpopts
pub fn dumpopts(ed: &mut Editor, all: bool) {
    let mut col: i32 = 0;

    for (i, d) in OPTS.iter().enumerate() {
        if !all && ed.opts.flags[i] & SET == 0 {
            continue;
        }

        if col > 52 {
            ed.curses.addch(b'\n');
            col = 0;
        } else if col > 26 {
            while col < 52 {
                ed.curses.qaddch(b' ');
                col += 1;
            }
        } else if col > 0 {
            while col < 26 {
                ed.curses.qaddch(b' ');
                col += 1;
            }
        }

        match d.typ {
            BOOL => {
                if ed.opts.num(d.value)[0] == 0 {
                    ed.curses.qaddch(b'n');
                    ed.curses.qaddch(b'o');
                    col += 2;
                }
                ed.curses.qaddstr(d.name);
                col += d.name.len() as i32;
            }
            NUM => {
                let v = ed.opts.num(d.value)[0] as u8;
                let nbuf = sprintf(b"%-3d", &[Arg::D(v as i64)]);
                ed.curses.qaddstr(d.name);
                ed.curses.qaddch(b'=');
                ed.curses.qaddstr(&nbuf);
                col += 4 + d.name.len() as i32;
            }
            _ => {
                let val = cstr(ed.opts.string(d.value)).to_vec();
                ed.curses.qaddstr(d.name);
                ed.curses.qaddch(b'=');
                ed.curses.qaddch(b'"');
                ed.curses.qaddstr(&val);
                ed.curses.qaddch(b'"');
                col += 3 + d.name.len() as i32 + val.len() as i32;
            }
        }
        exrefresh(ed);
    }
    if col > 0 {
        ed.curses.addch(b'\n');
        exrefresh(ed);
    }
}

// C: opts.c:setopts
pub fn setopts(ed: &mut Editor, assignments: &[u8]) {
    let mut a = cstr(assignments).to_vec();
    a.push(0);

    let mut name = 0;
    while a[name] != 0 {
        if a[name] == b' ' || a[name] == b'\t' {
            name += 1;
            continue;
        }

        let mut scan = name;
        while a[scan].is_ascii_lowercase() {
            scan += 1;
        }
        let value;
        if a[scan] == b'=' {
            a[scan] = 0;
            scan += 1;
            if a[scan] == b'"' {
                scan += 1;
                value = Some(scan);
                while a[scan] != 0 && a[scan] != b'"' {
                    scan += 1;
                }
                if a[scan] != 0 {
                    a[scan] = 0;
                    scan += 1;
                }
            } else {
                value = Some(scan);
                while a[scan] != 0 && a[scan] != b' ' && a[scan] != b'\t' {
                    scan += 1;
                }
                if a[scan] != 0 {
                    a[scan] = 0;
                    scan += 1;
                }
            }
        } else {
            if a[scan] != 0 {
                a[scan] = 0;
                scan += 1;
            }
            value = None;
            if a[name] == b'n' && a[name + 1] == b'o' {
                name += 2;
            }
        }

        let nm = cstr(&a[name..]);
        let mut i = 0;
        while i < NOPTS && OPTS[i].name != nm && OPTS[i].nm != nm {
            i += 1;
        }

        if i >= NOPTS {
            msg(ed, b"invalid option name \"%s\"", &[Arg::S(nm)]);
        } else if ed.opts.flags[i] & CANSET != CANSET {
            msg(ed, b"option \"%s\" can't be altered", &[Arg::S(nm)]);
        } else if ed.opts.flags[i] & RCSET != CANSET && ed.vars.nlines >= 1 {
            msg(
                ed,
                b"option \"%s\" can only be set in a %s file",
                &[Arg::S(nm), Arg::S(EXRC)],
            );
        } else if let Some(v) = value {
            let val = cstr(&a[v..]);
            let d = &OPTS[i];
            match d.typ {
                BOOL => {
                    msg(ed, b"option \"[no]%s\" is boolean", &[Arg::S(nm)]);
                }
                NUM => {
                    let j = atoi(val);
                    let (lo, hi) = {
                        let n = ed.opts.num(d.value);
                        (n[1] as i32, n[2] as i32 & 0xff)
                    };
                    if j == 0 && ch(val, 0) != b'0' {
                        msg(
                            ed,
                            b"option \"%s\" must have a numeric value",
                            &[Arg::S(nm)],
                        );
                    } else if j < lo || j > hi {
                        msg(
                            ed,
                            b"option \"%s\" must have a value between %d and %d",
                            &[Arg::S(nm), Arg::D(lo as i64), Arg::D(hi as i64)],
                        );
                    } else {
                        ed.opts.num(d.value)[0] = atoi(val) as i8;
                        ed.opts.flags[i] |= SET;
                    }
                }
                _ => {
                    if setstr(ed.opts.string(d.value), val) {
                        ed.opts.flags[i] |= SET;
                    } else {
                        msg(ed, b"option \"%s\" is too long", &[Arg::S(nm)]);
                    }
                }
            }
            if ed.opts.flags[i] & MR != 0 {
                ed.vars.mustredraw = true;
            }
        } else if OPTS[i].typ == BOOL {
            // C tests name[-1]; at the very start that byte lies before the
            // string, and is never 'o' in practice.
            let prev = if name > 0 { a[name - 1] } else { 0 };
            ed.opts.num(OPTS[i].value)[0] = (prev != b'o') as i8;
            ed.opts.flags[i] |= SET;
            if ed.opts.flags[i] & MR != 0 {
                ed.vars.mustredraw = true;
            }
        } else {
            msg(ed, b"option \"%s\" must be given a value", &[Arg::S(nm)]);
        }

        name = scan;
    }

    if ed.opts.o_readonly[0] != 0 {
        ed.vars.viflags.setflag(READONLY);
    }
}

#[cfg(test)]
mod setopts_tests {
    use super::*;

    #[test]
    fn assignments() {
        let mut ed = Editor::new();
        setopts(&mut ed, b"ts=4 noai\tlist sw=200 kp=\"man -k\" ws");
        assert_eq!(ed.opts.o_tabstop[0], 4);
        assert_eq!(ed.opts.o_autoindent[0], 0);
        assert_eq!(ed.opts.o_list[0], 1);
        assert_eq!(ed.opts.o_shiftwidth[0], 200u8 as i8);
        assert_eq!(cstr(&ed.opts.o_keywordprg), b"man -k");
        assert!(ed.vars.mustredraw);
        let i = OPTS.iter().position(|d| d.name == b"tabstop").unwrap();
        assert_eq!(ed.opts.flags[i], CANSET | MR | SET);
        setopts(&mut ed, b"ai,ro");
        assert_eq!(ed.opts.o_autoindent[0], 1);
        assert_eq!(ed.opts.o_readonly[0], 1);
        assert!(ed.vars.viflags.tstflag(READONLY));
    }

    #[test]
    fn string_overflow_is_refused() {
        let mut buf = [0u8; 4];
        assert!(setstr(&mut buf, b"abc"));
        assert_eq!(&buf, b"abc\0");
        assert!(!setstr(&mut buf, b"abcd"));
        assert_eq!(&buf, b"abc\0");
    }

    #[test]
    fn atoi_like_c() {
        assert_eq!(atoi(b" 12x"), 12);
        assert_eq!(atoi(b"-3"), -3);
        assert_eq!(atoi(b"x"), 0);
    }
}
