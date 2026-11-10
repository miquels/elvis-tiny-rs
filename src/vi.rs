//! `vi.c`: the visual-mode command loop.

use crate::misc::{fetchline, pfetch};
use crate::move1::{
    m_bparagraph, m_bsection, m_fparagraph, m_front, m_fsection, m_left, m_match, m_rear, m_right,
    m_tocol, m_tomark, m_updnto,
};
use crate::move2::{m_Nsrch, m_bsrch, m_fsrch, m_nsrch};
use crate::move3::{m__ch, m_Fch, m_Tch, m_fch, m_tch};
use crate::move4::{m_row, m_scroll, m_z};
use crate::move5::{m_bword, m_eword, m_fword};
use crate::os;
use crate::redraw::{idx2col, redraw};
use crate::tio::{Arg, beep, getkey, msg, vgets};
use crate::vars::Editor;
use crate::vcmd::{
    v_1ex, v_again, v_change, v_delete, v_filter, v_increment, v_insert, v_join, v_keyword,
    v_lshift, v_mark, v_overtype, v_paste, v_quit, v_redraw, v_replace, v_rshift, v_selcut,
    v_status, v_subst, v_switch, v_tag, v_ulcase, v_undo, v_undoline, v_xchar, v_xit, v_yank,
};
use crate::vi_h::LINEMASK;
use crate::vi_h::{
    BLKSIZE, MARK_FIRST, MARK_UNSET, MODE_COLON, MODE_VI, Mark, R, WHEN_VICMD, isupper, isword,
    mark_at_line, mark_last, markidx, markline, schar,
};

#[derive(Default)]
pub struct Vi {
    pub adjmove_colno: i32,
}

/*------------------------------------------------------------------------*/
/* struct keystru and vikeys[]                                             */

const NO_FLAGS: u8 = 0x00;
const MVMT: u8 = 0x01;
const PTMV: u8 = 0x02;
const FRNT: u8 = 0x04;
const INCL: u8 = 0x08;
const LNMD: u8 = 0x10;
const NCOL: u8 = 0x20;
const NREL: u8 = 0x40;
const SDOT: u8 = 0x80;

/// What a key does and the arguments its function takes (C's `args`
/// field together with the `func` union).
#[derive(Clone, Copy)]
enum Func {
    None,
    Zero,
    Digit,
    NoArg(fn(&mut Editor) -> R<Mark>),
    Keyword(fn(&mut Editor, &[u8], Mark, i64) -> R<Mark>),
    Cursor(fn(&mut Editor, Mark) -> R<Mark>),
    CursorCount(fn(&mut Editor, Mark, i64) -> R<Mark>),
    /// Reads one more key and passes it.
    CursorCntKey(fn(&mut Editor, Mark, i64, i32) -> R<Mark>),
    /// Passes the command key itself.
    CursorCntCmd(fn(&mut Editor, Mark, i64, i32) -> R<Mark>),
    CursorText(fn(&mut Editor, Mark, &[u8]) -> R<Mark>),
    /// An operator that waits for a motion.
    CursorMoved(fn(&mut Editor, Mark, Mark) -> R<Mark>),
    /// An operator applied up to the end of the line.
    CursorEol(fn(&mut Editor, Mark, Mark) -> R<Mark>),
}

#[derive(Clone, Copy)]
struct Keystru {
    func: Func,
    flags: u8,
}

const fn k(func: Func, flags: u8) -> Keystru {
    Keystru { func, flags }
}

