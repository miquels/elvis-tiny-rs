//! `cmd2.c`: ex commands (part 2).

use crate::blk::{afterdo, beforedo, undo};
use crate::config::tread;
use crate::cut::{cut, cutname, paste};
use crate::ex::{doexrc, linespec, parseptrn};
use crate::misc::fetchline;
use crate::modify::{add, change, delete};
use crate::opts::Opt;
use crate::os;
use crate::redraw::redraw;
use crate::regexp::{regcomp, regexec};
use crate::regsub::regsub;
use crate::system::filter;
use crate::tio::{Arg, exrefresh, msg, sprintf, vgets};
use crate::vars::Editor;
use crate::vi_h::Mode;
use crate::vi_h::{
    BLKSIZE, Cmd, Interrupted, MARK_FIRST, MARK_UNSET, Mark, R, ch, cstr, mark_at_line, mark_last,
    markidx, markline, schar,
};

const BLK: Mark = BLKSIZE as Mark;

#[derive(Default)]
pub struct Cmd2 {
    pub cmd_substitute_optp: bool,
    pub cmd_substitute_optg: bool,
    pub cmd_substitute_optc: bool,
}

// C: cmd2.c:cmd_substitute
pub fn cmd_substitute(
    ed: &mut Editor,
    frommark: Mark,
    tomark: Mark,
    cmd: Cmd,
    bang: bool,
    extra: &[u8],
) -> R<()> {
    ed.vars.rptlines = -1;

    let mut buf = extra.to_vec();
    buf.push(0);
    let mut subst: Vec<u8>;
    let mut sidx = 0;
    let re = if cmd == Cmd::SubAgain {
        subst = if ed.opts.on(Opt::Magic) {
            b"~".to_vec()
        } else {
            b"\\~".to_vec()
        };
        let re = regcomp(ed, b"");

        if bang {
            ed.cmd2.cmd_substitute_optp = false;
            ed.cmd2.cmd_substitute_optc = false;
        }
        re
    } else {
        if ch(extra, 0) != b'/' && ch(extra, 0) != b'?' {
            msg(ed, b"Usage: s/regular expression/new text/", &[]);
            return Ok(());
        }

        sidx = parseptrn(&mut buf);
        subst = cstr(buf.get(sidx..).unwrap_or(&[])).to_vec();
        regcomp(ed, cstr(&buf[1..]))
    };

    let Some(mut re) = re else {
        return Ok(());
    };

    if cmd == Cmd::Substitute {
        let delim = buf[0];
        let mut opt = sidx;
        while ch(&buf, opt) != 0 && buf[opt] != delim {
            if buf[opt] == b'\\' && ch(&buf, opt + 1) != 0 {
                opt += 1;
            }
            opt += 1;
        }
        subst = buf[sidx.min(opt)..opt].to_vec();
        if ch(&buf, opt) != 0 {
            opt += 1;
        }

        if !ed.opts.on(Opt::Edcompatible) {
            ed.cmd2.cmd_substitute_optp = false;
            ed.cmd2.cmd_substitute_optg = false;
            ed.cmd2.cmd_substitute_optc = false;
        }
        while ch(&buf, opt) != 0 {
            let o = buf[opt];
            opt += 1;
            match o {
                b'p' => ed.cmd2.cmd_substitute_optp = !ed.cmd2.cmd_substitute_optp,
                b'g' => ed.cmd2.cmd_substitute_optg = !ed.cmd2.cmd_substitute_optg,
                b'c' => ed.cmd2.cmd_substitute_optc = !ed.cmd2.cmd_substitute_optc,
                b' ' | b'\t' => {}
                _ => {
                    msg(ed, b"Subst options are p, c, and g -- not %c", &[Arg::C(o)]);
                    return Ok(());
                }
            }
        }
    }
    let optp = ed.cmd2.cmd_substitute_optp;
    let optg = ed.cmd2.cmd_substitute_optg;
    let optc = ed.cmd2.cmd_substitute_optc;

    if (optc || optp) && ed.vars.mode == Mode::Vi {
        ed.curses.addch(b'\n');
        exrefresh(ed);
    }

    let mut toolong = false;
    beforedo(ed, false);
    let mut chline: i64 = 0;
    let mut chsub: i64 = 0;

    let mut l = markline(frommark);
    'lines: while l <= markline(tomark) {
        if os::take_sigint() {
            return Err(Interrupted);
        }
        let line = fetchline(ed, l);

        if regexec(ed, &mut re, &line, true) {
            chline += 1;

            // `base` is where the subject of the last regexec() starts.
            let mut base = 0;
            let mut s = 0;
            let mut d: Vec<u8> = Vec::with_capacity(BLKSIZE);

            loop {
                if os::take_sigint() {
                    return Err(Interrupted);
                }
                chsub += 1;

                ed.redraw.range(l, l + 1, l + 1);

                let sp = base + re.startp[0].unwrap_or(0);
                if s < sp {
                    d.extend_from_slice(&line[s..sp]);
                }

                regsub(ed, &re, &line[base..], &subst, &mut d);
                s = base + re.endp[0].unwrap_or(0);

                // C writes into the BLKSIZE-byte tmpblk without a bound.
                if d.len() + 2 > BLKSIZE {
                    toolong = true;
                    break 'lines;
                }

                if !(optg && regexec(ed, &mut re, &line[s..], false)) {
                    break;
                }
                base = s;
            }

            d.extend_from_slice(&line[s..]);
            d.push(b'\n');
            if d.len() + 1 > BLKSIZE {
                toolong = true;
                break 'lines;
            }
            change(ed, mark_at_line(l), mark_at_line(l + 1), &d);

            if optp {
                ed.curses.addstr(&d);
                exrefresh(ed);
            }

            ed.vars.cursor = mark_at_line(l);
        }
        l += 1;
    }
    afterdo(ed);

    ed.vars.mustredraw = true;

    if toolong {
        msg(ed, b"Substitution would make a line too long", &[]);
        return Ok(());
    }

    if ed.vars.doingglobal {
        ed.vars.rptlines = chline;
        ed.vars.rptlabel = b"changed";
        return Ok(());
    }

    if chsub == 0 {
        msg(ed, b"Substitution failed", &[]);
    } else if chline >= (ed.opts.num(Opt::Report) as i64) {
        msg(
            ed,
            b"%ld substitutions on %ld lines",
            &[Arg::D(chsub), Arg::D(chline)],
        );
    }
    Ok(())
}

