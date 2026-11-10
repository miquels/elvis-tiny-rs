//! `ex.c`: ex command parsing and dispatch.

use crate::cmd1::{
    cmd_args, cmd_cd, cmd_edit, cmd_file, cmd_global, cmd_map, cmd_mark, cmd_next, cmd_set,
    cmd_shell, cmd_tag, cmd_version, cmd_visual, cmd_write, cmd_xit,
};
use crate::cmd2::{
    cmd_append, cmd_delete, cmd_join, cmd_move, cmd_print, cmd_put, cmd_read, cmd_shift,
    cmd_source, cmd_substitute, cmd_undo,
};
use crate::config::{EXRC, tread};
use crate::misc::pfetch;
use crate::move1::{m_tomark, m_updnto};
use crate::move2::{m_bsrch, m_fsrch};
use crate::os;
use crate::system::wildcard;
use crate::tio::{Arg, exrefresh, msg, vgets};
use crate::vars::Editor;
use crate::vi_h::*;

#[derive(Default)]
pub struct Ex {
    pub ex_oldline: i64,
}

type Argt = i16;
const FROM: Argt = 1;
const TO: Argt = 2;
const BANG: Argt = 4;
const EXTRA: Argt = 8;
const XFILE: Argt = 16;
const NOSPC: Argt = 32;
const DFLALL: Argt = 64;
const DFLNONE: Argt = 128;
const NODFL: Argt = 256;
const EXRCOK: Argt = 512;
const NL: Argt = 1024;
const PLUS: Argt = 2048;
const ZERO: Argt = 4096;
const FILES: Argt = XFILE + EXTRA;
const WORD1: Argt = EXTRA + NOSPC;
const FILE1: Argt = FILES + NOSPC;
const NAMEDF: Argt = FILE1 + NODFL;
const NAMEDFS: Argt = FILES + NODFL;
const RANGE: Argt = FROM + TO;
const NONE: Argt = 0;

type CmdFn = fn(&mut Editor, Mark, Mark, Cmd, bool, &[u8]) -> R<()>;

struct CmdName {
    name: &'static [u8],
    code: Cmd,
    func: CmdFn,
    argt: Argt,
}

const fn c(name: &'static [u8], code: Cmd, func: CmdFn, argt: Argt) -> CmdName {
    CmdName {
        name,
        code,
        func,
        argt,
    }
}

/// The order is significant: an ambiguous abbreviation means the first match.
static CMDNAMES: [CmdName; 47] = [
    c(b"append", CMD_APPEND, cmd_append, FROM + ZERO),
    c(b"change", CMD_CHANGE, cmd_append, RANGE),
    c(b"delete", CMD_DELETE, cmd_delete, RANGE + WORD1),
    c(b"edit", CMD_EDIT, cmd_edit, BANG + FILE1 + PLUS),
    c(b"file", CMD_FILE, cmd_file, NAMEDF),
    c(
        b"global",
        CMD_GLOBAL,
        cmd_global,
        RANGE + BANG + EXTRA + DFLALL,
    ),
    c(b"insert", CMD_INSERT, cmd_append, FROM),
    c(b"join", CMD_INSERT, cmd_join, RANGE),
    c(b"k", CMD_MARK, cmd_mark, FROM + WORD1),
    c(b"list", CMD_LIST, cmd_print, RANGE + NL),
    c(b"move", CMD_MOVE, cmd_move, RANGE + EXTRA),
    c(b"next", CMD_NEXT, cmd_next, BANG + NAMEDFS),
    c(b"Next", CMD_PREVIOUS, cmd_next, BANG),
    c(b"print", CMD_PRINT, cmd_print, RANGE + NL),
    c(b"quit", CMD_QUIT, cmd_xit, BANG),
    c(b"read", CMD_READ, cmd_read, FROM + ZERO + NAMEDF),
    c(b"substitute", CMD_SUBSTITUTE, cmd_substitute, RANGE + EXTRA),
    c(b"to", CMD_COPY, cmd_move, RANGE + EXTRA),
    c(b"undo", CMD_UNDO, cmd_undo, NONE),
    c(b"vglobal", CMD_VGLOBAL, cmd_global, RANGE + EXTRA + DFLALL),
    c(
        b"write",
        CMD_WRITE,
        cmd_write,
        RANGE + BANG + FILE1 + DFLALL,
    ),
    c(b"xit", CMD_XIT, cmd_xit, BANG + NL),
    c(b"yank", CMD_YANK, cmd_delete, RANGE + WORD1),
    c(
        b"!",
        CMD_BANG,
        cmd_shell,
        EXRCOK + RANGE + NAMEDFS + DFLNONE + NL,
    ),
    c(b"<", CMD_SHIFTL, cmd_shift, RANGE),
    c(b">", CMD_SHIFTR, cmd_shift, RANGE),
    c(b"=", CMD_EQUAL, cmd_file, RANGE),
    c(b"&", CMD_SUBAGAIN, cmd_substitute, RANGE),
    c(b"args", CMD_ARGS, cmd_args, EXRCOK + NAMEDFS),
    c(b"cd", CMD_CD, cmd_cd, EXRCOK + NAMEDF),
    c(b"copy", CMD_COPY, cmd_move, RANGE + EXTRA),
    c(b"ex", CMD_EDIT, cmd_edit, BANG + FILE1),
    c(b"map", CMD_MAP, cmd_map, EXRCOK + BANG + EXTRA),
    c(b"number", CMD_NUMBER, cmd_print, RANGE + NL),
    c(b"put", CMD_PUT, cmd_put, FROM + ZERO + WORD1),
    c(b"set", CMD_SET, cmd_set, EXRCOK + EXTRA),
    c(b"shell", CMD_SHELL, cmd_shell, NL),
    c(b"source", CMD_SOURCE, cmd_source, EXRCOK + NAMEDF),
    c(b"tag", CMD_TAG, cmd_tag, BANG + WORD1),
    c(b"version", CMD_VERSION, cmd_version, EXRCOK + NONE),
    c(b"visual", CMD_VISUAL, cmd_visual, NONE),
    c(b"wq", CMD_WQUIT, cmd_xit, NL),
    c(b"chdir", CMD_CD, cmd_cd, EXRCOK + NAMEDF),
    c(b"mark", CMD_MARK, cmd_mark, FROM + WORD1),
    c(b"previous", CMD_PREVIOUS, cmd_next, BANG),
    c(b"rewind", CMD_REWIND, cmd_next, BANG),
    c(b"unmap", CMD_UNMAP, cmd_map, EXRCOK + BANG + EXTRA),
];

