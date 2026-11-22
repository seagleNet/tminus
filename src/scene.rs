//! The clock face: target glyphs, their placement, and their "settled" look.

use crate::{
    term::{Cell, Frame},
    theme::{Rgb, Theme},
    util::{glyph, hash},
};

/// How long a digit scrambles after it changes.
pub const TICK_FX: f64 = 0.28;

/// `sin(now * rate)` for an absolute time, keeping precision on long runs.
pub fn wave(now: f64, rate: f64) -> f32 {
    (now * rate).rem_euclid(std::f64::consts::TAU).sin() as f32
}

/// The current text block plus per-cell change timestamps.
pub struct Target {
    pub w: usize,
    pub h: usize,
    pub chars: Vec<char>,
    pub changed: Vec<f64>,
    /// First row of the title (rows from here down are title, not clock).
    pub title_row: usize,
    /// The tubes, with the nixie font: the clock's, and the title's.
    pub nixie: Option<crate::nixie::Layout>,
    pub nixie_title: Option<crate::nixie::Layout>,
    /// The nixie font's look, kept when the text falls back to another font
    /// (too long for tubes) so it keeps the tubes' colors.
    pub nixie_look: Option<crate::nixie::Style>,
}

impl Target {
    pub fn empty() -> Self {
        Target {
            w: 0,
            h: 0,
            chars: vec![],
            changed: vec![],
            title_row: usize::MAX,
            nixie: None,
            nixie_title: None,
            nixie_look: None,
        }
    }

    /// Swap in new glyphs. Returns true if the dimensions changed (relayout).
    ///
    /// Only cells drawn from input characters before index `fx_until` get the
    /// tick scramble; later ones (fast-changing fractions) just switch.
    pub fn update(&mut self, mut g: crate::font::Glyphs, now: f64, fx_until: usize) -> bool {
        if let (Some(new), Some(old)) = (&mut g.nixie, &self.nixie) {
            new.follow(old, now, fx_until);
        }
        if let Some(n) = &g.nixie {
            self.nixie_look = Some(n.style);
        }
        self.nixie = g.nixie;
        self.nixie_title = g.nixie_title;
        if g.w != self.w || g.h != self.h {
            self.w = g.w;
            self.h = g.h;
            self.changed = vec![f64::NEG_INFINITY; g.chars.len()];
            self.chars = g.chars;
            return true;
        }
        for (i, ch) in g.chars.into_iter().enumerate() {
            if self.chars[i] != ch {
                self.chars[i] = ch;
                if g.owner[i] < fx_until {
                    self.changed[i] = now;
                }
            }
        }
        false
    }

    /// True while any cell is still mid tick-scramble.
    pub fn ticking(&self, now: f64) -> bool {
        self.changed.iter().any(|&t| now - t < TICK_FX)
    }
}

#[allow(clippy::struct_excessive_bools, reason = "independent states of the frame")]
pub struct Scene<'a> {
    pub t: &'a Target,
    pub ox: i32,
    pub oy: i32,
    pub sw: i32,
    pub sh: i32,
    pub theme: &'a Theme,
    /// Seconds since startup. `f64` so slow animations stay smooth on long runs.
    pub now: f64,
    pub paused: bool,
    pub done: bool,
    pub tick_fx: bool,
    /// 0..1: how deep into the final-seconds alert window we are.
    pub urgency: f32,
    /// What the nixie tubes' pixels blend over: the frame's fill, else the
    /// terminal's background.
    pub rest: Rgb,
    /// 0..1: how far the nixie tubes are back at rest after an effect, from
    /// how it ended (`nixie::crossfade`).
    pub rest_k: f32,
    /// Whether the nixie separator is dark now, with --blink (even seconds).
    pub blink_dark: bool,
    /// 0..1: the alert pulse on each tick in the final seconds and after zero,
    /// which nixie tubes show as a flare (other fonts flash the background).
    pub pulse: f32,
}

