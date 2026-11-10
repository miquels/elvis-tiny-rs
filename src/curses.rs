//! `curses.h` and `curses.c`: a tiny subset of curses.
//!
//! Capabilities come from the small terminfo reader in `terminfo.rs`
//! (IMPLEMENTATION.md section 4) instead of the system termcap emulation.

use crate::opts::Opt;
use std::io;

use crate::os::{self, Termios};
use crate::terminfo::{Terminfo, tgoto, tputs_into};
use crate::vars::Editor;

pub const KBSIZ: usize = 6 * 1024;

/// Capability strings: `None` is a C null pointer, `Some(vec![])` is "".
pub type Cap = Option<Vec<u8>>;

/// Capability slots in `Curses::caps`, in `starttcap()` order.  The two
/// halves of a `PAIR` are adjacent.
pub const UP: usize = 0;
pub const VB: usize = 1;
pub const CM: usize = 2;
pub const SO: usize = 3;
pub const SE: usize = 4;
pub const TI: usize = 5;
pub const TE: usize = 6;
pub const US: usize = 7;
pub const UE: usize = 8;
pub const MD: usize = 9;
pub const ME: usize = 10;
pub const AS: usize = 11;
pub const AE: usize = 12;
pub const AL: usize = 13;
pub const DL: usize = 14;
pub const CE: usize = 15;
pub const CD: usize = 16;
pub const SR: usize = 17;
pub const IM: usize = 18;
pub const EI: usize = 19;
pub const IC: usize = 20;
pub const DC: usize = 21;
pub const KS: usize = 22;
pub const KE: usize = 23;
pub const KU: usize = 24;
pub const KD: usize = 25;
pub const KL: usize = 26;
pub const KR: usize = 27;
pub const PU: usize = 28;
pub const PD: usize = 29;
pub const HM: usize = 30;
pub const EN: usize = 31;
pub const AEND: usize = 32;
pub const NCAPS: usize = 33;

const fn mask(slots: &[usize]) -> u64 {
    let mut m = 0;
    let mut i = 0;
    while i < slots.len() {
        m |= 1 << slots[i];
        i += 1;
    }
    m
}

/// Slots whose `has_*` macro tests the first byte rather than the pointer.
const STR_TEST: u64 = mask(&[SO, SE, TI, TE, US, UE, MD, ME, AS, AE, IM, EI, IC, KS, KE]);
/// Slots that start as `""` instead of null.
const START_EMPTY: u64 = mask(&[US, UE, MD, ME, IC, AEND]);

/// The termcap name of each slot except `AEND`.
const NAMES: [&[u8; 2]; NCAPS - 1] = [
    b"up", b"vb", b"cm", b"so", b"se", b"ti", b"te", b"us", b"ue", b"md", b"me", b"as", b"ae",
    b"al", b"dl", b"ce", b"cd", b"sr", b"im", b"ei", b"ic", b"dc", b"ks", b"ke", b"ku", b"kd",
    b"kl", b"kr", b"kP", b"kN", b"kh", b"kH",
];
/// The terminfo string index ncurses reads for each termcap name.
const TINFO: [u8; NCAPS - 1] = [
    19, 45, 10, 35, 43, 28, 40, 36, 44, 27, 39, 25, 38, 53, 22, 6, 7, 130, 31, 42, 52, 21, 89, 88,
    87, 61, 79, 83, 82, 81, 76, 80,
];

pub struct Curses {
    pub ospeed: i16,
    /// `kbuf`; `stdscr` is `kbuf.len()`.
    pub kbuf: Vec<u8>,
    pub lines: i32,
    pub cols: i32,
    pub am: bool,
    pub pt: bool,
    /// Capability strings, indexed by the slot constants above.
    pub caps: [Cap; NCAPS],
    pub erasekey: u8,
    pub oldtermio: Termios,
    /// Whether `initscr()` has filled `oldtermio` (B4).
    pub have_oldtermio: bool,
    pub newtermio: Termios,
    pub curses_active: bool,
    /// `tgetnum("li")` and `tgetnum("co")` as ncurses computes them at
    /// `tgetent()` time; -1 when absent.
    pub tc_lines: i32,
    pub tc_cols: i32,
    /// Terminfo static variables `%PA`..`%PZ` persist across `tgoto()`.
    pub tvars: [i32; 26],
}