/// Index of the first command whose name starts with `word`, as the C
/// `strncmp` lookup finds it.
fn lookup(word: &[u8]) -> Option<usize> {
    CMDNAMES
        .iter()
        .position(|e| e.name.len() >= word.len() && &e.name[..word.len()] == word)
}

// C: ex.c:parseptrn
/// `ptrn` is a NUL-terminated buffer starting at the delimiter.  Writes a
/// NUL at the end of the pattern and returns the index just past it.
pub fn parseptrn(ptrn: &mut [u8]) -> usize {
    let delim = ch(ptrn, 0);
    let mut scan = 1;
    while ch(ptrn, scan) != 0 && ch(ptrn, scan) != delim {
        if ch(ptrn, scan) == b'\\' && ch(ptrn, scan + 1) != 0 {
            scan += 1;
        }
        scan += 1;
    }
    if ch(ptrn, scan) != 0 {
        ptrn[scan] = 0;
        scan += 1;
    }
    scan
}

#[inline]
fn sc(s: &[u8], i: usize) -> i32 {
    schar(ch(s, i))
}

// C: ex.c:linespec
/// Returns how many bytes of `s` were consumed.
pub fn linespec(ed: &mut Editor, s: &[u8], markptr: &mut Mark) -> R<usize> {
    let mut buf = cstr(s).to_vec();
    buf.push(0);
    let mut s = 0;

    loop {
        if buf[s] == b';' {
            s += 1;
        }

        while isascii(sc(&buf, s)) && isspace(sc(&buf, s)) {
            s += 1;
        }

        if buf[s] == b'.' {
            s += 1;
            *markptr = ed.vars.cursor;
        } else if buf[s] == b'$' {
            s += 1;
            *markptr = mark_last(ed.vars.nlines);
        } else if isascii(sc(&buf, s)) && isdigit(sc(&buf, s)) {
            let mut num: i64 = 0;
            while isascii(sc(&buf, s)) && isdigit(sc(&buf, s)) {
                num = num.wrapping_mul(10).wrapping_add((buf[s] - b'0') as i64);
                s += 1;
            }
            *markptr = mark_at_line(num);
        } else if buf[s] == b'\'' {
            s += 1;
            let key = sc(&buf, s);
            *markptr = m_tomark(ed, ed.vars.cursor, 1, key)?;
            if buf[s] != 0 {
                s += 1;
            } else {
                // C steps over the NUL here and goes on reading past it.
                return Ok(s);
            }
        } else if buf[s] == b'/' || buf[s] == b'?' {
            let t = s + parseptrn(&mut buf[s..]);
            let pat = cstr(&buf[s..]).to_vec();

            *markptr &= !(BLKSIZE as Mark - 1);
            if buf[s] == b'/' {
                pfetch(ed, markline(*markptr));
                if ed.vars.plen > 0 {
                    *markptr += ed.vars.plen as Mark - 1;
                }
                *markptr = m_fsrch(ed, *markptr, &pat)?;
            } else {
                *markptr = m_bsrch(ed, *markptr, &pat)?;
            }

            s = t;
        }

        if *markptr == 0 {
            return Ok(s);
        }

        let t = s;
        if buf[t] == b'-' || buf[t] == b'+' {
            s += 1;
            let mut num: i64 = 0;
            while buf[s].is_ascii_digit() {
                num = num.wrapping_mul(10).wrapping_add((buf[s] - b'0') as i64);
                s += 1;
            }
            if num == 0 {
                num = 1;
            }
            *markptr = m_updnto(ed, *markptr, num, buf[t] as i32)?;
        }

        if !(buf[s] == b';' || buf[s] == b'+' || buf[s] == b'-') {
            break;
        }
    }

    Ok(s)
}

