//! The single boundary to the operating system.
//!
//! Every `unsafe` block and every direct `libc` call in the crate lives here,
//! and everything exported is safe to call.  The only process-wide mutable
//! state is the signal flags and the terminal snapshot used by the panic hook.

use std::ffi::CString;
use std::fs::File;
use std::io;
#[cfg(test)]
use std::os::fd::OwnedFd;
use std::os::fd::{AsFd, AsRawFd, FromRawFd};
use std::os::unix::fs::MetadataExt;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

pub const O_RDONLY: i32 = libc::O_RDONLY;
pub const O_WRONLY: i32 = libc::O_WRONLY;
pub const O_RDWR: i32 = libc::O_RDWR;
pub const O_CREAT: i32 = libc::O_CREAT;
pub const O_TRUNC: i32 = libc::O_TRUNC;
pub const O_APPEND: i32 = libc::O_APPEND;
pub const O_EXCL: i32 = libc::O_EXCL;

pub const S_IFMT: u32 = libc::S_IFMT as u32;
pub const S_IFREG: u32 = libc::S_IFREG as u32;

pub const ENOENT: i32 = libc::ENOENT;
pub const SIGWINCH: i32 = libc::SIGWINCH;

static SIGINT_FLAG: AtomicBool = AtomicBool::new(false);
static SIGWINCH_FLAG: AtomicBool = AtomicBool::new(false);
static RESTORE: Mutex<Option<(Termios, Vec<u8>)>> = Mutex::new(None);

/*------------------------------------------------------------------------*/
/* Startup and exit                                                        */

/// Restores `SIGPIPE` to its default (the Rust runtime ignores it, C does
/// not) and installs a panic hook that puts the terminal back.
pub fn init() {
    set_disposition(libc::SIGPIPE, libc::SIG_DFL);
    // With immediate-abort panics no hook ever runs, and taking the default
    // hook would link in std's backtrace printer.
    #[cfg(not(panic = "immediate-abort"))]
    install_panic_hook();
}

#[cfg(not(panic = "immediate-abort"))]
fn install_panic_hook() {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore_terminal();
        prev(info);
    }));
}

/// Records what the panic hook must do to restore the terminal: write
/// `bytes` (for example `rmcup`/`rmkx`) and reinstate `saved` on fd 2.
pub fn save_restore_state(saved: &Termios, bytes: &[u8]) {
    if let Ok(mut g) = RESTORE.lock() {
        *g = Some((*saved, bytes.to_vec()));
    }
}

pub fn clear_restore_state() {
    if let Ok(mut g) = RESTORE.lock() {
        *g = None;
    }
}

fn restore_terminal() {
    if let Ok(g) = RESTORE.try_lock()
        && let Some((t, bytes)) = g.as_ref()
    {
        let _ = tty_write(bytes);
        let _ = tcsetattr_now(io::stderr(), t);
    }
}

pub fn getenv(name: &str) -> Option<std::ffi::OsString> {
    use std::os::unix::ffi::OsStringExt;
    let c = CString::new(name).ok()?;
    // SAFETY: `c` is NUL-terminated; getenv returns null or a C string.
    unsafe {
        let p = libc::getenv(c.as_ptr());
        if p.is_null() {
            None
        } else {
            Some(std::ffi::OsString::from_vec(
                std::ffi::CStr::from_ptr(p).to_bytes().to_vec(),
            ))
        }
    }
}

pub fn exit(code: i32) -> ! {
    std::process::exit(code)
}

pub fn sleep_secs(n: u64) {
    // SAFETY: sleep has no preconditions.
    unsafe { libc::sleep(n as u32) };
}

/*------------------------------------------------------------------------*/
/* Raw terminal I/O                                                        */

/// Result of one `read(2)` call on the terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadOutcome {
    Data(usize),
    Eof,
    /// The read was interrupted by a signal (`EINTR`).
    Interrupted,
    /// `tty_read_timeout` saw no input in time.
    Timeout,
    Error(i32),
}

/// `strerror(code)`.
pub fn strerror(code: i32) -> Vec<u8> {
    let mut buf = [0u8; 128];
    // SAFETY: `buf` is writable for `buf.len()` bytes and strerror_r
    // NUL-terminates what it writes.
    let r = unsafe { libc::strerror_r(code, buf.as_mut_ptr().cast(), buf.len()) };
    if r != 0 {
        return b"Unknown error".to_vec();
    }
    let n = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    buf[..n].to_vec()
}
fn last_errno() -> i32 {
    io::Error::last_os_error().raw_os_error().unwrap_or(0)
}