// C: cmd2.c:cmd_delete
pub fn cmd_delete(
    ed: &mut Editor,
    frommark: Mark,
    tomark: Mark,
    cmd: Cmd,
    _bang: bool,
    extra: &[u8],
) -> R<()> {
    let mut extra = extra;
    if ch(extra, 0) == b'"' {
        extra = &extra[1..];
    }
    if ch(extra, 0) != 0 {
        cutname(ed, schar(extra[0]));
    }

    let frommark = frommark & !(BLK - 1);
    let tomark = (tomark & !(BLK - 1)) + BLK;

    cut(ed, frommark, tomark);

    if cmd != Cmd::Yank {
        let curs2 = ed.vars.cursor;
        beforedo(ed, false);
        delete(ed, frommark, tomark);
        afterdo(ed);
        if curs2 > tomark {
            ed.vars.cursor = curs2 - tomark + frommark;
        } else if curs2 > frommark {
            ed.vars.cursor = frommark;
        }
    }
    Ok(())
}

// C: cmd2.c:cmd_append
pub fn cmd_append(
    ed: &mut Editor,
    frommark: Mark,
    tomark: Mark,
    cmd: Cmd,
    bang: bool,
    extra: &[u8],
) -> R<()> {
    beforedo(ed, false);
    if cmd == Cmd::Change {
        cmd_delete(ed, frommark, tomark, cmd, bang, extra)?;
    }

    let mut l = markline(frommark);
    if cmd == Cmd::Append {
        l += 1;
    }

    let mut buf = Vec::new();
    while vgets(ed, 0, &mut buf, BLKSIZE)? {
        ed.curses.addch(b'\n');
        if buf == b"." {
            break;
        }

        buf.push(b'\n');
        add(ed, mark_at_line(l), &buf);
        l += 1;
    }
    afterdo(ed);

    redraw(ed, MARK_UNSET, false);
    Ok(())
}

