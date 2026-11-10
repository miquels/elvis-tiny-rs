//! Constants, types, and helper macros from `vi.h`.

pub const VERSION: &[u8] = b"ELVIS 1.4, by Steve Kirkendall";
pub const COPYING: &[u8] = b"This version of ELVIS is freely redistributable.";

/*------------------------------------------------------------------------*/
/* Miscellaneous constants.                                                */

pub const INFINITY: i64 = 2000000001;
pub const LONGKEY: usize = 10;
pub const MAXRCLEN: usize = 1000;

/*------------------------------------------------------------------------*/
/* These describe how temporary files are divided into blocks              */

pub const BLKSIZE: usize = 1024;
pub const MAXBLKS: usize = BLKSIZE / 2;

/// The C `BLK` union: 1024 bytes viewed either as text (`c`) or as 512
/// native-endian `unsigned short`s (`n`, the header view).
#[derive(Clone, PartialEq, Eq)]
pub struct Blk {
    pub c: [u8; BLKSIZE],
}

impl Default for Blk {
    fn default() -> Blk {
        Blk { c: [0; BLKSIZE] }
    }
}

impl std::fmt::Debug for Blk {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Blk({:?}..)", &self.c[..16])
    }
}

impl Blk {
    #[inline]
    pub fn n(&self, i: usize) -> u16 {
        u16::from_ne_bytes([self.c[2 * i], self.c[2 * i + 1]])
    }

    #[inline]
    pub fn set_n(&mut self, i: usize, v: u16) {
        let b = v.to_ne_bytes();
        self.c[2 * i] = b[0];
        self.c[2 * i + 1] = b[1];
    }
}

/*------------------------------------------------------------------------*/
/* These are used to keep track of various flags                           */

#[derive(Debug, Default, Clone, Copy)]
pub struct Viflags {
    pub file: i16,
}

pub const NEWFILE: i16 = 0x0001;
pub const READONLY: i16 = 0x0002;
pub const HADNUL: i16 = 0x0004;
pub const MODIFIED: i16 = 0x0008;
pub const NOFILE: i16 = 0x0010;
pub const ADDEDNL: i16 = 0x0020;

impl Viflags {
    #[inline]
    pub fn setflag(&mut self, y: i16) {
        self.file |= y;
    }
    #[inline]
    pub fn clrflag(&mut self, y: i16) {
        self.file &= !y;
    }
    #[inline]
    pub fn tstflag(&self, y: i16) -> bool {
        self.file & y != 0
    }
    #[inline]
    pub fn initflags(&mut self) {
        self.file = 0;
    }
}

/*------------------------------------------------------------------------*/
/* These are used to refer to places in the text                           */

pub type Mark = i64;

#[inline]
pub fn markline(x: Mark) -> i64 {
    x / BLKSIZE as i64
}

#[inline]
pub fn markidx(x: Mark) -> usize {
    (x & (BLKSIZE as i64 - 1)) as usize
}

/// Clears the index part of a mark, leaving the start of its line.
pub const LINEMASK: Mark = !(BLKSIZE as Mark - 1);

pub const MARK_UNSET: Mark = 0;
pub const MARK_FIRST: Mark = BLKSIZE as Mark;

#[inline]
pub fn mark_last(nlines: i64) -> Mark {
    nlines * BLKSIZE as i64
}

#[inline]
pub fn mark_at_line(x: i64) -> Mark {
    x * BLKSIZE as i64
}

/// `buildmark(text)`: `idx` is the offset of `text` within `ptext`.
#[inline]
pub fn buildmark(pline: i64, idx: usize) -> Mark {
    BLKSIZE as i64 * pline + idx as i64
}

pub const NMARKS: usize = 29;

/*------------------------------------------------------------------------*/
/* misc housekeeping                                                        */

#[inline]
pub fn botline(topline: i64, lines: i32) -> i64 {
    topline + lines as i64 - 2
}

#[inline]
pub fn rightcol(leftcol: i32, cols: i32) -> i32 {
    leftcol + cols - 1
}

#[inline]
pub const fn ctrl(ch: u8) -> u8 {
    ch & 0o37
}

/// `DEFAULT(val)`: `if (cnt < 1) cnt = (val)`.
#[inline]
pub fn default(cnt: &mut i64, val: i64) {
    if *cnt < 1 {
        *cnt = val;
    }
}

/*------------------------------------------------------------------------*/
/* Signed char and <ctype.h> in the "C" locale                             */

/// The value C sees for a `char` holding `b` (signed because of
/// `-fsigned-char`).
#[inline]
pub fn schar(b: u8) -> i32 {
    b as i8 as i32
}

