//! `regexp.c` and `regexp.h`: regular expression compiler and matcher.

pub const NSUBEXP: usize = 10;

/// `struct regexp`; capture positions are offsets into the subject.
#[derive(Debug, Clone, Default)]
pub struct Regexp {
    pub startp: [Option<usize>; NSUBEXP],
    pub endp: [Option<usize>; NSUBEXP],
    pub minlen: i32,
    pub first: u8,
    pub bol: bool,
    pub program: Vec<u8>,
}

#[derive(Default)]
pub struct RegexpState {
    pub previous: Option<Vec<u8>>,
    pub class_cnt: i32,
    pub start_cnt: i32,
    pub end_stk: [i32; NSUBEXP],
    pub end_sp: i32,
    /// Offset into the pattern being compiled.
    pub retext: usize,
}
