//! Off-screen frame buffer plus a diffing terminal presenter.

use std::io::{self, BufWriter, Stdout, Write};

use crossterm::{
    cursor, execute, queue,
    style::{Color, Print, ResetColor, SetBackgroundColor, SetForegroundColor},
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

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Cell {
    pub ch: char,
    pub fg: Rgb,
}

impl Cell {
    fn looks_like(self, other: Cell) -> bool {
        let near = |a: u8, b: u8| a.abs_diff(b) <= COLOR_TOLERANCE;
        self.ch == other.ch && near(self.fg.0, other.fg.0) && near(self.fg.1, other.fg.1) && near(self.fg.2, other.fg.2)
    }
}

/// Display width of a string in terminal columns.
pub fn str_width(s: &str) -> i32 {
    s.chars().map(|c| c.width().unwrap_or(0) as i32).sum()
}

#[derive(Clone)]
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

    fn set(&mut self, x: i32, y: i32, cell: Option<Cell>) {
        if x >= 0 && y >= 0 && x < self.w && y < self.h {
            self.cells[(y * self.w + x) as usize] = cell;
        }
    }

    /// Put one char. Double-width chars also claim the cell to their right;
    /// one that would hang off the right edge is dropped.
    pub fn put(&mut self, x: i32, y: i32, ch: char, fg: Rgb) {
        match ch {
            WIDE_TAIL => {}
            ' ' => self.set(x, y, None),
            _ if ch.width() == Some(2) => {
                if x + 1 < self.w {
                    self.set(x, y, Some(Cell { ch, fg }));
                    self.set(x + 1, y, Some(Cell { ch: WIDE_TAIL, fg }));
                }
            }
            _ => self.set(x, y, Some(Cell { ch, fg })),
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

pub struct Term {
    out: BufWriter<Stdout>,
    /// Our own handle on the terminal, used to wait for input and to notice
    /// hangups without going through crossterm's reader (see `wait_input`).
    #[cfg(unix)]
    tty: std::fs::File,
    /// What the terminal currently shows (as last written by us).
    shown: Option<Frame>,
}

impl Term {
    pub fn enter() -> io::Result<Self> {
        let mut out = BufWriter::with_capacity(1 << 16, io::stdout());
        terminal::enable_raw_mode()?;
        execute!(
            out,
            terminal::EnterAlternateScreen,
            cursor::Hide,
            terminal::Clear(terminal::ClearType::All)
        )?;
        Ok(Term {
            out,
            shown: None,
            #[cfg(unix)]
            tty: std::fs::File::open("/dev/tty")?,
        })
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
        // A background change means every cell has to be rewritten.
        let full = self
            .shown
            .as_ref()
            .is_none_or(|s| s.w != f.w || s.h != f.h || s.bg != f.bg);
        queue!(self.out, terminal::BeginSynchronizedUpdate)?;
        if full {
            queue!(self.out, ResetColor)?;
            match f.bg {
                Some(Rgb(r, g, b)) => queue!(self.out, SetBackgroundColor(Color::Rgb { r, g, b }))?,
                // Clearing with the default background is enough when there's no fill.
                None => queue!(self.out, terminal::Clear(terminal::ClearType::All))?,
            }
            match &mut self.shown {
                Some(s) => s.clone_from(f),
                None => self.shown = Some(f.clone()),
            }
        }
        let shown = self.shown.as_mut().expect("set above when missing");
        let mut cur: Option<(i32, i32)> = None;
        let mut color: Option<Rgb> = None;
        for y in 0..f.h {
            for x in 0..f.w {
                let i = (y * f.w + x) as usize;
                let new = f.cells[i];
                if full {
                    // Blank cells were cleared already unless there's a fill to paint.
                    if new.is_none() && f.bg.is_none() {
                        continue;
                    }
                } else {
                    let same = match (new, shown.cells[i]) {
                        (None, None) => true,
                        (Some(a), Some(b)) => a.looks_like(b),
                        _ => false,
                    };
                    if same {
                        continue;
                    }
                    shown.cells[i] = new;
                }
                // The wide char to the left already covers this cell.
                if new.is_some_and(|c| c.ch == WIDE_TAIL) {
                    continue;
                }
                if cur != Some((x, y)) {
                    queue!(self.out, cursor::MoveTo(x as u16, y as u16))?;
                }
                let mut advance = 1;
                match new {
                    Some(c) => {
                        if color != Some(c.fg) {
                            let Rgb(r, g, b) = c.fg;
                            queue!(self.out, SetForegroundColor(Color::Rgb { r, g, b }))?;
                            color = Some(c.fg);
                        }
                        queue!(self.out, Print(c.ch))?;
                        advance = c.ch.width().unwrap_or(1) as i32;
                    }
                    None => queue!(self.out, Print(' '))?,
                }
                cur = Some((x + advance, y));
            }
        }
        queue!(self.out, terminal::EndSynchronizedUpdate)?;
        self.out.flush()
    }
}

pub fn restore() {
    let mut out = io::stdout();
    let _ = execute!(out, ResetColor, cursor::Show, terminal::LeaveAlternateScreen);
    let _ = terminal::disable_raw_mode();
}
