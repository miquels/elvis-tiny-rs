//! `misc.c`: line fetching and the regexp error hook.

use crate::vi_h::Blk;

/// `fetchline()`'s function-local statics.
#[derive(Default)]
pub struct Misc {
    pub fetchline_buf: Blk,
    pub fetchline_nextline: i64,
    pub fetchline_chglevel: i64,
    /// Offset into the cached block (C kept a pointer).
    pub fetchline_nextscan: usize,
    pub fetchline_nextlnum: i64,
}
