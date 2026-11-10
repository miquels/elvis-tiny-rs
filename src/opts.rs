//! `opts.c`: run-time options.  Booleans and numbers keep C's one-`char`
//! storage, so a number above 127 is stored as its low byte; string options
//! keep the size of their C array as a limit.

use crate::curses;
use crate::vars::Editor;

use std::os::unix::ffi::OsStrExt;

use crate::config::{EXRC, KEYWORDPRG, NEEDSYNC, SHELL, TMPDIR};
use crate::tio::{Arg, exrefresh, msg, sprintf};
use crate::vi_h::{READONLY, atoi, ch, cstr};

/// An option, in the order of the C `opts[]` table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Opt {
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

pub const NOPTS: usize = 29;

pub struct Opts {
    /// The value of each BOOL and NUM option.
    vals: [u8; NOPTS],
    directory: Vec<u8>,
    keywordprg: Vec<u8>,
    shell: Vec<u8>,
    term: Vec<u8>,
    /// `opts[i].flags`, which C changes at run time.
    pub flags: [u8; NOPTS],
}

impl Default for Opts {
    fn default() -> Opts {
        let mut vals = [0; NOPTS];
        let mut flags = [0; NOPTS];
        for (i, d) in OPTS.iter().enumerate() {
            vals[i] = d.dflt;
            flags[i] = d.flags;
        }
        Opts {
            vals,
            directory: TMPDIR.to_vec(),
            keywordprg: KEYWORDPRG.to_vec(),
            shell: SHELL.to_vec(),
            term: b"?".to_vec(),
            flags,
        }
    }
}

impl Opts {
    /// Whether a BOOL option is set.
    #[inline]
    pub fn on(&self, o: Opt) -> bool {
        self.vals[o as usize] != 0
    }

    #[inline]
    pub fn set(&mut self, o: Opt, on: bool) {
        self.vals[o as usize] = on as u8;
    }

    /// A NUM option's value, 0 to 255.
    #[inline]
    pub fn num(&self, o: Opt) -> i32 {
        self.vals[o as usize] as i32
    }

    /// Stores the low byte of `v`, as C's `char` assignment does.
    #[inline]
    pub fn set_num(&mut self, o: Opt, v: i32) {
        self.vals[o as usize] = v as u8;
    }

    /// `scroll`, which C reads as a signed `char`: setting it from a count
    /// over 127 makes it negative.
    #[inline]
    pub fn scroll(&self) -> i64 {
        self.vals[Opt::Scroll as usize] as i8 as i64
    }

    pub fn directory(&self) -> &[u8] {
        &self.directory
    }
    pub fn keywordprg(&self) -> &[u8] {
        &self.keywordprg
    }
    pub fn shell(&self) -> &[u8] {
        &self.shell
    }

    /// A STR option's value.
    pub fn str(&self, o: Opt) -> &[u8] {
        match o {
            Opt::Directory => &self.directory,
            Opt::Keywordprg => &self.keywordprg,
            Opt::Shell => &self.shell,
            _ => &self.term,
        }
    }

