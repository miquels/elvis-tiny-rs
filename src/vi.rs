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
use crate::vi_h::{
    BLKSIZE, MARK_FIRST, MARK_UNSET, MODE_COLON, MODE_VI, Mark, R, WHEN_VICMD, isalnum, isascii,
    isupper, mark_at_line, mark_last, markidx, markline, schar,
};

#[derive(Default)]
pub struct Vi {
    pub adjmove_colno: i32,
}

/*------------------------------------------------------------------------*/
/* struct keystru and vikeys[]                                             */

const NO_ARGS: u8 = 0;
const CURSOR_COUNT: u8 = 1;
const CURSOR: u8 = 2;
const CURSOR_CNT_KEY: u8 = 3;
const CURSOR_MOVED: u8 = 4;
const CURSOR_EOL: u8 = 5;
const ZERO: u8 = 6;
const DIGIT: u8 = 7;
const CURSOR_TEXT: u8 = 8;
const CURSOR_CNT_CMD: u8 = 9;
const KEYWORD: u8 = 10;

const NO_FLAGS: u8 = 0x00;
const MVMT: u8 = 0x01;
const PTMV: u8 = 0x02;
const FRNT: u8 = 0x04;
const INCL: u8 = 0x08;
const LNMD: u8 = 0x10;
const NCOL: u8 = 0x20;
const NREL: u8 = 0x40;
const SDOT: u8 = 0x80;

/// The C `func` union; `CURSOR_CNT_CMD` shares `CursorCntKey` and
/// `CURSOR_EOL` shares `CursorMoved`, as in C.
#[derive(Clone, Copy)]
enum Func {
    None,
    NoArg(fn(&mut Editor) -> R<Mark>),
    Keyword(fn(&mut Editor, &[u8], Mark, i64) -> R<Mark>),
    Cursor(fn(&mut Editor, Mark) -> R<Mark>),
    CursorCount(fn(&mut Editor, Mark, i64) -> R<Mark>),
    CursorCntKey(fn(&mut Editor, Mark, i64, i32) -> R<Mark>),
    CursorText(fn(&mut Editor, Mark, &[u8]) -> R<Mark>),
    CursorMoved(fn(&mut Editor, Mark, Mark) -> R<Mark>),
}

#[derive(Clone, Copy)]
struct Keystru {
    func: Func,
    args: u8,
    flags: u8,
}

// C: vi.c:m_rear_wrapper
fn m_rear_wrapper(ed: &mut Editor, m: Mark) -> R<Mark> {
    m_rear(ed, m, 0)
}

// C: vi.c:m_match_wrapper
fn m_match_wrapper(ed: &mut Editor, m: Mark) -> R<Mark> {
    m_match(ed, m, 0)
}

// C: vi.c:m_front_wrapper
fn m_front_wrapper(ed: &mut Editor, m: Mark) -> R<Mark> {
    m_front(ed, m, 0)
}

const NO_FUNC: Keystru = Keystru {
    func: Func::None,
    args: NO_ARGS,
    flags: NO_FLAGS,
};