#[inline]
fn ascii(c: i32) -> Option<u8> {
    if (0..128).contains(&c) {
        Some(c as u8)
    } else {
        None
    }
}

#[inline]
pub fn isascii(c: i32) -> bool {
    c & !0x7f == 0
}
#[inline]
pub fn isalnum(c: i32) -> bool {
    ascii(c).is_some_and(|b| b.is_ascii_alphanumeric())
}
#[inline]
pub fn isalpha(c: i32) -> bool {
    ascii(c).is_some_and(|b| b.is_ascii_alphabetic())
}
#[inline]
pub fn isdigit(c: i32) -> bool {
    ascii(c).is_some_and(|b| b.is_ascii_digit())
}
#[inline]
pub fn isupper(c: i32) -> bool {
    ascii(c).is_some_and(|b| b.is_ascii_uppercase())
}
#[inline]
pub fn islower(c: i32) -> bool {
    ascii(c).is_some_and(|b| b.is_ascii_lowercase())
}
/// C `isspace`: space, `\t`, `\n`, `\v`, `\f`, `\r`.
#[inline]
pub fn isspace(c: i32) -> bool {
    matches!(c, 0x20 | 0x09..=0x0d)
}
#[inline]
pub fn tolower(c: i32) -> i32 {
    if isupper(c) { c + 32 } else { c }
}
#[inline]
pub fn toupper(c: i32) -> i32 {
    if islower(c) { c - 32 } else { c }
}

/// `s[i]` as C sees it when `s` is a NUL-terminated string: 0 at or
/// past the end.
#[inline]
pub fn ch(s: &[u8], i: usize) -> u8 {
    s.get(i).copied().unwrap_or(0)
}

/// The bytes of a NUL-terminated C string stored in a fixed buffer.
pub fn cstr(buf: &[u8]) -> &[u8] {
    match buf.iter().position(|&b| b == 0) {
        Some(n) => &buf[..n],
        None => buf,
    }
}

/*------------------------------------------------------------------------*/
/* Interrupt propagation (replaces setjmp/longjmp, section 3.7)            */

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Interrupted;

pub type R<T> = Result<T, Interrupted>;

/*------------------------------------------------------------------------*/
/* EX command codes                                                         */

pub type Cmd = i32;
pub const CMD_NULL: Cmd = 0;
pub const CMD_ABBR: Cmd = 1;
pub const CMD_ARGS: Cmd = 2;
pub const CMD_APPEND: Cmd = 3;
pub const CMD_AT: Cmd = 4;
pub const CMD_BANG: Cmd = 5;
pub const CMD_CC: Cmd = 6;
pub const CMD_CD: Cmd = 7;
pub const CMD_CHANGE: Cmd = 8;
pub const CMD_COPY: Cmd = 9;
pub const CMD_DELETE: Cmd = 10;
pub const CMD_DIGRAPH: Cmd = 11;
pub const CMD_EDIT: Cmd = 12;
pub const CMD_EQUAL: Cmd = 13;
pub const CMD_ERRLIST: Cmd = 14;
pub const CMD_FILE: Cmd = 15;
pub const CMD_GLOBAL: Cmd = 16;
pub const CMD_INSERT: Cmd = 17;
pub const CMD_JOIN: Cmd = 18;
pub const CMD_LIST: Cmd = 19;
pub const CMD_MAKE: Cmd = 20;
pub const CMD_MAP: Cmd = 21;
pub const CMD_MARK: Cmd = 22;
pub const CMD_MKEXRC: Cmd = 23;
pub const CMD_MOVE: Cmd = 24;
pub const CMD_NEXT: Cmd = 25;
pub const CMD_NUMBER: Cmd = 26;
pub const CMD_PRESERVE: Cmd = 27;
pub const CMD_PREVIOUS: Cmd = 28;
pub const CMD_PRINT: Cmd = 29;
pub const CMD_PUT: Cmd = 30;
pub const CMD_QUIT: Cmd = 31;
pub const CMD_READ: Cmd = 32;
pub const CMD_RECOVER: Cmd = 33;
pub const CMD_REWIND: Cmd = 34;
pub const CMD_SET: Cmd = 35;
pub const CMD_SHELL: Cmd = 36;
pub const CMD_SHIFTL: Cmd = 37;
pub const CMD_SHIFTR: Cmd = 38;
pub const CMD_SOURCE: Cmd = 39;
pub const CMD_STOP: Cmd = 40;
pub const CMD_SUBAGAIN: Cmd = 41;
pub const CMD_SUBSTITUTE: Cmd = 42;
pub const CMD_SUSPEND: Cmd = 43;
pub const CMD_TR: Cmd = 44;
pub const CMD_TAG: Cmd = 45;
pub const CMD_UNABBR: Cmd = 46;
pub const CMD_UNDO: Cmd = 47;
pub const CMD_UNMAP: Cmd = 48;
pub const CMD_VERSION: Cmd = 49;
pub const CMD_VGLOBAL: Cmd = 50;
pub const CMD_VISUAL: Cmd = 51;
pub const CMD_WQUIT: Cmd = 52;
pub const CMD_WRITE: Cmd = 53;
pub const CMD_XIT: Cmd = 54;
pub const CMD_YANK: Cmd = 55;