    /// `strcpy` into a STR option.  A value that does not fit the C array
    /// (with its NUL) is refused, where C would overflow into the next option.
    pub fn set_str(&mut self, o: Opt, val: &[u8]) -> bool {
        if val.len() >= OPTS[o as usize].max as usize {
            return false;
        }
        let s = match o {
            Opt::Directory => &mut self.directory,
            Opt::Keywordprg => &mut self.keywordprg,
            Opt::Shell => &mut self.shell,
            _ => &mut self.term,
        };
        s.clear();
        s.extend_from_slice(val);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_c() {
        let o = Opts::default();
        assert_eq!(o.directory(), b"/tmp");
        assert_eq!(o.shell(), b"/bin/sh");
        assert_eq!(o.keywordprg(), b"ref");
        assert_eq!(o.str(Opt::Term), b"?");
        let num = |n: Opt| {
            let d = &OPTS[n as usize];
            (o.num(n), d.min, d.max)
        };
        // C's {80, 32, 255} in a signed char array, compared with `& 0xff`
        assert_eq!(num(Opt::Columns), (80, 32, 255));
        assert_eq!(num(Opt::Shiftwidth), (8, 1, 255));
        assert_eq!(num(Opt::Wrapmargin), (0, 0, 255));
        assert_eq!(num(Opt::Keytime), (2, 0, 5));
        assert_eq!(num(Opt::Lines), (25, 2, 50));
        assert_eq!(num(Opt::Report), (5, 1, 127));
        assert_eq!(num(Opt::Scroll), (12, 1, 127));
        assert_eq!(num(Opt::Sidescroll), (8, 1, 40));
        assert_eq!(num(Opt::Tabstop), (8, 1, 40));
        use Opt::*;
        let on = [
            Autoprint, Errorbells, Exrefresh, Magic, Vbell, Warn, Wrapscan,
        ];
        assert!(on.iter().all(|&v| o.on(v)));
        let off = [
            Autoindent,
            Autowrite,
            Edcompatible,
            Ignorecase,
            Inputmode,
            List,
            Readonly,
            Showmatch,
            Sync,
        ];
        assert!(off.iter().all(|&v| !o.on(v)));
        for (i, d) in OPTS.iter().enumerate() {
            assert_eq!(i, d.opt as usize);
        }
    }

    #[test]
    fn scroll_reads_signed() {
        let mut o = Opts::default();
        o.set_num(Opt::Scroll, 200);
        assert_eq!(o.scroll(), -56);
        assert_eq!(o.num(Opt::Scroll), 200);
    }
}

const BOOL: u8 = 0;
const NUM: u8 = 1;
const STR: u8 = 2;
const SET: u8 = 0x01;
const CANSET: u8 = 0x02;
const RCSET: u8 = 0x06;
const MR: u8 = 0x40;

struct OptDesc {
    name: &'static [u8],
    nm: &'static [u8],
    typ: u8,
    flags: u8,
    opt: Opt,
    /// Initial value of a BOOL or NUM option.
    dflt: u8,
    /// Smallest value of a NUM option.
    min: u8,
    /// Largest value of a NUM option; size of a STR option's C array.
    max: u8,
}

const fn b(name: &'static [u8], nm: &'static [u8], flags: u8, opt: Opt, dflt: bool) -> OptDesc {
    OptDesc {
        name,
        nm,
        typ: BOOL,
        flags,
        opt,
        dflt: dflt as u8,
        min: 0,
        max: 0,
    }
}

const fn n(
    name: &'static [u8],
    nm: &'static [u8],
    flags: u8,
    opt: Opt,
    [dflt, min, max]: [u8; 3],
) -> OptDesc {
    OptDesc {
        name,
        nm,
        typ: NUM,
        flags,
        opt,
        dflt,
        min,
        max,
    }
}

const fn s(name: &'static [u8], nm: &'static [u8], flags: u8, opt: Opt, size: u8) -> OptDesc {
    OptDesc {
        name,
        nm,
        typ: STR,
        flags,
        opt,
        dflt: 0,
        min: 0,
        max: size,
    }
}

static OPTS: [OptDesc; NOPTS] = [
    b(b"autoindent", b"ai", CANSET, Opt::Autoindent, false),
    b(b"autoprint", b"ap", CANSET, Opt::Autoprint, true),
    b(b"autowrite", b"aw", CANSET, Opt::Autowrite, false),
    n(b"columns", b"co", SET, Opt::Columns, [80, 32, 255]),
    s(b"directory", b"dir", RCSET, Opt::Directory, 30),
    b(b"edcompatible", b"ed", CANSET, Opt::Edcompatible, false),
    b(b"errorbells", b"eb", CANSET, Opt::Errorbells, true),
    b(b"exrefresh", b"er", CANSET, Opt::Exrefresh, true),
    b(b"ignorecase", b"ic", CANSET, Opt::Ignorecase, false),
    b(b"inputmode", b"im", CANSET, Opt::Inputmode, false),
    n(b"keytime", b"kt", CANSET, Opt::Keytime, [2, 0, 5]),
    s(b"keywordprg", b"kp", CANSET, Opt::Keywordprg, 80),
    n(b"lines", b"ls", SET, Opt::Lines, [25, 2, 50]),
    b(b"list", b"li", CANSET | MR, Opt::List, false),
    b(b"magic", b"ma", CANSET, Opt::Magic, true),
    b(b"readonly", b"ro", CANSET, Opt::Readonly, false),
    n(b"report", b"re", CANSET, Opt::Report, [5, 1, 127]),
    n(b"scroll", b"sc", CANSET, Opt::Scroll, [12, 1, 127]),
    s(b"shell", b"sh", CANSET, Opt::Shell, 60),
    b(b"showmatch", b"sm", CANSET, Opt::Showmatch, false),
    n(b"shiftwidth", b"sw", CANSET, Opt::Shiftwidth, [8, 1, 255]),
    n(b"sidescroll", b"ss", CANSET, Opt::Sidescroll, [8, 1, 40]),
    b(b"sync", b"sy", CANSET, Opt::Sync, NEEDSYNC),
    n(b"tabstop", b"ts", CANSET | MR, Opt::Tabstop, [8, 1, 40]),
    s(b"term", b"te", SET, Opt::Term, 30),
    b(b"vbell", b"vb", CANSET, Opt::Vbell, true),
    b(b"warn", b"wa", CANSET, Opt::Warn, true),
    n(b"wrapmargin", b"wm", CANSET, Opt::Wrapmargin, [0, 0, 255]),
    b(b"wrapscan", b"ws", CANSET, Opt::Wrapscan, true),
];
// C: opts.c:initopts
// Out of line: inlining this into `main` makes the binary larger.
#[inline(never)]
pub fn initopts(ed: &mut Editor) {
    if let Some(val) = crate::os::getenv("SHELL") {
        ed.opts.set_str(Opt::Shell, val.as_bytes());
    }

    if let Some(val) = crate::os::getenv("TERM") {
        ed.opts.set_str(Opt::Term, val.as_bytes());
    }

    ed.opts.set_num(Opt::Scroll, ed.curses.lines / 2 - 1);

    if !ed.curses.has(curses::VB) {
        ed.opts.flags[Opt::Vbell as usize] &= !CANSET;
        ed.opts.set(Opt::Vbell, false);
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
                if !ed.opts.on(d.opt) {
                    ed.curses.qaddch(b'n');
                    ed.curses.qaddch(b'o');
                    col += 2;
                }
                ed.curses.qaddstr(d.name);
                col += d.name.len() as i32;
            }
            NUM => {
                let v = ed.opts.num(d.opt);
                let nbuf = sprintf(b"%-3d", &[Arg::D(v as i64)]);
                ed.curses.qaddstr(d.name);
                ed.curses.qaddch(b'=');
                ed.curses.qaddstr(&nbuf);
                col += 4 + d.name.len() as i32;
            }
            _ => {
                let val = ed.opts.str(d.opt);
                ed.curses.qaddstr(d.name);
                ed.curses.qaddch(b'=');
                ed.curses.qaddch(b'"');
                ed.curses.qaddstr(val);
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
                    let (lo, hi) = (d.min as i32, d.max as i32);
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
                        ed.opts.set_num(d.opt, atoi(val));
                        ed.opts.flags[i] |= SET;
                    }
                }
                _ => {
                    if ed.opts.set_str(d.opt, val) {
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
            ed.opts.set(OPTS[i].opt, prev != b'o');
            ed.opts.flags[i] |= SET;
            if ed.opts.flags[i] & MR != 0 {
                ed.vars.mustredraw = true;
            }
        } else {
            msg(ed, b"option \"%s\" must be given a value", &[Arg::S(nm)]);
        }

        name = scan;
    }