fn read_fd(fd: i32, buf: &mut [u8]) -> ReadOutcome {
    // SAFETY: `buf` is a valid writable region of `buf.len()` bytes.
    let n = unsafe { libc::read(fd, buf.as_mut_ptr().cast(), buf.len()) };
    match n {
        0 => ReadOutcome::Eof,
        n if n > 0 => ReadOutcome::Data(n as usize),
        _ => match last_errno() {
            libc::EINTR => ReadOutcome::Interrupted,
            e => ReadOutcome::Error(e),
        },
    }
}

/// One `read(0, buf, len)`, like the C `ttyread` macro.
pub fn tty_read(buf: &mut [u8]) -> ReadOutcome {
    read_fd(0, buf)
}

/// Like `tty_read`, but gives up after `secs` seconds (0 means wait forever,
/// like `alarm(0)`).
pub fn tty_read_timeout(buf: &mut [u8], secs: u32) -> ReadOutcome {
    read_fd_timeout(0, buf, secs)
}

fn read_fd_timeout(fd: i32, buf: &mut [u8], secs: u32) -> ReadOutcome {
    if secs == 0 {
        return read_fd(fd, buf);
    }
    let mut pfd = libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    };
    let ms = (secs as i64 * 1000).min(i32::MAX as i64) as libc::c_int;
    // SAFETY: `pfd` is one valid pollfd and the count is 1.
    let r = unsafe { libc::poll(&mut pfd, 1, ms) };
    match r {
        0 => ReadOutcome::Timeout,
        r if r > 0 => read_fd(fd, buf),
        _ => match last_errno() {
            libc::EINTR => ReadOutcome::Interrupted,
            e => ReadOutcome::Error(e),
        },
    }
}

/// One `write(1, bytes, len)`, like the C `ttywrite` macro.
pub fn tty_write(bytes: &[u8]) -> io::Result<usize> {
    // SAFETY: `bytes` is a valid readable region of `bytes.len()` bytes.
    let n = unsafe { libc::write(1, bytes.as_ptr().cast(), bytes.len()) };
    if n < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(n as usize)
    }
}

/// One `write(2, bytes, len)`, used for fatal startup messages.
pub fn err_write(bytes: &[u8]) -> io::Result<usize> {
    // SAFETY: `bytes` is a valid readable region of `bytes.len()` bytes.
    let n = unsafe { libc::write(2, bytes.as_ptr().cast(), bytes.len()) };
    if n < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(n as usize)
    }
}

/*------------------------------------------------------------------------*/
/* Files                                                                   */

fn cpath(path: &[u8]) -> io::Result<CString> {
    CString::new(path).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))
}

