//! `blk.c`: the block cache for the temporary file, plus do/undo.

use crate::vi_h::{Blk, MAXBLKS, Mark};

pub const NBUFS: usize = 5;

/// `struct _blkbuf`
#[derive(Default, Clone)]
pub struct BlkBuf {
    pub buf: Blk,
    pub logical: u16,
    pub dirty: bool,
}

pub struct BlkState {
    pub hdr: Blk,
    pub b4cnt: i32,
    pub blk: [BlkBuf; NBUFS],
    /// Indexes into `blk` (C used pointers).
    pub toonew: Option<usize>,
    pub newtoo: Option<usize>,
    pub recycle: usize,
    pub undocurs: Mark,
    pub oldnlines: i64,
    pub oldlnum: [i64; MAXBLKS],
}

impl Default for BlkState {
    fn default() -> BlkState {
        BlkState {
            hdr: Blk::default(),
            b4cnt: 0,
            blk: Default::default(),
            toonew: None,
            newtoo: None,
            recycle: 0,
            undocurs: 0,
            oldnlines: 0,
            oldlnum: [0; MAXBLKS],
        }
    }
}
