//! `tmp.c`: the temporary file that holds the text being edited.

use crate::opts::Opt;
use crate::vi_h::Mode;
use std::fs::File;

use crate::blk::{blkget, blkinit};
use crate::config::{FILEPERMS, tmpname_fmt, tread, twrite};
use crate::curses::endwin;
use crate::cut::cutswitch;
use crate::os;
use crate::recycle::garbage;
use crate::redraw::redraw;
use crate::tio::{Arg, msg};
use crate::vars::Editor;
use crate::vi_h::{
    ADDEDNL, BLKSIZE, HADNUL, INFINITY, MARK_FIRST, MARK_UNSET, MAXBLKS, MODIFIED, NOFILE,
    READONLY, isalnum, isascii, markline, schar,
};

/// Capacity of `tmpname[80]`.
pub const TMPNAME_SIZE: usize = 80;

#[derive(Default)]
pub struct Tmp {
    pub tmpname: Vec<u8>,
}

/// `FAIL(why, arg)`: message, then exit 9.
fn fail(ed: &mut Editor, why: &[u8], arg: Arg<'_>) -> ! {
    ed.vars.mode = Mode::Ex;
    msg(ed, why, &[arg]);
    os::sleep_secs(1);
    endwin(ed);
    os::exit(9)
}

fn errno_of(e: &std::io::Error) -> i64 {
    e.raw_os_error().unwrap_or(0) as i64
}

/// C returns from `tmpstart` without closing `origfd`; keep the descriptor
/// open so later descriptor numbers (visible in cut file names) match.
fn leak(f: Option<File>) {
    std::mem::forget(f);
}