impl Default for Curses {
    fn default() -> Curses {
        Curses {
            ospeed: 0,
            kbuf: Vec::with_capacity(KBSIZ),
            lines: 0,
            cols: 0,
            am: false,
            pt: false,
            caps: std::array::from_fn(|i| (START_EMPTY & (1 << i) != 0).then(Vec::new)),
            erasekey: 0,
            oldtermio: Termios::zeroed(),
            have_oldtermio: false,
            newtermio: Termios::zeroed(),
            curses_active: false,
            tc_lines: -1,
            tc_cols: -1,
            tvars: [0; 26],
        }
    }
}

/*------------------------------------------------------------------------*/
/* termcap names                                                           */

/// `tgetstr(id)` for the termcap name of `slot`, read from the terminfo
/// string capability ncurses uses for it.
fn tgetstr(ti: &Terminfo, slot: usize) -> Cap {
    let s = ti.str(TINFO[slot] as usize)?;
    if slot == ME {
        return Some(ti.trim_sgr0(s));
    }
    Some(s.to_vec())
}
/*------------------------------------------------------------------------*/
/* Output buffer: qaddch, addch, and the do_* / has_* macros               */

#[inline]
pub fn addch_into(out: &mut Vec<u8>, ch: u8) {
    out.push(ch);
    if ch == b'\n' {
        out.push(b'\r');
    }
}

fn first_byte(c: &Cap) -> u8 {
    c.as_ref().and_then(|s| s.first().copied()).unwrap_or(0)
}

impl Curses {
    /// `qaddch(ch)`; returns `ch` like the C macro.
    #[inline]
    pub fn qaddch(&mut self, ch: u8) -> u8 {
        self.kbuf.push(ch);
        ch
    }

    /// `addch(ch)`
    #[inline]
    pub fn addch(&mut self, ch: u8) {
        addch_into(&mut self.kbuf, ch);
    }

    /// `qaddstr(str)`
    pub fn qaddstr(&mut self, s: &[u8]) {
        self.kbuf.extend_from_slice(s);
    }

    /// `addstr(str)`: `qaddstr` then `_addCR`.
    pub fn addstr(&mut self, s: &[u8]) {
        self.qaddstr(s);
        if self.kbuf.last() == Some(&b'\n') {
            self.qaddch(b'\r');
        }
    }

    /// `move(y, x)`: `tputs(tgoto(CM, x, y), 1, faddch)`.
    pub fn mv(&mut self, y: i32, x: i32) {
        if let Some(cm) = &self.caps[CM]
            && let Some(s) = tgoto(&mut self.tvars, cm, x, y)
        {
            tputs_into(&s, 1, &mut self.kbuf);
        }
    }

    /// `refresh()` / `wrefresh(stdscr)`
    pub fn refresh(&mut self) {
        if !self.kbuf.is_empty() {
            let _ = os::tty_write(&self.kbuf);
            self.kbuf.clear();
        }
    }

    /// `wqrefresh(stdscr)`
    pub fn wqrefresh(&mut self) {
        if self.kbuf.len() > 2000 {
            let _ = os::tty_write(&self.kbuf);
            self.kbuf.clear();
        }
    }

    /// The `do_*` macros: output a capability through `tputs()`.
    pub fn put(&mut self, slot: usize) {
        let aff = match slot {
            AL | DL => self.lines,
            DC => self.cols,
            _ => 1,
        };
        if let Some(s) = &self.caps[slot] {
            tputs_into(s, aff, &mut self.kbuf);
        }
    }