impl Scene<'_> {
    pub fn tw(&self) -> usize {
        self.t.w
    }
    pub fn th(&self) -> usize {
        self.t.h
    }
    pub fn len(&self) -> usize {
        self.t.chars.len()
    }
    pub fn ch(&self, i: usize) -> char {
        self.t.chars[i]
    }
    pub fn rc(&self, i: usize) -> (usize, usize) {
        (i / self.t.w, i % self.t.w)
    }
    /// Screen position of cell `i`.
    pub fn pos(&self, i: usize) -> (i32, i32) {
        let (r, c) = self.rc(i);
        (self.ox + c as i32, self.oy + r as i32)
    }

    /// Gradient color for cell `i`, drifting slowly over time.
    pub fn base_color(&self, i: usize) -> Rgb {
        if self.t.nixie_look.is_some() {
            // The nixie font's colors, also when it falls back to another font.
            return self.wire_color(i);
        }
        self.gradient_color(i, true)
    }

    /// The theme gradient at cell `i`, slowly drifting over time with `drift`.
    pub fn gradient_color(&self, i: usize, drift: bool) -> Rgb {
        let (r, c) = self.rc(i);
        let mut u = c as f32 / self.t.w.max(1) as f32 * 0.55 + r as f32 / self.t.h.max(1) as f32 * 0.15;
        if r >= self.t.title_row {
            // Title takes the other half of the gradient to set it apart.
            u += 0.5;
        }
        if drift {
            u += (self.now * 0.025).fract() as f32;
        }
        self.tint(self.theme.gradient_at(u))
    }

    /// `col` as the timer's state shows it: drawn to the alert color in the
    /// final seconds, dimmed while paused, flashing once done.
    pub fn tint(&self, mut col: Rgb) -> Rgb {
        if self.urgency > 0.0 {
            col = col.lerp(self.theme.alert, 0.15 + self.urgency * 0.45);
        }
        if self.paused {
            // Dimmed and gently pulsing, but still easy to read.
            col = col.scale(0.7 + 0.1 * wave(self.now, 2.5));
        } else if self.done {
            let k = 0.5 + 0.5 * wave(self.now, 5.0);
            col = if self.t.nixie_look.is_some() {
                // Tubes flare in their own color rather than fade to the flash color.
                col.scale(0.75 + 0.5 * k)
            } else {
                col.lerp(self.theme.flash, k * 0.6)
            };
        }
        col
    }

    /// The fully-resolved look of cell `i` (includes the per-tick scramble).
    pub fn settled(&self, i: usize) -> Option<Cell> {
        let ch = self.ch(i);
        let age = self.now - self.t.changed[i];
        if self.t.nixie.is_some() {
            // Nixie wires, fading over (see `nixie::draw`) rather than
            // scrambling, in the color they're lit in (so effects end on it),
            // and a plain title under them in the tubes' color.
            let wire = self.rc(i).0 < self.t.title_row || self.t.nixie_title.is_some();
            return (ch != ' ').then(|| Cell {
                ch,
                fg: if wire {
                    self.lit_wire_color(i)
                } else {
                    self.wire_color(i)
                },
                bg: None,
            });
        }
        if self.tick_fx && age < TICK_FX {
            let p = (age / TICK_FX) as f32;
            let g = glyph(hash(i as u64 ^ ((self.now * 24.0) as u64) << 32));
            return if ch == ' ' {
                // Dissolve out.
                (p < 0.7).then(|| Cell {
                    ch: g,
                    fg: self.base_color(i).lerp(self.theme.dim, p).scale(1.0 - p * 0.6),
                    bg: None,
                })
            } else {
                Some(Cell {
                    ch: if p > 0.75 { ch } else { g },
                    fg: self.theme.flash.lerp(self.base_color(i), p),
                    bg: None,
                })
            };
        }
        (ch != ' ').then(|| Cell {
            ch,
            fg: self.base_color(i),
            bg: None,
        })
    }

    /// Draw cell `i` at its home position, blending in from `from` over `p` in [0,1].
    pub fn draw_from(&self, f: &mut Frame, i: usize, from: Rgb, p: f32) {
        if let Some(c) = self.settled(i) {
            let (x, y) = self.pos(i);
            f.put(x, y, c.ch, from.lerp(c.fg, p));
        }
    }

    pub fn draw_settled(&self, f: &mut Frame) {
        match &self.t.nixie {
            Some(nx) => {
                crate::nixie::draw(self, nx, f, true);
                self.draw_title(f);
            }
            None => self.draw_cells(f, 0),
        }
    }

    /// The title under the clock, at rest.
    pub fn draw_title(&self, f: &mut Frame) {
        match &self.t.nixie_title {
            Some(nx) => crate::nixie::draw(self, nx, f, true),
            None => self.draw_cells(f, self.t.title_row.saturating_mul(self.t.w).min(self.len())),
        }
    }

    /// The settled cells from index `from` on (for nixie tubes: their wires).
    pub fn draw_cells(&self, f: &mut Frame, from: usize) {
        for i in from..self.len() {
            if let Some(c) = self.settled(i) {
                let (x, y) = self.pos(i);
                f.put(x, y, c.ch, c.fg);
            }
        }
    }
}
