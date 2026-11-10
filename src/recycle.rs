//! `recycle.c`: garbage collection and block allocation.

use std::io::{Seek, SeekFrom};

use crate::blk::lseek_read;
use crate::cut::cutneeds;
use crate::tio::msg;
use crate::vars::Editor;
use crate::vi_h::{Blk, MAXBLKS};

pub struct Recycle {
    /// Bitmap of free blocks in the first 4096 blocks of the tmp file.
    pub bitmap: [u8; 512],
}

impl Default for Recycle {
    fn default() -> Recycle {
        Recycle { bitmap: [0; 512] }
    }
}

const MAXBIT: usize = 512 << 3;

#[inline]
fn btst(bitno: usize, byte: u8) -> bool {
    byte & (1 << bitno) != 0
}

#[inline]
fn tst(bitmap: &[u8; 512], blkno: usize) -> bool {
    if blkno < MAXBIT {
        btst(blkno & 7, bitmap[blkno >> 3])
    } else {
        true
    }
}

#[inline]
fn clr(bitmap: &mut [u8; 512], blkno: usize) {
    if blkno < MAXBIT {
        bitmap[blkno >> 3] &= !(1 << (blkno & 7));
    }
}

// C: recycle.c:garbage
pub fn garbage(ed: &mut Editor) {
    let mut oldhdr = Blk::default();

    ed.recycle.bitmap = [255; 512];

    clr(&mut ed.recycle.bitmap, 0);

    for i in 1..MAXBLKS {
        let n = ed.blk.hdr.n(i) as usize;
        clr(&mut ed.recycle.bitmap, n);
    }

    if lseek_read(ed.vars.tmpfd.as_ref(), 0, &mut oldhdr.c) != oldhdr.c.len() as isize {
        msg(ed, b"garbage() failed to read oldhdr??", &[]);
        ed.recycle.bitmap = [0; 512];
        return;
    }
    for i in 1..MAXBLKS {
        clr(&mut ed.recycle.bitmap, oldhdr.n(i) as usize);
    }

    let mut i = cutneeds(ed, &mut oldhdr) - 1;
    while i >= 0 {
        clr(&mut ed.recycle.bitmap, oldhdr.n(i as usize) as usize);
        i -= 1;
    }
}

// C: recycle.c:allocate
pub fn allocate(ed: &mut Editor) -> i64 {
    let bitmap = &mut ed.recycle.bitmap;
    let mut i = 0;
    while i < bitmap.len() && bitmap[i] == 0 {
        i += 1;
    }

    if i == bitmap.len() {
        match ed.vars.tmpfd.as_ref() {
            Some(mut f) => match f.seek(SeekFrom::End(0)) {
                Ok(p) => p as i64,
                Err(_) => -1,
            },
            None => -1,
        }
    } else {
        i <<= 3;
        while !tst(bitmap, i) {
            i += 1;
        }
        let offset = i as i64 * crate::vi_h::BLKSIZE as i64;
        clr(bitmap, i);
        offset
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bitmap_ops() {
        let mut bm = [255u8; 512];
        clr(&mut bm, 0);
        clr(&mut bm, 9);
        assert!(!tst(&bm, 0) && tst(&bm, 1) && !tst(&bm, 9));
        // out-of-range blocks always look free and are never cleared
        clr(&mut bm, MAXBIT + 3);
        assert!(tst(&bm, MAXBIT + 3));
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