    /// The `has_*` macros.
    pub fn has(&self, slot: usize) -> bool {
        if STR_TEST & (1 << slot) != 0 {
            first_byte(&self.caps[slot]) != 0
        } else {
            self.caps[slot].is_some()
        }
    }

    /// `do_TI()`: written raw, so delays are output literally.
    pub fn do_ti(&mut self) {
        if let Some(s) = &self.caps[TI] {
            let _ = os::tty_write(s);
        }
    }

    /// `do_TE()`
    pub fn do_te(&mut self) {
        if let Some(s) = &self.caps[TE] {
            let _ = os::tty_write(s);
        }
    }

    pub fn has_am(&self) -> bool {
        self.am
    }
    pub fn has_pt(&self) -> bool {
        self.pt
    }

    /// `standout()` and friends.
    pub fn standout(&mut self) {
        self.put(SO);
    }
    pub fn standend(&mut self) {
        self.put(SE);
    }
    pub fn clrtoeol(&mut self) {
        self.put(CE);
    }
    /// Queues a control character as `^X`.
    pub fn qaddctl(&mut self, c: u8) {
        self.qaddch(b'^');
        self.qaddch(c ^ b'@');
    }
    /// Moves to the start of the bottom line and clears it.
    pub fn clear_last_line(&mut self) {
        self.mv(self.lines - 1, 0);
        self.clrtoeol();
    }
    pub fn insertln(&mut self) {
        self.put(AL);
    }
    pub fn deleteln(&mut self) {
        self.put(DL);
    }
}

/*------------------------------------------------------------------------*/
/* curses.c                                                                */

// C: curses.c:initscr
// Out of line: inlining this into `main` makes the binary larger.
#[inline(never)]
pub fn initscr(ed: &mut Editor) {
    if crate::os::getenv("TERM").is_none() {
        let _ = os::err_write(b"Environment variable TERM must be set\n");
        os::exit(1);
    } else {
        starttcap(ed);
    }

    ed.curses.kbuf.clear();

    // C ignores the tcgetattr() result and keeps the zeroed static.
    if let Ok(t) = os::tcgetattr(io::stderr()) {
        ed.curses.oldtermio = t;
    }
    ed.curses.have_oldtermio = true;

    ed.curses.resume();
}

impl Curses {
    // C: curses.c:suspend_curses
    /// Also C's `endwin()`, which only calls `suspend_curses()`.
    pub fn suspend(&mut self) {
        if self.has(TE) {
            self.do_te();
        }
        if self.has(KE) {
            self.put(KE);
        }
        self.refresh();

        // Before initscr() the terminal is still untouched (B4).
        if self.have_oldtermio {
            let _ = os::tcsetattr_now(io::stderr(), &self.oldtermio);
        }
        os::clear_restore_state();
        self.curses_active = false;
    }

    /// The terminal half of `resume_curses()`.
    pub fn resume(&mut self) {
        if self.curses_active {
            return;
        }
        self.ospeed = os::ospeed_short(&self.oldtermio);
        self.erasekey = os::erase_char(&self.oldtermio);
        self.newtermio = os::elvis_mode(&self.oldtermio);
        let _ = os::tcsetattr_now(io::stderr(), &self.newtermio);

        if self.has(TI) {
            self.do_ti();
        }
        if self.has(KS) {
            self.put(KS);
        }

        let mut restore = Vec::new();
        if self.has(TE) {
            restore.extend_from_slice(self.caps[TE].as_deref().unwrap_or_default());
        }
        if self.has(KE) {
            tputs_into(
                self.caps[KE].as_deref().unwrap_or_default(),
                1,
                &mut restore,
            );
        }
        os::save_restore_state(&self.oldtermio, &restore);

        self.curses_active = true;
    }
}

// C: curses.c:resume_curses
pub fn resume_curses(ed: &mut Editor, quietly: bool) {
    ed.curses.resume();
    if !quietly {
        resume_curses_prompt(ed);
    }
}

