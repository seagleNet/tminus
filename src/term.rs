//! Off-screen frame buffer plus a diffing terminal presenter.

use std::io::{self, BufWriter, Stdout, Write};

use crossterm::{
    cursor, execute, queue,
    style::{Color, Print, ResetColor, SetBackgroundColor},
    terminal,
};
use unicode_width::UnicodeWidthChar;

use crate::theme::Rgb;

/// Placeholder in the cell to the right of a double-width char. It is never
/// printed: the wide char already covers it.
pub const WIDE_TAIL: char = '\0';

/// Colors closer than this (per channel) count as unchanged when diffing, so
/// slow gradient drift doesn't rewrite every cell on every frame.
const COLOR_TOLERANCE: u8 = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cell {
    pub ch: char,
    pub fg: Rgb,
    /// This cell's own background (the nixie font's lower pixel); `None` shows
    /// the frame's.
    pub bg: Option<Rgb>,
}

impl Cell {
    fn looks_like(self, other: Cell) -> bool {
        let near = |a: Rgb, b: Rgb| {
            let n = |a: u8, b: u8| a.abs_diff(b) <= COLOR_TOLERANCE;
            n(a.0, b.0) && n(a.1, b.1) && n(a.2, b.2)
        };
        let bg = match (self.bg, other.bg) {
            (None, None) => true,
            (Some(a), Some(b)) => near(a, b),
            _ => false,
        };
        self.ch == other.ch && near(self.fg, other.fg) && bg
    }
}

/// Display width of a string in terminal columns.
pub fn str_width(s: &str) -> i32 {
    s.chars().map(|c| c.width().unwrap_or(0) as i32).sum()
}

#[derive(Clone, PartialEq)]
pub struct Frame {
    pub w: i32,
    pub h: i32,
    /// Whole-screen background; `None` keeps the terminal's own.
    pub bg: Option<Rgb>,
    cells: Vec<Option<Cell>>,
}

impl Frame {
    pub fn new(w: u16, h: u16) -> Self {
        Frame {
            w: i32::from(w),
            h: i32::from(h),
            bg: None,
            cells: vec![None; w as usize * h as usize],
        }
    }

    pub fn clear(&mut self) {
        self.bg = None;
        self.cells.fill(None);
    }

    /// The characters drawn, row by row (for tests).
    #[cfg(test)]
    pub fn chars(&self) -> String {
        self.cells.iter().map(|c| c.map_or(' ', |c| c.ch)).collect()
    }

    /// Put a whole cell, background included (`None` clears it).
    pub fn set(&mut self, x: i32, y: i32, cell: Option<Cell>) {
        if x >= 0 && y >= 0 && x < self.w && y < self.h {
            self.cells[(y * self.w + x) as usize] = cell;
        }
    }

    /// The cell at (`x`, `y`), `None` if empty or off the frame.
    pub fn get(&self, x: i32, y: i32) -> Option<Cell> {
        if x >= 0 && y >= 0 && x < self.w && y < self.h {
            self.cells[(y * self.w + x) as usize]
        } else {
            None
        }
    }

    fn bg_at(&self, x: i32, y: i32) -> Option<Rgb> {
        self.get(x, y).and_then(|c| c.bg)
    }

    /// Put one char. Double-width chars also claim the cell to their right;
    /// one that would hang off the right edge is dropped. A background
    /// already in the cell (nixie glass behind an effect) is kept.
    pub fn put(&mut self, x: i32, y: i32, ch: char, fg: Rgb) {
        let bg = self.bg_at(x, y);
        match ch {
            WIDE_TAIL => {}
            ' ' => self.set(x, y, bg.map(|bg| Cell { ch, fg, bg: Some(bg) })),
            _ if ch.width() == Some(2) => {
                if x + 1 < self.w {
                    let bg2 = self.bg_at(x + 1, y);
                    self.set(x, y, Some(Cell { ch, fg, bg }));
                    self.set(
                        x + 1,
                        y,
                        Some(Cell {
                            ch: WIDE_TAIL,
                            fg,
                            bg: bg2,
                        }),
                    );
                }
            }
            _ => self.set(x, y, Some(Cell { ch, fg, bg })),
        }
    }

