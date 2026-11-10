//! `vars.c`: variables shared by the whole editor, plus the `Editor`
//! struct that owns every module's state (IMPLEMENTATION.md 3.1).

use std::fs::File;

use crate::vi_h::{Blk, MAXBLKS, Mark, Mode, NMARKS, Viflags};
use crate::{
    blk, cmd1, cmd2, curses, cut, ex, misc, move2, move3, opts, recycle, redraw, regexp, regsub,
    tio, tmp, vcmd, vi,
};

/// Capacity of `args[BLKSIZE]`.
pub const ARGS_SIZE: usize = crate::vi_h::BLKSIZE;
/// Capacity of `origname[256]` and `prevorig[256]`.
pub const NAME_SIZE: usize = 256;

pub struct Vars {
    pub viflags: Viflags,
    pub lnum: [i64; MAXBLKS],
    pub nlines: i64,
    /// `tmpfd`; `None` is C's -1.
    pub tmpfd: Option<File>,
    pub origtime: i64,
    pub origname: Vec<u8>,
    pub prevorig: Vec<u8>,
    pub prevline: i64,
    pub mark: [Mark; NMARKS],
    pub cursor: Mark,
    pub mode: Mode,
    pub args: Vec<u8>,
    pub argno: i32,
    pub nargs: i32,
    pub mustredraw: bool,
    pub changes: i64,
    pub significant: bool,
    /// Set when `add` refuses text that would make a line too long (B16).
    pub toolong: bool,
    pub plen: i32,
    pub pline: i64,
    pub pchgs: i64,
    /// Copy of the line last fetched by `pfetch` (C pointed into a cache).
    pub ptext: Vec<u8>,
    pub tmpblk: Blk,
    pub topline: i64,
    pub leftcol: i32,
    pub physcol: i32,
    pub physrow: i32,
    pub exwrote: bool,
    pub doingdot: bool,
    pub doingglobal: bool,
    pub rptlines: i64,
    /// C points `rptlabel` at string literals only.
    pub rptlabel: &'static [u8],
    pub u_line: i64,
    pub u_text: Vec<u8>,
}

impl Default for Vars {
    fn default() -> Vars {
        Vars {
            viflags: Viflags::default(),
            lnum: [0; MAXBLKS],
            nlines: 0,
            tmpfd: None,
            origtime: 0,
            origname: Vec::new(),
            prevorig: Vec::new(),
            prevline: 1,
            mark: [0; NMARKS],
            cursor: 0,
            mode: Mode::Ex,
            args: Vec::new(),
            argno: 0,
            nargs: 0,
            mustredraw: false,
            changes: 0,
            significant: false,
            toolong: false,
            plen: 0,
            pline: 0,
            pchgs: 0,
            ptext: Vec::new(),
            tmpblk: Blk::default(),
            topline: 0,
            leftcol: 0,
            physcol: 0,
            physrow: 0,
            exwrote: false,
            doingdot: false,
            doingglobal: false,
            rptlines: 0,
            rptlabel: b"",
            u_line: 0,
            u_text: Vec::new(),
        }
    }
}

/// All editor state.  Created once in `main` and passed as `&mut Editor`.
#[derive(Default)]
pub struct Editor {
    pub vars: Vars,
    pub blk: blk::BlkState,
    pub recycle: recycle::Recycle,
    pub tmp: tmp::Tmp,
    pub misc: misc::Misc,
    pub cut: cut::Cut,
    pub regexp: regexp::RegexpState,
    pub regsub: regsub::Regsub,
    pub opts: opts::Opts,
    pub ex: ex::Ex,
    pub cmd1: cmd1::Cmd1,
    pub cmd2: cmd2::Cmd2,
    pub curses: curses::Curses,
    pub tio: tio::Tio,
    pub move2: move2::Move2,
    pub move3: move3::Move3,
    pub vcmd: vcmd::Vcmd,
    pub vi: vi::Vi,
    pub redraw: redraw::Redraw,
}

impl Editor {
    pub fn new() -> Editor {
        Editor::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_values_match_c() {
        let ed = Editor::new();
        assert!(ed.vars.tmpfd.is_none());
        assert_eq!(ed.vars.prevline, 1);
        assert_eq!(ed.vars.nlines, 0);
        assert_eq!(ed.vars.mark, [0; NMARKS]);
        assert_eq!(ed.vars.viflags.file, 0);
        assert_eq!(ed.blk.recycle, 0);
        assert_eq!(ed.opts.num(opts::Opt::Tabstop), 8);
    }
}
