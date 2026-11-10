//! `system.c`: shell commands, wildcards, and filters.  Process
//! primitives live in `os.rs`.  No module state.

use crate::opts::Opt;
use crate::vi_h::Mode;
use std::fs::File;
use std::io::{Seek, SeekFrom};

use crate::blk::{afterdo, beforedo};
use crate::cmd1::cmd_write2;
use crate::config::{scratchout_fmt, tread};
use crate::modify::{add, delete};
use crate::os::{self, ShellRun, SpawnOpts};
use crate::tio::{Arg, msg, sprintf};
use crate::vars::Editor;
use crate::vi_h::LINEMASK;
use crate::vi_h::{BLKSIZE, MODIFIED, Mark, R, cstr};

// C: system.c:elvis_system
/// `None` is `cmd == o_shell` (run an interactive shell).  True if the
/// shell exited with status 0.
pub fn elvis_system(ed: &mut Editor, cmd: Option<&[u8]>) -> bool {
    if ed.opts.on(Opt::Warn) && ed.vars.viflags.tstflag(MODIFIED) {
        if ed.vars.mode == Mode::Vi {
            ed.vars.mode = Mode::Colon;
        }
        let name = ed.vars.origname.clone();
        msg(
            ed,
            b"Warning: \"%s\" has been modified but not yet saved",
            &[Arg::S(&name)],
        );
    }

    os::ignore_sigint();
    let shell = ed.opts.shell().to_vec();
    // The C child reports a failed exec with msg(), which shows up only when
    // not in visual mode.
    let mut fail = Vec::new();
    if ed.vars.mode != Mode::Vi {
        fail = sprintf(b"execle(\"%s\", ...) failed", &[Arg::S(&shell)]);
        fail.extend_from_slice(b"\n\r");
    }
    let run = match cmd {
        None => ShellRun::Interactive,
        Some(c) => ShellRun::Command(cstr(c)),
    };
    let opts = SpawnOpts {
        close_3_to_59: true,
        output: None,
        close_first: None,
        input: None,
        exec_fail_output: &fail,
    };
    match os::spawn_shell(&shell, run, &opts) {
        Err(_) => {
            msg(ed, b"fork() failed", &[]);
            false
        }
        Ok(child) => {
            let status = os::wait_child(&child);
            os::trap_sigint();
            status == 0
        }
    }
}

/// The read end of an `rpipe()` and the child writing into it.
struct Rpipe {
    fd: File,
    child: os::Child,
}

// C: system.c:rpipe
fn rpipe(ed: &mut Editor, cmd: &[u8], input: Option<&File>) -> Option<Rpipe> {
    let (r0, w1) = os::pipe().ok()?;

    os::ignore_sigint();

    let shell = ed.opts.shell().to_vec();
    let opts = SpawnOpts {
        close_3_to_59: false,
        output: Some(&w1),
        close_first: Some(&r0),
        input,
        exec_fail_output: b"",
    };
    let child = os::spawn_shell(&shell, ShellRun::Command(cstr(cmd)), &opts).ok()?;
    drop(w1);
    Some(Rpipe { fd: r0, child })
}

// C: system.c:rpclose
/// True if the command exited with status 0.
fn rpclose(p: Rpipe) -> bool {
    drop(p.fd);
    let status = os::wait_child(&p.child);
    os::trap_sigint();
    status == 0
}

const PROG: &[u8] = b"echo ";

// C: system.c:wildcard
/// Leaves the `echo` command, then the shell's output, in `tmpblk` as C
/// does; `ex.c` reads the result from there.
pub fn wildcard(ed: &mut Editor, names: &[u8]) -> Vec<u8> {
    let names = cstr(names).to_vec();
    let n = names.len().min(BLKSIZE - 1 - PROG.len());
    let c = &mut ed.vars.tmpblk.c;
    c[..PROG.len()].copy_from_slice(PROG);
    c[PROG.len()..PROG.len() + n].copy_from_slice(&names[..n]);
    c[PROG.len() + n] = 0;

    let cmd = cstr(&ed.vars.tmpblk.c).to_vec();
    let Some(mut p) = rpipe(ed, &cmd, None) else {
        return names;
    };
    let mut i: isize = 0;
    let mut j;
    loop {
        j = tread(&mut p.fd, &mut ed.vars.tmpblk.c[i as usize..BLKSIZE]);
        i += j;
        if j <= 0 || i < 0 {
            break;
        }
    }

    if rpclose(p) && j == 0 && i < BLKSIZE as isize && i > 0 {
        ed.vars.tmpblk.c[i as usize - 1] = 0;
        cstr(&ed.vars.tmpblk.c).to_vec()
    } else {
        names
    }
}

// C: system.c:filter
pub fn filter(ed: &mut Editor, mut from: Mark, mut to: Mark, cmd: &[u8]) -> R<()> {
    let mut scratch: Option<File> = None;
    let mut scrout: Vec<u8> = Vec::new();
    let mut new: Mark;

    if to != 0 {
        scrout = scratchout_fmt(ed.opts.directory());
        let Ok(mut f) = os::mkstemp(&mut scrout) else {
            return Ok(());
        };
        if let Err(e) = cmd_write2(ed, from, to, &mut f) {
            drop(f);
            let _ = os::unlink(&scrout);
            return Err(e);
        }
        let _ = f.seek(SeekFrom::Start(0));
        scratch = Some(f);
    }

    let Some(mut fd) = rpipe(ed, cmd, scratch.as_ref()) else {
        if to != 0 {
            drop(scratch);
            let _ = os::unlink(&scrout);
        }
        return Ok(());
    };

    beforedo(ed, false);
    {
        from &= LINEMASK;
        if to != 0 {
            to &= LINEMASK;
            to += BLKSIZE as Mark;
            new = to;
        } else {
            new = from + BLKSIZE as Mark;
        }

        loop {
            let i = tread(&mut fd.fd, &mut ed.vars.tmpblk.c[..BLKSIZE - 1]);
            if i <= 0 {
                break;
            }
            ed.vars.tmpblk.c[i as usize] = 0;
            let text = cstr(&ed.vars.tmpblk.c).to_vec();
            add(ed, new, &text);
            for &b in &text {
                if b == b'\n' {
                    new = (new & LINEMASK) + BLKSIZE as Mark;
                } else {
                    new += 1;
                }
            }
        }
    }
    afterdo(ed);

    if to != 0 {
        delete(ed, from, to);
    }

    ed.vars.rptlabel = b"more";
    if ed.vars.rptlines < 0 {
        ed.vars.rptlines = -ed.vars.rptlines;
        ed.vars.rptlabel = b"less";
    }

    rpclose(fd);
    if to != 0 {
        drop(scratch);
        let _ = os::unlink(&scrout);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wildcard_expands_with_shell() {
        let d = std::env::temp_dir().join(format!("elvis-wild-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("a1.txt"), b"").unwrap();
        std::fs::write(d.join("a2.txt"), b"").unwrap();
        let mut ed = Editor::new();
        let pat = format!("{}/a*.txt", d.display());
        let out = wildcard(&mut ed, pat.as_bytes());
        let want = format!("{0}/a1.txt {0}/a2.txt", d.display());
        assert_eq!(out, want.as_bytes());
        assert_eq!(cstr(&ed.vars.tmpblk.c), want.as_bytes());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn wildcard_returns_names_when_shell_fails() {
        let mut ed = Editor::new();
        ed.opts.set_str(Opt::Shell, b"/nonexist");
        assert_eq!(wildcard(&mut ed, b"x*"), b"x*");
    }
}