// C: tmp.c:tmpstart
pub fn tmpstart(ed: &mut Editor, filename: &[u8]) {
    ed.vars.changes += 1;
    redraw(ed, MARK_UNSET, false);

    let zero = os::Stat {
        ino: 0,
        dev: 0,
        mode: 0,
        uid: 0,
        size: 0,
        mtime: 0,
    };
    let mut origfd: Option<File> = None;
    let statb: os::Stat;

    ed.vars.origname.clear();
    if !filename.is_empty() {
        ed.vars.origname = filename.to_vec();
        let origname = ed.vars.origname.clone();
        match os::open(&origname, os::O_RDONLY, 0) {
            Ok(f) => origfd = Some(f),
            Err(e) if e.raw_os_error() != Some(os::ENOENT) => {
                msg(ed, b"Can't open \"%s\"", &[Arg::S(&origname)]);
                return tmpstart(ed, b"");
            }
            Err(_) => {}
        }
        if origfd.is_some() {
            statb = match os::stat(&origname) {
                Ok(s) => s,
                Err(_) => fail(ed, b"Can't stat \"%s\"", Arg::S(&origname)),
            };
            if statb.mode & os::S_IFMT != os::S_IFREG {
                msg(ed, b"\"%s\" is not a regular file", &[Arg::S(&origname)]);
                leak(origfd);
                return tmpstart(ed, b"");
            }
            if statb.size > 500000 {
                msg(
                    ed,
                    b"\"%s\" is too big (>500000 bytes)",
                    &[Arg::S(&origname)],
                );
                leak(origfd);
                return tmpstart(ed, b"");
            }
        } else {
            statb = os::stat(b".").unwrap_or(zero);
        }
        if origfd.is_some() {
            ed.vars.origtime = statb.mtime;
            let bits = if statb.uid != os::geteuid() {
                0o022
            } else {
                0o200
            };
            if ed.opts.on(Opt::Readonly) || statb.mode & bits == 0 {
                ed.vars.viflags.setflag(READONLY);
            }
        } else {
            ed.vars.origtime = 0;
        }
    } else {
        ed.vars.viflags.setflag(NOFILE);
        ed.vars.origtime = 0;
        statb = os::stat(b".").unwrap_or(zero);
    }

    // checksum of the trailing alphanumerics and dots of the name
    let mut sum: i32 = 0;
    {
        let on = &ed.vars.origname;
        let mut k = on.len();
        while k > 0 {
            let c = schar(on[k - 1]);
            if !(isascii(c) && isalnum(c) || c == b'.' as i32) {
                break;
            }
            sum += c;
            k -= 1;
        }
    }
    sum &= 0xf;

    let dir = ed.opts.directory().to_vec();
    let mut tmpname = tmpname_fmt(&dir, sum, statb.ino, statb.dev);
    tmpname.truncate(TMPNAME_SIZE - 1);
    ed.tmp.tmpname = tmpname.clone();

    if os::exists(&tmpname) {
        if !ed.vars.origname.is_empty() {
            msg(ed, b"\"%s\" is busy", &[Arg::S(filename)]);
            leak(origfd);
            return tmpstart(ed, b"");
        }
        fail(ed, b"\"%s\" is busy", Arg::S(filename));
    }

    let mut scan = dir.clone();
    scan.extend_from_slice(b"/elvXXXXXX");
    let mut errno: i64 = 0;
    match os::mkstemp(&mut scan) {
        Ok(f) => {
            match os::link(&scan, &tmpname) {
                Ok(()) => ed.vars.tmpfd = Some(f),
                Err(e) => {
                    errno = errno_of(&e);
                    drop(f);
                    ed.vars.tmpfd = None;
                }
            }
            if let Err(e) = os::unlink(&scan) {
                errno = errno_of(&e);
            }
        }
        Err(e) => {
            errno = errno_of(&e);
            ed.vars.tmpfd = None;
        }
    }
    if ed.vars.tmpfd.is_none() {
        fail(ed, b"Can't create temporary file, errno=%d", Arg::D(errno));
    }

    // space for the header
    if let Some(mut f) = ed.vars.tmpfd.as_ref() {
        twrite(&mut f, &ed.blk.hdr.c);
    }

    // Must happen before the first write to the new file.
    garbage(ed);

    for i in 1..MAXBLKS {
        ed.vars.lnum[i] = INFINITY;
    }
    ed.vars.lnum[0] = 0;

    let origname = ed.vars.origname.clone();
    match origfd.as_mut() {
        None => {
            // inode 0 marks a new file
            ed.blk.hdr.set_n(0, 0);

            let this = blkget(ed, 1);
            ed.blk.blk[this].buf.c[0] = b'\n';
            ed.blk.blk[this].buf.c[1] = 0;

            ed.vars.lnum[1] = 1;
            ed.vars.nlines = 1;

            if !origname.is_empty() {
                msg(
                    ed,
                    b"\"%s\" [NEW FILE]  1 line, 1 char",
                    &[Arg::S(&origname)],
                );
            } else {
                msg(ed, b"\"[NO FILE]\"  1 line, 1 char", &[]);
            }
        }
        Some(of) => {
            ed.blk.hdr.set_n(0, statb.ino as u16);
            let mut nbytes: i64 = 0;
            ed.vars.nlines = 0;

            let mut i: usize = 1;
            let mut next = blkget(ed, i as i32);
            let mut inbuf: usize = 0;

            loop {
                let this = next;

                let nread = tread(of, &mut ed.blk.blk[this].buf.c[inbuf..BLKSIZE - 1]);
                if nread < 0 {
                    // origfd is closed when it goes out of scope; C closes it first.
                    ed.vars.tmpfd = None;
                    let _ = os::unlink(&ed.tmp.tmpname.clone());
                    fail(ed, b"Error reading \"%s\"", Arg::S(&origname));
                }
                let nread = nread as usize;

                for k in inbuf..inbuf + nread {
                    if ed.blk.blk[this].buf.c[k] == 0 {
                        ed.vars.viflags.setflag(HADNUL);
                        ed.blk.blk[this].buf.c[k] = 0x80;
                    }
                }
                inbuf += nread;

                if inbuf == 0 {
                    break;
                }

                // search backward for the last newline
                let c = &ed.blk.blk[this].buf.c;
                let mut k: isize = inbuf as isize;
                loop {
                    k -= 1;
                    if k < 0 || c[k as usize] == b'\n' {
                        break;
                    }
                }
                let k = if k < 0 {
                    if inbuf >= BLKSIZE - 1 { 80 } else { inbuf }
                } else {
                    k as usize + 1
                };

                i += 1;
                next = blkget(ed, i as i32);

                // move the fragmentary last line to the next buffer
                inbuf -= k;
                let tail = ed.blk.blk[this].buf.c[k..].to_vec();
                ed.blk.blk[this].buf.c[k..].fill(0);
                ed.blk.blk[next].buf.c[..tail.len()].copy_from_slice(&tail);

                let c = &mut ed.blk.blk[this].buf.c;
                let mut k: isize = (BLKSIZE - inbuf) as isize;
                loop {
                    k -= 1;
                    if k < 0 || c[k as usize] != 0 {
                        break;
                    }
                }
                if k < 0 || c[k as usize] != b'\n' {
                    ed.vars.viflags.setflag(ADDEDNL);
                    c[(k + 1) as usize] = b'\n';
                }

                let mut k = 0;
                while k < BLKSIZE && c[k] != 0 {
                    if c[k] == b'\n' {
                        ed.vars.nlines += 1;
                    }
                    nbytes += 1;
                    k += 1;
                }
                ed.vars.lnum[i - 1] = ed.vars.nlines;
            }

            if ed.vars.nlines == 0 {
                let this = blkget(ed, 1);
                ed.blk.blk[this].buf.c[0] = b'\n';
                ed.blk.blk[this].buf.c[1] = 0;

                ed.vars.lnum[1] = 1;
                ed.vars.nlines = 1;
                nbytes = 1;
            }

            let nlines = ed.vars.nlines;
            let ro: &[u8] = if ed.vars.viflags.tstflag(READONLY) {
                b"[READONLY]"
            } else {
                b""
            };
            msg(
                ed,
                b"\"%s\" %s %ld line%s, %ld char%s",
                &[
                    Arg::S(&origname),
                    Arg::S(ro),
                    Arg::D(nlines),
                    Arg::S(if nlines == 1 { b"" } else { b"s" }),
                    Arg::D(nbytes),
                    Arg::S(if nbytes == 1 { b"" } else { b"s" }),
                ],
            );
        }
    }

    ed.vars.cursor = MARK_FIRST;

    drop(origfd);

    if ed.vars.viflags.tstflag(HADNUL) {
        msg(
            ed,
            b"This file contained NULs.  They've been changed to \\x80 chars",
            &[],
        );
    }
    if ed.vars.viflags.tstflag(ADDEDNL) {
        msg(
            ed,
            b"Newline characters have been inserted to break up long lines",
            &[],
        );
    }
}

