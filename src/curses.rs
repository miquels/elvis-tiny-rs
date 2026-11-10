//! `curses.h` and `curses.c`: a tiny subset of curses.

use crate::os::Termios;

pub const KBSIZ: usize = 6 * 1024;

pub const A_NORMAL: i32 = 0;
pub const A_STANDOUT: i32 = 1;
pub const A_BOLD: i32 = 2;
pub const A_UNDERLINE: i32 = 3;
pub const A_ALTCHARSET: i32 = 4;

/// Capability strings: `None` is a C null pointer, `Some(vec![])` is "".
pub type Cap = Option<Vec<u8>>;

pub struct Curses {
    pub ospeed: i16,
    pub pc: u8,
    /// `kbuf`; `stdscr` is `kbuf.len()`.
    pub kbuf: Vec<u8>,
    pub lines: i32,
    pub cols: i32,
    pub am: bool,
    pub pt: bool,
    pub vb: Cap,
    pub up: Cap,
    pub so: Cap,
    pub se: Cap,
    pub us: Cap,
    pub ue: Cap,
    pub md: Cap,
    pub me: Cap,
    pub as_: Cap,
    pub ae: Cap,
    pub cm: Cap,
    pub ce: Cap,
    pub cd: Cap,
    pub al: Cap,
    pub dl: Cap,
    pub sr: Cap,
    pub ks: Cap,
    pub ke: Cap,
    pub ku: Cap,
    pub kd: Cap,
    pub kl: Cap,
    pub kr: Cap,
    pub hm: Cap,
    pub en: Cap,
    pub pu: Cap,
    pub pd: Cap,
    pub im: Cap,
    pub ic: Cap,
    pub ei: Cap,
    pub dc: Cap,
    pub ti: Cap,
    pub te: Cap,
    pub aend: Cap,
    pub erasekey: u8,
    pub oldtermio: Termios,
    pub newtermio: Termios,
    pub curses_active: bool,
}

impl Default for Curses {
    fn default() -> Curses {
        let empty = || Some(Vec::new());
        Curses {
            ospeed: 0,
            pc: 0,
            kbuf: Vec::with_capacity(KBSIZ),
            lines: 0,
            cols: 0,
            am: false,
            pt: false,
            vb: None,
            up: None,
            so: None,
            se: None,
            us: empty(),
            ue: empty(),
            md: empty(),
            me: empty(),
            as_: None,
            ae: None,
            cm: None,
            ce: None,
            cd: None,
            al: None,
            dl: None,
            sr: None,
            ks: None,
            ke: None,
            ku: None,
            kd: None,
            kl: None,
            kr: None,
            hm: None,
            en: None,
            pu: None,
            pd: None,
            im: None,
            ic: empty(),
            ei: None,
            dc: None,
            ti: None,
            te: None,
            aend: empty(),
            erasekey: 0,
            oldtermio: Termios::zeroed(),
            newtermio: Termios::zeroed(),
            curses_active: false,
        }
    }
}
