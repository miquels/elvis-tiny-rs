//! `cmd1.c`: ex commands (part 1).

use crate::opts::Opt;
use crate::vi_h::Mode;
use std::fs::File;

use crate::blk::{afterdo, beforedo};
use crate::config::{FILEPERMS, TAGS, tread, twrite};
use crate::curses::{resume_curses, suspend_curses};
use crate::ex::{doexcmd, linespec, parseptrn};
use crate::misc::fetchline;
use crate::opts::{dumpopts, setopts};
use crate::os;
use crate::regexp::{regcomp, regexec};
use crate::system::{elvis_system, filter};
use crate::tio::{Arg, dumpkey, exrefresh, mapkey, msg};
use crate::tmp::{elvis_access, tmpabort, tmpend, tmpsave, tmpstart};
use crate::vars::{ARGS_SIZE, Editor, NAME_SIZE};
use crate::vi_h::{
    BLKSIZE, COPYING, Cmd, Interrupted, MARK_FIRST, MARK_UNSET, MODIFIED, Mark, R, READONLY,
    VERSION, WHEN_VICMD, WHEN_VIINP, WHEN_VIREP, ch, cstr, isspace, mark_at_line, markline, schar,
};

/// Capacity of `prevextra[80]`.
const PREVEXTRA_SIZE: usize = 80;
/// Capacity of `prevtag[30]`.
const PREVTAG_SIZE: usize = 30;
/// Capacity of `cmdln[100]` in `cmd_global`.
const CMDLN_SIZE: usize = 100;

#[derive(Default)]
pub struct Cmd1 {
    /// `static char prevextra[80]` in `cmd_shell`.
    pub cmd_shell_prevextra: Vec<u8>,
    pub cmd_xit_whenwarned: i64,
    /// `static char prevtag[30]` in `cmd_tag`.
    pub cmd_tag_prevtag: Vec<u8>,
}

fn is_ascii_space(b: u8) -> bool {
    isspace(schar(b))
}

// C: cmd1.c:cmd_mark
pub fn cmd_mark(
    ed: &mut Editor,
    _frommark: Mark,
    tomark: Mark,
    _cmd: Cmd,
    _bang: bool,
    extra: &[u8],
) -> R<()> {
    let c = ch(extra, 0);
    if !c.is_ascii_lowercase() || ch(extra, 1) != 0 {
        msg(ed, b"Invalid mark name", &[]);
        return Ok(());
    }

    ed.vars.mark[(c - b'a') as usize] = tomark;
    Ok(())
}

// C: cmd1.c:cmd_write
pub fn cmd_write(
    ed: &mut Editor,
    frommark: Mark,
    tomark: Mark,
    _cmd: Cmd,
    bang: bool,
    extra: &[u8],
) -> R<()> {
    let mut extra = extra;

    if frommark == MARK_FIRST && tomark == mark_at_line(ed.vars.nlines) {
        tmpsave(ed, Some(extra), bang);
        return Ok(());
    }

    let mut append = false;
    if ch(extra, 0) == b'>' && ch(extra, 1) == b'>' {
        extra = &extra[2..];
        append = true;
    }

    if elvis_access(extra, 0) == 0 && !bang && !append {
        msg(ed, b"File already exists - Use :w! to overwrite", &[]);
        return Ok(());
    }

    let mut fd = if append {
        os::open(extra, os::O_WRONLY | os::O_APPEND, 0).ok()
    } else {
        None
    };

    if fd.is_none() {
        fd = os::creat(extra, FILEPERMS).ok();
        if fd.is_none() {
            msg(ed, b"Can't write to \"%s\"", &[Arg::S(extra)]);
            return Ok(());
        }
    }
    let Some(mut fd) = fd else { return Ok(()) };
    cmd_write2(ed, frommark, tomark, &mut fd)
}

