//! `tmp.c`: the temporary file that holds the text being edited.

/// Capacity of `tmpname[80]`.
pub const TMPNAME_SIZE: usize = 80;

#[derive(Default)]
pub struct Tmp {
    pub tmpname: Vec<u8>,
}
