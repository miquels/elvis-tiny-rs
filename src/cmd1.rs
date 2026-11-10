//! `cmd1.c`: ex commands (part 1).

#[derive(Default)]
pub struct Cmd1 {
    /// `static char prevextra[80]` in `cmd_shell`.
    pub cmd_shell_prevextra: Vec<u8>,
    pub cmd_xit_whenwarned: i64,
    /// `static char prevtag[30]` in `cmd_tag`.
    pub cmd_tag_prevtag: Vec<u8>,
}
