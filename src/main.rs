//! elvis-tiny, ported from C.  See IMPLEMENTATION.md.

#![deny(unsafe_code)]

mod blk;
mod cmd1;
mod cmd2;
mod config;
mod curses;
mod cut;
mod ex;
mod fmt;
mod input;
mod modify;
mod motion;
mod opts;
#[allow(unsafe_code)]
mod os;
mod recycle;
mod redraw;
mod regexp;
mod regsub;
mod system;
mod tio;
mod tmp;
mod vars;
mod vcmd;
mod vi;
mod vi_h;

use crate::opts::Opt;
use crate::vi_h::Cmd;
use crate::vi_h::Mode;
use std::os::unix::ffi::OsStrExt;

use vi_h::{MARK_FIRST, When, ch};

// C: main.c:main
fn main() {
    os::init();
    let mut editor = vars::Editor::new();
    let ed = &mut editor;

    let args: Vec<std::ffi::OsString> = std::env::args_os().collect();
    let argv: Vec<&[u8]> = args.iter().map(|a| a.as_bytes()).collect();
    let argc = argv.len();
    let mut cmd: Option<Vec<u8>> = None;
    let mut tag: Option<Vec<u8>> = None;
    let firstarg: Vec<u8>;

    match argv.first().and_then(|a| a.last().copied()).unwrap_or(0) {
        b'x' => {
            ed.vars.mode = Mode::Ex;
        }
        b'w' => {
            ed.vars.mode = Mode::Vi;
            ed.opts.set(Opt::Readonly, true);
        }
        b't' => {
            ed.vars.mode = Mode::Vi;
            ed.opts.set(Opt::Inputmode, true);
        }
        _ => {
            ed.vars.mode = Mode::Vi;
        }
    }

    os::ignore_sigquit();

    os::ignore_sigint();

    curses::initscr(ed);

    opts::initopts(ed);

    const ARROWS: [(usize, &[u8], &[u8]); 8] = [
        (curses::KU, b"k", b"<Up>"),
        (curses::KD, b"j", b"<Down>"),
        (curses::KL, b"h", b"<Left>"),
        (curses::KR, b"l", b"<Right>"),
        (curses::HM, b"^", b"<Home>"),
        (curses::EN, b"$", b"<End>"),
        (curses::PU, b"\x02", b"<PgUp>"),
        (curses::PD, b"\x06", b"<PgDn>"),
    ];
    for (slot, cooked, name) in ARROWS {
        if let Some(cap) = ed.curses.caps[slot].clone() {
            tio::mapkey(ed, &cap, cooked, When::VICMD | When::INMV, Some(name));
        }
    }
    if ed.curses.erasekey != 0o177 {
        tio::mapkey(ed, b"\x7f", b"x", When::VICMD | When::INMV, Some(b"<Del>"));
    }

    let mut i = 1;
    while i < argc && argv[i].first() == Some(&b'-') {
        match ch(argv[i], 1) {
            b'R' => {
                ed.opts.set(Opt::Readonly, true);
            }

            b'r' => {
                tio::msg(ed, b"Use the `virec` program to recover lost files", &[]);
                tio::endmsgs(ed);
                ed.curses.refresh();
                os::sleep_secs(1);
                curses::endwin(ed);
                os::exit(0);
            }

            b't' => {
                if ch(argv[i], 2) != 0 {
                    tag = Some(argv[i][2..].to_vec());
                } else {
                    i += 1;
                    tag = argv.get(i).map(|a| a.to_vec());
                }
            }

            b'v' => {
                ed.vars.mode = Mode::Vi;
            }

            b'e' => {
                ed.vars.mode = Mode::Ex;
            }
            b'i' => {
                ed.opts.set(Opt::Inputmode, true);
            }
            _ => {
                tio::msg(ed, b"Ignoring unknown flag \"%s\"", &[fmt::Arg::S(argv[i])]);
            }
        }
        i += 1;
    }

    if i < argc && argv[i].first() == Some(&b'+') {
        if ch(argv[i], 1) != 0 {
            cmd = Some(argv[i][1..].to_vec());
            i += 1;
        } else {
            cmd = Some(b"$".to_vec());
            i += 1;
        }
    }

    ed.vars.nargs = argc as i32 - i as i32;
    if ed.vars.nargs > 0 {
        firstarg = argv[i].to_vec();
        ed.vars.args.clear();
        while i < argc {
            if argv[i].len() > 128 {
                tio::msg(ed, b"file name too long", &[]);
                tio::endmsgs(ed);
                ed.curses.refresh();
                os::sleep_secs(1);
                curses::endwin(ed);
                os::exit(1);
            }
            if ed.vars.args.len() + argv[i].len() + 1 >= vars::ARGS_SIZE {
                tio::msg(ed, b"too many files on command line", &[]);
                tio::endmsgs(ed);
                ed.curses.refresh();
                os::sleep_secs(1);
                curses::endwin(ed);
                os::exit(1);
            }
            if !ed.vars.args.is_empty() {
                ed.vars.args.push(b' ');
            }
            ed.vars.args.extend_from_slice(argv[i]);
            i += 1;
        }
    } else {
        firstarg = Vec::new();
    }
    ed.vars.argno = 0;

    // SIGINT is still ignored here, so these cannot be interrupted unless an
    // .exrc command resumed curses; C would then jump through an unset jmpenv.
    if let Some(home) = crate::os::getenv("HOME") {
        let mut path = home.as_bytes().to_vec();
        path.push(config::SLASH);
        path.extend_from_slice(config::HMEXRC);
        path.truncate(vi_h::BLKSIZE - 1);
        let _ = ex::doexrc(ed, &path);
    }
    let _ = ex::doexrc(ed, config::EXRC);
    if let Some(s) = crate::os::getenv(config::EXINIT) {
        let _ = ex::exstring(ed, s.as_bytes());
    }

    blk::blkinit(ed);
    if let Some(tag) = &tag {
        let _ = cmd1::cmd_tag(ed, MARK_FIRST, MARK_FIRST, Cmd::Tag, false, tag);
    }

    if ed.vars.tmpfd.is_none() {
        tmp::tmpstart(ed, &firstarg);
        if !ed.vars.origname.is_empty() {
            blk::beforedo(ed, false);
            blk::afterdo(ed);
            ed.vars.viflags.clrflag(vi_h::MODIFIED);
        }
    }

    if let Some(cmd) = &cmd {
        let _ = ex::doexcmd(ed, cmd);
    }

    while ed.vars.mode != Mode::Quit {
        let mut jumped = false;
        loop {
            if jumped {
                blk::abortdo(ed);
            }
            os::trap_sigint();

            let r = match ed.vars.mode {
                Mode::Vi => vi::vi(ed),
                Mode::Ex => ex::ex(ed),
                _ => Ok(()),
            };
            if r.is_err() {
                // C: trapint(), then the siglongjmp back to the loop body
                curses::resume_curses(ed, false);
                blk::abortdo(ed);
                jumped = true;
                continue;
            }
            break;
        }
    }

    cut::cutend(ed);

    tio::endmsgs(ed);
    ed.curses.clear_last_line();
    ed.curses.refresh();
    curses::endwin(ed);
    os::exit(0);
}