/// The "[Press <RETURN> to continue]" half of `resume_curses()`.
fn resume_curses_prompt(ed: &mut Editor) {
    os::ignore_sigint();

    let c = &mut ed.curses;
    c.mv(c.lines - 1, 0);
    c.put(SO);
    c.qaddstr(b"[Press <RETURN> to continue]");
    c.put(SE);
    c.refresh();
    // C reads into kbuf, which refresh() has just emptied.
    let mut buf = [0u8; 20];
    let _ = crate::config::ttyread(&mut buf);
    if buf[0] == b':' {
        ed.vars.mode = crate::types::Mode::Colon;
        ed.curses.addch(b'\n');
        ed.curses.refresh();
    } else {
        ed.vars.mode = crate::types::Mode::Vi;
        crate::redraw::redraw(ed, crate::types::MARK_UNSET, false);
    }
    ed.vars.exwrote = false;

    os::trap_sigint();
}

// C: curses.c:lacking
fn lacking(s: &[u8; 2]) -> ! {
    let _ = os::err_write(b"This termcap entry lacks the :");
    let _ = os::err_write(s);
    let _ = os::err_write(b"=: capability\n");
    os::exit(1);
}

/// The C `MUSTHAVE`, `MAYHAVE` and `PAIR` macros.
#[derive(Clone, Copy)]
enum Need {
    Must,
    May,
    /// Both `slot` and `slot + 1`, or neither.
    Pair,
}
use Need::{May, Must, Pair};

fn load(ed: &mut Editor, ti: &Terminfo, steps: &[(Need, usize)]) {
    let caps = &mut ed.curses.caps;
    for &(rule, s) in steps {
        let v = tgetstr(ti, s);
        match rule {
            Must => {
                if v.is_none() {
                    lacking(NAMES[s]);
                }
                caps[s] = v;
            }
            May => {
                if v.is_some() {
                    caps[s] = v;
                }
            }
            Pair => {
                let w = tgetstr(ti, s + 1);
                if v.is_none() || w.is_none() {
                    caps[s] = Some(Vec::new());
                    caps[s + 1] = Some(Vec::new());
                } else {
                    caps[s] = v;
                    caps[s + 1] = w;
                }
            }
        }
    }
}

// C: curses.c:starttcap
fn starttcap(ed: &mut Editor) {
    let term = crate::os::getenv("TERM").unwrap_or_default();
    let Some(ti) = Terminfo::load(&term) else {
        let _ = os::err_write(b"Unrecognized TERM type\n");
        os::exit(3);
    };

    let (li, co) = ti.screensize();
    ed.curses.tc_lines = li;
    ed.curses.tc_cols = co;

    load(
        ed,
        &ti,
        &[(Must, UP), (May, VB), (Must, CM), (Pair, SO), (Pair, TI)],
    );
    if ti.num("ug") <= 0 {
        load(ed, &ti, &[(Pair, US), (Pair, MD), (Pair, AS)]);
        let c = &mut ed.curses.caps;
        if first_byte(&c[AS]) == 0 {
            c[AS] = c[US].clone();
            c[AE] = c[UE].clone();
        }
    }
    #[rustfmt::skip]
    load(ed, &ti, &[
        (May, AL), (May, DL), (Must, CE), (May, CD), (May, SR), (Pair, IM), (May, IC), (May, DC),
    ]);

    ed.curses.am = ti.flag("am");
    ed.curses.pt = ti.flag("pt");
    getsize(ed, true);

    #[rustfmt::skip]
    load(ed, &ti, &[
        (Pair, KS), (May, KU), (May, KD), (May, KL), (May, KR), (May, PU), (May, PD), (May, HM),
        (May, EN),
    ]);
}