// C: ex.c:ex
pub fn ex(ed: &mut Editor) -> R<()> {
    let mut cmdbuf: Vec<u8> = Vec::with_capacity(80);

    ed.vars.significant = false;
    ed.ex.ex_oldline = markline(ed.vars.cursor);

    while ed.vars.mode == MODE_EX {
        let cmdlen = vgets(ed, b':', &mut cmdbuf, 80)?;
        if cmdlen < 0 {
            return Ok(());
        }

        if cmdlen == 0 {
            cmdbuf.clear();
            cmdbuf.extend_from_slice(b".+1");
            ed.curses.qaddch(b'\r');
            ed.curses.clrtoeol();
        } else {
            ed.curses.addch(b'\n');
        }
        ed.curses.refresh();

        doexcmd(ed, &cmdbuf)?;

        if ed.vars.significant || markline(ed.vars.cursor) != ed.ex.ex_oldline {
            ed.vars.significant = false;
            ed.ex.ex_oldline = markline(ed.vars.cursor);
            if ed.opts.o_autoprint[0] != 0 && ed.vars.mode == MODE_EX {
                let cur = ed.vars.cursor;
                cmd_print(ed, cur, cur, CMD_PRINT, false, b"")?;
            }
        }
    }
    Ok(())
}

/// `strcpy(build, src)` into `tmpblk.c`, keeping room for the NUL.
fn put_tmpblk(c: &mut [u8; BLKSIZE], build: &mut usize, src: &[u8]) {
    for &b in src {
        if *build < BLKSIZE - 1 {
            c[*build] = b;
            *build += 1;
        }
    }
}

