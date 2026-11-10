//! `regsub.c`: substitution of matched text.

#[derive(Default)]
pub struct Regsub {
    pub previous: Option<Vec<u8>>,
}
