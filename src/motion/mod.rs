//! Cursor motions: `move1.c` (basic), `move2.c` (searching), `move3.c`
//! (character searches), `move4.c` (screen-relative) and `move5.c`
//! (words).

mod basic;
mod charsearch;
mod screen;
mod search;
mod word;

pub use basic::*;
pub use charsearch::*;
pub use screen::*;
pub use search::*;
pub use word::*;

#[derive(Default)]
pub struct Motion {
    pub search: search::Search,
    pub charsearch: charsearch::LastCharSearch,
}