/// `open(2)` without `O_CLOEXEC`, so children inherit descriptors as in C.
pub fn open(path: &[u8], flags: i32, mode: u32) -> io::Result<File> {
    let p = cpath(path)?;
    // SAFETY: `p` is a valid NUL-terminated string.
    let fd = unsafe { libc::open(p.as_ptr(), flags, mode as libc::c_uint) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `fd` was just returned by open() and is owned by nobody else.
    Ok(unsafe { File::from_raw_fd(fd) })
}

/// `creat(2)`: `open(path, O_WRONLY|O_CREAT|O_TRUNC, mode)`.
pub fn creat(path: &[u8], mode: u32) -> io::Result<File> {
    open(path, O_WRONLY | O_CREAT | O_TRUNC, mode)
}

/// The `stat` fields the editor uses, converted the way the C code converts
/// them (`st_dev` goes through `unsigned long long`, sign-extending on macOS).
#[derive(Debug, Clone, Copy)]
pub struct Stat {
    pub ino: u64,
    pub dev: u64,
    pub mode: u32,
    pub uid: u32,
    pub size: i64,
    pub mtime: i64,
}

pub fn stat(path: &[u8]) -> io::Result<Stat> {
    use std::os::unix::ffi::OsStrExt;
    let m = std::fs::metadata(std::ffi::OsStr::from_bytes(path))?;
    Ok(Stat {
        ino: m.ino(),
        dev: m.dev(),
        mode: m.mode(),
        uid: m.uid(),
        size: m.size() as i64,
        mtime: m.mtime(),
    })
}

pub fn geteuid() -> u32 {
    // SAFETY: geteuid() has no preconditions.
    unsafe { libc::geteuid() }
}

/// `access(2)`; returns 0 on success and -1 on failure, like C.
pub fn access(path: &[u8], mode: i32) -> i32 {
    let Ok(p) = cpath(path) else { return -1 };
    // SAFETY: `p` is a valid NUL-terminated string.
    unsafe { libc::access(p.as_ptr(), mode) }
}

pub fn link(from: &[u8], to: &[u8]) -> io::Result<()> {
    use std::os::unix::ffi::OsStrExt;
    std::fs::hard_link(
        std::ffi::OsStr::from_bytes(from),
        std::ffi::OsStr::from_bytes(to),
    )
}

pub fn unlink(path: &[u8]) -> io::Result<()> {
    use std::os::unix::ffi::OsStrExt;
    std::fs::remove_file(std::ffi::OsStr::from_bytes(path))
}

pub fn rename(from: &[u8], to: &[u8]) -> io::Result<()> {
    use std::os::unix::ffi::OsStrExt;
    std::fs::rename(
        std::ffi::OsStr::from_bytes(from),
        std::ffi::OsStr::from_bytes(to),
    )
}

pub fn chdir(path: &[u8]) -> io::Result<()> {
    use std::os::unix::ffi::OsStrExt;
    std::env::set_current_dir(std::ffi::OsStr::from_bytes(path))
}

pub fn getpid() -> u32 {
    std::process::id()
}

/// `mkstemp(3)`: replaces the trailing `XXXXXX` of `template` in place and
/// returns the open file (without `O_CLOEXEC`, as in C).
pub fn mkstemp(template: &mut Vec<u8>) -> io::Result<File> {
    let mut buf = template.clone();
    if buf.contains(&0) {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    buf.push(0);
    // SAFETY: `buf` is NUL-terminated and writable; mkstemp only rewrites
    // the template characters before the NUL.
    let fd = unsafe { libc::mkstemp(buf.as_mut_ptr().cast()) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    buf.pop();
    *template = buf;
    // SAFETY: `fd` was just returned by mkstemp() and is owned by nobody else.
    Ok(unsafe { File::from_raw_fd(fd) })
}

/// `dup(2)`: the lowest free descriptor, without `O_CLOEXEC`.
pub fn dup(f: &File) -> io::Result<File> {
    // SAFETY: dup() only reads the descriptor number; `f` keeps it open.
    let fd = unsafe { libc::dup(f.as_raw_fd()) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `fd` was just returned by dup() and is owned by nobody else.
    Ok(unsafe { File::from_raw_fd(fd) })
}

/// `sync(2)`.
pub fn sync() {
    // SAFETY: sync() has no preconditions.
    unsafe { libc::sync() }
}

/// `pipe(2)` without `O_CLOEXEC`; returns (read end, write end).
pub fn pipe() -> io::Result<(File, File)> {
    let mut fds = [0 as libc::c_int; 2];
    // SAFETY: `fds` has room for the two descriptors pipe() writes.
    if unsafe { libc::pipe(fds.as_mut_ptr()) } < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: both descriptors were just created and are owned by nobody else.
    unsafe { Ok((File::from_raw_fd(fds[0]), File::from_raw_fd(fds[1]))) }
}

/*------------------------------------------------------------------------*/
/* Terminal modes                                                          */

/// An opaque `struct termios`.  `zeroed()` matches a C static that was
/// never filled in.
#[derive(Clone, Copy)]
pub struct Termios(libc::termios);

impl Termios {
    pub fn zeroed() -> Termios {
        // SAFETY: termios is plain old data; all-zero is a valid value.
        Termios(unsafe { std::mem::zeroed() })
    }
}

pub fn tcgetattr(fd: impl AsFd) -> io::Result<Termios> {
    let mut t = Termios::zeroed();
    // SAFETY: the descriptor is valid for the borrow and `t.0` is writable.
    if unsafe { libc::tcgetattr(fd.as_fd().as_raw_fd(), &mut t.0) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(t)
}

pub fn tcsetattr_now(fd: impl AsFd, t: &Termios) -> io::Result<()> {
    // SAFETY: the descriptor is valid for the borrow and `t.0` is readable.
    if unsafe { libc::tcsetattr(fd.as_fd().as_raw_fd(), libc::TCSANOW, &t.0) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// The flag edits `resume_curses()` makes to the saved terminal settings.
pub fn elvis_mode(old: &Termios) -> Termios {
    let mut t = *old;
    t.0.c_iflag &= libc::IXON | libc::IXOFF | libc::IXANY | libc::ISTRIP | libc::IGNBRK;
    t.0.c_oflag &= !libc::OPOST;
    t.0.c_lflag &= libc::ISIG;
    t.0.c_cc[libc::VINTR] = 0o37 & b'C' as libc::cc_t;
    t.0.c_cc[libc::VSUSP] = 0;
    t.0.c_cc[libc::VLNEXT] = 0;
    t.0.c_cc[libc::VMIN] = 1;
    t.0.c_cc[libc::VTIME] = 0;
    t
}

pub fn erase_char(t: &Termios) -> u8 {
    t.0.c_cc[libc::VERASE]
}

/// `cfgetospeed()` stored into a C `short`, with the same truncation.
/// Linux returns a `B*` code, macOS the baud rate itself.
pub fn ospeed_short(t: &Termios) -> i16 {
    // SAFETY: `t.0` is a valid termios value.
    (unsafe { libc::cfgetospeed(&t.0) }) as i16
}

/// `TIOCGWINSZ`: `Some((rows, cols))` if the ioctl succeeds (values may be 0).
pub fn window_size(fd: impl AsFd) -> Option<(u16, u16)> {
    // SAFETY: winsize is plain old data.
    let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
    // SAFETY: TIOCGWINSZ writes one winsize into `ws`.
    let r = unsafe { libc::ioctl(fd.as_fd().as_raw_fd(), libc::TIOCGWINSZ, &mut ws) };
    if r < 0 {
        None
    } else {
        Some((ws.ws_row, ws.ws_col))
    }
}

/*------------------------------------------------------------------------*/
/* Signals                                                                 */

extern "C" fn on_sigint(_: libc::c_int) {
    SIGINT_FLAG.store(true, Ordering::SeqCst);
}

extern "C" fn on_sigwinch(_: libc::c_int) {
    SIGWINCH_FLAG.store(true, Ordering::SeqCst);
}

/// Installs a disposition with `sa_flags` 0 (no `SA_RESTART`), so a
/// blocking read returns `EINTR`.
fn set_disposition(sig: libc::c_int, handler: libc::sighandler_t) {
    // SAFETY: sigaction is plain old data; all-zero is valid.
    let mut sa: libc::sigaction = unsafe { std::mem::zeroed() };
    sa.sa_sigaction = handler;
    sa.sa_flags = 0;
    // SAFETY: `sa` is valid; the handlers only touch atomics, which is
    // async-signal-safe.
    unsafe {
        libc::sigemptyset(&mut sa.sa_mask);
        libc::sigaction(sig, &sa, std::ptr::null_mut());
    }
}

pub fn ignore_sigint() {
    set_disposition(libc::SIGINT, libc::SIG_IGN);
}

pub fn default_sigint() {
    set_disposition(libc::SIGINT, libc::SIG_DFL);
}

pub fn trap_sigint() {
    set_disposition(libc::SIGINT, on_sigint as *const () as libc::sighandler_t);
}

pub fn ignore_sigquit() {
    set_disposition(libc::SIGQUIT, libc::SIG_IGN);
}

pub fn trap_sigwinch() {
    set_disposition(
        libc::SIGWINCH,
        on_sigwinch as *const () as libc::sighandler_t,
    );
}

pub fn take_sigint() -> bool {
    SIGINT_FLAG.swap(false, Ordering::SeqCst)
}

/// Sets the `SIGINT` flag again, so the next checkpoint sees an interrupt
/// that was taken by a function that cannot propagate it.
pub fn rearm_sigint() {
    SIGINT_FLAG.store(true, Ordering::SeqCst);
}

pub fn take_sigwinch() -> bool {
    SIGWINCH_FLAG.swap(false, Ordering::SeqCst)
}

/*------------------------------------------------------------------------*/
/* Child processes                                                         */

/// How the shell is run: `execle(shell, shell, NULL)` or
/// `execle(shell, shell, "-c", cmd, NULL)`.
#[derive(Debug, Clone, Copy)]
pub enum ShellRun<'a> {
    Interactive,
    Command(&'a [u8]),
}

/// What the child does between `fork` and `exec`.
pub struct SpawnOpts<'a> {
    /// `elvis_system`: close descriptors 3 to 59 in the child.
    pub close_3_to_59: bool,
    /// `rpipe`: the child's stdout and stderr (the pipe's write end).
    pub output: Option<&'a File>,
    /// `rpipe`: closed first in the child (the pipe's read end).
    pub close_first: Option<&'a File>,
    /// `rpipe`: the child's stdin, when it is not descriptor 0.
    pub input: Option<&'a File>,
    /// Bytes the child writes to descriptor 1 if `exec` fails, before
    /// exiting with status 1 (C formats a message there).
    pub exec_fail_output: &'a [u8],
}

pub struct Child {
    pid: libc::pid_t,
}

/// `fork` + `execve` with exactly the C child-side steps.  All allocation
/// happens before the fork; the child only makes async-signal-safe calls.
pub fn spawn_shell(shell: &[u8], run: ShellRun<'_>, opts: &SpawnOpts<'_>) -> io::Result<Child> {
    let prog = cpath(shell)?;
    let dash_c = CString::new("-c").unwrap();
    let cmd = match run {
        ShellRun::Interactive => None,
        ShellRun::Command(c) => Some(cpath(c)?),
    };
    let mut argv: Vec<*const libc::c_char> = vec![prog.as_ptr()];
    if let Some(c) = &cmd {
        argv.push(dash_c.as_ptr());
        argv.push(c.as_ptr());
    }
    argv.push(std::ptr::null());

    let output = opts.output.map(|f| f.as_raw_fd());
    let close_first = opts.close_first.map(|f| f.as_raw_fd());
    let input = opts.input.map(|f| f.as_raw_fd()).filter(|&fd| fd != 0);
    let fail = opts.exec_fail_output;

    // SAFETY: fork() has no memory-safety preconditions.  In the child only
    // async-signal-safe functions (close, dup, sigaction, execve, write,
    // _exit) are called, using memory prepared above.
    let pid = unsafe { libc::fork() };
    if pid < 0 {
        return Err(io::Error::last_os_error());
    }
    if pid == 0 {
        // SAFETY: see above; every pointer refers to live CStrings.
        unsafe {
            if opts.close_3_to_59 {
                for fd in 3..60 {
                    libc::close(fd);
                }
            }
            if let Some(r) = close_first {
                libc::close(r);
            }
            if let Some(w) = output {
                libc::close(1);
                libc::dup(w);
                libc::close(2);
                libc::dup(w);
                libc::close(w);
            }
            if let Some(i) = input {
                libc::close(0);
                libc::dup(i);
                libc::close(i);
            }
            let mut sa: libc::sigaction = std::mem::zeroed();
            sa.sa_sigaction = libc::SIG_DFL;
            libc::sigaction(libc::SIGINT, &sa, std::ptr::null_mut());
            libc::execv(prog.as_ptr(), argv.as_ptr());
            if !fail.is_empty() {
                libc::write(1, fail.as_ptr().cast(), fail.len());
            }
            libc::_exit(1);
        }
    }
    Ok(Child { pid })
}

/// Waits for `child` and returns the raw wait status, as `wait(&status)`.
pub fn wait_child(child: &Child) -> i32 {
    let mut status: libc::c_int = 0;
    loop {
        // SAFETY: `status` is a valid out-pointer.
        let r = unsafe { libc::waitpid(child.pid, &mut status, 0) };
        if r >= 0 || last_errno() != libc::EINTR {
            return status;
        }
    }
}

/*------------------------------------------------------------------------*/
/* Test helpers                                                            */

#[cfg(test)]
pub fn openpty() -> io::Result<(OwnedFd, OwnedFd)> {
    let (mut m, mut s) = (0, 0);
    // SAFETY: m and s are valid out-pointers; name, termp and winp may be null.
    let r = unsafe {
        libc::openpty(
            &mut m,
            &mut s,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if r < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: both descriptors were just created by openpty().
    unsafe { Ok((OwnedFd::from_raw_fd(m), OwnedFd::from_raw_fd(s))) }
}

#[cfg(test)]
pub fn raise_sigint() {
    // SAFETY: raise() has no memory-safety preconditions.
    unsafe {
        libc::raise(libc::SIGINT);
    }
}

#[cfg(test)]
pub fn raise_sigwinch() {
    // SAFETY: raise() has no memory-safety preconditions.
    unsafe {
        libc::raise(libc::SIGWINCH);
    }
}

#[cfg(test)]
pub fn exit_status(raw: i32) -> Option<i32> {
    if libc::WIFEXITED(raw) {
        Some(libc::WEXITSTATUS(raw))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Seek, SeekFrom, Write};

    fn testdir(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("elvis-os-{}-{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn b(p: &std::path::Path) -> Vec<u8> {
        use std::os::unix::ffi::OsStrExt;
        p.as_os_str().as_bytes().to_vec()
    }

    #[test]
    fn mkstemp_link_unlink() {
        let d = testdir("mkstemp");
        let mut t = b(&d.join("elvXXXXXX"));
        let mut f = mkstemp(&mut t).unwrap();
        assert!(!t.ends_with(b"XXXXXX"));
        f.write_all(b"hello").unwrap();
        let name = b(&d.join("elvtmp"));
        link(&t, &name).unwrap();
        unlink(&t).unwrap();
        assert_eq!(access(&t, 0), -1);
        assert_eq!(access(&name, 0), 0);
        let st = stat(&name).unwrap();
        assert_eq!(st.mode & S_IFMT, S_IFREG);
        assert_eq!(st.mode & 0o777, 0o600);
        assert_eq!(st.size, 5);
        f.seek(SeekFrom::Start(0)).unwrap();
        let mut s = String::new();
        f.read_to_string(&mut s).unwrap();
        assert_eq!(s, "hello");
        let renamed = b(&d.join("renamed"));
        rename(&name, &renamed).unwrap();
        unlink(&renamed).unwrap();
        assert!(stat(&renamed).is_err());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn open_flags_and_creat() {
        let d = testdir("open");
        let p = b(&d.join("f"));
        assert_eq!(
            open(&p, O_RDONLY, 0).unwrap_err().raw_os_error(),
            Some(ENOENT)
        );
        let mut f = creat(&p, 0o666).unwrap();
        f.write_all(b"ab").unwrap();
        drop(f);
        let mut f = open(&p, O_WRONLY | O_APPEND, 0).unwrap();
        f.write_all(b"cd").unwrap();
        drop(f);
        assert_eq!(std::fs::read(d.join("f")).unwrap(), b"abcd");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn termios_round_trip_on_pty() {
        let (_m, s) = openpty().unwrap();
        let old = tcgetattr(&s).unwrap();
        let new = elvis_mode(&old);
        tcsetattr_now(&s, &new).unwrap();
        let got = tcgetattr(&s).unwrap();
        assert_eq!(got.0.c_lflag & libc::ICANON, 0);
        assert_eq!(got.0.c_lflag & libc::ECHO, 0);
        assert_eq!(got.0.c_oflag & libc::OPOST, 0);
        assert_eq!(got.0.c_cc[libc::VINTR], 3);
        assert_eq!(got.0.c_cc[libc::VMIN], 1);
        assert_eq!(got.0.c_cc[libc::VSUSP], 0);
        tcsetattr_now(&s, &old).unwrap();
        let back = tcgetattr(&s).unwrap();
        // The kernel may set PENDIN when leaving non-canonical mode.
        assert_eq!(
            back.0.c_lflag & !libc::PENDIN,
            old.0.c_lflag & !libc::PENDIN
        );
        assert_eq!(back.0.c_iflag, old.0.c_iflag);
        assert_eq!(back.0.c_oflag, old.0.c_oflag);
        assert_eq!(erase_char(&back), erase_char(&old));
        let _ = ospeed_short(&back);
        assert!(window_size(&s).is_some());
        assert!(tcgetattr(File::open("/dev/null").unwrap()).is_err());
    }

    #[test]
    fn signal_flags() {
        trap_sigint();
        trap_sigwinch();
        let _ = take_sigint();
        assert!(!take_sigint());
        raise_sigint();
        assert!(take_sigint());
        assert!(!take_sigint());
        raise_sigwinch();
        assert!(take_sigwinch());
        assert!(!take_sigwinch());
        ignore_sigint();
        raise_sigint();
        assert!(!take_sigint());
    }

    #[test]
    fn read_timeout_on_pipe() {
        let (r, mut w) = pipe().unwrap();
        let mut buf = [0u8; 4];
        let t = std::time::Instant::now();
        assert_eq!(
            read_fd_timeout(r.as_raw_fd(), &mut buf, 1),
            ReadOutcome::Timeout
        );
        assert!(t.elapsed() >= std::time::Duration::from_millis(900));
        w.write_all(b"ab").unwrap();
        assert_eq!(
            read_fd_timeout(r.as_raw_fd(), &mut buf, 1),
            ReadOutcome::Data(2)
        );
        drop(w);
        assert_eq!(
            read_fd_timeout(r.as_raw_fd(), &mut buf, 0),
            ReadOutcome::Eof
        );
    }

    fn sh() -> &'static [u8] {
        b"/bin/sh"
    }

    #[test]
    fn spawn_with_pipe_and_input() {
        let d = testdir("spawn");
        let p = b(&d.join("in"));
        let mut inf = creat(&p, 0o600).unwrap();
        inf.write_all(b"c\nb\na\n").unwrap();
        drop(inf);
        let input = open(&p, O_RDONLY, 0).unwrap();
        let (mut r, w) = pipe().unwrap();
        let child = spawn_shell(
            sh(),
            ShellRun::Command(b"sort; echo err >&2"),
            &SpawnOpts {
                close_3_to_59: false,
                output: Some(&w),
                close_first: Some(&r),
                input: Some(&input),
                exec_fail_output: b"",
            },
        )
        .unwrap();
        drop(w);
        let mut out = Vec::new();
        r.read_to_end(&mut out).unwrap();
        let st = wait_child(&child);
        assert_eq!(exit_status(st), Some(0));
        assert_eq!(out, b"a\nb\nc\nerr\n");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn wait_status_codes() {
        for code in [0, 3] {
            let (mut r, w) = pipe().unwrap();
            let cmd = format!("exit {code}");
            let child = spawn_shell(
                sh(),
                ShellRun::Command(cmd.as_bytes()),
                &SpawnOpts {
                    close_3_to_59: false,
                    output: Some(&w),
                    close_first: Some(&r),
                    input: None,
                    exec_fail_output: b"",
                },
            )
            .unwrap();
            drop(w);
            let mut out = Vec::new();
            r.read_to_end(&mut out).unwrap();
            let st = wait_child(&child);
            assert_eq!(exit_status(st), Some(code));
            assert_eq!(st == 0, code == 0);
        }
    }

    #[test]
    fn exec_failure_writes_message_and_exits_1() {
        let (mut r, w) = pipe().unwrap();
        let child = spawn_shell(
            b"/nonexistent/shell",
            ShellRun::Command(b"true"),
            &SpawnOpts {
                close_3_to_59: false,
                output: Some(&w),
                close_first: Some(&r),
                input: None,
                exec_fail_output: b"exec failed\n",
            },
        )
        .unwrap();
        drop(w);
        let mut out = Vec::new();
        r.read_to_end(&mut out).unwrap();
        assert_eq!(exit_status(wait_child(&child)), Some(1));
        assert_eq!(out, b"exec failed\n");
    }

    #[test]
    fn child_closes_descriptors_3_to_59() {
        let d = testdir("close");
        let keep = open(b"/dev/null", O_RDONLY, 0).unwrap();
        let fd = keep.as_raw_fd();
        assert!((3..60).contains(&fd));
        let out = b(&d.join("out"));
        let cmd = format!(
            "if [ -e /dev/fd/{fd} ]; then echo open; else echo closed; fi > '{}'",
            String::from_utf8_lossy(&out)
        );
        let child = spawn_shell(
            sh(),
            ShellRun::Command(cmd.as_bytes()),
            &SpawnOpts {
                close_3_to_59: true,
                output: None,
                close_first: None,
                input: None,
                exec_fail_output: b"",
            },
        )
        .unwrap();
        assert_eq!(exit_status(wait_child(&child)), Some(0));
        assert_eq!(std::fs::read(d.join("out")).unwrap(), b"closed\n");

        let child = spawn_shell(
            sh(),
            ShellRun::Command(cmd.as_bytes()),
            &SpawnOpts {
                close_3_to_59: false,
                output: None,
                close_first: None,
                input: None,
                exec_fail_output: b"",
            },
        )
        .unwrap();
        assert_eq!(exit_status(wait_child(&child)), Some(0));
        assert_eq!(std::fs::read(d.join("out")).unwrap(), b"open\n");
        drop(keep);
        std::fs::remove_dir_all(&d).unwrap();
    }
}