const fn no_args_f(f: fn(&mut Editor) -> R<Mark>, flags: u8) -> Keystru {
    Keystru {
        func: Func::NoArg(f),
        args: NO_ARGS,
        flags,
    }
}
const fn keyword_f(f: fn(&mut Editor, &[u8], Mark, i64) -> R<Mark>, flags: u8) -> Keystru {
    Keystru {
        func: Func::Keyword(f),
        args: KEYWORD,
        flags,
    }
}
const fn cursor_f(f: fn(&mut Editor, Mark) -> R<Mark>, flags: u8) -> Keystru {
    Keystru {
        func: Func::Cursor(f),
        args: CURSOR,
        flags,
    }
}
const fn cursor_count_f(f: fn(&mut Editor, Mark, i64) -> R<Mark>, flags: u8) -> Keystru {
    Keystru {
        func: Func::CursorCount(f),
        args: CURSOR_COUNT,
        flags,
    }
}
const fn cursor_cnt_key_f(f: fn(&mut Editor, Mark, i64, i32) -> R<Mark>, flags: u8) -> Keystru {
    Keystru {
        func: Func::CursorCntKey(f),
        args: CURSOR_CNT_KEY,
        flags,
    }
}
const fn cursor_text_f(f: fn(&mut Editor, Mark, &[u8]) -> R<Mark>, flags: u8) -> Keystru {
    Keystru {
        func: Func::CursorText(f),
        args: CURSOR_TEXT,
        flags,
    }
}
const fn cursor_cnt_cmd_f(f: fn(&mut Editor, Mark, i64, i32) -> R<Mark>, flags: u8) -> Keystru {
    Keystru {
        func: Func::CursorCntKey(f),
        args: CURSOR_CNT_CMD,
        flags,
    }
}
const fn cursor_moved_f(f: fn(&mut Editor, Mark, Mark) -> R<Mark>, flags: u8) -> Keystru {
    Keystru {
        func: Func::CursorMoved(f),
        args: CURSOR_MOVED,
        flags,
    }
}
const fn cursor_eol_f(f: fn(&mut Editor, Mark, Mark) -> R<Mark>, flags: u8) -> Keystru {
    Keystru {
        func: Func::CursorMoved(f),
        args: CURSOR_EOL,
        flags,
    }
}
const fn no_func(args: u8, flags: u8) -> Keystru {
    Keystru {
        func: Func::None,
        args,
        flags,
    }
}