// C: cmd1.c:cmd_write2
pub fn cmd_write2(ed: &mut Editor, frommark: Mark, tomark: Mark, fd: &mut File) -> R<()> {
    let mut l = markline(frommark);
    while l <= markline(tomark) {
        if os::take_sigint() {
            return Err(Interrupted);
        }
        let mut scan = fetchline(ed, l);
        scan.push(b'\n');
        twrite(fd, &scan);
        l += 1;
    }
    Ok(())
}

// C: cmd1.c:cmd_shell
pub fn cmd_shell(
    ed: &mut Editor,
    frommark: Mark,
    tomark: Mark,
    cmd: Cmd,
    _bang: bool,
    extra: &[u8],
) -> R<()> {
    let mut frommark = frommark;
    let mut tomark = tomark;
    // C compares the command pointer with `o_shell` to pick an interactive shell.
    let mut interactive = false;
    let mut extra = extra.to_vec();

    if cmd == Cmd::Shell {
        extra = ed.opts.shell().to_vec();
        interactive = true;
        frommark = 0;
        tomark = 0;
    }

    if ch(&extra, 0) == b'!' {
        if ed.cmd1.cmd_shell_prevextra.is_empty() {
            msg(ed, b"No previous shell command to substitute for '!'", &[]);
            return Ok(());
        }
        extra = ed.cmd1.cmd_shell_prevextra.clone();
        interactive = false;
    } else if cmd == Cmd::Bang && extra.len() < PREVEXTRA_SIZE - 1 {
        ed.cmd1.cmd_shell_prevextra = extra.clone();
    }

    suspend_curses(ed);
    if frommark == 0 {
        elvis_system(ed, if interactive { None } else { Some(&extra) });
    } else {
        filter(ed, frommark, tomark, &extra)?;
    }

    let quietly = ed.vars.mode == Mode::Ex;
    resume_curses(ed, quietly);
    Ok(())
}

// C: cmd1.c:cmd_global
pub fn cmd_global(
    ed: &mut Editor,
    frommark: Mark,
    tomark: Mark,
    cmd: Cmd,
    bang: bool,
    extra: &[u8],
) -> R<()> {
    let mut cmd = cmd;

    if ed.vars.doingglobal {
        msg(ed, b"Can't nest global commands.", &[]);
        ed.vars.rptlines = -1;
        return Ok(());
    }

    if bang {
        cmd = Cmd::VGlobal;
    }
    let gv = if cmd == Cmd::Global { b'g' } else { b'v' };

    if ch(extra, 0) != b'/' && ch(extra, 0) != b'?' {
        msg(ed, b"Usage: %c /regular expression/ command", &[Arg::C(gv)]);
        return Ok(());
    }

    let mut buf = extra.to_vec();
    buf.push(0);
    let cmdptr = parseptrn(&mut buf);
    if ch(&buf, 1) == 0 {
        msg(
            ed,
            b"Can't use empty regular expression with '%c' command",
            &[Arg::C(gv)],
        );
        return Ok(());
    }
    let Some(mut re) = regcomp(ed, cstr(&buf[1..])) else {
        return Ok(());
    };
    let cmdln = cstr(buf.get(cmdptr..).unwrap_or(&[]));
    if cmdln.len() >= CMDLN_SIZE {
        msg(ed, b"Global command too long", &[]);
        return Ok(());
    }

    ed.vars.doingglobal = true;
    beforedo(ed, false);
    let mut l = ed.vars.nlines - markline(frommark);
    let mut lqty = markline(tomark) - markline(frommark) + 1;
    let mut nchanged: i64 = 0;
    while lqty > 0 && ed.vars.nlines - l >= 0 && nchanged >= 0 {
        if os::take_sigint() {
            ed.vars.doingglobal = false;
            return Err(Interrupted);
        }
        let line = fetchline(ed, ed.vars.nlines - l);

        if regexec(ed, &mut re, &line, true) == (cmd == Cmd::Global) {
            ed.vars.cursor = mark_at_line(ed.vars.nlines - l);
            ed.vars.rptlines = 0;
            if let Err(e) = doexcmd(ed, cmdln) {
                ed.vars.doingglobal = false;
                return Err(e);
            }
            nchanged += ed.vars.rptlines;
        }
        l -= 1;
        lqty -= 1;
    }
    afterdo(ed);
    ed.vars.doingglobal = false;

    ed.vars.rptlines = nchanged;
    Ok(())
}