    if ed.opts.on(Opt::Readonly) {
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
        assert_eq!(ed.opts.num(Opt::Tabstop), 4);
        assert!(!ed.opts.on(Opt::Autoindent));
        assert!(ed.opts.on(Opt::List));
        assert_eq!(ed.opts.num(Opt::Shiftwidth), 200);
        assert_eq!(ed.opts.keywordprg(), b"man -k");
        assert!(ed.vars.mustredraw);
        let i = OPTS.iter().position(|d| d.name == b"tabstop").unwrap();
        assert_eq!(ed.opts.flags[i], CANSET | MR | SET);
        setopts(&mut ed, b"ai,ro");
        assert!(ed.opts.on(Opt::Autoindent));
        assert!(ed.opts.on(Opt::Readonly));
        assert!(ed.vars.viflags.tstflag(READONLY));
    }

    #[test]
    fn string_overflow_is_refused() {
        let mut o = Opts::default();
        assert!(o.set_str(Opt::Term, &[b'x'; 29]));
        assert_eq!(o.str(Opt::Term), &[b'x'; 29]);
        assert!(!o.set_str(Opt::Term, &[b'y'; 30]));
        assert_eq!(o.str(Opt::Term), &[b'x'; 29]);
    }

    #[test]
    fn atoi_like_c() {
        assert_eq!(atoi(b" 12x"), 12);
        assert_eq!(atoi(b"-3"), -3);
        assert_eq!(atoi(b"x"), 0);
    }
}
