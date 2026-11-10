//! `recycle.c`: garbage collection and block allocation.

use std::io::{Seek, SeekFrom};

use crate::config::lseek_read;
use crate::cut::cutneeds;
use crate::tio::msg;
use crate::vars::Editor;
use crate::vi_h::{Blk, MAXBLKS};

/// Bitmap of free blocks in the first `MAXBIT` blocks of the tmp file.
/// Blocks past the end of the map always look free.
pub struct Recycle {
    bitmap: [u8; 512],
}

impl Default for Recycle {
    fn default() -> Recycle {
        Recycle { bitmap: [0; 512] }
    }
}

const MAXBIT: usize = 512 << 3;

impl Recycle {
    #[cfg(test)]
    fn is_free(&self, blk: usize) -> bool {
        blk >= MAXBIT || self.bitmap[blk >> 3] & (1 << (blk & 7)) != 0
    }

    fn mark_used(&mut self, blk: usize) {
        if blk < MAXBIT {
            self.bitmap[blk >> 3] &= !(1 << (blk & 7));
        }
    }

    /// The lowest free block, now marked used.
    fn take_free(&mut self) -> Option<usize> {
        let byte = self.bitmap.iter().position(|&b| b != 0)?;
        let blk = (byte << 3) + self.bitmap[byte].trailing_zeros() as usize;
        self.mark_used(blk);
        Some(blk)
    }
}

// C: recycle.c:garbage
pub fn garbage(ed: &mut Editor) {
    let mut oldhdr = Blk::default();
    let rc = &mut ed.recycle;

    rc.bitmap.fill(0xff);
    rc.mark_used(0);
    for i in 1..MAXBLKS {
        rc.mark_used(ed.blk.hdr.n(i) as usize);
    }

    if !lseek_read(ed.vars.tmpfd.as_ref(), 0, &mut oldhdr.c) {
        msg(ed, b"garbage() failed to read oldhdr??", &[]);
        ed.recycle.bitmap.fill(0);
        return;
    }
    for i in 1..MAXBLKS {
        ed.recycle.mark_used(oldhdr.n(i) as usize);
    }

    for i in 0..cutneeds(ed, &mut oldhdr) {
        ed.recycle.mark_used(oldhdr.n(i) as usize);
    }
}

// C: recycle.c:allocate
/// The tmp file offset for a new block, or -1 if the file can't be
/// extended.  (An `Option<u64>` costs more code here.)
pub fn allocate(ed: &mut Editor) -> i64 {
    match ed.recycle.take_free() {
        Some(blk) => blk as i64 * crate::vi_h::BLKSIZE as i64,
        None => match ed.vars.tmpfd.as_ref() {
            Some(mut f) => match f.seek(SeekFrom::End(0)) {
                Ok(p) => p as i64,
                Err(_) => -1,
            },
            None => -1,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bitmap_ops() {
        let mut rc = Recycle { bitmap: [255; 512] };
        rc.mark_used(0);
        rc.mark_used(9);
        assert!(!rc.is_free(0) && rc.is_free(1) && !rc.is_free(9));
        // out-of-range blocks always look free and are never marked
        rc.mark_used(MAXBIT + 3);
        assert!(rc.is_free(MAXBIT + 3));
    }

    #[test]
    fn allocate_takes_first_free_block() {
        let mut ed = Editor::new();
        ed.recycle.bitmap = [0; 512];
        ed.recycle.bitmap[2] = 0b0010_0000;
        assert_eq!(allocate(&mut ed), 21 * crate::vi_h::BLKSIZE as i64);
        assert_eq!(ed.recycle.bitmap[2], 0);
        // bitmap exhausted and no tmp file: lseek fails
        assert_eq!(allocate(&mut ed), -1);
    }
}