// C: cmd1.c:cmd_file
pub fn cmd_file(
    ed: &mut Editor,
    frommark: Mark,
    tomark: Mark,
    cmd: Cmd,
    _bang: bool,
    _extra: &[u8],
) -> R<()> {
    if cmd == Cmd::File {
        let name = if ed.vars.origname.is_empty() {
            b"[NO FILE]".to_vec()
        } else {
            ed.vars.origname.clone()
        };
        let modified: &[u8] = if ed.vars.viflags.tstflag(MODIFIED) {
            b"[MODIFIED]"
        } else {
            b""
        };
        let readonly: &[u8] = if ed.vars.viflags.tstflag(READONLY) {
            b"[READONLY]"
        } else {
            b""
        };
        let nlines = ed.vars.nlines;
        let pct = (markline(frommark) * 100).checked_div(nlines).unwrap_or(0);
        msg(
            ed,
            b"\"%s\" %s%s %ld lines,  line %ld [%ld%%]",
            &[
                Arg::S(&name),
                Arg::S(modified),
                Arg::S(readonly),
                Arg::D(nlines),
                Arg::D(markline(frommark)),
                Arg::D(pct),
            ],
        );
    } else if markline(frommark) == markline(tomark) {
        msg(ed, b"%ld", &[Arg::D(markline(frommark))]);
    } else {
        msg(
            ed,
            b"range \"%ld,%ld\" contains %ld lines",
            &[
                Arg::D(markline(frommark)),
                Arg::D(markline(tomark)),
                Arg::D(markline(tomark) - markline(frommark) + 1),
            ],
        );
    }
    Ok(())
}

// C: cmd1.c:cmd_edit
pub fn cmd_edit(
    ed: &mut Editor,
    _frommark: Mark,
    _tomark: Mark,
    _cmd: Cmd,
    bang: bool,
    extra: &[u8],
) -> R<()> {
    let mut line: i64 = 1;

    if extra == ed.vars.prevorig.as_slice() {
        line = ed.vars.prevline;
    }

    if tmpabort(ed, bang) {
        tmpstart(ed, extra);
        if line <= ed.vars.nlines && line >= 1 {
            ed.vars.cursor = mark_at_line(line);
        }
    } else {
        msg(ed, b"Use edit! to abort changes, or w to save changes", &[]);

        ed.vars.prevorig = extra[..extra.len().min(NAME_SIZE - 1)].to_vec();
        ed.vars.prevline = 1;
    }
    Ok(())
}

// C: cmd1.c:cmd_next
pub fn cmd_next(
    ed: &mut Editor,
    frommark: Mark,
    tomark: Mark,
    cmd: Cmd,
    bang: bool,
    extra: &[u8],
) -> R<()> {
    if cmd == Cmd::Next && !extra.is_empty() {
        cmd_args(ed, frommark, tomark, cmd, bang, extra)?;
    }

    let i = if cmd == Cmd::Next {
        ed.vars.argno + 1
    } else if cmd == Cmd::Previous {
        ed.vars.argno - 1
    } else {
        0
    };
    if i < 0 || i >= ed.vars.nargs {
        let more: &[u8] = if cmd == Cmd::Rewind { b"" } else { b"more " };
        msg(ed, b"No %sfiles to edit", &[Arg::S(more)]);
        return Ok(());
    }

    let args = &ed.vars.args;
    let mut scan = 0;
    for _ in 0..i {
        while scan < args.len() && !is_ascii_space(args[scan]) {
            scan += 1;
        }
        while scan < args.len() && is_ascii_space(args[scan]) {
            scan += 1;
        }
    }
    let mut build = Vec::new();
    while ch(args, scan) != 0 && !is_ascii_space(args[scan]) {
        build.push(args[scan]);
        scan += 1;
    }

    if tmpabort(ed, bang) {
        tmpstart(ed, &build);
        ed.vars.argno = i;
    } else {
        let name: &[u8] = if cmd == Cmd::Next {
            b"next"
        } else if cmd == Cmd::Previous {
            b"previous"
        } else {
            b"rewind"
        };
        msg(
            ed,
            b"Use :%s! to abort changes, or w to save changes",
            &[Arg::S(name)],
        );
    }
    Ok(())
}