// C: cmd2.c:cmd_put
pub fn cmd_put(
    ed: &mut Editor,
    frommark: Mark,
    _tomark: Mark,
    _cmd: Cmd,
    _bang: bool,
    extra: &[u8],
) -> R<()> {
    let mut extra = extra;
    if ch(extra, 0) == b'"' {
        extra = &extra[1..];
    }
    if ch(extra, 0) != 0 {
        cutname(ed, schar(extra[0]));
    }

    beforedo(ed, false);
    ed.vars.cursor = paste(ed, frommark, true, false);
    afterdo(ed);
    Ok(())
}

// C: cmd2.c:cmd_join
pub fn cmd_join(
    ed: &mut Editor,
    frommark: Mark,
    tomark: Mark,
    _cmd: Cmd,
    _bang: bool,
    _extra: &[u8],
) -> R<()> {
    let mut frommark = frommark;
    let mut tomark = tomark;

    if markline(frommark) == ed.vars.nlines {
        msg(ed, b"Nothing to join with this line", &[]);
        return Ok(());
    }
    if markline(frommark) == markline(tomark) {
        tomark += BLK;
    }

    let mut l = markline(frommark);
    let mut text = fetchline(ed, l);

    loop {
        l += 1;
        if l > markline(tomark) {
            break;
        }
        let line = fetchline(ed, l);

        let mut scan = 0;
        while scan < line.len() && (line[scan] == b'\t' || line[scan] == b' ') {
            scan += 1;
        }
        let scan = cstr(&line[scan..]);

        if scan.len() + text.len() + 3 > BLKSIZE {
            msg(
                ed,
                b"Can't join -- the resulting line would be too long",
                &[],
            );
            return Ok(());
        }

        if let Some(&last) = text.last()
            && (last == b'.' || last == b'?' || last == b'!')
        {
            text.push(b' ');
        }
        text.push(b' ');
        text.extend_from_slice(scan);
    }
    text.push(b'\n');

    beforedo(ed, false);
    frommark &= !(BLK - 1);
    tomark &= !(BLK - 1);
    tomark += BLK;
    change(ed, frommark, tomark, &text);
    afterdo(ed);

    ed.vars.rptlines = markline(tomark) - markline(frommark) - 1;
    ed.vars.rptlabel = b"joined";
    Ok(())
}

// C: cmd2.c:cmd_shift
pub fn cmd_shift(
    ed: &mut Editor,
    frommark: Mark,
    tomark: Mark,
    cmd: Cmd,
    bang: bool,
    _extra: &[u8],
) -> R<()> {
    if markline(frommark) != markline(tomark) {
        ed.vars.mustredraw = true;
        ed.redraw.range(
            markline(frommark),
            markline(tomark) + 1,
            markline(tomark) + 1,
        );
    }

    beforedo(ed, false);
    let mut l = markline(frommark);
    while l <= markline(tomark) {
        if os::take_sigint() {
            return Err(Interrupted);
        }
        let text = fetchline(ed, l);
        if text.is_empty() && !bang {
            l += 1;
            continue;
        }

        let tabstop = ed.opts.num(Opt::Tabstop);
        let mut oldidx = 0usize;
        let mut oldcol: i32 = 0;
        while ch(&text, oldidx) == b' ' || ch(&text, oldidx) == b'\t' {
            if text[oldidx] == b' ' {
                oldcol += 1;
            } else {
                oldcol += tabstop - (oldcol % tabstop);
            }
            oldidx += 1;
        }

        let sw = ed.opts.num(Opt::Shiftwidth);
        let mut newcol = if cmd == Cmd::ShiftRight {
            oldcol + sw
        } else {
            (oldcol - sw).max(0)
        };

        if oldcol == newcol {
            l += 1;
            continue;
        }

        let mut indent = Vec::new();
        while newcol >= tabstop {
            indent.push(b'\t');
            newcol -= tabstop;
        }
        while newcol > 0 {
            indent.push(b' ');
            newcol -= 1;
        }

        change(
            ed,
            mark_at_line(l),
            mark_at_line(l) + oldidx as Mark,
            &indent,
        );
        l += 1;
    }
    afterdo(ed);

    ed.vars.rptlines = markline(tomark) - markline(frommark) + 1;
    ed.vars.rptlabel = if cmd == Cmd::ShiftRight {
        b">ed"
    } else {
        b"<ed"
    };
    Ok(())
}