/*------------------------------------------------------------------------*/
/* These describe what mode we're in                                        */

pub const MODE_EX: i32 = 1;
pub const MODE_VI: i32 = 2;
pub const MODE_COLON: i32 = 3;
pub const MODE_QUIT: i32 = 4;

pub const WHEN_VICMD: i32 = 1;
pub const WHEN_VIINP: i32 = 2;
pub const WHEN_VIREP: i32 = 4;
pub const WHEN_EX: i32 = 8;
pub const WHEN_MSG: i32 = 16;
pub const WHEN_INMV: i32 = 256;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mark_arithmetic() {
        let m = buildmark(7, 5);
        assert_eq!(m, 7 * 1024 + 5);
        assert_eq!(markline(m), 7);
        assert_eq!(markidx(m), 5);
        assert_eq!(markidx(mark_at_line(3) + 1023), 1023);
        assert_eq!(markline(mark_at_line(3) + 1023), 3);
        assert_eq!(MARK_FIRST, mark_at_line(1));
        assert_eq!(mark_last(42), mark_at_line(42));
        assert_eq!(markline(MARK_UNSET), 0);
        // C: (int)((x) & (BLKSIZE - 1)) on a negative long
        assert_eq!(markidx(-1), 1023);
        assert_eq!(markline(-1), 0);
        assert_eq!(markline(-1025), -1);
    }

    #[test]
    fn blk_header_round_trip() {
        let mut b = Blk::default();
        b.set_n(0, 0x3517);
        b.set_n(1, 1);
        b.set_n(MAXBLKS - 1, 0xffff);
        assert_eq!(b.n(0), 0x3517);
        assert_eq!(b.n(1), 1);
        assert_eq!(b.n(MAXBLKS - 1), 0xffff);
        assert_eq!(&b.c[0..2], &0x3517u16.to_ne_bytes());
        assert_eq!(MAXBLKS, 512);
    }

    #[test]
    fn flags() {
        let mut f = Viflags::default();
        f.setflag(MODIFIED);
        f.setflag(READONLY);
        assert!(f.tstflag(MODIFIED) && f.tstflag(READONLY) && !f.tstflag(NOFILE));
        f.clrflag(MODIFIED);
        assert!(!f.tstflag(MODIFIED));
        f.initflags();
        assert_eq!(f.file, 0);
    }

    #[test]
    fn ctype_signed_bytes() {
        for b in 0u8..=255 {
            let c = schar(b);
            assert_eq!(c < 0, b >= 128);
            let a = b < 128;
            assert_eq!(isascii(c), a);
            assert_eq!(isalnum(c), a && b.is_ascii_alphanumeric());
            assert_eq!(isalpha(c), a && b.is_ascii_alphabetic());
            assert_eq!(isdigit(c), a && b.is_ascii_digit());
            assert_eq!(isupper(c), a && b.is_ascii_uppercase());
            assert_eq!(islower(c), a && b.is_ascii_lowercase());
            assert_eq!(
                isspace(c),
                matches!(b, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r')
            );
            assert_eq!(tolower(c), if b.is_ascii_uppercase() { c + 32 } else { c });
            assert_eq!(toupper(c), if b.is_ascii_lowercase() { c - 32 } else { c });
        }
        assert!(!isascii(256));
        assert!(!isspace(-1));
    }

    #[test]
    fn misc_helpers() {
        assert_eq!(ctrl(b'C'), 3);
        assert_eq!(ctrl(b'['), 27);
        let mut cnt = 0;
        default(&mut cnt, 5);
        assert_eq!(cnt, 5);
        default(&mut cnt, 9);
        assert_eq!(cnt, 5);
        assert_eq!(botline(1, 24), 23);
        assert_eq!(rightcol(0, 80), 79);
        assert_eq!(cstr(b"abc\0def"), b"abc");
        assert_eq!(cstr(b"abc"), b"abc");
    }
}
