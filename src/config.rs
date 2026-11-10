//! `config.h`, resolved for the shared Unix settings (UNIXV, ANY_UNIX).

use crate::os;

pub const TMPDIR: &[u8] = b"/tmp";
/// `TMPNAME` is `"%s/elv%x%04llx%03llx"`; see `tmpname_fmt`.
pub const CUTNAME_PREFIX: &[u8] = b"/elv_";
pub const EXRC: &[u8] = b".exrc";
pub const HMEXRC: &[u8] = EXRC;
pub const EXINIT: &str = "EXINIT";
pub const SHELL: &[u8] = b"/bin/sh";
pub const KEYWORDPRG: &[u8] = b"ref";
pub const TAGS: &[u8] = b"tags";
pub const REDIRECT: &[u8] = b"2>";
pub const SLASH: u8 = b'/';
pub const NEEDSYNC: bool = false;
pub const FILEPERMS: u32 = 0o666;
pub const MAXMAPS: usize = 20;

/// `TMPNAME`: `sprintf("%s/elv%x%04llx%03llx", dir, sum, ino, dev)`.
pub fn tmpname_fmt(dir: &[u8], sum: i32, ino: u64, dev: u64) -> Vec<u8> {
    let mut v = dir.to_vec();
    v.extend_from_slice(format!("/elv{:x}{:04x}{:03x}", sum as u32, ino, dev).as_bytes());
    v
}

/// `CUTNAME`: `sprintf("%s/elv_%04x%03x", dir, pid, fd)`.
pub fn cutname_fmt(dir: &[u8], pid: u32, fd: i32) -> Vec<u8> {
    let mut v = dir.to_vec();
    v.extend_from_slice(CUTNAME_PREFIX);
    v.extend_from_slice(format!("{:04x}{:03x}", pid, fd as u32).as_bytes());
    v
}

/// `SCRATCHOUT`: `sprintf("%s/soXXXXXX", dir)`.
pub fn scratchout_fmt(dir: &[u8]) -> Vec<u8> {
    let mut v = dir.to_vec();
    v.extend_from_slice(b"/soXXXXXX");
    v
}

/// `ttyread(buf, len)`: `read(0, buf, len)`.
#[inline]
pub fn ttyread(buf: &mut [u8]) -> os::ReadOutcome {
    os::tty_read(buf)
}

/// `ttywrite(buf, len)`: `write(1, buf, len)`.
#[inline]
pub fn ttywrite(buf: &[u8]) -> std::io::Result<usize> {
    os::tty_write(buf)
}

/// `tread(fd, buf, n)`: one `read` call; -1 on error, like C.
pub fn tread(fd: &mut impl std::io::Read, buf: &mut [u8]) -> isize {
    match fd.read(buf) {
        Ok(n) => n as isize,
        Err(_) => -1,
    }
}

/// `twrite(fd, buf, n)`: one `write` call; -1 on error, like C.
pub fn twrite(fd: &mut impl std::io::Write, buf: &[u8]) -> isize {
    match fd.write(buf) {
        Ok(n) => n as isize,
        Err(_) => -1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_formats_match_c() {
        // Values from the macOS reference run (c-behavior.md).
        assert_eq!(
            tmpname_fmt(b"/w", 4, 0x26c13517, 0x100000e),
            b"/w/elv426c13517100000e".to_vec()
        );
        assert_eq!(
            tmpname_fmt(b"/w", 0xf, 0x12, 0x3),
            b"/w/elvf0012003".to_vec()
        );
        assert_eq!(cutname_fmt(b"/w", 0x1a, 5), b"/w/elv_001a005".to_vec());
        assert_eq!(scratchout_fmt(b"/w"), b"/w/soXXXXXX".to_vec());
    }

    #[test]
    fn tread_twrite_return_counts() {
        let mut out = Vec::new();
        assert_eq!(twrite(&mut out, b"abc"), 3);
        let mut inp: &[u8] = b"xy";
        let mut buf = [0u8; 8];
        assert_eq!(tread(&mut inp, &mut buf), 2);
        assert_eq!(tread(&mut inp, &mut buf), 0);
    }
}