// C: cmd2.c:cmd_read
pub fn cmd_read(
    ed: &mut Editor,
    frommark: Mark,
    _tomark: Mark,
    _cmd: Cmd,
    _bang: bool,
    extra: &[u8],
) -> R<()> {
    if ch(extra, 0) == b'!' {
        filter(ed, frommark, MARK_UNSET, &extra[1..])?;
        return Ok(());
    }

    let Ok(mut fd) = os::open(extra, os::O_RDONLY, 0) else {
        msg(ed, b"Can't open \"%s\"", &[Arg::S(extra)]);
        return Ok(());
    };

    beforedo(ed, false);
    let frommark = (frommark | (BLK - 1)) + 1;
    let mut tomark = frommark;
    let mut len: i32 = 0;
    let mut hadnul = false;
    let mut addnl = false;

    add(ed, tomark, b"\n");

    let mut buf = [0u8; BLKSIZE];
    loop {
        let rc = tread(&mut fd, &mut buf[..BLKSIZE - 1]);
        if rc <= 0 {
            break;
        }
        if os::take_sigint() {
            return Err(Interrupted);
        }
        let rc = rc as usize;

        let mut lines: i64 = 0;
        for b in &mut buf[..rc] {
            if *b != b'\n' && len + 2 > BLKSIZE as i32 {
                *b = b'\n';
                addnl = true;
            }

            if *b == 0 {
                *b = 0x80;
                hadnul = true;
            }

            if *b == b'\n' {
                len = 0;
                lines += 1;
            } else {
                len += 1;
            }
        }

        add(ed, tomark, &buf[..rc]);
        tomark += mark_at_line(lines) + len as Mark - markidx(tomark) as Mark;
    }

    if len > 0 {
        msg(ed, b"Last line had no newline", &[]);
        tomark += BLK;
    } else {
        delete(ed, tomark, (tomark | (BLK - 1)) + 1);
    }
    afterdo(ed);

    drop(fd);

    ed.vars.rptlines = markline(tomark) - markline(frommark);
    ed.vars.rptlabel = b"read";

    if addnl {
        msg(ed, b"Newlines were added to break up long lines", &[]);
    }
    if hadnul {
        msg(ed, b"NULs were converted to 0x80", &[]);
    }
    Ok(())
}

// C: cmd2.c:cmd_undo
pub fn cmd_undo(
    ed: &mut Editor,
    _frommark: Mark,
    _tomark: Mark,
    _cmd: Cmd,
    _bang: bool,
    _extra: &[u8],
) -> R<()> {
    undo(ed);
    Ok(())
}

/// `sprintf("\\%03o", *scan)` with `*scan` a negative signed char.
fn octal_escape(b: u8) -> Vec<u8> {
    sprintf(b"\\%03o", &[Arg::D(schar(b) as i64)])
}