// C: tmp.c:tmpsave
/// `filename` `None` is C's null pointer (save to the original file).
pub fn tmpsave(ed: &mut Editor, filename: Option<&[u8]>, bang: bool) -> bool {
    let mut filename: Vec<u8> = match filename {
        Some(f) if !f.is_empty() => f.to_vec(),
        _ => ed.vars.origname.clone(),
    };

    if filename.is_empty() {
        msg(ed, b"Don't know a name for this file -- NOT WRITTEN", &[]);
        return false;
    }

    if filename == ed.vars.origname && ed.opts.on(Opt::Readonly) && !bang {
        msg(
            ed,
            b"\"%s\" [READONLY] -- NOT WRITTEN",
            &[Arg::S(&filename)],
        );
        return false;
    }

    let fd = if filename.starts_with(b">>") {
        let mut s = 2;
        while s < filename.len() && (filename[s] == b' ' || filename[s] == b'\t') {
            s += 1;
        }
        filename.drain(..s);
        os::open(&filename, os::O_WRONLY | os::O_APPEND, 0)
    } else {
        // The file must not exist, or be the original, or we need a bang.
        if filename != ed.vars.origname && os::exists(&filename) && !bang {
            msg(ed, b"File already exists - Use :w! to overwrite", &[]);
            return false;
        }
        os::creat(&filename, FILEPERMS)
    };
    let Ok(mut fd) = fd else {
        msg(
            ed,
            b"Can't write to \"%s\" -- NOT WRITTEN",
            &[Arg::S(&filename)],
        );
        return false;
    };

    let mut bytes: i64 = 0;
    let mut i = 1;
    while i < MAXBLKS {
        let this = blkget(ed, i as i32);
        let c = &ed.blk.blk[this].buf.c;
        if c[0] == 0 {
            break;
        }
        let mut len = 0;
        while len < BLKSIZE && c[len] != 0 {
            len += 1;
        }
        twrite(&mut fd, &c[..len]);
        bytes += len as i64;
        i += 1;
    }

    ed.vars.viflags.clrflag(MODIFIED);
    ed.vars.significant = false;

    if !filename.starts_with(ed.opts.directory()) {
        let nlines = ed.vars.nlines;
        msg(
            ed,
            b"Wrote \"%s\"  %ld lines, %ld characters",
            &[Arg::S(&filename), Arg::D(nlines), Arg::D(bytes)],
        );
    }

    drop(fd);

    true
}