// C: ex.c:doexcmd
pub fn doexcmd(ed: &mut Editor, cmdbuf: &[u8]) -> R<()> {
    let mut cmdbuf = cstr(cmdbuf);
    // C leaves these uninitialized when there is no file.
    let mut frommark: Mark = MARK_UNSET;
    let mut tomark: Mark = MARK_UNSET;

    ed.vars.u_line = 0;

    if ch(cmdbuf, 0) == b'"' {
        return Ok(());
    }

    while ch(cmdbuf, 0) == b':' {
        cmdbuf = &cmdbuf[1..];
    }

    let mut scan: usize = 0;
    if ed.vars.nlines < 1 {
        // no file, so don't allow addresses
    } else if ch(cmdbuf, scan) == b'%' {
        frommark = MARK_FIRST;
        tomark = mark_last(ed.vars.nlines);
        scan += 1;
    } else if ch(cmdbuf, scan) == b'0' {
        frommark = MARK_UNSET;
        tomark = MARK_UNSET;
        scan += 1;
    } else {
        frommark = ed.vars.cursor;
        scan += linespec(ed, &cmdbuf[scan..], &mut frommark)?;
        tomark = frommark;
        if frommark != 0 && ch(cmdbuf, scan) == b',' {
            scan += 1;
            scan += linespec(ed, &cmdbuf[scan.min(cmdbuf.len())..], &mut tomark)?;
        }
        if tomark == 0 {
            return Ok(());
        }
        if frommark > tomark {
            msg(ed, b"first address exceeds the second", &[]);
            return Ok(());
        }
        if markline(tomark) > markline(mark_last(ed.vars.nlines)) {
            let n = ed.vars.nlines;
            msg(ed, b"there are only %d lines in the file", &[Arg::D(n)]);
            return Ok(());
        }
    }
    let isdfl = scan == 0;

    while isascii(sc(cmdbuf, scan)) && isspace(sc(cmdbuf, scan)) {
        scan += 1;
    }

    if ch(cmdbuf, scan) == 0 {
        ed.vars.cursor = tomark;
        return Ok(());
    }

    let cmdlen = if isascii(sc(cmdbuf, scan)) && !isalpha(sc(cmdbuf, scan)) {
        1
    } else {
        let mut n = 1;
        while !isascii(sc(cmdbuf, scan + n)) || isalpha(sc(cmdbuf, scan + n)) {
            n += 1;
        }
        n
    };

    let word = &cmdbuf[scan..scan + cmdlen];
    let Some(cmdidx) = lookup(word) else {
        msg(
            ed,
            b"Unknown command \"%.*s\"",
            &[Arg::D(cmdlen as i64), Arg::S(word)],
        );
        return Ok(());
    };
    let ent = &CMDNAMES[cmdidx];
    let argt = ent.argt;
    let cmd = ent.code;
    let name = ent.name;

    scan += cmdlen;
    let forceit = if argt & BANG != 0 && ch(cmdbuf, scan) == b'!' {
        scan += 1;
        true
    } else {
        false
    };

    while isascii(sc(cmdbuf, scan)) && isspace(sc(cmdbuf, scan)) {
        scan += 1;
    }

    let rest = &cmdbuf[scan.min(cmdbuf.len())..];
    let extra: Vec<u8> = if argt & XFILE != 0 {
        if !rest.is_empty() {
            let mut build = 0;
            let mut iswild = false;
            let mut didsub = false;
            for &b in rest {
                match b {
                    b'%' => {
                        if ed.vars.origname.is_empty() {
                            msg(ed, b"No filename to substitute for %%", &[]);
                            return Ok(());
                        }
                        put_tmpblk(&mut ed.vars.tmpblk.c, &mut build, cstr(&ed.vars.origname));
                        didsub = true;
                    }
                    b'#' => {
                        if ed.vars.prevorig.is_empty() {
                            msg(ed, b"No filename to substitute for #", &[]);
                            return Ok(());
                        }
                        put_tmpblk(&mut ed.vars.tmpblk.c, &mut build, cstr(&ed.vars.prevorig));
                        didsub = true;
                    }
                    b'*' | b'?' | b'[' | b'`' | b'{' | b'$' | b'~' => {
                        put_tmpblk(&mut ed.vars.tmpblk.c, &mut build, &[b]);
                        iswild = true;
                    }
                    _ => put_tmpblk(&mut ed.vars.tmpblk.c, &mut build, &[b]),
                }
            }
            ed.vars.tmpblk.c[build] = 0;

            let t0 = ed.vars.tmpblk.c[0];
            if cmd == CMD_BANG
                || (cmd == CMD_READ && t0 == b'!')
                || (cmd == CMD_WRITE && t0 == b'!')
            {
                if didsub {
                    if ed.vars.mode != MODE_EX {
                        ed.curses.addch(b'\n');
                    }
                    let t = cstr(&ed.vars.tmpblk.c).to_vec();
                    ed.curses.addstr(&t);
                    ed.curses.addch(b'\n');
                    exrefresh(ed);
                }
            } else if iswild && t0 != b'>' {
                let names = cstr(&ed.vars.tmpblk.c).to_vec();
                wildcard(ed, &names);
            }
        } else if argt & NODFL == 0 {
            let mut build = 0;
            let o = ed.vars.origname.clone();
            put_tmpblk(&mut ed.vars.tmpblk.c, &mut build, cstr(&o));
            ed.vars.tmpblk.c[build] = 0;
        } else {
            ed.vars.tmpblk.c[0] = 0;
        }

        cstr(&ed.vars.tmpblk.c).to_vec()
    } else {
        rest.to_vec()
    };

    if argt & EXRCOK == 0 && ed.vars.nlines < 1 {
        msg(
            ed,
            b"Can't use the \"%s\" command in a %s file",
            &[Arg::S(name), Arg::S(EXRC)],
        );
        return Ok(());
    }
    if argt & (ZERO | EXRCOK) == 0 && frommark == MARK_UNSET {
        msg(
            ed,
            b"Can't use address 0 with \"%s\" command.",
            &[Arg::S(name)],
        );
        return Ok(());
    }
    if argt & FROM == 0 && frommark != ed.vars.cursor && ed.vars.nlines >= 1 {
        msg(
            ed,
            b"Can't use address with \"%s\" command.",
            &[Arg::S(name)],
        );
        return Ok(());
    }
    if argt & TO == 0 && tomark != frommark && ed.vars.nlines >= 1 {
        msg(
            ed,
            b"Can't use a range with \"%s\" command.",
            &[Arg::S(name)],
        );
        return Ok(());
    }
    if argt & EXTRA == 0 && !extra.is_empty() {
        msg(
            ed,
            b"Extra characters after \"%s\" command.",
            &[Arg::S(name)],
        );
        return Ok(());
    }
    if argt & NOSPC != 0 && !(cmd == CMD_READ && (forceit || ch(&extra, 0) == b'!')) {
        for &b in &extra {
            if isspace(schar(b)) {
                let what: &[u8] = if argt & XFILE != 0 {
                    b"filenames"
                } else {
                    b"arguments"
                };
                msg(
                    ed,
                    b"Too many %s to \"%s\" command.",
                    &[Arg::S(what), Arg::S(name)],
                );
                return Ok(());
            }
        }
    }

    if isdfl && argt & DFLALL != 0 {
        frommark = MARK_FIRST;
        tomark = mark_last(ed.vars.nlines);
    } else if isdfl && argt & DFLNONE != 0 {
        frommark = 0;
        tomark = 0;
    }

    if argt & NL != 0 && ed.vars.mode != MODE_EX && !ed.vars.exwrote {
        ed.curses.addch(b'\n');
        exrefresh(ed);
    }

    (ent.func)(ed, frommark, tomark, cmd, forceit, &extra)
}