// C: cmd1.c:cmd_xit
pub fn cmd_xit(
    ed: &mut Editor,
    _frommark: Mark,
    _tomark: Mark,
    cmd: Cmd,
    bang: bool,
    _extra: &[u8],
) -> R<()> {
    if ed.vars.argno + 1 < ed.vars.nargs
        && ed.cmd1.cmd_xit_whenwarned != ed.vars.changes
        && (!bang || cmd != Cmd::Quit)
    {
        msg(
            ed,
            b"More files to edit -- Use \":n\" to go to next file",
            &[],
        );
        ed.cmd1.cmd_xit_whenwarned = ed.vars.changes;
        return Ok(());
    }

    if cmd == Cmd::Quit {
        if tmpabort(ed, bang) {
            ed.vars.mode = Mode::Quit;
        } else {
            msg(ed, b"Use q! to abort changes, or wq to save changes", &[]);
        }
    } else {
        let oldflag = ed.vars.viflags.tstflag(MODIFIED);
        if cmd == Cmd::WriteQuit {
            ed.vars.viflags.setflag(MODIFIED);
        }
        if tmpend(ed, bang) {
            ed.vars.mode = Mode::Quit;
        } else {
            msg(
                ed,
                b"Could not save file -- use quit! to abort changes, or w filename",
                &[],
            );
        }
        if !oldflag {
            ed.vars.viflags.clrflag(MODIFIED);
        }
    }
    Ok(())
}

// C: cmd1.c:cmd_args
pub fn cmd_args(
    ed: &mut Editor,
    _frommark: Mark,
    _tomark: Mark,
    _cmd: Cmd,
    _bang: bool,
    extra: &[u8],
) -> R<()> {
    if extra.is_empty() {
        let args = ed.vars.args.clone();
        let mut scan = 0;
        let mut col: i32 = 0;
        let mut arg: i32 = 0;
        let mut scrolled = false;
        while ch(&args, scan) != 0 {
            while ch(&args, scan) != 0 && is_ascii_space(args[scan]) {
                scan += 1;
            }
            let mut eow = scan;
            while ch(&args, eow) != 0 && {
                eow += 1;
                !is_ascii_space(ch(&args, eow))
            } {}
            let addcols = if arg == ed.vars.argno { 2 } else { 0 };
            if col + addcols + (eow - scan) as i32 + 1 >= ed.curses.cols {
                ed.curses.addch(b'\n');
                scrolled = true;
                col = 0;
            } else if arg != 0 {
                ed.curses.qaddch(b' ');
                col += 1;
            }
            if arg == ed.vars.argno {
                ed.curses.qaddch(b'[');
            }
            while scan < eow {
                ed.curses.qaddch(args[scan]);
                scan += 1;
                col += 1;
            }
            if arg == ed.vars.argno {
                ed.curses.qaddch(b']');
            }
            arg += 1;
            col += addcols;
        }
        let mode = ed.vars.mode;
        if (mode == Mode::Ex || mode == Mode::Colon || scrolled) && col != 0 {
            ed.curses.addch(b'\n');
        }
        exrefresh(ed);
    } else {
        ed.vars.args = extra[..extra.len().min(ARGS_SIZE - 1)].to_vec();
        ed.vars.argno = -1;

        let args = &ed.vars.args;
        let mut nargs = 0;
        let mut scan = 0;
        while scan < args.len() {
            while scan < args.len() && !is_ascii_space(args[scan]) {
                scan += 1;
            }
            while scan < args.len() && is_ascii_space(args[scan]) {
                scan += 1;
            }
            nargs += 1;
        }
        ed.vars.nargs = nargs;
        msg(ed, b"%d files to edit", &[Arg::D(nargs as i64)]);
    }
    Ok(())
}