const NO_FUNC: Keystru = k(Func::None, NO_FLAGS);
#[rustfmt::skip]
static VIKEYS: [Keystru; 128] = [
/* NUL not defined      */ NO_FUNC,
/* ^A  not defined      */ NO_FUNC,
/* ^B  page backward    */ k(Func::CursorCntCmd(m_scroll), FRNT),
/* ^C  not defined      */ NO_FUNC,
/* ^D  scroll dn 1/2page*/ k(Func::CursorCntCmd(m_scroll), NCOL),
/* ^E  scroll up        */ k(Func::CursorCntCmd(m_scroll), NCOL),
/* ^F  page forward     */ k(Func::CursorCntCmd(m_scroll), FRNT),
/* ^G  show file status */ k(Func::NoArg(v_status), NO_FLAGS),
/* ^H  move left, like h*/ k(Func::CursorCount(m_left), MVMT),
/* ^I  not defined      */ NO_FUNC,
/* ^J  move down        */ k(Func::CursorCntCmd(m_updnto), MVMT|LNMD),
/* ^K  not defined      */ NO_FUNC,
/* ^L  redraw screen    */ k(Func::NoArg(v_redraw), NO_FLAGS),
/* ^M  mv front next ln */ k(Func::CursorCntCmd(m_updnto), MVMT|FRNT|LNMD),
/* ^N  move down        */ k(Func::CursorCntCmd(m_updnto), MVMT|LNMD),
/* ^O  not defined      */ NO_FUNC,
/* ^P  move up          */ k(Func::CursorCntCmd(m_updnto), MVMT|LNMD),
/* ^Q  not defined      */ NO_FUNC,
/* ^R  redraw screen    */ k(Func::NoArg(v_redraw), NO_FLAGS),
/* ^S  not defined      */ NO_FUNC,
/* ^T  not defined      */ NO_FUNC,
/* ^U  scroll up 1/2page*/ k(Func::CursorCntCmd(m_scroll), NCOL),
/* ^V  not defined      */ NO_FUNC,
/* ^W  not defined      */ NO_FUNC,
/* ^X  not defined      */ NO_FUNC,
/* ^Y  scroll down      */ k(Func::CursorCntCmd(m_scroll), NCOL),
/* ^Z  not defined      */ NO_FUNC,
/* ESC not defined      */ NO_FUNC,
/* ^\  not defined      */ NO_FUNC,
/* ^]  keyword is tag   */ k(Func::Keyword(v_tag), NO_FLAGS),
/* ^^  previous file    */ k(Func::NoArg(v_switch), NO_FLAGS),
/* ^_  not defined      */ NO_FUNC,
/* SPC move right,like l*/ k(Func::CursorCount(m_right), MVMT),
/*  !  run thru filter  */ k(Func::CursorMoved(v_filter), FRNT|LNMD|INCL),
/*  "  select cut buffer*/ k(Func::CursorCntKey(v_selcut), PTMV),
/*  #  increment number */ k(Func::Keyword(v_increment), SDOT),
/*  $  move to rear     */ k(Func::Cursor(m_rear), MVMT|INCL),
/*  %  move to match    */ k(Func::Cursor(m_match), MVMT|INCL),
/*  &  repeat subst     */ k(Func::CursorMoved(v_again), SDOT|NCOL|LNMD|INCL),
/*  '  move to a mark   */ k(Func::CursorCntKey(m_tomark), MVMT|FRNT|NREL|LNMD|INCL),
/*  (  not defined      */ NO_FUNC,
/*  )  not defined      */ NO_FUNC,
/*  *  not defined      */ NO_FUNC,
/*  +  mv front next ln */ k(Func::CursorCntCmd(m_updnto), MVMT|FRNT|LNMD),
/*  ,  reverse [fFtT] cmd*/ k(Func::CursorCntCmd(m__ch), MVMT|INCL),
/*  -  mv front prev ln */ k(Func::CursorCntCmd(m_updnto), MVMT|FRNT|LNMD),
/*  .  special...       */ NO_FUNC,
/*  /  forward search   */ k(Func::CursorText(m_fsrch), MVMT|NREL),
/*  0  part of count?   */ k(Func::Zero, MVMT|PTMV),
/*  1  part of count    */ k(Func::Digit, PTMV),
/*  2  part of count    */ k(Func::Digit, PTMV),
/*  3  part of count    */ k(Func::Digit, PTMV),
/*  4  part of count    */ k(Func::Digit, PTMV),
/*  5  part of count    */ k(Func::Digit, PTMV),
/*  6  part of count    */ k(Func::Digit, PTMV),
/*  7  part of count    */ k(Func::Digit, PTMV),
/*  8  part of count    */ k(Func::Digit, PTMV),
/*  9  part of count    */ k(Func::Digit, PTMV),
/*  :  run single EX cmd*/ k(Func::CursorText(v_1ex), NO_FLAGS),
/*  ;  repeat [fFtT] cmd*/ k(Func::CursorCntCmd(m__ch), MVMT|INCL),
/*  <  shift text left  */ k(Func::CursorMoved(v_lshift), SDOT|FRNT|LNMD|INCL),
/*  =  not defined      */ NO_FUNC,
/*  >  shift text right */ k(Func::CursorMoved(v_rshift), SDOT|FRNT|LNMD|INCL),
/*  ?  backward search  */ k(Func::CursorText(m_bsrch), MVMT|NREL),
/*  @  undefined        */ NO_FUNC,
/*  A  append at EOL    */ k(Func::CursorCntCmd(v_insert), SDOT),
/*  B  move back Word   */ k(Func::CursorCntCmd(m_bword), MVMT),
/*  C  change to EOL    */ k(Func::CursorEol(v_change), SDOT),
/*  D  delete to EOL    */ k(Func::CursorEol(v_delete), SDOT),
/*  E  move end of Word */ k(Func::CursorCntCmd(m_eword), MVMT|INCL),
/*  F  move bk to char  */ k(Func::CursorCntKey(m_Fch), MVMT|INCL),
/*  G  move to line #   */ k(Func::CursorCntCmd(m_updnto), MVMT|NREL|LNMD|FRNT|INCL),
/*  H  move to row      */ k(Func::CursorCntCmd(m_row), MVMT|FRNT),
/*  I  insert at front  */ k(Func::CursorCntCmd(v_insert), SDOT),
/*  J  join lines       */ k(Func::CursorCount(v_join), SDOT),
/*  K  look up keyword  */ k(Func::Keyword(v_keyword), NO_FLAGS),
/*  L  move to last row */ k(Func::CursorCntCmd(m_row), MVMT|FRNT),
/*  M  move to mid row  */ k(Func::CursorCntCmd(m_row), MVMT|FRNT),
/*  N  reverse prev srch*/ k(Func::Cursor(m_Nsrch), MVMT),
/*  O  insert above line*/ k(Func::CursorCntCmd(v_insert), SDOT),
/*  P  paste before     */ k(Func::CursorCntCmd(v_paste), NO_FLAGS),
/*  Q  quit to EX mode  */ k(Func::NoArg(v_quit), NO_FLAGS),
/*  R  overtype         */ k(Func::Cursor(v_overtype), SDOT),
/*  S  change line      */ k(Func::CursorMoved(v_change), SDOT),
/*  T  move bk to char  */ k(Func::CursorCntKey(m_Tch), MVMT|INCL),
/*  U  undo whole line  */ k(Func::Cursor(v_undoline), FRNT),
/*  V  not defined      */ NO_FUNC,
/*  W  move forward Word*/ k(Func::CursorCntCmd(m_fword), MVMT),
/*  X  delete to left   */ k(Func::CursorCntCmd(v_xchar), SDOT),
/*  Y  yank text        */ k(Func::CursorMoved(v_yank), NCOL),
/*  Z  save file & exit */ k(Func::CursorCntKey(v_xit), NO_FLAGS),
/*  [  move back section*/ k(Func::CursorCntKey(m_bsection), MVMT|LNMD|NREL),
/*  \  not defined      */ NO_FUNC,
/*  ]  move fwd section */ k(Func::CursorCntKey(m_fsection), MVMT|LNMD|NREL),
/*  ^  move to front    */ k(Func::Cursor(m_front), MVMT),
/*  _  not defined      */ NO_FUNC,
/*  `  move to mark     */ k(Func::CursorCntKey(m_tomark), MVMT|NREL),
/*  a  append at cursor */ k(Func::CursorCntCmd(v_insert), SDOT),
/*  b  move back word   */ k(Func::CursorCntCmd(m_bword), MVMT),
/*  c  change text      */ k(Func::CursorMoved(v_change), SDOT),
/*  d  delete op        */ k(Func::CursorMoved(v_delete), SDOT|NCOL),
/*  e  move end word    */ k(Func::CursorCntCmd(m_eword), MVMT|INCL),
/*  f  move fwd for char*/ k(Func::CursorCntKey(m_fch), MVMT|INCL),
/*  g  not defined      */ NO_FUNC,
/*  h  move left        */ k(Func::CursorCount(m_left), MVMT),
/*  i  insert at cursor */ k(Func::CursorCntCmd(v_insert), SDOT),
/*  j  move down        */ k(Func::CursorCntCmd(m_updnto), MVMT|NCOL|LNMD),
/*  k  move up          */ k(Func::CursorCntCmd(m_updnto), MVMT|NCOL|LNMD),
/*  l  move right       */ k(Func::CursorCount(m_right), MVMT),
/*  m  define a mark    */ k(Func::CursorCntKey(v_mark), NO_FLAGS),
/*  n  repeat prev srch */ k(Func::Cursor(m_nsrch), MVMT),
/*  o  insert below line*/ k(Func::CursorCntCmd(v_insert), SDOT),
/*  p  paste after      */ k(Func::CursorCntCmd(v_paste), NO_FLAGS),
/*  q  not defined      */ NO_FUNC,
/*  r  replace chars    */ k(Func::CursorCntKey(v_replace), SDOT),
/*  s  subst N chars    */ k(Func::CursorCount(v_subst), SDOT),
/*  t  move fwd to char */ k(Func::CursorCntKey(m_tch), MVMT|INCL),
/*  u  undo             */ k(Func::Cursor(v_undo), NO_FLAGS),
/*  v  not defined      */ NO_FUNC,
/*  w  move fwd word    */ k(Func::CursorCntCmd(m_fword), MVMT),
/*  x  delete character */ k(Func::CursorCntCmd(v_xchar), SDOT),
/*  y  yank text        */ k(Func::CursorMoved(v_yank), NCOL),
/*  z  adjust scrn row  */ k(Func::CursorCntKey(m_z), NCOL),
/*  {  back paragraph   */ k(Func::CursorCount(m_bparagraph), MVMT|LNMD),
/*  |  move to column   */ k(Func::CursorCount(m_tocol), NREL),
/*  }  fwd paragraph    */ k(Func::CursorCount(m_fparagraph), MVMT|LNMD),
/*  ~  upper/lowercase  */ k(Func::CursorCount(v_ulcase), SDOT),
/* DEL not defined      */ NO_FUNC,
];