#[rustfmt::skip]
static VIKEYS: [Keystru; 128] = [
/* NUL not defined      */ NO_FUNC,
/* ^A  not defined      */ NO_FUNC,
/* ^B  page backward    */ cursor_cnt_cmd_f(m_scroll, FRNT),
/* ^C  not defined      */ NO_FUNC,
/* ^D  scroll dn 1/2page*/ cursor_cnt_cmd_f(m_scroll, NCOL),
/* ^E  scroll up        */ cursor_cnt_cmd_f(m_scroll, NCOL),
/* ^F  page forward     */ cursor_cnt_cmd_f(m_scroll, FRNT),
/* ^G  show file status */ no_args_f(v_status, NO_FLAGS),
/* ^H  move left, like h*/ cursor_count_f(m_left, MVMT),
/* ^I  not defined      */ NO_FUNC,
/* ^J  move down        */ cursor_cnt_cmd_f(m_updnto, MVMT|LNMD),
/* ^K  not defined      */ NO_FUNC,
/* ^L  redraw screen    */ no_args_f(v_redraw, NO_FLAGS),
/* ^M  mv front next ln */ cursor_cnt_cmd_f(m_updnto, MVMT|FRNT|LNMD),
/* ^N  move down        */ cursor_cnt_cmd_f(m_updnto, MVMT|LNMD),
/* ^O  not defined      */ NO_FUNC,
/* ^P  move up          */ cursor_cnt_cmd_f(m_updnto, MVMT|LNMD),
/* ^Q  not defined      */ NO_FUNC,
/* ^R  redraw screen    */ no_args_f(v_redraw, NO_FLAGS),
/* ^S  not defined      */ NO_FUNC,
/* ^T  not defined      */ NO_FUNC,
/* ^U  scroll up 1/2page*/ cursor_cnt_cmd_f(m_scroll, NCOL),
/* ^V  not defined      */ NO_FUNC,
/* ^W  not defined      */ NO_FUNC,
/* ^X  not defined      */ NO_FUNC,
/* ^Y  scroll down      */ cursor_cnt_cmd_f(m_scroll, NCOL),
/* ^Z  not defined      */ NO_FUNC,
/* ESC not defined      */ NO_FUNC,
/* ^\  not defined      */ NO_FUNC,
/* ^]  keyword is tag   */ keyword_f(v_tag, NO_FLAGS),
/* ^^  previous file    */ no_args_f(v_switch, NO_FLAGS),
/* ^_  not defined      */ NO_FUNC,
/* SPC move right,like l*/ cursor_count_f(m_right, MVMT),
/*  !  run thru filter  */ cursor_moved_f(v_filter, FRNT|LNMD|INCL),
/*  "  select cut buffer*/ cursor_cnt_key_f(v_selcut, PTMV),
/*  #  increment number */ keyword_f(v_increment, SDOT),
/*  $  move to rear     */ cursor_f(m_rear_wrapper, MVMT|INCL),
/*  %  move to match    */ cursor_f(m_match_wrapper, MVMT|INCL),
/*  &  repeat subst     */ cursor_moved_f(v_again, SDOT|NCOL|LNMD|INCL),
/*  '  move to a mark   */ cursor_cnt_key_f(m_tomark, MVMT|FRNT|NREL|LNMD|INCL),
/*  (  not defined      */ NO_FUNC,
/*  )  not defined      */ NO_FUNC,
/*  *  not defined      */ NO_FUNC,
/*  +  mv front next ln */ cursor_cnt_cmd_f(m_updnto, MVMT|FRNT|LNMD),
/*  ,  reverse [fFtT] cmd*/ cursor_cnt_cmd_f(m__ch, MVMT|INCL),
/*  -  mv front prev ln */ cursor_cnt_cmd_f(m_updnto, MVMT|FRNT|LNMD),
/*  .  special...       */ NO_FUNC,
/*  /  forward search   */ cursor_text_f(m_fsrch, MVMT|NREL),
/*  0  part of count?   */ no_func(ZERO, MVMT|PTMV),
/*  1  part of count    */ no_func(DIGIT, PTMV),
/*  2  part of count    */ no_func(DIGIT, PTMV),
/*  3  part of count    */ no_func(DIGIT, PTMV),
/*  4  part of count    */ no_func(DIGIT, PTMV),
/*  5  part of count    */ no_func(DIGIT, PTMV),
/*  6  part of count    */ no_func(DIGIT, PTMV),
/*  7  part of count    */ no_func(DIGIT, PTMV),
/*  8  part of count    */ no_func(DIGIT, PTMV),
/*  9  part of count    */ no_func(DIGIT, PTMV),
/*  :  run single EX cmd*/ cursor_text_f(v_1ex, NO_FLAGS),
/*  ;  repeat [fFtT] cmd*/ cursor_cnt_cmd_f(m__ch, MVMT|INCL),
/*  <  shift text left  */ cursor_moved_f(v_lshift, SDOT|FRNT|LNMD|INCL),
/*  =  not defined      */ NO_FUNC,
/*  >  shift text right */ cursor_moved_f(v_rshift, SDOT|FRNT|LNMD|INCL),
/*  ?  backward search  */ cursor_text_f(m_bsrch, MVMT|NREL),
/*  @  undefined        */ NO_FUNC,
/*  A  append at EOL    */ cursor_cnt_cmd_f(v_insert, SDOT),
/*  B  move back Word   */ cursor_cnt_cmd_f(m_bword, MVMT),
/*  C  change to EOL    */ cursor_eol_f(v_change, SDOT),
/*  D  delete to EOL    */ cursor_eol_f(v_delete, SDOT),
/*  E  move end of Word */ cursor_cnt_cmd_f(m_eword, MVMT|INCL),
/*  F  move bk to char  */ cursor_cnt_key_f(m_Fch, MVMT|INCL),
/*  G  move to line #   */ cursor_cnt_cmd_f(m_updnto, MVMT|NREL|LNMD|FRNT|INCL),
/*  H  move to row      */ cursor_cnt_cmd_f(m_row, MVMT|FRNT),
/*  I  insert at front  */ cursor_cnt_cmd_f(v_insert, SDOT),
/*  J  join lines       */ cursor_count_f(v_join, SDOT),
/*  K  look up keyword  */ keyword_f(v_keyword, NO_FLAGS),
/*  L  move to last row */ cursor_cnt_cmd_f(m_row, MVMT|FRNT),
/*  M  move to mid row  */ cursor_cnt_cmd_f(m_row, MVMT|FRNT),
/*  N  reverse prev srch*/ cursor_f(m_Nsrch, MVMT),
/*  O  insert above line*/ cursor_cnt_cmd_f(v_insert, SDOT),
/*  P  paste before     */ cursor_cnt_cmd_f(v_paste, NO_FLAGS),
/*  Q  quit to EX mode  */ no_args_f(v_quit, NO_FLAGS),
/*  R  overtype         */ cursor_f(v_overtype, SDOT),
/*  S  change line      */ cursor_moved_f(v_change, SDOT),
/*  T  move bk to char  */ cursor_cnt_key_f(m_Tch, MVMT|INCL),
/*  U  undo whole line  */ cursor_f(v_undoline, FRNT),
/*  V  not defined      */ NO_FUNC,
/*  W  move forward Word*/ cursor_cnt_cmd_f(m_fword, MVMT),
/*  X  delete to left   */ cursor_cnt_cmd_f(v_xchar, SDOT),
/*  Y  yank text        */ cursor_moved_f(v_yank, NCOL),
/*  Z  save file & exit */ cursor_cnt_key_f(v_xit, NO_FLAGS),
/*  [  move back section*/ cursor_cnt_key_f(m_bsection, MVMT|LNMD|NREL),
/*  \  not defined      */ NO_FUNC,
/*  ]  move fwd section */ cursor_cnt_key_f(m_fsection, MVMT|LNMD|NREL),
/*  ^  move to front    */ cursor_f(m_front_wrapper, MVMT),
/*  _  not defined      */ NO_FUNC,
/*  `  move to mark     */ cursor_cnt_key_f(m_tomark, MVMT|NREL),
/*  a  append at cursor */ cursor_cnt_cmd_f(v_insert, SDOT),
/*  b  move back word   */ cursor_cnt_cmd_f(m_bword, MVMT),
/*  c  change text      */ cursor_moved_f(v_change, SDOT),
/*  d  delete op        */ cursor_moved_f(v_delete, SDOT|NCOL),
/*  e  move end word    */ cursor_cnt_cmd_f(m_eword, MVMT|INCL),
/*  f  move fwd for char*/ cursor_cnt_key_f(m_fch, MVMT|INCL),
/*  g  not defined      */ NO_FUNC,
/*  h  move left        */ cursor_count_f(m_left, MVMT),
/*  i  insert at cursor */ cursor_cnt_cmd_f(v_insert, SDOT),
/*  j  move down        */ cursor_cnt_cmd_f(m_updnto, MVMT|NCOL|LNMD),
/*  k  move up          */ cursor_cnt_cmd_f(m_updnto, MVMT|NCOL|LNMD),
/*  l  move right       */ cursor_count_f(m_right, MVMT),
/*  m  define a mark    */ cursor_cnt_key_f(v_mark, NO_FLAGS),
/*  n  repeat prev srch */ cursor_f(m_nsrch, MVMT),
/*  o  insert below line*/ cursor_cnt_cmd_f(v_insert, SDOT),
/*  p  paste after      */ cursor_cnt_cmd_f(v_paste, NO_FLAGS),
/*  q  not defined      */ NO_FUNC,
/*  r  replace chars    */ cursor_cnt_key_f(v_replace, SDOT),
/*  s  subst N chars    */ cursor_count_f(v_subst, SDOT),
/*  t  move fwd to char */ cursor_cnt_key_f(m_tch, MVMT|INCL),
/*  u  undo             */ cursor_f(v_undo, NO_FLAGS),
/*  v  not defined      */ NO_FUNC,
/*  w  move fwd word    */ cursor_cnt_cmd_f(m_fword, MVMT),
/*  x  delete character */ cursor_cnt_cmd_f(v_xchar, SDOT),
/*  y  yank text        */ cursor_moved_f(v_yank, NCOL),
/*  z  adjust scrn row  */ cursor_cnt_key_f(m_z, NCOL),
/*  {  back paragraph   */ cursor_count_f(m_bparagraph, MVMT|LNMD),
/*  |  move to column   */ cursor_count_f(m_tocol, NREL),
/*  }  fwd paragraph    */ cursor_count_f(m_fparagraph, MVMT|LNMD),
/*  ~  upper/lowercase  */ cursor_count_f(v_ulcase, SDOT),
/* DEL not defined      */ NO_FUNC,
];

