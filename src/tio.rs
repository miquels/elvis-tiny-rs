//! `tio.c`: terminal input, messages, and key mapping.

use crate::config::MAXMAPS;
use crate::vi_h::LONGKEY;

/// Capacity of `pmsg[80]`.
pub const PMSG_SIZE: usize = 80;
/// Capacity of `keybuf[100]`.
pub const KEYBUF_SIZE: usize = 100;
/// Capacity of `struct _keymap.cooked[80]`.
pub const COOKED_SIZE: usize = 80;

/// `struct _keymap`
#[derive(Default, Clone)]
pub struct KeyMap {
    pub name: Option<&'static [u8]>,
    pub rawin: [u8; LONGKEY],
    pub cooked: Vec<u8>,
    pub len: i32,
    pub when: i32,
}

/// Where `getkey()`'s static `cooked` pointer points: an offset into
/// `mapped[map].cooked`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cooked {
    pub map: usize,
    pub off: usize,
}

pub struct Tio {
    pub manymsgs: bool,
    pub pmsg: Vec<u8>,
    pub keybuf: [u8; KEYBUF_SIZE],
    pub nkeys: i32,
    pub next: i32,
    pub mapped: [KeyMap; MAXMAPS],
    pub getkey_cooked: Option<Cooked>,
    pub getkey_oldwhen: i32,
    pub getkey_oldleft: i32,
    pub getkey_oldtop: i64,
    pub getkey_oldnlines: i64,
}

impl Default for Tio {
    fn default() -> Tio {
        Tio {
            manymsgs: false,
            pmsg: Vec::new(),
            keybuf: [0; KEYBUF_SIZE],
            nkeys: 0,
            next: 0,
            mapped: Default::default(),
            getkey_cooked: None,
            getkey_oldwhen: 0,
            getkey_oldleft: 0,
            getkey_oldtop: 0,
            getkey_oldnlines: 0,
        }
    }
}