fn kwchar(b: u8) -> bool {
    isword(schar(b))
}

// C: vi.c:vi
// Out of line: inlining this into `main` makes the binary larger.
#[inline(never)]
pub fn vi(ed: &mut Editor) -> R<()> {
    let mut key: i32;
    let mut count: i64;
    let mut kp: usize;
    let mut tcurs: Mark;
    let mut prevkey: i32;
    let mut range: Mark = 0;
    let mut dotkey: i32;
    let mut dotpkey: i32 = 0;
    let mut dotkey2: i32 = 0;
    let mut dotcnt: i64 = 0;
    let mut firstkey: i32;
    let mut i: i32;

    redraw(ed, MARK_UNSET, false);

    dotkey = 0;

    firstkey = 0;
    if ed.opts.o_inputmode[0] != 0 {
        firstkey = b'i' as i32;
    }

    count = 0;
    prevkey = 0;
    while ed.vars.mode == MODE_VI {
        if markline(ed.vars.cursor) != ed.vars.u_line {
            ed.vars.u_line = 0;
        }

        if ed.vars.rptlines >= ed.opts.o_report[0] as i64 {
            let c = ed.vars.cursor;
            redraw(ed, c, false);
            let (n, label) = (ed.vars.rptlines, ed.vars.rptlabel);
            msg(ed, b"%ld lines %s", &[Arg::D(n), Arg::S(label)]);
        }
        ed.vars.rptlines = 0;

        if firstkey != 0 {
            key = firstkey;
            firstkey = 0;
        } else {
            loop {
                key = getkey(ed, WHEN_VICMD)?;
                if (0..=127).contains(&key) {
                    break;
                }
            }
        }

        if prevkey == b'c' as i32 {
            if key == b'w' as i32 {
                key = b'e' as i32;
            } else if key == b'W' as i32 {
                key = b'E' as i32;
            }

            if markidx(ed.vars.cursor) > 0 && (key == b'e' as i32 || key == b'E' as i32) {
                ed.vars.cursor -= 1;
            }
        }

        kp = key as usize;

        if prevkey != 0 && key != prevkey && (VIKEYS[kp].flags & (MVMT | PTMV)) == 0 {
            beep(ed);
            prevkey = 0;
            count = 0;
            continue;
        }

        if (VIKEYS[kp].flags & SDOT) != 0
            || (prevkey != 0 && (VIKEYS[prevkey as usize].flags & SDOT) != 0)
        {
            dotkey = key;
            dotpkey = prevkey;
            dotkey2 = 0;
            dotcnt = count;

            if ed.vars.u_line != markline(ed.vars.cursor) {
                ed.vars.u_line = markline(ed.vars.cursor);
                let l = ed.vars.u_line;
                ed.vars.u_text = fetchline(ed, l);
            }
        }

        if key == b'.' as i32 {
            key = dotkey;
            kp = key as usize;
            prevkey = dotpkey;
            if prevkey != 0 {
                range = ed.vars.cursor;
            }
            if count == 0 {
                count = dotcnt;
            }
            ed.vars.doingdot = true;

            if ed.vars.u_line != markline(ed.vars.cursor) {
                ed.vars.u_line = markline(ed.vars.cursor);
                let l = ed.vars.u_line;
                ed.vars.u_text = fetchline(ed, l);
            }
        } else {
            ed.vars.doingdot = false;
        }

        let cursor = ed.vars.cursor;
        tcurs = cursor;
        let keyptr = VIKEYS[kp];
        match keyptr.func {
            Func::Zero | Func::Digit => {
                if matches!(keyptr.func, Func::Zero) && count == 0 {
                    tcurs = cursor & LINEMASK;
                } else {
                    count = count
                        .wrapping_mul(10)
                        .wrapping_add((key - b'0' as i32) as i64);
                }
            }

            Func::Keyword(f) => 'kw: {
                pfetch(ed, markline(cursor));
                let ptext = &ed.vars.ptext;
                let at = |k: usize| ptext.get(k).copied().unwrap_or(0);
                let mut k = markidx(cursor);
                if !kwchar(at(k)) {
                    tcurs = MARK_UNSET;
                    key = k as i32;
                    break 'kw;
                }

                while k > 0 && kwchar(at(k - 1)) {
                    k -= 1;
                }
                tcurs = (cursor & LINEMASK) + k as Mark;

                // C copies into `char text[100]` without a bound.
                let mut text = Vec::new();
                loop {
                    text.push(at(k));
                    k += 1;
                    if !kwchar(at(k)) {
                        break;
                    }
                }
                key = k as i32;

                tcurs = f(ed, &text, tcurs, count)?;
                count = 0;
            }

            Func::NoArg(f) => {
                f(ed)?;
                count = 0;
            }

            Func::None => {
                beep(ed);
                count = 0;
            }

            Func::CursorCount(f) => {
                tcurs = f(ed, cursor, count)?;
                count = 0;
            }

            Func::Cursor(f) => {
                tcurs = f(ed, cursor)?;
                count = 0;
            }

            Func::CursorCntKey(f) => 'ck: {
                if ed.vars.doingdot {
                    tcurs = f(ed, cursor, count, dotkey2)?;
                } else {
                    i = getkey(ed, 0)?;
                    if i == 0o33 {
                        count = 0;
                        tcurs = MARK_UNSET;
                        break 'ck;
                    } else if i == (b'V' & 0x1f) as i32 {
                        i = getkey(ed, 0)?;
                    }

                    if (keyptr.flags & SDOT) != 0
                        || (prevkey != 0 && (VIKEYS[prevkey as usize].flags & SDOT) != 0)
                    {
                        dotkey2 = i;
                    }

                    tcurs = f(ed, cursor, count, i)?;
                }
                count = 0;
            }

            Func::CursorMoved(_) => {
                if key == b'&' as i32 || isupper(key) {
                    prevkey = key;
                }

                if prevkey != 0 {
                    if count == 0 {
                        count = 1;
                    }
                    range = cursor;
                    tcurs = range + mark_at_line(count - 1);
                    count = 0;
                } else {
                    prevkey = key;
                    range = cursor;
                    key = -1;
                }
            }

            Func::CursorEol(_) => {
                prevkey = key;
                pfetch(ed, markline(cursor));
                if ed.vars.plen == 0 {
                    range = cursor;
                    tcurs = cursor;
                    key = b' ' as i32;
                } else {
                    range = cursor;
                    tcurs = m_rear(ed, cursor)?;
                    key = b'$' as i32;
                }
                count = 0;
                kp = key as usize;
            }

            Func::CursorText(f) => {
                loop {
                    let mut buf = Vec::new();
                    if vgets(ed, key as u8, &mut buf, 100 - 1)? >= 0 {
                        ed.curses.qaddch(b'\r');
                        ed.curses.refresh();

                        let mut text = vec![key as u8];
                        text.extend_from_slice(&buf);
                        let c = ed.vars.cursor;
                        tcurs = f(ed, c, &text)?;
                    } else {
                        if ed.vars.exwrote || ed.vars.mode == MODE_COLON {
                            redraw(ed, MARK_UNSET, false);
                        }
                        ed.vars.mode = MODE_VI;
                    }
                    if ed.vars.mode != MODE_COLON {
                        break;
                    }
                }
                count = 0;
            }

            Func::CursorCntCmd(f) => {
                tcurs = f(ed, cursor, count, key)?;
                count = 0;
            }
        }

        if ed.vars.mode != MODE_VI {
            break;
        }

        let keyptr = VIKEYS[kp];
        let c = ed.vars.cursor;
        if matches!(keyptr.func, Func::CursorMoved(_)) {
            tcurs = adjmove(ed, c, tcurs, 0);
        } else {
            tcurs = adjmove(ed, c, tcurs, keyptr.flags as i32);
        }

        if prevkey != 0 && (prevkey == key || (keyptr.flags & MVMT) != 0) && count == 0 {
            if tcurs == MARK_UNSET {
                prevkey = 0;
                count = 0;
                continue;
            }

            ed.vars.cursor = range;
            if tcurs < range {
                range = tcurs;
                tcurs = ed.vars.cursor;
            }

            let pk = VIKEYS[prevkey as usize];
            let mut fl = keyptr.flags | pk.flags;
            if key == prevkey {
                fl |= INCL | LNMD;
            }
            match fl & (INCL | LNMD) {
                INCL => tcurs += 1,
                x if x == INCL | LNMD => {
                    tcurs += BLKSIZE as Mark;
                    range &= LINEMASK;
                    tcurs &= LINEMASK;
                }
                LNMD => {
                    range &= LINEMASK;
                    tcurs &= LINEMASK;
                }
                _ => {}
            }

            tcurs = match pk.func {
                Func::CursorMoved(f) | Func::CursorEol(f) => f(ed, range, tcurs)?,
                _ => MARK_UNSET,
            };
            let c = ed.vars.cursor;
            let _ = adjmove(ed, c, c, 0);
            let c = ed.vars.cursor;
            ed.vars.cursor = adjmove(ed, c, tcurs, pk.flags as i32);

            prevkey = 0;
        } else if prevkey == 0 {
            ed.vars.cursor = tcurs;
        }
    }
    Ok(())
}