const LINEMASK: Mark = !(BLKSIZE as Mark - 1);

/// `isascii(c) && !isalnum(c) && c != '_'` negated: part of a keyword.
fn kwchar(b: u8) -> bool {
    let c = schar(b);
    !isascii(c) || isalnum(c) || b == b'_'
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
        match keyptr.args {
            ZERO | DIGIT => {
                if keyptr.args == ZERO && count == 0 {
                    tcurs = cursor & LINEMASK;
                } else {
                    count = count
                        .wrapping_mul(10)
                        .wrapping_add((key - b'0' as i32) as i64);
                }
            }

            KEYWORD => 'kw: {
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

                if let Func::Keyword(f) = keyptr.func {
                    tcurs = f(ed, &text, tcurs, count)?;
                }
                count = 0;
            }

            NO_ARGS => {
                if let Func::NoArg(f) = keyptr.func {
                    f(ed)?;
                } else {
                    beep(ed);
                }
                count = 0;
            }

            CURSOR_COUNT => {
                if let Func::CursorCount(f) = keyptr.func {
                    tcurs = f(ed, cursor, count)?;
                }
                count = 0;
            }

            CURSOR => {
                if let Func::Cursor(f) = keyptr.func {
                    tcurs = f(ed, cursor)?;
                }
                count = 0;
            }

            CURSOR_CNT_KEY => 'ck: {
                let Func::CursorCntKey(f) = keyptr.func else {
                    break 'ck;
                };
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

            CURSOR_MOVED => {
                if key == b'&' as i32 || (isascii(key) && isupper(key)) {
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

            CURSOR_EOL => {
                prevkey = key;
                pfetch(ed, markline(cursor));
                if ed.vars.plen == 0 {
                    range = cursor;
                    tcurs = cursor;
                    key = b' ' as i32;
                } else {
                    range = cursor;
                    tcurs = m_rear(ed, cursor, 1)?;
                    key = b'$' as i32;
                }
                count = 0;
                kp = key as usize;
            }

            CURSOR_TEXT => {
                loop {
                    let mut buf = Vec::new();
                    if vgets(ed, key as u8, &mut buf, 100 - 1)? >= 0 {
                        ed.curses.qaddch(b'\r');
                        ed.curses.refresh();

                        let mut text = vec![key as u8];
                        text.extend_from_slice(&buf);
                        if let Func::CursorText(f) = keyptr.func {
                            let c = ed.vars.cursor;
                            tcurs = f(ed, c, &text)?;
                        }
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

            CURSOR_CNT_CMD => {
                if let Func::CursorCntKey(f) = keyptr.func {
                    tcurs = f(ed, cursor, count, key)?;
                }
                count = 0;
            }

            _ => {}
        }

        if ed.vars.mode != MODE_VI {
            break;
        }

        let keyptr = VIKEYS[kp];
        let c = ed.vars.cursor;
        if keyptr.args == CURSOR_MOVED {
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
                Func::CursorMoved(f) => f(ed, range, tcurs)?,
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
        new = match m_front(ed, new, 1) {
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
        assert_eq!(VIKEYS[b'0' as usize].args, ZERO);
        assert_eq!(VIKEYS[b'0' as usize].flags, MVMT | PTMV);
        for d in b'1'..=b'9' {
            assert_eq!(VIKEYS[d as usize].args, DIGIT);
        }
        assert_eq!(VIKEYS[b'C' as usize].args, CURSOR_EOL);
        assert_eq!(VIKEYS[b'd' as usize].flags, SDOT | NCOL);
        assert_eq!(
            VIKEYS[b'G' as usize].flags,
            MVMT | NREL | LNMD | FRNT | INCL
        );
        assert_eq!(VIKEYS[b':' as usize].args, CURSOR_TEXT);
        assert_eq!(VIKEYS[b'#' as usize].args, KEYWORD);
        assert_eq!(VIKEYS[b'~' as usize].args, CURSOR_COUNT);
        assert_eq!(VIKEYS[b'z' as usize].args, CURSOR_CNT_KEY);
        assert_eq!(VIKEYS[b'x' as usize].args, CURSOR_CNT_CMD);
        assert!(matches!(VIKEYS[b'.' as usize].func, Func::None));
        assert!(matches!(VIKEYS[127].func, Func::None));
        let defined = VIKEYS
            .iter()
            .filter(|k| !matches!(k.func, Func::None))
            .count();
        assert_eq!(defined, 128 - 29 - 10);
    }

    #[test]
    fn keyword_chars() {
        assert!(kwchar(b'a') && kwchar(b'_') && kwchar(b'9') && kwchar(0xe9));
        assert!(!kwchar(b' ') && !kwchar(0) && !kwchar(b'.'));
    }
}