    /// Put a string starting at column `x`, advancing by display width.
    pub fn text(&mut self, x: i32, y: i32, s: &str, fg: Rgb) {
        let mut x = x;
        for ch in s.chars() {
            self.put(x, y, ch, fg);
            x += ch.width().unwrap_or(0) as i32;
        }
    }
}

/// Result of waiting for terminal input.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Input {
    None,
    Ready,
    /// The terminal is gone (e.g. its window was closed).
    HungUp,
}

/// Passes writes on, counting the bytes (for the debug panel).
struct Counted<W> {
    inner: W,
    bytes: u64,
}

impl<W: Write> Write for Counted<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let n = self.inner.write(buf)?;
        self.bytes += n as u64;
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

/// What the last `present` wrote.
#[derive(Clone, Copy, Default)]
pub struct Presented {
    /// Cells rewritten.
    pub cells: usize,
    /// The whole screen was repainted.
    pub full: bool,
}

pub struct Term {
    out: BufWriter<Counted<Stdout>>,
    /// Our own handle on the terminal, used to wait for input and to notice
    /// hangups without going through crossterm's reader (see `wait_input`).
    #[cfg(unix)]
    tty: std::fs::File,
    /// What the terminal currently shows (as last written by us).
    shown: Option<Frame>,
    /// The terminal's own background color, if it told us.
    background: Option<Rgb>,
    /// Clearing fills with the current background color (terminfo `bce`),
    /// so a background fill needn't print every blank cell.
    bce: bool,
    /// What the last `present` wrote.
    pub presented: Presented,
    /// A frame's output, reused.
    buf: Vec<u8>,
}

impl Term {
    pub fn enter() -> io::Result<Self> {
        let mut out = BufWriter::with_capacity(
            1 << 16,
            Counted {
                inner: io::stdout(),
                bytes: 0,
            },
        );
        terminal::enable_raw_mode()?;
        execute!(
            out,
            terminal::EnterAlternateScreen,
            cursor::Hide,
            terminal::Clear(terminal::ClearType::All)
        )?;
        #[cfg(unix)]
        let tty = std::fs::File::open("/dev/tty")?;
        // Neovim's (and Vim's) :terminal answer with libvterm's default,
        // black, not the color they actually draw; the theme's guess is better.
        #[cfg(unix)]
        let background = if ["NVIM", "VIM_TERMINAL"].iter().any(|v| std::env::var_os(v).is_some()) {
            None
        } else {
            query_background(&mut out, &tty)
        };
        #[cfg(not(unix))]
        let background = None;
        Ok(Term {
            out,
            shown: None,
            #[cfg(unix)]
            tty,
            background,
            bce: back_color_erase(),
            presented: Presented::default(),
            buf: Vec::new(),
        })
    }

    /// The terminal's background color, as it reported at startup. Fades to
    /// "no fill" should end on this, or they visibly jump at the last frame.
    pub fn background(&self) -> Option<Rgb> {
        self.background
    }

    /// Whether clearing paints with the background color (terminfo `bce`).
    pub fn bce(&self) -> bool {
        self.bce
    }

    /// Bytes written to the terminal so far (flushed ones).
    pub fn written(&self) -> u64 {
        self.out.get_ref().bytes
    }

    /// Force a full repaint on the next `present` (e.g. after a resize).
    pub fn invalidate(&mut self) {
        self.shown = None;
    }

