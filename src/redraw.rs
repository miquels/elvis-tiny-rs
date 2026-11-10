//! `redraw.c`: screen updates.

use crate::vi_h::Mark;

#[derive(Default)]
pub struct Redraw {
    pub smartlno: i64,
    pub redrawafter: i64,
    pub preredraw: i64,
    pub postredraw: i64,
    pub idx2col_pcursor: Mark,
    pub idx2col_pcol: Mark,
    pub idx2col_chgs: i64,
    pub redraw_chgs: i64,
}
