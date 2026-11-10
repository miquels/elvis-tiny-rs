//! `move2.c`: searching (`/`, `?`, `n`, `N`).

use crate::regexp::Regexp;

#[derive(Default)]
pub struct Move2 {
    pub re: Option<Regexp>,
    pub prevsf: bool,
}