// C: vi.c:adjmove
pub fn adjmove(ed: &mut Editor, old: Mark, new: Mark, flags: i32) -> Mark {
    let mut new = new;
    let flags = flags as u8;

    if new == MARK_UNSET {
        beep(ed);
        return old;
    }

    if flags & NREL != 0 {
        ed.vars.mark[26] = old;
    }

    if markline(new) < 1 {
        new = MARK_FIRST;
    } else if markline(new) > ed.vars.nlines {
        new = mark_last(ed.vars.nlines);
    }

    pfetch(ed, markline(new));

    if flags & FRNT != 0 {
        new = match m_front(ed, new) {
            Ok(m) => m,
            Err(_) => {
                os::rearm_sigint();
                new
            }
        };
    }

    if flags & NCOL == 0 {
        let i = markidx(new);
        let plen = ed.vars.plen as Mark;
        if i == BLKSIZE - 1 {
            new &= LINEMASK;
            if plen > 0 {
                new += plen - 1;
            }
            ed.vi.adjmove_colno = (BLKSIZE * 8) as i32;
        } else if plen > 0 {
            if i as Mark >= plen {
                new = (new & LINEMASK) + plen - 1;
            }
            let ptext = ed.vars.ptext.clone();
            ed.vi.adjmove_colno = idx2col(ed, new, &ptext, false);
        } else {
            new &= LINEMASK;
            ed.vi.adjmove_colno = 0;
        }
    } else {
        let colno = ed.vi.adjmove_colno;
        let tabstop = ed.opts.o_tabstop[0] as i32;
        let list = ed.opts.o_list[0] != 0;
        let ptext = &ed.vars.ptext;
        let mut i: i32 = 0;
        let mut t: usize = 0;
        while i <= colno && t < ptext.len() && ptext[t] != 0 {
            let c = ptext[t];
            if c == b'\t' && !list {
                i += tabstop - (i % tabstop);
            } else if c < b' ' || c == 127 {
                i += 2;
            } else {
                i += 1;
            }
            t += 1;
        }
        t = t.saturating_sub(1);
        new = (new & LINEMASK) + t as Mark;
    }

    new
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vikeys_table_matches_c() {
        assert_eq!(VIKEYS.len(), 128);
        assert!(matches!(VIKEYS[b'0' as usize].func, Func::Zero));
        assert_eq!(VIKEYS[b'0' as usize].flags, MVMT | PTMV);
        for d in b'1'..=b'9' {
            assert!(matches!(VIKEYS[d as usize].func, Func::Digit));
        }
        assert!(matches!(VIKEYS[b'C' as usize].func, Func::CursorEol(_)));
        assert_eq!(VIKEYS[b'd' as usize].flags, SDOT | NCOL);
        assert_eq!(
            VIKEYS[b'G' as usize].flags,
            MVMT | NREL | LNMD | FRNT | INCL
        );
        assert!(matches!(VIKEYS[b':' as usize].func, Func::CursorText(_)));
        assert!(matches!(VIKEYS[b'#' as usize].func, Func::Keyword(_)));
        assert!(matches!(VIKEYS[b'~' as usize].func, Func::CursorCount(_)));
        assert!(matches!(VIKEYS[b'z' as usize].func, Func::CursorCntKey(_)));
        assert!(matches!(VIKEYS[b'x' as usize].func, Func::CursorCntCmd(_)));
        assert!(matches!(VIKEYS[b'.' as usize].func, Func::None));
        assert!(matches!(VIKEYS[127].func, Func::None));
        let defined = VIKEYS
            .iter()
            .filter(|k| !matches!(k.func, Func::None | Func::Zero | Func::Digit))
            .count();
        assert_eq!(defined, 128 - 29 - 10);
    }

    #[test]
    fn keyword_chars() {
        assert!(kwchar(b'a') && kwchar(b'_') && kwchar(b'9') && kwchar(0xe9));
        assert!(!kwchar(b' ') && !kwchar(0) && !kwchar(b'.'));
    }
}