// C: cmd2.c:cmd_print
pub fn cmd_print(
    ed: &mut Editor,
    frommark: Mark,
    tomark: Mark,
    cmd: Cmd,
    _bang: bool,
    _extra: &[u8],
) -> R<()> {
    let mut l = markline(frommark);
    while l <= markline(tomark) {
        if os::take_sigint() {
            return Err(Interrupted);
        }
        let mut col: i32;
        if cmd == Cmd::Number {
            let num = sprintf(b"%6ld  ", &[Arg::D(l)]);
            ed.curses.qaddstr(&num);
            col = 8;
        } else {
            col = 0;
        }

        let line = fetchline(ed, l);
        let tabstop = ed.opts.num(Opt::Tabstop);
        for &b in &line {
            let c = schar(b);
            if b == b'\t' && cmd != Cmd::List {
                loop {
                    ed.curses.qaddch(b' ');
                    col += 1;
                    if col % tabstop == 0 {
                        break;
                    }
                }
            } else if (0..b' ' as i32).contains(&c) || b == 0o177 {
                ed.curses.qaddctl(b);
                col += 2;
            } else if b & 0x80 != 0 && cmd == Cmd::List {
                ed.curses.qaddstr(&octal_escape(b));
                col += 4;
            } else {
                ed.curses.qaddch(b);
                col += 1;
            }

            if !ed.curses.has_am() && col >= ed.curses.cols {
                ed.curses.addch(b'\n');
                col -= ed.curses.cols;
            }
        }
        if cmd == Cmd::List {
            ed.curses.qaddch(b'$');
        }
        ed.curses.addch(b'\n');
        exrefresh(ed);
        l += 1;
    }
    Ok(())
}

// C: cmd2.c:cmd_move
pub fn cmd_move(
    ed: &mut Editor,
    frommark: Mark,
    tomark: Mark,
    cmd: Cmd,
    _bang: bool,
    extra: &[u8],
) -> R<()> {
    let mut destmark = ed.vars.cursor;
    if extra == b"0" {
        destmark = 0;
    } else if linespec(ed, extra, &mut destmark)? == 0 || destmark == 0 {
        msg(ed, b"invalid destination address", &[]);
        return Ok(());
    }

    let frommark = frommark & !(BLK - 1);
    let tomark = (tomark & !(BLK - 1)) + BLK;
    destmark = (destmark & !(BLK - 1)) + BLK;

    // C reports the error but still performs the move.
    if cmd == Cmd::Move && destmark >= frommark && destmark < tomark {
        msg(ed, b"invalid destination address", &[]);
    }

    beforedo(ed, false);
    cutname(ed, 0);
    cut(ed, frommark, tomark);

    if cmd != Cmd::Copy {
        delete(ed, frommark, tomark);
        if destmark >= frommark {
            destmark -= tomark - frommark;
        }
    }

    paste(ed, destmark, false, false);
    afterdo(ed);

    ed.vars.cursor = destmark + (tomark - frommark) - BLK;
    if ed.vars.cursor < MARK_FIRST || ed.vars.cursor >= mark_last(ed.vars.nlines) + BLK {
        ed.vars.cursor = mark_last(ed.vars.nlines);
    }

    ed.vars.rptlabel = if cmd == Cmd::Copy {
        b"copied"
    } else {
        b"moved"
    };
    Ok(())
}

// C: cmd2.c:cmd_source
pub fn cmd_source(
    ed: &mut Editor,
    _frommark: Mark,
    _tomark: Mark,
    _cmd: Cmd,
    _bang: bool,
    extra: &[u8],
) -> R<()> {
    if extra.is_empty() {
        msg(ed, b"\"source\" requires a filename", &[]);
        return Ok(());
    }

    doexrc(ed, extra)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn octal_escape_matches_c_signed_char() {
        // printf("\\%03o", (char)0x80) with signed char promotes to -128.
        assert_eq!(octal_escape(0x80), b"\\37777777600".to_vec());
        assert_eq!(octal_escape(0xff), b"\\37777777777".to_vec());
    }
}