    /// Wait up to `timeout` for input.
    ///
    /// crossterm's event reader spins at 100% CPU forever once the terminal
    /// hangs up (crossterm-rs/crossterm#793), so callers only hand control to
    /// it when this reports `Ready`; a hangup surfaces here as `HungUp`.
    #[cfg(unix)]
    pub fn wait_input(&self, timeout: std::time::Duration) -> io::Result<Input> {
        use std::os::fd::AsRawFd;
        let mut pfd = libc::pollfd {
            fd: self.tty.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        let ms = timeout.as_millis().min(i32::MAX as u128) as i32;
        // SAFETY: `pfd` is a single valid, writable pollfd.
        let n = unsafe { libc::poll(&raw mut pfd, 1, ms) };
        if n < 0 {
            let e = io::Error::last_os_error();
            // A signal (e.g. SIGWINCH on resize) cut the wait short.
            return if e.kind() == io::ErrorKind::Interrupted {
                Ok(Input::None)
            } else {
                Err(e)
            };
        }
        Ok(if pfd.revents & (libc::POLLHUP | libc::POLLERR | libc::POLLNVAL) != 0 {
            Input::HungUp
        } else if pfd.revents & libc::POLLIN != 0 {
            Input::Ready
        } else {
            Input::None
        })
    }

    #[cfg(not(unix))]
    pub fn wait_input(&self, timeout: std::time::Duration) -> io::Result<Input> {
        Ok(if crossterm::event::poll(timeout)? {
            Input::Ready
        } else {
            Input::None
        })
    }

    pub fn bell(&mut self) -> io::Result<()> {
        self.out.write_all(b"\x07")?;
        self.out.flush()
    }

    pub fn present(&mut self, f: &Frame) -> io::Result<()> {
        self.buf.clear();
        self.presented = diff(&mut self.buf, &mut self.shown, f, self.bce)?;
        // Nothing changed: write nothing, so the terminal isn't woken for an empty frame.
        if self.buf.is_empty() {
            return Ok(());
        }
        queue!(self.out, terminal::BeginSynchronizedUpdate)?;
        self.out.write_all(&self.buf)?;
        queue!(self.out, terminal::EndSynchronizedUpdate)?;
        self.out.flush()
    }
}

/// Writes what changed from `shown` (what the terminal shows) to `f`, and
/// updates `shown`. `bce`: clearing paints the current background.
fn diff(out: &mut impl Write, shown: &mut Option<Frame>, f: &Frame, bce: bool) -> io::Result<Presented> {
    // A background change means every cell has to be rewritten.
    let full = shown.as_ref().is_none_or(|s| s.w != f.w || s.h != f.h || s.bg != f.bg);
    if full {
        queue!(out, ResetColor)?;
        if let Some(Rgb(r, g, b)) = f.bg {
            queue!(out, SetBackgroundColor(Color::Rgb { r, g, b }))?;
        }
        // Without a fill, or with `bce`, clearing paints the blank cells.
        if f.bg.is_none() || bce {
            queue!(out, terminal::Clear(terminal::ClearType::All))?;
        }
        match shown {
            Some(s) => s.clone_from(f),
            None => *shown = Some(f.clone()),
        }
    }
    let shown = shown.as_mut().expect("set above when missing");
    let mut cells = 0;
    let mut cur: Option<(i32, i32)> = None;
    let mut color: Option<Rgb> = None;
    // The background in effect: `Some(None)` is the terminal's default.
    let mut back: Option<Option<Rgb>> = full.then_some(f.bg);
    for y in 0..f.h {
        let row = (y * f.w) as usize;
        for x in 0..f.w {
            let i = row + x as usize;
            let new = f.cells[i];
            if full {
                // Blank cells were cleared already unless there's a fill to paint.
                if new.is_none() && (f.bg.is_none() || bce) {
                    continue;
                }
            } else {
                if unchanged(new, shown.cells[i]) {
                    continue;
                }
                shown.cells[i] = new;
            }
            // The wide char to the left already covers this cell.
            if new.is_some_and(|c| c.ch == WIDE_TAIL) {
                continue;
            }
            cells += 1;
            let want = new.and_then(|c| c.bg).or(f.bg);
            if cur != Some((x, y)) {
                // A short gap on the same row: printing the unchanged cells
                // again can be cheaper than a cursor move.
                match cur {
                    Some((cx, cy)) if cy == y && cx < x => {
                        let gap = &f.cells[row + cx as usize..i];
                        if reprint_is_cheaper(gap, f.bg, back, color) {
                            for c in gap {
                                write!(out, "{}", c.map_or(' ', |c| c.ch))?;
                            }
                            shown.cells[row + cx as usize..i].copy_from_slice(gap);
                        } else {
                            queue!(out, cursor::MoveRight((x - cx) as u16))?;
                        }
                    }
                    _ => queue!(out, cursor::MoveTo(x as u16, y as u16))?,
                }
            }
            // The rest of the row is blank: erase it (`ESC [ K`) instead
            // of printing spaces, when erasing paints what blanks show.
            let rest = &f.cells[i..row + f.w as usize];
            if (f.bg.is_none() || bce)
                && rest.iter().all(Option::is_none)
                && rest.iter().zip(&shown.cells[i..]).any(|(&a, &b)| !unchanged(a, b))
            {
                if back != Some(f.bg) {
                    sgr(out, None, Some(f.bg))?;
                    back = Some(f.bg);
                }
                queue!(out, terminal::Clear(terminal::ClearType::UntilNewLine))?;
                shown.cells[i..row + f.w as usize].fill(None);
                cur = Some((x, y));
                break;
            }
            // Both colors in one SGR sequence when both change.
            let fg = new.map(|c| c.fg).filter(|&fg| color != Some(fg));
            let bg = (back != Some(want)).then_some(want);
            if fg.is_some() || bg.is_some() {
                sgr(out, fg, bg)?;
                color = fg.or(color);
                back = bg.or(back);
            }
            let mut advance = 1;
            match new {
                Some(c) => {
                    queue!(out, Print(c.ch))?;
                    advance = c.ch.width().unwrap_or(1) as i32;
                }
                None => queue!(out, Print(' '))?,
            }
            cur = Some((x + advance, y));
        }
    }
    Ok(Presented { cells, full })
}

/// Whether printing the unchanged cells of a gap on the row again costs fewer
/// bytes than moving past them (`ESC [ n C`); only cells that print as they
/// are with the colors already set (`back`, `color`) qualify.
#[allow(
    clippy::option_option,
    reason = "`Some(None)` is the terminal's default background, `None` unknown"
)]
fn reprint_is_cheaper(
    gap: &[Option<Cell>],
    frame_bg: Option<Rgb>,
    back: Option<Option<Rgb>>,
    color: Option<Rgb>,
) -> bool {
    let fits = gap.iter().all(|c| {
        back == Some(c.and_then(|c| c.bg).or(frame_bg))
            && c.is_none_or(|c| (c.ch == ' ' || Some(c.fg) == color) && c.ch.width() == Some(1))
    });
    let bytes: usize = gap.iter().map(|c| c.map_or(1, |c| c.ch.len_utf8())).sum();
    fits && bytes < 3 + digits(gap.len() as i32)
}