// C: tmp.c:tmpabort
pub fn tmpabort(ed: &mut Editor, bang: bool) -> bool {
    if ed.vars.tmpfd.is_none() {
        return true;
    }

    if !bang && ed.vars.viflags.tstflag(MODIFIED) {
        if ed.opts.on(Opt::Autowrite) {
            return tmpend(ed, bang);
        } else {
            return false;
        }
    }

    let tmpname = ed.tmp.tmpname.clone();
    cutswitch(ed, &tmpname);
    ed.vars.tmpfd = None;
    let _ = os::unlink(&tmpname);
    ed.vars.prevorig = ed.vars.origname.clone();
    ed.vars.prevline = markline(ed.vars.cursor);
    ed.vars.origname.clear();
    ed.vars.origtime = 0;
    blkinit(ed);
    ed.vars.nlines = 0;
    ed.vars.viflags.initflags();
    true
}

// C: tmp.c:tmpend
pub fn tmpend(ed: &mut Editor, bang: bool) -> bool {
    if ed.vars.viflags.tstflag(MODIFIED) && !tmpsave(ed, None, false) && !bang {
        return false;
    }

    tmpabort(ed, true);

    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blk::{afterdo, beforedo, undo};
    use crate::cut::{cut, cutname, paste};
    use crate::misc::fetchline;
    use crate::modify::{add, change, delete};
    use crate::vi_h::mark_at_line;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static SEQ: AtomicUsize = AtomicUsize::new(0);

    fn testdir() -> Vec<u8> {
        let n = SEQ.fetch_add(1, Ordering::SeqCst);
        let d = format!("/tmp/elvt{}_{}", std::process::id(), n);
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d.into_bytes()
    }

    fn editor(dir: &[u8]) -> Editor {
        let mut ed = Editor::new();
        ed.vars.mode = Mode::Vi;
        ed.opts.set_str(Opt::Directory, dir);
        ed
    }

    fn path(dir: &[u8], name: &str) -> Vec<u8> {
        let mut p = dir.to_vec();
        p.push(b'/');
        p.extend_from_slice(name.as_bytes());
        p
    }

    fn text_of(ed: &mut Editor) -> Vec<u8> {
        let mut v = Vec::new();
        for l in 1..=ed.vars.nlines {
            v.extend_from_slice(&fetchline(ed, l));
            v.push(b'\n');
        }
        v
    }

    fn sample(n: usize) -> Vec<u8> {
        let mut v = Vec::new();
        for i in 0..n {
            v.extend_from_slice(format!("line {i} {}\n", "x".repeat(i % 37)).as_bytes());
        }
        v
    }

    fn start(ed: &mut Editor, dir: &[u8], content: &[u8]) -> Vec<u8> {
        let f = path(dir, "orig.txt");
        std::fs::write(std::str::from_utf8(&f).unwrap(), content).unwrap();
        tmpstart(ed, &f);
        f
    }

    fn cleanup(ed: &mut Editor, dir: &[u8]) {
        tmpabort(ed, true);
        let _ = std::fs::remove_dir_all(std::str::from_utf8(dir).unwrap());
    }

    #[test]
    fn tmpstart_loads_and_fetches() {
        let dir = testdir();
        let mut ed = editor(&dir);
        let content = sample(400);
        start(&mut ed, &dir, &content);
        assert_eq!(ed.vars.nlines, 400);
        assert!(ed.vars.lnum[2] > 0 && ed.vars.lnum[2] < 400);
        assert_eq!(
            ed.tio.pmsg,
            format!(
                "\"{}/orig.txt\"  400 lines, {} chars",
                std::str::from_utf8(&dir).unwrap(),
                content.len()
            )
            .into_bytes()
        );
        assert_eq!(text_of(&mut ed), content);
        // random access, including backwards
        assert_eq!(
            fetchline(&mut ed, 399),
            b"line 398 "
                .iter()
                .chain(&[b'x'; 398 % 37])
                .copied()
                .collect::<Vec<u8>>()
        );
        assert_eq!(fetchline(&mut ed, 1), b"line 0 ".to_vec());
        let tmpname = ed.tmp.tmpname.clone();
        assert!(os::exists(&tmpname));
        tmpabort(&mut ed, true);
        assert!(!os::exists(&tmpname));
        assert!(ed.vars.tmpfd.is_none());
        assert_eq!(ed.vars.prevorig, path(&dir, "orig.txt"));
        cleanup(&mut ed, &dir);
    }

    #[test]
    fn tmp_file_layout_matches_c() {
        let dir = testdir();
        let mut ed = editor(&dir);
        let content = sample(300);
        let f = start(&mut ed, &dir, &content);
        let ino = os::stat(&f).unwrap().ino;
        beforedo(&mut ed, false);
        afterdo(&mut ed);
        let raw = std::fs::read(std::str::from_utf8(&ed.tmp.tmpname).unwrap()).unwrap();
        let n = |i: usize| u16::from_ne_bytes([raw[2 * i], raw[2 * i + 1]]) as usize;
        assert_eq!(n(0), ino as u16 as usize);
        let mut text = Vec::new();
        let mut i = 1;
        while n(i) != 0 {
            let b = &raw[n(i) * BLKSIZE..(n(i) + 1) * BLKSIZE];
            let len = b.iter().position(|&c| c == 0).unwrap();
            assert!(b[len..].iter().all(|&c| c == 0));
            assert_eq!(b[len - 1], b'\n');
            text.extend_from_slice(&b[..len]);
            i += 1;
        }
        assert_eq!(text, content);
        cleanup(&mut ed, &dir);
    }

    #[test]
    fn tmpsave_round_trip_and_append() {
        let dir = testdir();
        let mut ed = editor(&dir);
        let content = sample(250);
        start(&mut ed, &dir, &content);
        let out = path(&dir, "out.txt");
        assert!(tmpsave(&mut ed, Some(&out), false));
        assert_eq!(
            std::fs::read(std::str::from_utf8(&out).unwrap()).unwrap(),
            content
        );
        // exists, no bang
        ed.tio.manymsgs = false;
        assert!(!tmpsave(&mut ed, Some(&out), false));
        assert_eq!(
            ed.tio.pmsg,
            b"File already exists - Use :w! to overwrite".to_vec()
        );
        let mut app = b">> ".to_vec();
        app.extend_from_slice(&out);
        assert!(tmpsave(&mut ed, Some(&app), false));
        let mut twice = content.clone();
        twice.extend_from_slice(&content);
        assert_eq!(
            std::fs::read(std::str::from_utf8(&out).unwrap()).unwrap(),
            twice
        );
        cleanup(&mut ed, &dir);
    }

    #[test]
    fn nul_and_long_lines() {
        let dir = testdir();
        let mut ed = editor(&dir);
        ed.vars.mode = Mode::Ex;
        let mut content = b"a\0b\n".to_vec();
        content.extend_from_slice(&[b'y'; 2000]);
        start(&mut ed, &dir, &content);
        assert!(ed.vars.viflags.tstflag(HADNUL));
        assert!(ed.vars.viflags.tstflag(ADDEDNL));
        assert_eq!(fetchline(&mut ed, 1), b"a\x80b".to_vec());
        // lengths and message checked against the C reference binary
        let mut lens: Vec<usize> = (1..=ed.vars.nlines)
            .map(|l| fetchline(&mut ed, l).len())
            .collect();
        assert_eq!(lens.remove(0), 3);
        assert_eq!(lens.pop(), Some(960));
        assert_eq!(lens, vec![80; 13]);
        cleanup(&mut ed, &dir);
    }

    #[test]
    fn modify_and_undo() {
        let dir = testdir();
        let mut ed = editor(&dir);
        let content = sample(300);
        start(&mut ed, &dir, &content);

        beforedo(&mut ed, false);
        delete(&mut ed, mark_at_line(10), mark_at_line(200));
        afterdo(&mut ed);
        assert_eq!(ed.vars.nlines, 110);
        assert_eq!(
            fetchline(&mut ed, 10),
            b"line 199 "
                .iter()
                .chain(&[b'x'; 199 % 37])
                .copied()
                .collect::<Vec<u8>>()
        );
        assert!(ed.vars.viflags.tstflag(MODIFIED));

        assert!(undo(&mut ed));
        assert_eq!(ed.vars.nlines, 300);
        assert_eq!(text_of(&mut ed), content);

        // a large multi-block insertion in the middle of a line
        let big = sample(500);
        beforedo(&mut ed, false);
        add(&mut ed, mark_at_line(5) + 3, &big);
        afterdo(&mut ed);
        let mut expect = Vec::new();
        for (i, l) in content.split_inclusive(|&b| b == b'\n').enumerate() {
            if i == 4 {
                expect.extend_from_slice(&l[..3]);
                expect.extend_from_slice(&big);
                expect.extend_from_slice(&l[3..]);
            } else {
                expect.extend_from_slice(l);
            }
        }
        assert_eq!(ed.vars.nlines, 800);
        assert_eq!(text_of(&mut ed), expect);

        change(&mut ed, mark_at_line(1), mark_at_line(1) + 1, b"L");
        assert_eq!(fetchline(&mut ed, 1), b"Line 0 ".to_vec());
        change(&mut ed, mark_at_line(2), mark_at_line(3), b"two\nlines\n");
        assert_eq!(fetchline(&mut ed, 2), b"two".to_vec());
        assert_eq!(fetchline(&mut ed, 3), b"lines".to_vec());
        assert_eq!(ed.vars.nlines, 801);
        assert_eq!(ed.vars.rptlabel, b"changed");

        // B16: lines may grow to BLKSIZE - 1 bytes including the newline
        ed.tio.manymsgs = false;
        let end = mark_at_line(1) + 7;
        assert!(add(&mut ed, end, &[b'x'; BLKSIZE - 9]));
        assert_eq!(fetchline(&mut ed, 1).len(), BLKSIZE - 2);
        assert!(!add(&mut ed, end, b"y"));
        assert!(ed.vars.toolong);
        let mut long = b"y\n".to_vec();
        long.extend_from_slice(&[b'x'; BLKSIZE - 1]);
        long.push(b'\n');
        ed.tio.manymsgs = false;
        assert!(!add(&mut ed, mark_at_line(2), &long));
        assert_eq!(ed.vars.nlines, 801);
        assert_eq!(fetchline(&mut ed, 1).len(), BLKSIZE - 2);

        // deleting everything leaves one empty line
        beforedo(&mut ed, false);
        delete(&mut ed, mark_at_line(1), mark_at_line(802));
        afterdo(&mut ed);
        assert_eq!(ed.vars.nlines, 1);
        assert_eq!(fetchline(&mut ed, 1), Vec::<u8>::new());
        cleanup(&mut ed, &dir);
    }

    #[test]
    fn cut_and_paste() {
        let dir = testdir();
        let mut ed = editor(&dir);
        let content = sample(200);
        start(&mut ed, &dir, &content);
        let lines: Vec<&[u8]> = content.split_inclusive(|&b| b == b'\n').collect();

        // line-mode yank of lines 20..=149 into "a, paste after line 1
        cutname(&mut ed, b'a' as i32);
        cut(&mut ed, mark_at_line(20), mark_at_line(150));
        assert_eq!(ed.vars.rptlines, 130);
        cutname(&mut ed, b'a' as i32);
        let m = paste(&mut ed, mark_at_line(1), true, false);
        assert_eq!(m, mark_at_line(2));
        assert_eq!(ed.vars.nlines, 330);
        let mut expect = lines[0].to_vec();
        expect.extend(lines[19..149].concat());
        expect.extend(lines[1..].concat());
        assert_eq!(text_of(&mut ed), expect);

        // character-mode cut into the anonymous buffer
        cut(&mut ed, mark_at_line(1) + 2, mark_at_line(1) + 4);
        let end = paste(&mut ed, mark_at_line(1), false, true);
        assert_eq!(fetchline(&mut ed, 1), b"neline 0 ".to_vec());
        assert_eq!(end, mark_at_line(1) + 1);

        ed.tio.manymsgs = false;
        cutname(&mut ed, b'q' as i32);
        assert_eq!(paste(&mut ed, mark_at_line(1), false, false), MARK_UNSET);
        assert_eq!(ed.tio.pmsg, b"Cut buffer \"q is empty".to_vec());
        cleanup(&mut ed, &dir);
    }

    #[test]
    fn named_cut_survives_file_switch() {
        let dir = testdir();
        let mut ed = editor(&dir);
        let content = sample(50);
        start(&mut ed, &dir, &content);
        cutname(&mut ed, b'b' as i32);
        cut(&mut ed, mark_at_line(3), mark_at_line(5));
        assert!(tmpabort(&mut ed, true));
        assert_eq!(ed.cut.files.len(), 1);

        let f2 = path(&dir, "second.txt");
        std::fs::write(std::str::from_utf8(&f2).unwrap(), b"one\ntwo\n").unwrap();
        ed.tio.manymsgs = false;
        tmpstart(&mut ed, &f2);
        cutname(&mut ed, b'b' as i32);
        paste(&mut ed, mark_at_line(1), false, false);
        let lines: Vec<&[u8]> = content.split_inclusive(|&b| b == b'\n').collect();
        let mut expect = lines[2..4].concat();
        expect.extend_from_slice(b"one\ntwo\n");
        assert_eq!(text_of(&mut ed), expect);

        crate::cut::cutend(&mut ed);
        assert!(ed.cut.files.is_empty());
        let left: Vec<_> = std::fs::read_dir(std::str::from_utf8(&dir).unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .filter(|n| n.starts_with("elv_"))
            .collect();
        assert!(left.is_empty(), "{left:?}");
        cleanup(&mut ed, &dir);
    }

    #[test]
    fn tmpname_checksum() {
        let dir = testdir();
        let mut ed = editor(&dir);
        let f = start(&mut ed, &dir, b"x\n");
        let st = os::stat(&f).unwrap();
        // "orig.txt": sum of the bytes, masked to 4 bits
        let sum = b"orig.txt".iter().map(|&b| b as i32).sum::<i32>() & 0xf;
        let mut want = tmpname_fmt(&dir, sum, st.ino, st.dev);
        want.truncate(TMPNAME_SIZE - 1);
        assert_eq!(ed.tmp.tmpname, want);
        cleanup(&mut ed, &dir);
    }
}