// C: curses.c:getsize
/// `startup` is C's `signo == 0`; otherwise the window was resized.
pub fn getsize(ed: &mut Editor, startup: bool) {
    if startup {
        os::trap_sigwinch();
    }

    let mut lines = 0;
    let mut cols = 0;
    if let Some((r, c)) = os::window_size(io::stderr()) {
        lines = r as i32;
        cols = c as i32;
    }
    let c = &mut ed.curses;
    if (lines == 0 || cols == 0) && startup {
        c.lines = c.tc_lines;
        c.cols = c.tc_cols;
    }
    if lines >= 2 && cols >= 30 {
        c.lines = lines;
        c.cols = cols;
    }

    if c.lines < 2 || c.cols < 30 {
        let _ = os::err_write(b"Screen too small\n");
        ed.curses.suspend();
        os::exit(2);
    }

    ed.opts.set_num(Opt::Columns, ed.curses.cols);
    ed.opts.set_num(Opt::Lines, ed.curses.lines);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slot(name: &str) -> usize {
        NAMES
            .iter()
            .position(|n| &n[..] == name.as_bytes())
            .unwrap()
    }

    /// The `capprobe.c` report, produced from the Rust side.
    fn report(file: &[u8]) -> String {
        fn esc(out: &mut String, s: &[u8]) {
            for &c in s {
                if (0x20..0x7f).contains(&c) && c != b'\\' {
                    out.push(c as char);
                } else {
                    out.push_str(&format!("\\x{c:02x}"));
                }
            }
        }
        const STRS: [&str; 32] = [
            "up", "vb", "cm", "so", "se", "ti", "te", "us", "ue", "md", "me", "as", "ae", "al",
            "dl", "ce", "cd", "sr", "im", "ei", "ic", "dc", "ks", "ke", "ku", "kd", "kl", "kr",
            "kP", "kN", "kh", "kH",
        ];
        let ti = Terminfo::parse(file).unwrap();
        let mut o = String::from("tgetent=1\nospeed=0 PC=0\n");
        for s in STRS {
            o.push_str(&format!("str {s}="));
            match tgetstr(&ti, slot(s)) {
                None => o.push_str("(null)"),
                Some(v) => {
                    o.push('"');
                    esc(&mut o, &v);
                    o.push('"');
                }
            }
            o.push('\n');
        }
        for f in ["am", "pt", "xo", "nx"] {
            o.push_str(&format!("flag {f}={}\n", ti.flag(f) as i32));
        }
        for n in ["ug", "li", "co", "pb"] {
            // The probe ran on a 24x80 pty with no LINES/COLUMNS.
            let v = match (n, ti.num(n)) {
                ("li", v) if v <= 0 => 24,
                ("co", v) if v <= 0 => 80,
                (_, v) => v,
            };
            o.push_str(&format!("num {n}={v}\n"));
        }
        if let Some(cm) = tgetstr(&ti, CM) {
            let mut e = [0; 26];
            for r in [0, 1, 9, 10, 23, 99] {
                for c in [0, 1, 7, 79, 132] {
                    let mut out = Vec::new();
                    if let Some(s) = tgoto(&mut e, &cm, c, r) {
                        tputs_into(&s, 1, &mut out);
                    }
                    o.push_str(&format!("cm row={r} col={c} \""));
                    esc(&mut o, &out);
                    o.push_str("\"\n");
                }
            }
        }
        for s in STRS {
            let Some(v) = tgetstr(&ti, slot(s)) else {
                continue;
            };
            for aff in [1, 24] {
                o.push_str(&format!("tputs {s} aff={aff} \""));
                let mut out = Vec::new();
                tputs_into(&v, aff, &mut out);
                esc(&mut o, &out);
                o.push_str("\"\n");
            }
        }
        o
    }

    fn root() -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
    }

    fn fixture_file(name: &str) -> Option<Vec<u8>> {
        let dir = root().join("target/terminfo-fixtures");
        let first = format!("{:x}", name.as_bytes()[0]);
        std::fs::read(dir.join(first).join(name)).ok()
    }

    /// ncurses rewrites `%{50}` as `%'2'` when returning `cm` through
    /// `tgetstr`; elvis only ever expands `cm`, so the stored spelling is
    /// not observable.
    fn normalize(s: &str) -> String {
        s.replace("%'2'", "%{50}")
    }

    #[test]
    fn capprobe_golden() {
        let golden = root().join("tests/golden/macos/capprobe");
        let mut checked = 0;
        let mut diffs = Vec::new();
        for t in [
            "xterm",
            "vt100",
            "linux",
            "screen",
            "missing-cup",
            "missing-el",
            "asym-cup",
            "pad-cup",
        ] {
            let Some(file) = fixture_file(t) else {
                eprintln!("skipping {t}: run tests/tools/build-terminfo.sh");
                continue;
            };
            let got = report(&file);
            for speed in ["1200", "38400"] {
                let want = std::fs::read_to_string(golden.join(format!("{t}.{speed}.txt")))
                    .expect("golden capprobe report");
                let (g, w) = (normalize(&got), normalize(&want));
                for (a, b) in g.lines().zip(w.lines()) {
                    if a != b {
                        diffs.push(format!("{t}.{speed}: got {a} want {b}"));
                    }
                }
                if g.lines().count() != w.lines().count() {
                    diffs.push(format!("{t}.{speed}: line counts differ"));
                }
                checked += 1;
            }
        }
        if checked == 0 {
            eprintln!("capprobe_golden: no fixtures compiled");
        }
        assert!(diffs.is_empty(), "{}", diffs.join("\n"));
    }

    /// Child half of `exit_paths`: runs `initscr()` in a subprocess.
    #[test]
    fn exit_path_child() {
        if crate::os::getenv("ELVIS_EXIT_PATH_CHILD").is_none() {
            return;
        }
        let mut ed = Editor::new();
        initscr(&mut ed);
        ed.curses.suspend();
        os::exit(0);
    }

    #[test]
    fn exit_paths() {
        let dir = root().join("target/terminfo-fixtures");
        if !dir.exists() {
            eprintln!("skipping: run tests/tools/build-terminfo.sh");
            return;
        }
        let exe = std::env::current_exe().unwrap();
        let run = |term: Option<&str>, lines: &str, cols: &str| {
            let mut cmd = std::process::Command::new(&exe);
            cmd.args(["--exact", "curses::tests::exit_path_child", "--nocapture"])
                .env("ELVIS_EXIT_PATH_CHILD", "1")
                .env("TERMINFO", &dir)
                .env("LINES", lines)
                .env("COLUMNS", cols)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::piped());
            match term {
                Some(t) => cmd.env("TERM", t),
                None => cmd.env_remove("TERM"),
            };
            let out = cmd.output().unwrap();
            let err = String::from_utf8_lossy(&out.stderr).into_owned();
            (out.status.code(), err)
        };
        let (code, err) = run(None, "24", "80");
        assert_eq!(code, Some(1));
        assert!(err.contains("Environment variable TERM must be set\n"));
        let (code, err) = run(Some("no-such-terminal"), "24", "80");
        assert_eq!(code, Some(3));
        assert!(err.contains("Unrecognized TERM type\n"));
        let (code, err) = run(Some("missing-cup"), "24", "80");
        assert_eq!(code, Some(1));
        assert!(err.contains("This termcap entry lacks the :cm=: capability\n"));
        let (code, err) = run(Some("missing-el"), "24", "80");
        assert_eq!(code, Some(1));
        assert!(err.contains("This termcap entry lacks the :ce=: capability\n"));
        let (code, err) = run(Some("vt100"), "1", "80");
        assert_eq!(code, Some(2));
        assert!(err.contains("Screen too small\n"));
        let (code, err) = run(Some("vt100"), "24", "29");
        assert_eq!(code, Some(2));
        assert!(err.contains("Screen too small\n"));
        let (code, _) = run(Some("vt100"), "24", "80");
        assert_eq!(code, Some(0));
    }
}