// C: ex.c:doexrc
pub fn doexrc(ed: &mut Editor, filename: &[u8]) -> R<bool> {
    let mut buf = [0u8; MAXRCLEN];

    let Ok(mut fd) = os::open(cstr(filename), os::O_RDONLY, 0) else {
        return Ok(false);
    };
    let len = tread(&mut fd, &mut buf);
    drop(fd);

    exstring(ed, &buf[..len.max(0) as usize])?;

    Ok(true)
}

// C: ex.c:exstring
pub fn exstring(ed: &mut Editor, buf: &[u8]) -> R<()> {
    let len = buf.len();
    let mut cmd = 0;
    while cmd < len {
        let mut end = cmd;
        while end < len && buf[end] != b'\n' && buf[end] != b'|' {
            end += 1;
        }

        doexcmd(ed, &buf[cmd..end])?;
        cmd = end + 1;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parseptrn_splits_pattern() {
        let mut b = b"/a\\/b/rest\0".to_vec();
        let n = parseptrn(&mut b);
        assert_eq!(&b[..6], b"/a\\/b\0");
        assert_eq!(cstr(&b[n..]), b"rest");
        let mut b = b"?abc".to_vec();
        assert_eq!(parseptrn(&mut b), 4);
        assert_eq!(b, b"?abc");
        let mut b = b"/x\\".to_vec();
        assert_eq!(parseptrn(&mut b), 3);
    }

    #[test]
    fn abbreviations_resolve_in_table_order() {
        let name = |w: &[u8]| lookup(w).map(|i| CMDNAMES[i].name);
        assert_eq!(name(b"r"), Some(&b"read"[..]));
        assert_eq!(name(b"re"), Some(&b"read"[..]));
        assert_eq!(name(b"rew"), Some(&b"rewind"[..]));
        assert_eq!(name(b"s"), Some(&b"substitute"[..]));
        assert_eq!(name(b"se"), Some(&b"set"[..]));
        assert_eq!(name(b"sh"), Some(&b"shell"[..]));
        assert_eq!(name(b"c"), Some(&b"change"[..]));
        assert_eq!(name(b"co"), Some(&b"copy"[..]));
        assert_eq!(name(b"ch"), Some(&b"change"[..]));
        assert_eq!(name(b"chd"), Some(&b"chdir"[..]));
        assert_eq!(name(b"N"), Some(&b"Next"[..]));
        assert_eq!(name(b"wq"), Some(&b"wq"[..]));
        assert_eq!(name(b"w"), Some(&b"write"[..]));
        assert_eq!(name(b"ma"), Some(&b"map"[..]));
        assert_eq!(name(b"!"), Some(&b"!"[..]));
        assert_eq!(name(b"abbr"), None);
        assert_eq!(name(b"setx"), None);
    }

    #[test]
    fn comment_and_empty_commands() {
        let mut ed = Editor::new();
        ed.vars.cursor = 5;
        doexcmd(&mut ed, b"\"set ts=4").unwrap();
        assert_eq!(ed.vars.cursor, 5);
        // with no file, an empty command moves the cursor to MARK_UNSET
        doexcmd(&mut ed, b"::").unwrap();
        assert_eq!(ed.vars.cursor, 0);
        exstring(&mut ed, b"\"a|\"b\n").unwrap();
    }
}