/// The text `perror()` prints for an error.
fn strerror(e: &std::io::Error) -> Vec<u8> {
    os::strerror(e.raw_os_error().unwrap_or(0))
}

// C: cmd1.c:cmd_cd
pub fn cmd_cd(
    ed: &mut Editor,
    _frommark: Mark,
    _tomark: Mark,
    _cmd: Cmd,
    _bang: bool,
    extra: &[u8],
) -> R<()> {
    use std::os::unix::ffi::OsStrExt;

    let mut dir = extra.to_vec();
    if dir.is_empty() {
        match crate::os::getenv("HOME") {
            Some(h) => dir = h.as_bytes().to_vec(),
            None => {
                msg(ed, b"environment variable $HOME not set", &[]);
                return Ok(());
            }
        }
    }

    if let Err(e) = os::chdir(&dir) {
        let mut out = dir.clone();
        out.extend_from_slice(b": ");
        out.extend_from_slice(&strerror(&e));
        out.push(b'\n');
        let _ = os::err_write(&out);
    }
    Ok(())
}

// C: cmd1.c:cmd_map
pub fn cmd_map(
    ed: &mut Editor,
    _frommark: Mark,
    _tomark: Mark,
    _cmd: Cmd,
    bang: bool,
    extra: &[u8],
) -> R<()> {
    let when = if bang {
        WHEN_VIINP | WHEN_VIREP
    } else {
        WHEN_VICMD
    };
    if extra.is_empty() {
        dumpkey(ed, when);
    } else {
        let mut mapto = 0;
        while mapto < extra.len() && extra[mapto] != b' ' && extra[mapto] != b'\t' {
            mapto += 1;
        }
        let key = &extra[..mapto];
        while mapto < extra.len() && (extra[mapto] == b' ' || extra[mapto] == b'\t') {
            mapto += 1;
        }

        mapkey(ed, key, &extra[mapto..], when as i16, None);
    }
    Ok(())
}

// C: cmd1.c:cmd_set
pub fn cmd_set(
    ed: &mut Editor,
    _frommark: Mark,
    _tomark: Mark,
    _cmd: Cmd,
    _bang: bool,
    extra: &[u8],
) -> R<()> {
    if extra.is_empty() {
        dumpopts(ed, false);
    } else if extra == b"all" {
        dumpopts(ed, true);
    } else {
        setopts(ed, extra);

        ed.vars.changes += 1;
    }
    Ok(())
}

