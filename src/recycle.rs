//! `recycle.c`: garbage collection and block allocation.

pub struct Recycle {
    /// Bitmap of free blocks in the first 4096 blocks of the tmp file.
    pub bitmap: [u8; 512],
}

impl Default for Recycle {
    fn default() -> Recycle {
        Recycle { bitmap: [0; 512] }
    }
}
