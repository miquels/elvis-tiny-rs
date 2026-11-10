//! `cut.c`: cut buffers.

use std::fs::File;

pub const NANNONS: usize = 9;

/// `struct cutbuf`
#[derive(Default)]
pub struct CutBuf {
    /// Block numbers holding the text; `None` is C's null pointer.
    pub phys: Option<Vec<i16>>,
    pub nblks: i32,
    pub start: i32,
    pub end: i32,
    /// The cut buffer's own file; `None` means "use tmpfd" (C's -1).
    pub fd: Option<File>,
    pub lnmode: bool,
}

#[derive(Default)]
pub struct Cut {
    pub named: [CutBuf; 27],
    pub annon: [CutBuf; NANNONS],
    pub cbname: u8,
}