// C: cmd1.c:cmd_tag
pub fn cmd_tag(
    ed: &mut Editor,
    _frommark: Mark,
    _tomark: Mark,
    _cmd: Cmd,
    bang: bool,
    extra: &[u8],
) -> R<()> {
    let extra: Vec<u8> = if extra.is_empty() {
        if ed.cmd1.cmd_tag_prevtag.is_empty() {
            msg(ed, b"No previous tag", &[]);
            return Ok(());
        }
        ed.cmd1.cmd_tag_prevtag.clone()
    } else {
        ed.cmd1.cmd_tag_prevtag = extra[..extra.len().min(PREVTAG_SIZE)].to_vec();
        extra.to_vec()
    };

    let Ok(mut fd) = os::open(TAGS, os::O_RDONLY, 0) else {
        msg(ed, b"No tags file", &[]);
        return Ok(());
    };

    let mut scan: usize = 0;
    let mut end: usize = 0;
    let mut cmp: Option<usize> = Some(0);
    loop {
        if scan >= end {
            end = tread(&mut fd, &mut ed.vars.tmpblk.c).max(0) as usize;
            scan = 0;
            if scan >= end {
                msg(ed, b"tag \"%s\" not found", &[Arg::S(&extra)]);
                return Ok(());
            }
        }

        let c = ed.vars.tmpblk.c[scan];
        if let Some(i) = cmp {
            if ch(&extra, i) == 0 && c == b'\t' {
                break;
            }
            cmp = if ch(&extra, i) != c {
                None
            } else {
                Some(i + 1)
            };
        }

        if cmp.is_none() {
            while scan < end && ed.vars.tmpblk.c[scan] != b'\n' {
                scan += 1;
            }
            if scan < end {
                cmp = Some(0);
            }
        }
        scan += 1;
    }

    let buf = &mut ed.vars.tmpblk.c;
    let mut cmp = 0;
    scan += 1;
    while scan < end && buf[scan] != b'\n' {
        buf[cmp] = buf[scan];
        cmp += 1;
        scan += 1;
    }
    if scan == end {
        tread(&mut fd, &mut buf[cmp..BLKSIZE]);
    }

    drop(fd);

    // C scans past the buffer when the line has no second tab; stop at the
    // end of the line instead.
    let (name, addr) = match buf.iter().position(|&b| b == b'\t') {
        Some(tab) => (cstr(&buf[..tab]).to_vec(), cstr(&buf[tab + 1..]).to_vec()),
        None => {
            let line = cstr(&buf[..]);
            let n = line.iter().position(|&b| b == b'\n').unwrap_or(line.len());
            (line[..n].to_vec(), Vec::new())
        }
    };
    if ed.vars.origname != name {
        if !tmpabort(ed, bang) {
            msg(
                ed,
                b"Use :tag! to abort changes, or :w to save changes",
                &[],
            );
            return Ok(());
        }
        tmpstart(ed, &name);
    }

    let wasmagic = ed.opts.on(Opt::Magic);
    ed.opts.set(Opt::Magic, false);
    ed.vars.cursor = MARK_FIRST;
    let mut cursor = MARK_FIRST;
    let r = linespec(ed, &addr, &mut cursor);
    ed.vars.cursor = cursor;
    if ed.vars.cursor == MARK_UNSET {
        ed.vars.cursor = MARK_FIRST;
    }
    ed.opts.set(Opt::Magic, wasmagic);
    r.map(|_| ())
}

// C: cmd1.c:cmd_visual
pub fn cmd_visual(
    ed: &mut Editor,
    _frommark: Mark,
    _tomark: Mark,
    _cmd: Cmd,
    _bang: bool,
    _extra: &[u8],
) -> R<()> {
    ed.vars.mode = Mode::Vi;
    msg(ed, b"", &[]);
    Ok(())
}

// C: cmd1.c:cmd_version
pub fn cmd_version(
    ed: &mut Editor,
    _frommark: Mark,
    _tomark: Mark,
    _cmd: Cmd,
    _bang: bool,
    _extra: &[u8],
) -> R<()> {
    msg(ed, b"%s", &[Arg::S(VERSION)]);
    msg(ed, b"%s", &[Arg::S(COPYING)]);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strerror_strips_os_error_suffix() {
        let e = std::io::Error::from_raw_os_error(2);
        assert_eq!(strerror(&e), b"No such file or directory".to_vec());
    }

    #[test]
    fn ascii_space_is_signed_aware() {
        assert!(is_ascii_space(b' '));
        assert!(is_ascii_space(b'\t'));
        assert!(!is_ascii_space(0xa0));
        assert!(!is_ascii_space(0));
    }

    #[test]
    fn cmd_mark_sets_mark() {
        let mut ed = Editor::new();
        cmd_mark(&mut ed, 0, 5 * 1024, crate::vi_h::Cmd::Mark, false, b"c").unwrap();
        assert_eq!(ed.vars.mark[2], 5 * 1024);
    }
}
