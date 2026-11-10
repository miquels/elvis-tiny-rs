//! elvis-tiny, ported from C.  See IMPLEMENTATION.md.

#![deny(unsafe_code)]
// Items are ported milestone by milestone and many are not referenced until
// later milestones.  Remove this allowance in M9.
#![allow(dead_code)]

mod blk;
mod cmd1;
mod cmd2;
mod config;
mod curses;
mod cut;
mod ex;
mod input;
mod misc;
mod modify;
mod move1;
mod move2;
mod move3;
mod move4;
mod move5;
mod opts;
#[allow(unsafe_code)]
mod os;
mod recycle;
mod redraw;
mod regexp;
mod regsub;
mod system;
mod tio;
mod tmp;
mod vars;
mod vcmd;
mod vi;
mod vi_h;

// C: main.c:main
fn main() {
    os::init();
    let _ed = vars::Editor::new();
    todo!("ledger: main.c:main")
}