/// `ESC [ 38;2;r;g;b ; 48;2;r;g;b m`, with either part left out; a `Some(None)`
/// background is the terminal's default (`49`).
#[allow(clippy::option_option, reason = "`Some(None)` is the terminal's default background")]
fn sgr(out: &mut impl Write, fg: Option<Rgb>, bg: Option<Option<Rgb>>) -> io::Result<()> {
    out.write_all(b"\x1b[")?;
    if let Some(Rgb(r, g, b)) = fg {
        write!(out, "38;2;{r};{g};{b}")?;
        if bg.is_some() {
            out.write_all(b";")?;
        }
    }
    match bg {
        Some(Some(Rgb(r, g, b))) => write!(out, "48;2;{r};{g};{b}")?,
        Some(None) => out.write_all(b"49")?,
        None => {}
    }
    out.write_all(b"m")
}

/// Decimal digits of a positive number, for escape sequence lengths.
fn digits(n: i32) -> usize {
    n.to_string().len()
}

fn unchanged(new: Option<Cell>, shown: Option<Cell>) -> bool {
    match (new, shown) {
        (None, None) => true,
        (Some(a), Some(b)) => a.looks_like(b),
        _ => false,
    }
}

/// Whether the terminal's terminfo entry has `bce` (background color erase).
/// `tput` exits 0 only when it does; no `tput`, or an unknown or older
/// terminal (`screen`), means no, which keeps painting every blank cell.
fn back_color_erase() -> bool {
    use std::process::{Command, Stdio};
    Command::new("tput")
        .arg("bce")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Ask the terminal for its background color (OSC 11). A device-attributes
/// query (DA1) follows it: every terminal answers that, so we know when to
/// stop waiting even if OSC 11 isn't supported. Both replies are read here,
/// before crossterm's reader starts, so they never turn into key presses.
#[cfg(unix)]
fn query_background(out: &mut impl Write, tty: &std::fs::File) -> Option<Rgb> {
    use std::io::Read;
    use std::os::fd::AsRawFd;
    const WAIT: std::time::Duration = std::time::Duration::from_millis(300);
    out.write_all(b"\x1b]11;?\x1b\\\x1b[c").ok()?;
    out.flush().ok()?;
    let deadline = std::time::Instant::now() + WAIT;
    let mut reply = Vec::new();
    let mut buf = [0u8; 256];
    while !has_da1_reply(&reply) {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        if left.is_zero() {
            break;
        }
        let mut pfd = libc::pollfd {
            fd: tty.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: `pfd` is a single valid, writable pollfd.
        let n = unsafe { libc::poll(&raw mut pfd, 1, left.as_millis().max(1) as i32) };
        if n <= 0 || pfd.revents & libc::POLLIN == 0 {
            break;
        }
        match (&*tty).read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => reply.extend_from_slice(&buf[..n]),
        }
    }
    parse_osc11(&reply)
}

/// Whether `bytes` contain a DA1 reply (`ESC [ ? ... c`).
fn has_da1_reply(bytes: &[u8]) -> bool {
    bytes.windows(3).enumerate().any(|(i, w)| {
        w == b"\x1b[?"
            && bytes[i + 3..]
                .iter()
                .find(|b| !(b.is_ascii_digit() || **b == b';'))
                .is_some_and(|&b| b == b'c')
    })
}

/// The color in an OSC 11 reply: `ESC ] 11 ; rgb:RRRR/GGGG/BBBB` (1 to 4 hex
/// digits per channel; `rgba:` too), ended by BEL or ST.
fn parse_osc11(bytes: &[u8]) -> Option<Rgb> {
    let text = String::from_utf8_lossy(bytes);
    let start = text.find("\x1b]11;")? + 5;
    let rest = &text[start..];
    let end = rest.find(['\x07', '\x1b']).unwrap_or(rest.len());
    let spec = &rest[..end];
    let spec = spec.strip_prefix("rgb:").or_else(|| spec.strip_prefix("rgba:"))?;
    let mut channels = spec.split('/').map(|h| {
        if h.is_empty() || h.len() > 4 {
            return None;
        }
        let v = u32::from_str_radix(h, 16).ok()?;
        let max = (1u32 << (4 * h.len())) - 1;
        Some(((v * 255 + max / 2) / max) as u8)
    });
    Some(Rgb(channels.next()??, channels.next()??, channels.next()??))
}

pub fn restore() {
    let mut out = io::stdout();
    let _ = execute!(out, ResetColor, cursor::Show, terminal::LeaveAlternateScreen);
    let _ = terminal::disable_raw_mode();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Plays `diff`'s output on a minimal terminal (bce, the sequences it
    /// writes) and checks the screen shows each of a run of random frames.
    #[test]
    #[allow(clippy::too_many_lines, reason = "a terminal emulator and a frame generator")]
    fn diff_output_draws_the_frame() {
        type Screen = Vec<(char, Rgb, Option<Rgb>)>;
        fn play(screen: &mut Screen, w: usize, bytes: &[u8]) {
            let text = std::str::from_utf8(bytes).unwrap();
            let (mut cx, mut cy, mut fg, mut bg) = (0, 0, Rgb(0, 0, 0), None);
            let mut it = text.chars().peekable();
            while let Some(ch) = it.next() {
                if ch != '\x1b' {
                    screen[cy * w + cx] = (ch, fg, bg);
                    cx += 1;
                    continue;
                }
                assert_eq!(it.next(), Some('['));
                let mut params = String::new();
                let end = loop {
                    let c = it.next().unwrap();
                    if c.is_ascii_alphabetic() {
                        break c;
                    }
                    params.push(c);
                };
                let n: Vec<u16> = params.split(';').filter_map(|p| p.parse().ok()).collect();
                match end {
                    'H' => (cy, cx) = (usize::from(n[0]) - 1, usize::from(n[1]) - 1),
                    'C' => cx += usize::from(n[0]),
                    'J' => screen.fill((' ', fg, bg)),
                    'K' => screen[cy * w + cx..(cy + 1) * w].fill((' ', fg, bg)),
                    'h' | 'l' => {} // synchronized update
                    'm' => {
                        let mut k = 0;
                        while k < n.len() {
                            let rgb = || Rgb(n[k + 2] as u8, n[k + 3] as u8, n[k + 4] as u8);
                            match n[k] {
                                0 => (fg, bg, k) = (Rgb(0, 0, 0), None, k + 1),
                                38 => (fg, k) = (rgb(), k + 5),
                                48 => (bg, k) = (Some(rgb()), k + 5),
                                49 => (bg, k) = (None, k + 1),
                                p => panic!("unexpected SGR {p}"),
                            }
                        }
                    }
                    e => panic!("unexpected escape {e}"),
                }
            }
        }
        let mut seed = 7u64;
        let mut rnd = |n: u64| {
            seed = seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (seed >> 33) % n
        };
        let (w, h) = (16, 6);
        for bce in [false, true] {
            let mut f = Frame::new(w, h);
            let mut shown = None;
            let mut screen: Screen = vec![(' ', Rgb(0, 0, 0), None); usize::from(w * h)];
            for step in 0..400 {
                if rnd(40) == 0 {
                    f.bg = [None, Some(Rgb(20, 20, 30))][rnd(2) as usize];
                }
                for _ in 0..rnd(30) {
                    let (x, y) = (rnd(u64::from(w)) as i32, rnd(u64::from(h)) as i32);
                    let pal = [Rgb(200, 10, 10), Rgb(10, 200, 10), Rgb(201, 11, 9)];
                    let cell = (rnd(3) > 0).then(|| Cell {
                        ch: ['a', 'b', ' ', '▀'][rnd(4) as usize],
                        fg: pal[rnd(3) as usize],
                        bg: [None, Some(Rgb(5, 5, 90))][rnd(2) as usize],
                    });
                    f.set(x, y, cell);
                }
                if rnd(5) == 0 {
                    // Blank the end of a row.
                    let (x, y) = (rnd(u64::from(w)) as i32, rnd(u64::from(h)) as i32);
                    (x..i32::from(w)).for_each(|x| f.set(x, y, None));
                }
                let mut out = Vec::new();
                diff(&mut out, &mut shown, &f, bce).unwrap();
                play(&mut screen, usize::from(w), &out);
                for (i, (&want, &(ch, fg, bg))) in f.cells.iter().zip(&screen).enumerate() {
                    let near = |a: Rgb, b: Rgb| {
                        Cell {
                            ch: ' ',
                            fg: a,
                            bg: None,
                        }
                        .looks_like(Cell {
                            ch: ' ',
                            fg: b,
                            bg: None,
                        })
                    };
                    let bg_ok = |want: Option<Rgb>| match (bg, want) {
                        (None, None) => true,
                        (Some(a), Some(b)) => near(a, b),
                        _ => false,
                    };
                    let ok = match want {
                        None => ch == ' ' && bg_ok(f.bg),
                        Some(c) => ch == c.ch && (c.ch == ' ' || near(fg, c.fg)) && bg_ok(c.bg.or(f.bg)),
                    };
                    assert!(
                        ok,
                        "bce {bce}, step {step}, cell {i}: want {want:?}, screen {:?}",
                        (ch, fg, bg)
                    );
                }
            }
        }
    }

    #[test]
    fn parses_background_replies() {
        let da1 = "\x1b[?62;22c";
        assert_eq!(
            parse_osc11(format!("\x1b]11;rgb:1d1d/1616/2626\x1b\\{da1}").as_bytes()),
            Some(Rgb(0x1d, 0x16, 0x26))
        );
        assert_eq!(parse_osc11(b"\x1b]11;rgb:0c/0c/0c\x07"), Some(Rgb(12, 12, 12)));
        assert_eq!(parse_osc11(b"\x1b]11;rgb:f/8/0\x07"), Some(Rgb(255, 136, 0)));
        assert_eq!(
            parse_osc11(b"\x1b]11;rgba:ffff/0000/0000/ffff\x07"),
            Some(Rgb(255, 0, 0))
        );
        assert_eq!(parse_osc11(da1.as_bytes()), None);
        assert_eq!(parse_osc11(b"\x1b]11;rgb:zz/00/00\x07"), None);
        assert!(has_da1_reply(da1.as_bytes()));
        assert!(has_da1_reply(b"junk\x1b]11;rgb:0/0/0\x07\x1b[?1;2c"));
        assert!(!has_da1_reply(b"\x1b[?62;22"));
        assert!(!has_da1_reply(b"\x1b]11;rgb:0/0/0\x07"));
    }
}
