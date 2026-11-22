//! Effects where characters travel into place.
//!
//! Designs from `TerminalTextEffects` (MIT, © 2023 `ChrisBuilds`) and its Rust
//! port `ttfx` (MIT, © 2026 37signals) — see LICENSE. Timelines are shortened
//! so the clock stays readable.

use rand::{
    Rng, RngExt,
    seq::{IndexedRandom, SliceRandom},
};

use super::{Effect, WHITE, hexes, max, reveal, row_gap};
use crate::{
    scene::Scene,
    term::Frame,
    theme::Rgb,
    util::{
        ease_in_cubic, ease_in_expo, ease_in_out_back, ease_in_out_expo, ease_in_out_quad, ease_in_out_quart,
        ease_in_out_sine, ease_in_quad, ease_in_quart, ease_out_bounce, ease_out_circ, ease_out_expo, ease_out_quint,
        ease_out_sine, hash,
    },
};

pub(super) type Pos = (f32, f32);

/// Indices of the cells that currently show something.
pub(super) fn ink(sc: &Scene) -> Vec<usize> {
    (0..sc.len()).filter(|&i| sc.ch(i) != ' ').collect()
}

fn target(sc: &Scene, i: usize) -> Pos {
    let (x, y) = sc.pos(i);
    (x as f32, y as f32)
}

fn center(sc: &Scene) -> Pos {
    (sc.ox as f32 + sc.tw() as f32 / 2.0, sc.oy as f32 + sc.th() as f32 / 2.0)
}

fn lerp(a: Pos, b: Pos, p: f32) -> Pos {
    (a.0 + (b.0 - a.0) * p, a.1 + (b.1 - a.1) * p)
}

fn put(f: &mut Frame, p: Pos, ch: char, col: Rgb) {
    f.put(p.0.round() as i32, p.1.round() as i32, ch, col);
}

pub(super) fn random_screen_pos(sc: &Scene, rng: &mut impl Rng) -> Pos {
    (
        rng.random_range(0.0..sc.sw.max(1) as f32),
        rng.random_range(0.0..sc.sh.max(1) as f32),
    )
}

/// A random point on the screen's edge.
fn random_edge_pos(sc: &Scene, rng: &mut impl Rng) -> Pos {
    let (w, h) = ((sc.sw - 1).max(0) as f32, (sc.sh - 1).max(0) as f32);
    match rng.random_range(0..4) {
        0 => (rng.random_range(0.0..=w), 0.0),
        1 => (rng.random_range(0.0..=w), h),
        2 => (0.0, rng.random_range(0.0..=h)),
        _ => (w, rng.random_range(0.0..=h)),
    }
}

// --- travel: the shared "fly from somewhere into place" effect ------------

/// Each cell flies from `start` to its place along `ease`, colored `color`
/// on the way (optionally drawn as a stand-in `glyph`), then settles.
struct Travel {
    start: Vec<Pos>,
    delay: Vec<f32>,
    dur: Vec<f32>,
    ease: fn(f32) -> f32,
    color: Vec<Rgb>,
    glyphs: Option<Vec<char>>,
    fade: f32,
    /// `Some((secs, after_landing))`: instead of blending on the way and a
    /// flash after, fade `color` into the cell's own over `secs`, counted
    /// from launch or from landing (tte's gradient scenes).
    tint: Option<(f32, bool)>,
    end: f32,
}

impl Travel {
    fn new(start: Vec<Pos>, delay: Vec<f32>, dur: Vec<f32>, ease: fn(f32) -> f32, color: Vec<Rgb>) -> Self {
        let end = delay.iter().zip(&dur).map(|(d, u)| d + u).fold(0.0, f32::max);
        Travel {
            start,
            delay,
            dur,
            ease,
            color,
            glyphs: None,
            fade: 0.4,
            tint: None,
            end,
        }
    }

    /// Fade `color` into each cell's own over `secs` (see `tint`; 0: over
    /// each cell's own flight, tte's distance-synced scenes).
    fn tinted(mut self, secs: f32, after_landing: bool) -> Self {
        self.tint = Some((secs, after_landing));
        self.end = (0..self.dur.len())
            .map(|i| {
                let done = if after_landing {
                    self.dur[i] + secs
                } else {
                    self.dur[i].max(secs)
                };
                self.delay[i] + done
            })
            .fold(0.0, f32::max);
        self.fade = 0.0;
        self
    }
}

impl Effect for Travel {
    fn draw(&mut self, sc: &Scene, t: f32, f: &mut Frame) -> bool {
        for i in 0..sc.len() {
            let Some(cell) = sc.settled(i) else { continue };
            let lt = t - self.delay[i];
            if lt < 0.0 {
                continue;
            }
            let p = lt / self.dur[i];
            if let Some((secs, after_landing)) = self.tint {
                let secs = if secs > 0.0 { secs } else { self.dur[i] };
                let k = if after_landing { lt - self.dur[i] } else { lt } / secs;
                let (pos, ch) = if p < 1.0 {
                    let pos = lerp(self.start[i], target(sc, i), (self.ease)(p));
                    (pos, self.glyphs.as_ref().map_or(cell.ch, |g| g[i]))
                } else {
                    (target(sc, i), cell.ch)
                };
                put(f, pos, ch, self.color[i].lerp(cell.fg, k));
            } else if p < 1.0 {
                let pos = lerp(self.start[i], target(sc, i), (self.ease)(p));
                let ch = self.glyphs.as_ref().map_or(cell.ch, |g| g[i]);
                put(f, pos, ch, self.color[i].lerp(cell.fg, p * p));
            } else {
                reveal(sc, f, i, lt - self.dur[i], self.fade);
            }
        }
        t > self.end + self.fade
    }
}

/// How long each cell takes from `start` to its place at `speed` cells a
/// second (rows count double, as in tte), sped up so none takes over `most`.
fn flight_times(sc: &Scene, start: &[Pos], speed: &[f32], most: f32) -> Vec<f32> {
    let home: Vec<Pos> = (0..sc.len()).map(|i| target(sc, i)).collect();
    flight_times_between(start, &home, speed, most)
}

/// `flight_times` between any two sets of points.
fn flight_times_between(from: &[Pos], to: &[Pos], speed: &[f32], most: f32) -> Vec<f32> {
    let secs: Vec<f32> = from
        .iter()
        .zip(to)
        .zip(speed)
        .map(|((a, b), v)| (b.0 - a.0).hypot(2.0 * (b.1 - a.1)) / v)
        .collect();
    let k = (secs.iter().copied().fold(0.0, f32::max) / most).max(1.0);
    secs.iter().map(|s| (s / k).max(0.1)).collect()
}

/// scattered: characters start scattered across the screen and swing into
/// place at one speed, shifting from the gradient's first color to their own.
pub fn scattered(sc: &Scene, rng: &mut impl Rng) -> Box<dyn Effect> {
    let n = sc.len();
    let start: Vec<Pos> = (0..n).map(|_| random_screen_pos(sc, rng)).collect();
    // tte: 0.5 cells a frame, eased in-out-back. (tte holds the scatter
    // 25 frames first, which looks frozen on a clock.)
    let dur = flight_times(sc, &start, &vec![30.0; n], 1.4);
    let color = vec![sc.theme.gradient_at(0.0); n];
    Box::new(Travel::new(start, vec![0.0; n], dur, ease_in_out_back, color).tinted(0.0, false))
}

/// spray: characters are sprayed from the right edge in random order, each
/// at its own speed, fading from a random gradient color into its own.
pub fn spray(sc: &Scene, rng: &mut impl Rng) -> Box<dyn Effect> {
    let n = sc.len();
    let source = ((sc.sw - 1) as f32, (sc.sh / 2) as f32);
    let mut order: Vec<usize> = (0..n).collect();
    order.shuffle(rng);
    let mut delay = vec![0.0; n];
    for (rank, &i) in order.iter().enumerate() {
        delay[i] = rank as f32 / n.max(1) as f32 * 1.4;
    }
    // tte: 0.6-1.4 cells a frame, eased out-expo.
    let speed: Vec<f32> = (0..n).map(|_| rng.random_range(36.0..84.0)).collect();
    let dur = flight_times(sc, &vec![source; n], &speed, 1.1);
    let color = (0..n).map(|_| sc.theme.gradient_at(rng.random())).collect();
    Box::new(Travel::new(vec![source; n], delay, dur, ease_out_expo, color).tinted(1.0, false))
}

/// slice: the text is sliced down the middle; the left half slides in from
/// above the screen and the right half from below, already in its colors.
pub fn slice(sc: &Scene, _rng: &mut impl Rng) -> Box<dyn Effect> {
    let n = sc.len();
    let half = sc.tw() / 2;
    let start: Vec<Pos> = (0..n)
        .map(|i| (target(sc, i).0, if sc.rc(i).1 < half { -1.0 } else { sc.sh as f32 }))
        .collect();
    // tte: 0.25 cells a frame, eased in-out-expo, no color scenes.
    let dur = flight_times(sc, &start, &vec![15.0; n], 1.4);
    let mut fx = Travel::new(start, vec![0.0; n], dur, ease_in_out_expo, vec![WHITE; n]);
    // Drawn in its own colors all the way.
    fx = fx.tinted(f32::MIN_POSITIVE, false);
    Box::new(fx)
}

/// pour: rows are poured in from the top, bottom row first, each row back
/// and forth, every character fading from white into its color.
pub fn pour(sc: &Scene, rng: &mut impl Rng) -> Box<dyn Effect> {
    let n = sc.len();
    let (tw, th) = (sc.tw(), sc.th());
    let mut order = Vec::with_capacity(n);
    for (k, r) in (0..th).rev().enumerate() {
        let cols: Vec<usize> = if k % 2 == 0 {
            (0..tw).collect()
        } else {
            (0..tw).rev().collect()
        };
        order.extend(cols.into_iter().map(|c| r * tw + c).filter(|&i| sc.ch(i) != ' '));
    }
    let mut delay = vec![0.0; n];
    for (rank, &i) in order.iter().enumerate() {
        delay[i] = rank as f32 / order.len().max(1) as f32 * 1.5;
    }
    let start: Vec<Pos> = (0..n).map(|i| (target(sc, i).0, -1.0)).collect();
    // tte: 0.4-0.6 cells a frame, eased in-quad; white to its color in 1s.
    let speed: Vec<f32> = (0..n).map(|_| rng.random_range(24.0..36.0)).collect();
    let dur = flight_times(sc, &start, &speed, 0.8);
    Box::new(Travel::new(start, delay, dur, ease_in_quad, vec![WHITE; n]).tinted(1.0, false))
}

/// bouncyballs: from the bottom row up, characters drop as balls from
/// above the screen, bounce into place and fade into their colors.
pub fn bouncyballs(sc: &Scene, rng: &mut impl Rng) -> Box<dyn Effect> {
    let n = sc.len();
    let colors = const { hexes(["#d1f4a5", "#96e2a4", "#5acda9"]) };
    // tte drops each ball from up to half the screen's height above it.
    let start: Vec<Pos> = (0..n)
        .map(|i| (target(sc, i).0, -1.0 - rng.random_range(0.0..0.5) * sc.sh as f32))
        .collect();
    // Bottom row first, in random order within a row (tte: 2-6 balls every 5 frames).
    let mut order = ink(sc);
    order.shuffle(rng);
    order.sort_by_key(|&i| std::cmp::Reverse(sc.rc(i).0));
    let mut delay = vec![0.0; n];
    for (rank, &i) in order.iter().enumerate() {
        delay[i] = rank as f32 / order.len().max(1) as f32 * 1.2;
    }
    // tte: 0.45 cells a frame, eased out-bounce.
    let dur = flight_times(sc, &start, &vec![27.0; n], 1.2);
    let color = (0..n).map(|_| colors[rng.random_range(0..colors.len())]).collect();
    let mut fx = Travel::new(start, delay, dur, ease_out_bounce, color).tinted(0.6, true);
    fx.glyphs = Some(
        (0..n)
            .map(|_| ['*', 'o', 'O', '0', '.'][rng.random_range(0..5)])
            .collect(),
    );
    Box::new(fx)
}

// --- middleout ---------------------------------------------------------------

/// middleout: everything gathers in the middle, spreads into a row, then
/// expands vertically into place, fading from white into its colors.
pub struct MiddleOut {
    /// Per cell: how long its move along the row takes, and out of it.
    along: Vec<f32>,
    out: Vec<f32>,
}

pub fn middleout(sc: &Scene, _rng: &mut impl Rng) -> Box<dyn Effect> {
    // tte: 0.6 cells a frame for both moves, eased in-out-sine.
    let n = sc.len();
    let c = center(sc);
    let on_row: Vec<Pos> = (0..n).map(|i| (target(sc, i).0, c.1)).collect();
    let speed = vec![36.0; n];
    let along = flight_times_between(&vec![c; n], &on_row, &speed, 0.7);
    let out = flight_times(sc, &on_row, &speed, 0.7);
    Box::new(MiddleOut { along, out })
}

impl Effect for MiddleOut {
    fn draw(&mut self, sc: &Scene, t: f32, f: &mut Frame) -> bool {
        // tte: the full expansion starts once every character is on the row.
        const FADE: f32 = 1.0;
        let c = center(sc);
        let row_done = max(&self.along);
        for i in 0..sc.len() {
            let Some(cell) = sc.settled(i) else { continue };
            let (tx, ty) = target(sc, i);
            let (pos, col) = if t < row_done {
                (lerp(c, (tx, c.1), ease_in_out_sine(t / self.along[i])), WHITE)
            } else {
                let lt = t - row_done;
                (
                    lerp((tx, c.1), (tx, ty), ease_in_out_sine(lt / self.out[i])),
                    WHITE.lerp(cell.fg, lt / FADE),
                )
            };
            put(f, pos, cell.ch, col);
        }
        t > row_done + max(&self.out).max(FADE)
    }
}

// --- binarypath ----------------------------------------------------------------

/// binarypath: each character's binary code snakes in from off screen along
/// a zigzag path; on arrival it collapses into the character, dimmed, and a
/// final diagonal wipe brightens the text.
pub struct BinaryPath {
    /// Per cell: its path in unit steps, ending on its place.
    path: Vec<Vec<(i32, i32)>>,
    bits: Vec<Vec<char>>,
    start: Vec<f32>,
    /// Cells a second along the path.
    speed: f32,
    /// When the wipe starts, and diagonals a second.
    wipe_at: f32,
    wipe_rate: f32,
}

/// tte: the arrived character goes from white to half brightness (7 steps
/// of 3 frames, eased in-quad), and brightens again in the wipe.
const BINARY_COLLAPSE: f32 = 21.0 / 60.0;
const BINARY_BRIGHTEN: f32 = 0.4;

pub fn binarypath(sc: &Scene, rng: &mut impl Rng) -> Box<dyn Effect> {
    let n = sc.len();
    let mut path = vec![vec![]; n];
    let mut bits = vec![vec![]; n];
    let mut order = ink(sc);
    for &i in &order {
        path[i] = zigzag(sc, rng, i);
        bits[i] = format!("{:08b}", u32::from(sc.ch(i))).chars().collect();
    }
    // tte keeps 8% of the binary groups moving at once, at a cell a frame;
    // here they all set off within 1.2s and need at most 0.9s.
    order.shuffle(rng);
    let mut start = vec![0.0; n];
    for (rank, &i) in order.iter().enumerate() {
        start[i] = rank as f32 / order.len().max(1) as f32 * 1.2;
    }
    let longest = order.iter().map(|&i| path[i].len() + bits[i].len()).max().unwrap_or(0) as f32;
    let speed = 60.0f32.max(longest / 0.9);
    let arrived = order
        .iter()
        .map(|&i| start[i] + (path[i].len() + bits[i].len()) as f32 / speed)
        .fold(0.0, f32::max);
    let span = (sc.tw() + sc.th()).max(1) as f32;
    Box::new(BinaryPath {
        path,
        bits,
        start,
        speed,
        wipe_at: arrived + BINARY_COLLAPSE,
        // tte wipes 2 diagonals a frame; at most 0.5s on a big clock.
        wipe_rate: 120.0f32.max(span / 0.5),
    })
}

/// tte's path: from a random spot off screen, alternating moves along a
/// column (up to max(10, a fifth of the width) rows) and along a row (up to
/// 4 columns), in unit steps.
fn zigzag(sc: &Scene, rng: &mut impl Rng, i: usize) -> Vec<(i32, i32)> {
    let to = sc.pos(i);
    let edge = random_edge_pos(sc, rng);
    let mut at = (edge.0 as i32, edge.1 as i32);
    // Push the start just off the screen.
    if at.0 <= 0 {
        at.0 = -1;
    } else if at.0 >= sc.sw - 1 {
        at.0 = sc.sw;
    } else if at.1 <= 0 {
        at.1 = -1;
    } else {
        at.1 = sc.sh;
    }
    let row_limit = 10.max(sc.sw / 5);
    let mut along_column: bool = rng.random();
    let mut out = vec![at];
    while at != to {
        let (dx, dy) = (to.0 - at.0, to.1 - at.1);
        let rows = along_column && dy != 0 || dx == 0;
        let steps = if rows {
            rng.random_range(1..=dy.abs().min(row_limit))
        } else {
            rng.random_range(1..=dx.abs().min(4))
        };
        for _ in 0..steps {
            if rows {
                at.1 += dy.signum();
            } else {
                at.0 += dx.signum();
            }
            out.push(at);
        }
        along_column = !rows;
    }
    out
}

impl Effect for BinaryPath {
    fn draw(&mut self, sc: &Scene, t: f32, f: &mut Frame) -> bool {
        let greens = const { hexes(["#044E29", "#157e38", "#45bf55", "#95ed87"]) };
        let (tw, th) = (sc.tw(), sc.th());
        for i in 0..sc.len() {
            let Some(cell) = sc.settled(i) else { continue };
            let (path, bits) = (&self.path[i], &self.bits[i]);
            if path.is_empty() {
                sc.draw_from(f, i, cell.fg, 1.0);
                continue;
            }
            let lt = t - self.start[i];
            if lt < 0.0 {
                continue;
            }
            // The head moves along the path, the other bits a cell apart.
            let head = (lt * self.speed) as usize;
            let arrive = (path.len() + bits.len()) as f32 / self.speed;
            if lt < arrive {
                for (k, &b) in bits.iter().enumerate() {
                    if let Some(&(x, y)) = head.checked_sub(k).and_then(|s| path.get(s)) {
                        let shade = greens[(hash(i as u64 ^ (k as u64) << 32) % 4) as usize];
                        f.put(x, y, b, shade);
                    }
                }
                continue;
            }
            let (r, c) = sc.rc(i);
            let dim = cell.fg.brightness(0.5);
            let wiped = self.wipe_at + (tw - 1 - c + r) as f32 / self.wipe_rate;
            let col = if t >= wiped {
                dim.lerp(cell.fg, (t - wiped) / BINARY_BRIGHTEN)
            } else {
                WHITE.lerp(dim, ease_in_quad((lt - arrive) / BINARY_COLLAPSE))
            };
            let (x, y) = sc.pos(i);
            f.put(x, y, cell.ch, col);
        }
        t > self.wipe_at + (tw + th) as f32 / self.wipe_rate + BINARY_BRIGHTEN
    }
}

// --- errorcorrect -------------------------------------------------------------

/// errorcorrect: a tenth of the characters start swapped in pairs, blinking
/// red; pair by pair, each wipes into a block that slides to its place,
/// turning green, wipes back into its character and fades into its color.
pub struct ErrorCorrect {
    /// For each cell, the cell it is swapped with (or itself), when its pair
    /// is corrected, and how long the slide takes.
    partner: Vec<usize>,
    swap_at: Vec<f32>,
    slide: Vec<f32>,
    end: f32,
}

/// tte: wipe in over 8 block symbols, out over 7 (3 frames each), then
/// green to the final color in 10 steps of 3 frames.
const WIPE_IN: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
const WIPE_OUT: [char; 7] = ['▇', '▆', '▅', '▄', '▃', '▂', '▁'];
const WIPE: f32 = 8.0 * 3.0 / 60.0;
const CORRECT_FADE: f32 = 0.5;

pub fn errorcorrect(sc: &Scene, rng: &mut impl Rng) -> Box<dyn Effect> {
    let mut cells = ink(sc);
    cells.shuffle(rng);
    // tte: error-pairs 0.1 is the share of characters swapped, as pairs.
    let pairs = ((cells.len() as f32 * 0.1) as usize).max(1).min(cells.len() / 2);
    let mut partner: Vec<usize> = (0..sc.len()).collect();
    let mut swap_at = vec![0.0; sc.len()];
    let mut slide = vec![0.0; sc.len()];
    // tte corrects a pair every 6 frames; all of them within 1.4s here.
    let gap = (6.0 / 60.0f32).min(1.4 / pairs.max(1) as f32);
    for k in 0..pairs.min(cells.len() / 2) {
        let (a, b) = (cells[2 * k], cells[2 * k + 1]);
        partner[a] = b;
        partner[b] = a;
        let at = 0.3 + k as f32 * gap;
        // tte: 0.9 cells a frame, capped.
        let (pa, pb) = (target(sc, a), target(sc, b));
        let secs = ((pa.0 - pb.0).hypot(2.0 * (pa.1 - pb.1)) / 54.0).clamp(0.1, 0.4);
        for i in [a, b] {
            swap_at[i] = at;
            slide[i] = secs;
        }
    }
    let end = (0..sc.len())
        .filter(|&i| partner[i] != i)
        .map(|i| swap_at[i] + WIPE + slide[i] + WIPE + CORRECT_FADE)
        .fold(0.0, f32::max);
    Box::new(ErrorCorrect {
        partner,
        swap_at,
        slide,
        end,
    })
}

impl Effect for ErrorCorrect {
    fn draw(&mut self, sc: &Scene, t: f32, f: &mut Frame) -> bool {
        let (error, correct) = (const { Rgb::hex("#e74c3c") }, const { Rgb::hex("#45bf55") });
        for i in 0..sc.len() {
            let Some(cell) = sc.settled(i) else { continue };
            let j = self.partner[i];
            if j == i {
                put(f, target(sc, i), cell.ch, cell.fg);
                continue;
            }
            let lt = t - self.swap_at[i];
            let (from, to) = (target(sc, j), target(sc, i));
            let slide = self.slide[i];
            if lt < 0.0 {
                // In the wrong place, blinking between white and red.
                let col = if ((t * 8.0) as u32).is_multiple_of(2) {
                    error
                } else {
                    WHITE
                };
                put(f, from, cell.ch, col);
            } else if lt < WIPE {
                put(f, from, WIPE_IN[((lt / WIPE * 8.0) as usize).min(7)], error);
            } else if lt < WIPE + slide {
                let p = (lt - WIPE) / slide;
                put(f, lerp(from, to, p), '█', error.lerp(correct, p));
            } else if lt < 2.0 * WIPE + slide {
                let p = (lt - WIPE - slide) / WIPE;
                put(f, to, WIPE_OUT[((p * 7.0) as usize).min(6)], correct);
            } else {
                put(
                    f,
                    to,
                    cell.ch,
                    correct.lerp(cell.fg, (lt - 2.0 * WIPE - slide) / CORRECT_FADE),
                );
            }
        }
        t > self.end
    }
}

// --- unstable ---------------------------------------------------------------

/// unstable: characters appear jumbled, turning orange while jolts shake
/// them ever more often, burst to the screen edges, then reassemble.
pub struct Unstable {
    jumbled: Vec<Pos>,
    edge: Vec<Pos>,
    burst: Vec<f32>,
    back: Vec<f32>,
    /// When the jolts happen (each lasts a frame or so).
    jolts: Vec<f32>,
}

const RUMBLE: f32 = 1.2;
const UNSTABLE_HOLD: f32 = 0.3;
const UNSTABLE_FADE: f32 = 0.6;

pub fn unstable(sc: &Scene, rng: &mut impl Rng) -> Box<dyn Effect> {
    let n = sc.len();
    let cells = ink(sc);
    let mut spots: Vec<Pos> = cells.iter().map(|&i| target(sc, i)).collect();
    spots.shuffle(rng);
    let mut jumbled: Vec<Pos> = (0..n).map(|i| target(sc, i)).collect();
    for (k, &i) in cells.iter().enumerate() {
        jumbled[i] = spots[k];
    }
    let edge: Vec<Pos> = (0..n).map(|_| random_edge_pos(sc, rng)).collect();
    // tte: 1 cell a frame out and back, eased out-expo.
    let speed = vec![60.0; n];
    let burst = flight_times_between(&jumbled, &edge, &speed, 0.6);
    let back = flight_times(sc, &edge, &speed, 0.8);
    // tte: after 30 of its 120 rumble frames, a jolt every 18 frames, one
    // frame sooner each time.
    let mut jolts = vec![];
    let (mut frame, mut every) = (30.0f32, 18.0f32);
    while frame < 120.0 {
        jolts.push(frame / 120.0 * RUMBLE);
        frame += every;
        every = (every - 1.0).max(1.0);
    }
    Box::new(Unstable {
        jumbled,
        edge,
        burst,
        back,
        jolts,
    })
}

impl Effect for Unstable {
    fn draw(&mut self, sc: &Scene, t: f32, f: &mut Frame) -> bool {
        let orange = const { Rgb::hex("#ff9200") };
        let jolt = self.jolts.iter().position(|&j| (j..j + 0.05).contains(&t));
        let burst_end = RUMBLE + max(&self.burst) + UNSTABLE_HOLD;
        for i in 0..sc.len() {
            let Some(cell) = sc.settled(i) else { continue };
            let (pos, ch, col) = if t < RUMBLE {
                let mut p = self.jumbled[i];
                if let Some(k) = jolt {
                    // Everything jumps by the same -1, 0 or 1 cell.
                    let h = hash(k as u64);
                    p = (p.0 + (h % 3) as f32 - 1.0, p.1 + (h / 3 % 3) as f32 - 1.0);
                }
                (p, cell.ch, cell.fg.lerp(orange, t / RUMBLE))
            } else if t < RUMBLE + self.burst[i] {
                let p = ease_out_expo((t - RUMBLE) / self.burst[i]);
                (lerp(self.jumbled[i], self.edge[i], p), cell.ch, orange)
            } else if t < burst_end {
                (self.edge[i], cell.ch, orange)
            } else {
                let lt = t - burst_end;
                let p = ease_out_expo(lt / self.back[i]);
                (
                    lerp(self.edge[i], target(sc, i), p),
                    cell.ch,
                    orange.lerp(cell.fg, (lt - self.back[i]) / UNSTABLE_FADE),
                )
            };
            put(f, pos, ch, col);
        }
        t > burst_end + max(&self.back) + UNSTABLE_FADE
    }
}

// --- crumble ------------------------------------------------------------------

/// crumble: the weakened text falls apart in ever bigger, quicker groups,
/// bouncing to the bottom as dust; the dust is vacuumed up through the
/// middle of the screen, and the text drops back in with a flash.
pub struct Crumble {
    fall_at: Vec<f32>,
    fall: Vec<f32>,
    vacuum_at: Vec<f32>,
    dust: Vec<char>,
    /// When the vacuuming starts, and when the text drops back.
    reset_at: f32,
}

const CRUMBLE_FALLS: f32 = 0.9;
const CRUMBLE_VACUUM: f32 = 0.4;
const CRUMBLE_RESET: f32 = 0.45;
const CRUMBLE_FLASH: f32 = 0.45;

pub fn crumble(sc: &Scene, rng: &mut impl Rng) -> Box<dyn Effect> {
    let n = sc.len();
    let mut order = ink(sc);
    order.shuffle(rng);
    // tte: groups of 1..max fall every min-max frames (12 to start); 60% of
    // the time the groups grow and the gaps shrink.
    let mut fall_at = vec![0.0; n];
    let (mut frame, mut size, mut lo, mut hi) = (0.0f32, 1usize, 9i32, 12i32);
    let mut k = 0;
    while k < order.len() {
        for _ in 0..rng.random_range(1..=size) {
            if let Some(&i) = order.get(k) {
                fall_at[i] = frame;
                k += 1;
            }
        }
        frame += rng.random_range(lo..=hi) as f32 + 1.0;
        if rng.random_range(1..=10) > 4 {
            size += 1;
            lo = (lo - 1).max(0);
            hi = (hi - 1).max(0);
        }
    }
    let scale = (1.0 / 60.0f32).min(CRUMBLE_FALLS / frame.max(1.0));
    for f in &mut fall_at {
        *f *= scale;
    }
    // tte: 0.65 cells a frame down to the bottom, eased out-bounce.
    let bottom = (sc.sh - 1).max(0) as f32;
    let fall: Vec<f32> = (0..n)
        .map(|i| ((bottom - target(sc, i).1).max(0.0) * 2.0 / 39.0).clamp(0.1, 0.5))
        .collect();
    let fallen = (0..n).map(|i| fall_at[i] + fall[i]).fold(0.0, f32::max);
    // tte vacuums 3-10 a frame, in random order.
    let mut vacuum_at = vec![0.0; n];
    order.shuffle(rng);
    for (rank, &i) in order.iter().enumerate() {
        vacuum_at[i] = fallen + rank as f32 / order.len().max(1) as f32 * CRUMBLE_VACUUM;
    }
    // Cells that only turn into text as the clock ticks during the effect
    // take a random character's timing, so they don't fly up at once.
    for i in 0..n {
        if sc.ch(i) == ' '
            && let Some(&j) = order.choose(rng)
        {
            fall_at[i] = fall_at[j];
            vacuum_at[i] = vacuum_at[j];
        }
    }
    Box::new(Crumble {
        fall_at,
        fall,
        vacuum_at,
        dust: (0..n).map(|_| ['*', '.', ','][rng.random_range(0..3)]).collect(),
        reset_at: fallen + 2.0 * CRUMBLE_VACUUM,
    })
}

impl Effect for Crumble {
    fn draw(&mut self, sc: &Scene, t: f32, f: &mut Frame) -> bool {
        let bottom = (sc.sh - 1).max(0) as f32;
        let mid = (sc.sw as f32 / 2.0, sc.sh as f32 / 2.0);
        for i in 0..sc.len() {
            let Some(cell) = sc.settled(i) else { continue };
            let (tx, ty) = target(sc, i);
            let (weak, dusty) = (cell.fg.brightness(0.65), cell.fg.brightness(0.55));
            let (pos, ch, col) = if t < self.fall_at[i] {
                ((tx, ty), cell.ch, weak)
            } else if t < self.fall_at[i] + self.fall[i] {
                let p = (t - self.fall_at[i]) / self.fall[i];
                ((tx, ty + (bottom - ty) * ease_out_bounce(p)), self.dust[i], dusty)
            } else if t < self.vacuum_at[i] {
                ((tx, bottom), self.dust[i], dusty)
            } else if t < self.reset_at {
                // Up to the top along a curve through the middle (tte: a
                // bezier, eased out-quint).
                let p = ease_out_quint(((t - self.vacuum_at[i]) / CRUMBLE_VACUUM).min(1.0));
                let a = lerp((tx, bottom), mid, p);
                let b = lerp(mid, (tx, 0.0), p);
                (lerp(a, b, p), self.dust[i], dusty)
            } else if t < self.reset_at + CRUMBLE_RESET {
                let p = (t - self.reset_at) / CRUMBLE_RESET;
                ((tx, ty * p), cell.ch, weak)
            } else {
                // tte: the color flashes to white and back.
                let lt = t - self.reset_at - CRUMBLE_RESET;
                let up = CRUMBLE_FLASH * 0.4;
                let col = if lt < up {
                    cell.fg.lerp(WHITE, lt / up)
                } else {
                    WHITE.lerp(cell.fg, (lt - up) / (CRUMBLE_FLASH - up))
                };
                ((tx, ty), cell.ch, col)
            };
            put(f, pos, ch, col);
        }
        t > self.reset_at + CRUMBLE_RESET + CRUMBLE_FLASH
    }
}

// --- fireworks ----------------------------------------------------------------

/// fireworks: shells of characters launch from the bottom, burst above the
/// text, bloom downwards and swing into place, fading from the shell's
/// color into their own.
pub struct Fireworks {
    shells: Vec<Shell>,
    /// Per cell: its shell (`usize::MAX`: none), where it bursts to, and
    /// how long each leg takes (burst, bloom, home).
    shell: Vec<usize>,
    burst: Vec<Pos>,
    legs: Vec<[f32; 3]>,
}

struct Shell {
    launch_at: f32,
    from: Pos,
    apex: Pos,
    rise: f32,
    color: Rgb,
}

const FIREWORK_FADE: f32 = 0.8;

pub fn fireworks(sc: &Scene, rng: &mut impl Rng) -> Box<dyn Effect> {
    let colors = const { hexes(["#88F7E2", "#44D492", "#F5EB67", "#FFA15C", "#FA233E"]) };
    let n = sc.len();
    let cells = ink(sc);
    // tte: shells of 5% of the characters, in reading order; a shell every
    // 45 frames (all within 1.4s here).
    let per = ((cells.len() as f32 * 0.05).round() as usize).max(1);
    let count = cells.len().div_ceil(per).max(1);
    let gap = (45.0 / 60.0f32).min(1.4 / count as f32);
    // tte bursts within a fifth of the width (columns; half as many rows).
    let reach = (sc.sw as f32 * 0.2).max(2.0);
    let mut shell = vec![usize::MAX; n];
    let mut burst = vec![(0.0, 0.0); n];
    let mut legs = vec![[0.0; 3]; n];
    let mut shells = vec![];
    for (k, group) in cells.chunks(per).enumerate() {
        // Up from the bottom at a random column, to a random height above
        // the shell's first character (0.35 cells a frame, eased out-expo).
        let top = target(sc, group[0]).1;
        let apex = (
            rng.random_range(0.0..sc.sw.max(1) as f32),
            rng.random_range(0.0..top.max(1.0)),
        );
        let from = (apex.0, sc.sh as f32);
        let rise = ((from.1 - apex.1) * 2.0 / 21.0).clamp(0.1, 0.6);
        for &i in group {
            shell[i] = k;
            let a: f32 = rng.random_range(0.0..std::f32::consts::TAU);
            let d = reach * rng.random::<f32>().sqrt();
            burst[i] = (apex.0 + a.cos() * d, apex.1 + a.sin() * d / 2.0);
            // Out at 0.2-0.4 cells a frame, a bloom, then home at 0.6.
            let out = (d / rng.random_range(12.0..24.0)).clamp(0.1, 0.4);
            let (hx, hy) = target(sc, i);
            let home = ((hx - burst[i].0).hypot(2.0 * (hy - burst[i].1)) / 36.0).clamp(0.2, 0.8);
            legs[i] = [out, 0.3, home];
        }
        shells.push(Shell {
            launch_at: k as f32 * gap,
            from,
            apex,
            rise,
            color: colors[rng.random_range(0..colors.len())],
        });
    }
    Box::new(Fireworks {
        shells,
        shell,
        burst,
        legs,
    })
}

/// A point along the quadratic curve from `a` to `b` bent towards `c`.
fn bezier(a: Pos, c: Pos, b: Pos, p: f32) -> Pos {
    lerp(lerp(a, c, p), lerp(c, b, p), p)
}

impl Effect for Fireworks {
    fn draw(&mut self, sc: &Scene, t: f32, f: &mut Frame) -> bool {
        let mut end: f32 = 0.0;
        for i in 0..sc.len() {
            let Some(s) = self.shells.get(self.shell[i]) else {
                sc.draw_from(f, i, sc.theme.flash, 1.0);
                continue;
            };
            let [out, bloom, home] = self.legs[i];
            end = end.max(s.launch_at + s.rise + out + bloom + home.max(FIREWORK_FADE));
            let Some(cell) = sc.settled(i) else { continue };
            let lt = t - s.launch_at;
            if lt < 0.0 {
                continue;
            }
            // tte: shell color to white and back while it flies.
            let flicker = |p: f32| s.color.lerp(WHITE, 1.0 - (2.0 * p - 1.0).abs());
            if lt < s.rise {
                let p = lt / s.rise;
                put(f, lerp(s.from, s.apex, ease_out_expo(p)), 'o', flicker(p));
                continue;
            }
            let lt = lt - s.rise;
            let b = self.burst[i];
            // tte's bloom: past the burst point along its ray, 7 rows down.
            let ray = (b.0 - s.apex.0, b.1 - s.apex.1);
            let bend = (b.0 + ray.0 * 0.5, b.1 + ray.1 * 0.5);
            let bloomed = (bend.0, bend.1 + 7.0);
            let (pos, col) = if lt < out {
                let p = lt / out;
                (lerp(s.apex, b, ease_out_circ(p)), flicker(p))
            } else if lt < out + bloom {
                let p = (lt - out) / bloom;
                (bezier(b, bend, bloomed, p), s.color)
            } else {
                // Home along a curve that dips towards the bottom.
                let lt = lt - out - bloom;
                let low = (bloomed.0, (sc.sh - 1) as f32);
                let p = ease_in_out_quart(lt / home);
                (
                    bezier(bloomed, low, target(sc, i), p),
                    s.color.lerp(cell.fg, lt / FIREWORK_FADE),
                )
            };
            put(f, pos, cell.ch, col);
        }
        t > end
    }
}

// --- bubbles ------------------------------------------------------------------

/// bubbles: from the bottom row up, characters gather into rings of 5-20
/// that drift down from above the screen; when a ring reaches its lowest
/// character's row it pops, sending its characters out and then home,
/// fading from white into their colors.
pub struct Bubbles {
    bubbles: Vec<Bubble>,
    /// Per cell: its bubble, and its place on the ring.
    member: Vec<Option<(usize, usize)>>,
}

struct Bubble {
    /// When it appears, where it starts, and where and when it pops.
    release: f32,
    from: Pos,
    pop_at: Pos,
    fall: f32,
    radius: f32,
    size: usize,
    color: Rgb,
}

/// tte: a new bubble every 20 frames, falling 0.5 cells a frame; sped up
/// to fit a timer.
const BUBBLE_RELEASE: f32 = 1.2;
const BUBBLE_FALL: f32 = 30.0;
const BUBBLE_FALL_MAX: f32 = 1.3;
/// After the pop: `*` then `'` (9 frames each), the move out and home, and
/// the fade from white (8 steps of 6 frames).
const BUBBLE_POP: f32 = 0.3;
const BUBBLE_OUT: f32 = 0.2;
const BUBBLE_HOME: f32 = 0.7;
const BUBBLE_FADE: f32 = 0.8;

pub fn bubbles(sc: &Scene, rng: &mut impl Rng) -> Box<dyn Effect> {
    let colors = const { hexes(["#d33aff", "#7395c4", "#43c2a7", "#02ff7f"]) };
    let n = sc.len();
    // tte: rows from the bottom up, cut into groups of 5-20.
    let mut order = ink(sc);
    order.sort_by_key(|&i| std::cmp::Reverse(sc.rc(i).0));
    let mut groups: Vec<Vec<usize>> = vec![];
    let mut rest = &order[..];
    while !rest.is_empty() {
        let take = if rest.len() < 5 {
            rest.len()
        } else {
            rng.random_range(5..=rest.len().min(20))
        };
        groups.push(rest[..take].to_vec());
        rest = &rest[take..];
    }
    let gap = (20.0 / 60.0f32).min(BUBBLE_RELEASE / groups.len().max(1) as f32);
    let mut member = vec![None; n];
    let bubbles = groups
        .iter()
        .enumerate()
        .map(|(b, group)| {
            let radius = (group.len() / 5).max(1) as f32;
            // Starts 10 rows above the screen at a random column, and drifts
            // towards another until its bottom reaches its lowest row.
            let from = (rng.random_range(0.0..sc.sw.max(1) as f32), -10.0);
            let toward = (rng.random_range(0.0..sc.sw.max(1) as f32), 0.0);
            let lowest = group.iter().map(|&i| target(sc, i).1).fold(f32::MIN, f32::max);
            let q = ((lowest - radius - from.1) / (lowest - from.1)).clamp(0.0, 1.0);
            let pop_at = (from.0 + (toward.0 - from.0) * q, lowest - radius);
            let fall = ((pop_at.0 - from.0).hypot(pop_at.1 - from.1) / BUBBLE_FALL).min(BUBBLE_FALL_MAX);
            for (k, &i) in group.iter().enumerate() {
                member[i] = Some((b, k));
            }
            Bubble {
                release: b as f32 * gap,
                from,
                pop_at,
                fall,
                radius,
                size: group.len(),
                color: colors[rng.random_range(0..colors.len())],
            }
        })
        .collect();
    Box::new(Bubbles { bubbles, member })
}

/// Place `k` of `size` on a ring of `radius` around `c` (tte's
/// `find_coords_on_circle`: columns count double).
fn ring(c: Pos, radius: f32, k: usize, size: usize) -> Pos {
    let a = std::f32::consts::TAU * k as f32 / size.max(1) as f32;
    (c.0 + 2.0 * radius * a.cos(), c.1 + radius * a.sin())
}

impl Effect for Bubbles {
    fn draw(&mut self, sc: &Scene, t: f32, f: &mut Frame) -> bool {
        for i in 0..sc.len() {
            let (Some(cell), Some((b, k))) = (sc.settled(i), self.member[i]) else {
                continue;
            };
            let bubble = &self.bubbles[b];
            let lt = t - bubble.release;
            if lt < 0.0 {
                continue;
            }
            if lt < bubble.fall {
                let anchor = lerp(bubble.from, bubble.pop_at, lt / bubble.fall);
                put(f, ring(anchor, bubble.radius, k, bubble.size), cell.ch, bubble.color);
                continue;
            }
            let pt = lt - bubble.fall;
            let on_ring = ring(bubble.pop_at, bubble.radius, k, bubble.size);
            let out = ring(bubble.pop_at, bubble.radius + 3.0, k, bubble.size);
            let pos = if pt < BUBBLE_OUT {
                lerp(on_ring, out, ease_out_expo(pt / BUBBLE_OUT))
            } else {
                lerp(out, target(sc, i), ease_in_out_expo((pt - BUBBLE_OUT) / BUBBLE_HOME))
            };
            let (ch, col) = if pt < BUBBLE_POP {
                (if pt < BUBBLE_POP / 2.0 { '*' } else { '\'' }, WHITE)
            } else {
                (cell.ch, WHITE.lerp(cell.fg, (pt - BUBBLE_POP) / BUBBLE_FADE))
            };
            put(f, pos, ch, col);
        }
        let tail = (BUBBLE_OUT + BUBBLE_HOME).max(BUBBLE_POP + BUBBLE_FADE);
        let end = self.bubbles.iter().map(|b| b.release + b.fall).fold(0.0, f32::max);
        t > end + tail
    }
}

// --- orbittingvolley ----------------------------------------------------------

/// orbittingvolley: four launchers circle the clock, firing volleys of
/// characters that build it from the center out.
pub struct OrbittingVolley {
    fire_at: Vec<f32>,
    launcher: Vec<usize>,
    flight: Vec<f32>,
    last_fire: f32,
}

const VOLLEY_FLIGHT: f32 = 0.7;

pub fn orbittingvolley(sc: &Scene, _rng: &mut impl Rng) -> Box<dyn Effect> {
    let c = center(sc);
    let mut cells = ink(sc);
    let dist = |i: usize| {
        let p = target(sc, i);
        ((p.0 - c.0) / 2.0).hypot(p.1 - c.1)
    };
    cells.sort_by(|&a, &b| dist(a).total_cmp(&dist(b)));
    let per_volley = ((cells.len() as f32 * 0.03).ceil() as usize).max(1) * 4;
    let volleys = cells.len().div_ceil(per_volley).max(1);
    let gap = 2.2 / volleys as f32;
    let mut fire_at = vec![0.0; sc.len()];
    let mut launcher = vec![0; sc.len()];
    for (j, &i) in cells.iter().enumerate() {
        fire_at[i] = (j / per_volley) as f32 * gap;
        launcher[i] = j % 4;
    }
    // tte: launched characters fly 1.5 cells a frame, eased out-sine.
    let from: Vec<Pos> = (0..sc.len())
        .map(|i| OrbittingVolley::launcher_pos(sc, launcher[i], fire_at[i]))
        .collect();
    let flight = flight_times(sc, &from, &vec![90.0; sc.len()], VOLLEY_FLIGHT);
    let last_fire = max(&fire_at);
    Box::new(OrbittingVolley {
        fire_at,
        launcher,
        flight,
        last_fire,
    })
}

impl OrbittingVolley {
    /// Position of launcher `k` at time `t`, travelling around a box just
    /// outside the clock.
    /// Launcher `k` at time `t`: tte's four launchers start in the four
    /// corners of the screen and orbit its edge at 0.8 cells a frame.
    fn launcher_pos(sc: &Scene, k: usize, t: f32) -> Pos {
        let (w, h) = ((sc.sw - 1).max(1) as f32, (sc.sh - 1).max(1) as f32);
        let per = 2.0 * (w + h);
        let corner = [0.0, w, w + h, 2.0 * w + h][k % 4];
        let mut d = (corner + t * 48.0).rem_euclid(per);
        if d < w {
            return (d, 0.0);
        }
        d -= w;
        if d < h {
            return (w, d);
        }
        d -= h;
        if d < w {
            return (w - d, h);
        }
        (0.0, h - (d - w))
    }
}

impl Effect for OrbittingVolley {
    fn draw(&mut self, sc: &Scene, t: f32, f: &mut Frame) -> bool {
        for i in 0..sc.len() {
            let Some(cell) = sc.settled(i) else { continue };
            let lt = t - self.fire_at[i];
            if lt < 0.0 {
                continue;
            }
            let k = self.launcher[i];
            let p = lt / self.flight[i];
            let col = sc.theme.gradient_at(k as f32 * 0.25).lerp(cell.fg, p);
            let from = Self::launcher_pos(sc, k, self.fire_at[i]);
            put(f, lerp(from, target(sc, i), ease_out_sine(p.min(1.0))), cell.ch, col);
        }
        if t < self.last_fire + 0.3 {
            for k in 0..4 {
                put(
                    f,
                    Self::launcher_pos(sc, k, t),
                    '█',
                    sc.theme.gradient_at(k as f32 * 0.25),
                );
            }
        }
        t > (0..sc.len())
            .map(|i| self.fire_at[i] + self.flight[i])
            .fold(self.last_fire + 0.3, f32::max)
    }
}

// --- rings --------------------------------------------------------------------

/// rings: characters leave their places to form spinning rings around the
/// center, then disperse and return.
pub struct Rings {
    ring: Vec<usize>,
    angle: Vec<f32>,
    radii: Vec<f32>,
    spin: Vec<f32>,
    scatter: Vec<Pos>,
}

pub fn rings(sc: &Scene, rng: &mut impl Rng) -> Box<dyn Effect> {
    let c = center(sc);
    let mut cells = ink(sc);
    cells.sort_by(|&a, &b| {
        let d = |i| {
            let p = target(sc, i);
            ((p.0 - c.0) / 2.0).hypot(p.1 - c.1)
        };
        d(a).total_cmp(&d(b))
    });
    // tte: rings a tenth of the screen's shorter side apart, from radius 1,
    // each holding 7 characters per unit of radius.
    let gap = (0.1 * sc.sw.min(sc.sh) as f32).floor().max(1.0);
    let (mut ring_of, mut angle) = (vec![0; sc.len()], vec![0.0; sc.len()]);
    let mut radii = Vec::new();
    let mut rest = &cells[..];
    while !rest.is_empty() {
        let r = 1.0 + gap * radii.len() as f32;
        let cap = (7.0 * r) as usize;
        let (on, more) = rest.split_at(cap.min(rest.len()));
        for (j, &i) in on.iter().enumerate() {
            ring_of[i] = radii.len();
            angle[i] = j as f32 / on.len() as f32 * std::f32::consts::TAU;
        }
        radii.push(r);
        rest = more;
    }
    let spin = (0..radii.len())
        .map(|k| rng.random_range(0.8..2.0) * if k % 2 == 0 { 1.0 } else { -1.0 })
        .collect();
    let scatter = (0..sc.len())
        .map(|_| (rng.random_range(-8.0..8.0), rng.random_range(-3.0..3.0)))
        .collect();
    Box::new(Rings {
        ring: ring_of,
        angle,
        radii,
        spin,
        scatter,
    })
}

impl Effect for Rings {
    fn draw(&mut self, sc: &Scene, t: f32, f: &mut Frame) -> bool {
        const FORM: f32 = 0.7;
        const SPIN_END: f32 = 2.4;
        const SCATTER_END: f32 = 2.9;
        const HOME_END: f32 = 3.8;
        let colors = const { hexes(["#ab48ff", "#e7b2b2", "#fffebd"]) };
        let c = center(sc);
        for i in 0..sc.len() {
            let Some(cell) = sc.settled(i) else { continue };
            let k = self.ring[i];
            let Some(&r) = self.radii.get(k) else {
                sc.draw_from(f, i, cell.fg, 1.0);
                continue;
            };
            let color = colors[k % colors.len()];
            let a = self.angle[i] + self.spin[k] * (t - FORM).clamp(0.0, SPIN_END - FORM);
            let on_ring = (c.0 + a.cos() * r * 2.0, c.1 + a.sin() * r);
            let scattered = (on_ring.0 + self.scatter[i].0, on_ring.1 + self.scatter[i].1);
            let (pos, col) = if t < FORM {
                let p = t / FORM;
                (
                    lerp(target(sc, i), on_ring, ease_in_out_sine(p)),
                    cell.fg.lerp(color, p),
                )
            } else if t < SPIN_END {
                (on_ring, color)
            } else if t < SCATTER_END {
                (
                    lerp(
                        on_ring,
                        scattered,
                        ease_out_expo((t - SPIN_END) / (SCATTER_END - SPIN_END)),
                    ),
                    color,
                )
            } else {
                let p = ((t - SCATTER_END) / (HOME_END - SCATTER_END)).min(1.0);
                (lerp(scattered, target(sc, i), ease_out_expo(p)), color.lerp(cell.fg, p))
            };
            put(f, pos, cell.ch, col);
        }
        t > HOME_END
    }
}

// --- blackhole ----------------------------------------------------------------

/// blackhole: the characters become a starfield, a black hole forms and
/// swallows the stars, collapses, and explodes the text back into place.
pub struct Blackhole {
    star: Vec<Pos>,
    star_glyph: Vec<char>,
    /// Its shade in the starfield (tte: gray to white).
    field_color: Vec<Rgb>,
    /// Its color when the text explodes back.
    star_color: Vec<Rgb>,
    swallow_at: Vec<f32>,
    /// Where it shoots to in the explosion before drifting home (tte: a
    /// point 3 cells from home).
    near: Vec<Pos>,
}

pub fn blackhole(sc: &Scene, rng: &mut impl Rng) -> Box<dyn Effect> {
    let colors = const { hexes(["#ffcc0d", "#ff7326", "#ff194d", "#bf2669", "#702a8c", "#049dbf"]) };
    let dim = const { Rgb::hex("#4a4a4d") };
    let n = sc.len();
    Box::new(Blackhole {
        star: (0..n).map(|_| random_screen_pos(sc, rng)).collect(),
        star_glyph: (0..n)
            .map(|_| ['*', '\'', '`', '¤', '•', '°', '·'][rng.random_range(0..7)])
            .collect(),
        field_color: (0..n)
            .map(|_| dim.lerp(WHITE, rng.random_range(0..6) as f32 / 5.0))
            .collect(),
        star_color: (0..n).map(|_| colors[rng.random_range(0..colors.len())]).collect(),
        swallow_at: (0..n).map(|_| rng.random_range(1.0..1.9)).collect(),
        near: (0..n)
            .map(|i| {
                let a: f32 = rng.random_range(0.0..std::f32::consts::TAU);
                let (x, y) = target(sc, i);
                (x + 6.0 * a.cos(), y + 3.0 * a.sin())
            })
            .collect(),
    })
}

impl Effect for Blackhole {
    fn draw(&mut self, sc: &Scene, t: f32, f: &mut Frame) -> bool {
        const FORM: f32 = 1.0;
        const SWALLOW: f32 = 0.5;
        const COLLAPSE_AT: f32 = 2.5;
        const EXPLODE_AT: f32 = 2.9;
        const EXPLODE: f32 = 1.0;
        let c = center(sc);
        let r = (sc.th() as f32 * 0.6).clamp(2.0, 6.0);
        // The hole: a ring of stars, drawn in as it forms, shrinking as it collapses.
        if t < EXPLODE_AT {
            let (reach, rr) = if t < COLLAPSE_AT {
                ((t / FORM).min(1.0), r)
            } else {
                (1.0, r * (1.0 - (t - COLLAPSE_AT) / (EXPLODE_AT - COLLAPSE_AT)))
            };
            let steps = 32;
            for s in 0..((steps as f32 * reach) as usize) {
                let a = s as f32 / steps as f32 * std::f32::consts::TAU + t * 0.8;
                put(f, (c.0 + a.cos() * rr * 2.0, c.1 + a.sin() * rr), '*', WHITE);
            }
            if t >= COLLAPSE_AT {
                let k = ((t * 14.0) as usize) % 7;
                put(
                    f,
                    c,
                    ['◦', '◎', '◉', '●', '◉', '◎', '◦'][k],
                    // No stars at all on an empty canvas (`--done-text ""`).
                    self.star_color
                        .get(k % self.star_color.len().max(1))
                        .copied()
                        .unwrap_or(WHITE),
                );
            }
        }
        for i in 0..sc.len() {
            let Some(cell) = sc.settled(i) else { continue };
            if t < self.swallow_at[i] {
                let twinkle = if hash(i as u64 ^ (t * 4.0) as u64).is_multiple_of(5) {
                    0.6
                } else {
                    1.0
                };
                put(f, self.star[i], self.star_glyph[i], self.field_color[i].scale(twinkle));
            } else if t < self.swallow_at[i] + SWALLOW {
                // Pulled in, fading to black (tte's consumed scene).
                let p = ease_in_expo((t - self.swallow_at[i]) / SWALLOW);
                let col = self.field_color[i].lerp(Rgb(0, 0, 0), p);
                put(f, lerp(self.star[i], c, p), self.star_glyph[i], col);
            } else if t >= EXPLODE_AT {
                // tte: out fast to a point near home (eased out-expo), then
                // a slow drift in (in-cubic), from a star color to its own.
                const OUT: f32 = 0.35;
                let lt = t - EXPLODE_AT;
                let col = self.star_color[i].lerp(cell.fg, lt / EXPLODE);
                let pos = if lt < OUT {
                    lerp(c, self.near[i], ease_out_expo(lt / OUT))
                } else {
                    lerp(self.near[i], target(sc, i), ease_in_cubic((lt - OUT) / (EXPLODE - OUT)))
                };
                put(f, pos, cell.ch, col);
            }
        }
        t > EXPLODE_AT + EXPLODE
    }
}

// --- expand -------------------------------------------------------------------

/// expand: every character starts at the center and expands into place at
/// one speed, so nearer ones land first, shifting from the gradient's first
/// color to its own on the way.
pub fn expand(sc: &Scene, _rng: &mut impl Rng) -> Box<dyn Effect> {
    let n = sc.len();
    let c = center(sc);
    // tte: 0.35 cells a frame (rows count double); faster on a big clock.
    let dist = |i: usize| {
        let (x, y) = target(sc, i);
        (x - c.0).hypot(2.0 * (y - c.1))
    };
    let far = (0..n).map(dist).fold(0.0, f32::max);
    let speed = 21.0f32.max(far / 1.1);
    let dur = (0..n).map(|i| (dist(i) / speed).max(0.15)).collect();
    let color = sc.theme.gradient_at(0.0);
    Box::new(Travel::new(vec![c; n], vec![0.0; n], dur, ease_in_out_quart, vec![color; n]).tinted(0.0, false))
}

// --- rain ---------------------------------------------------------------------

/// rain: raindrops fall from the top, a row at a time from the bottom up,
/// and fade from the rain's blues into the text.
pub fn rain(sc: &Scene, rng: &mut impl Rng) -> Box<dyn Effect> {
    const RELEASE: f32 = 1.4;
    let blues = const {
        hexes([
            "#00315C", "#004C8F", "#0075DB", "#3F91D9", "#78B9F2", "#9AC8F5", "#B8D8F8", "#E3EFFC",
        ])
    };
    let n = sc.len();
    let mut order = ink(sc);
    order.shuffle(rng);
    // Bottom row first; the sort is stable, so each row stays shuffled.
    order.sort_by_key(|&i| std::cmp::Reverse(sc.rc(i).0));
    let mut delay = vec![0.0; n];
    for (k, &i) in order.iter().enumerate() {
        delay[i] = k as f32 / order.len().max(1) as f32 * RELEASE;
    }
    let start = (0..n).map(|i| (target(sc, i).0, 0.0)).collect();
    // tte falls 0.33-0.57 rows a frame; capped so tall screens don't drag.
    let dur = (0..n)
        .map(|i| (target(sc, i).1.max(1.0) / (60.0 * rng.random_range(0.33..0.57))).min(1.0))
        .collect();
    let color = (0..n).map(|_| blues[rng.random_range(0..blues.len())]).collect();
    let mut fx = Travel::new(start, delay, dur, ease_in_quart, color).tinted(0.35, true);
    fx.glyphs = Some(
        (0..n)
            .map(|_| ['o', '.', ',', '*', '|'][rng.random_range(0..5)])
            .collect(),
    );
    Box::new(fx)
}

// --- slide --------------------------------------------------------------------

/// slide: row by row from the top, characters slide in from the left edge,
/// the rightmost first, shifting from the gradient's first color to their own.
pub fn slide(sc: &Scene, _rng: &mut impl Rng) -> Box<dyn Effect> {
    /// At most this long to launch one row's characters (tte: one a frame).
    const ROW_LAUNCH: f32 = 0.6;
    let n = sc.len();
    let tw = sc.tw().max(1) as f32;
    let per_char = (1.0 / 60.0f32).min(ROW_LAUNCH / tw);
    let gap = row_gap(sc, 0.05);
    let delay = (0..n)
        .map(|i| {
            let (r, c) = sc.rc(i);
            r as f32 * gap + (tw - 1.0 - c as f32) * per_char
        })
        .collect();
    // tte moves 0.8 cells a frame; capped for wide screens.
    let dur = (0..n)
        .map(|i| ((target(sc, i).0 + 1.0) / 48.0).clamp(0.2, 0.9))
        .collect();
    let start = (0..n).map(|i| (-1.0, target(sc, i).1)).collect();
    let color = vec![sc.theme.gradient_at(0.0); n];
    Box::new(Travel::new(start, delay, dur, ease_in_out_quad, color).tinted(1.0, false))
}

// --- swarm --------------------------------------------------------------------

/// swarm: the characters gather in swarms of a tenth each that fly in from
/// the edge, mill about in two areas, flashing as they move, and settle.
/// Each character flies at tte's speeds on its own path, so a swarm strings
/// out instead of moving as one block.
pub struct Swarm {
    launch: Vec<f32>,
    /// Spawn, then a point and a hop in each of the two areas; home follows.
    points: Vec<[Pos; 5]>,
    /// How long each leg takes, to the next point (or home).
    legs: Vec<[f32; 5]>,
}

/// Per leg (out to an area, a hop within it, ..., home): cells a second
/// (tte's 0.4, 0.18 and 0.45 a frame, sped up), the longest it may take,
/// and its easing.
/// A leg of a swarm's flight: speed, longest time, easing.
type Leg = (f32, f32, fn(f32) -> f32);

const SWARM_LEGS: [Leg; 5] = [
    (48.0, 0.6, ease_out_sine),
    (27.0, 0.35, ease_in_out_sine),
    (48.0, 0.6, ease_out_sine),
    (27.0, 0.35, ease_in_out_sine),
    (54.0, 0.7, ease_in_out_quad),
];
const SWARM_LAND: f32 = 0.35;

pub fn swarm(sc: &Scene, rng: &mut impl Rng) -> Box<dyn Effect> {
    let n = sc.len();
    let order = ink(sc);
    let size = ((order.len() as f32 * 0.1).round() as usize).max(1);
    let count = order.len().div_ceil(size).max(1);
    let stagger = 1.2 / count as f32;
    // tte: areas are centered a radius of min(width, height) / 2 apart, and
    // are ellipses max(min(width, height) / 6, 1) * 2 columns across (half
    // as many rows).
    let short = sc.sw.min(sc.sh).max(1) as f32;
    let radius = (short / 2.0).max(1.0);
    let area = (short / 6.0).floor().max(1.0) * 2.0;
    let mut launch = vec![0.0; n];
    let mut points = vec![[(0.0, 0.0); 5]; n];
    let mut legs = vec![[0.0; 5]; n];
    for (s, group) in order.chunks(size).enumerate() {
        let spawn = random_edge_pos(sc, rng);
        let first = near(sc, rng, spawn, radius);
        let second = near(sc, rng, first, radius);
        for &i in group {
            launch[i] = s as f32 * stagger + rng.random_range(0.0..0.2);
            points[i] = [
                spawn,
                within(rng, first, area),
                within(rng, first, area),
                within(rng, second, area),
                within(rng, second, area),
            ];
            for (k, &(speed, longest, _)) in SWARM_LEGS.iter().enumerate() {
                let to = points[i].get(k + 1).copied().unwrap_or_else(|| target(sc, i));
                let from = points[i][k];
                let d = (to.0 - from.0).hypot(2.0 * (to.1 - from.1));
                legs[i][k] = (d / speed).clamp(0.08, longest);
            }
        }
    }
    // Cells that only turn into text as the clock ticks fly with a random
    // character's swarm.
    for i in 0..n {
        if sc.ch(i) == ' '
            && let Some(&j) = order.choose(rng)
        {
            launch[i] = launch[j];
            points[i] = points[j];
            legs[i] = legs[j];
        }
    }
    Box::new(Swarm { launch, points, legs })
}

/// A point on the screen on a circle of `radius` around `from` (tte's
/// next swarm area), else anywhere.
fn near(sc: &Scene, rng: &mut impl Rng, from: Pos, radius: f32) -> Pos {
    for _ in 0..8 {
        let a: f32 = rng.random_range(0.0..std::f32::consts::TAU);
        let p = (from.0 + a.cos() * radius * 2.0, from.1 + a.sin() * radius);
        if (0.0..sc.sw as f32).contains(&p.0) && (0.0..sc.sh as f32).contains(&p.1) {
            return p;
        }
    }
    random_screen_pos(sc, rng)
}

/// A random point in the ellipse `width` columns either side of `c` and
/// half as many rows (tte's `find_coords_in_circle`).
fn within(rng: &mut impl Rng, c: Pos, width: f32) -> Pos {
    let a: f32 = rng.random_range(0.0..std::f32::consts::TAU);
    let d = rng.random::<f32>().sqrt();
    (c.0 + a.cos() * d * width, c.1 + a.sin() * d * width / 2.0)
}

impl Effect for Swarm {
    fn draw(&mut self, sc: &Scene, t: f32, f: &mut Frame) -> bool {
        let base = const { Rgb::hex("#31a0d4") };
        let flash = const { Rgb::hex("#f2ea79") };
        let mut done = true;
        for i in 0..sc.len() {
            let flight: f32 = self.legs[i].iter().sum();
            let lt = t - self.launch[i];
            if lt < flight + SWARM_LAND {
                done = false;
            }
            let Some(cell) = sc.settled(i) else { continue };
            if lt < 0.0 {
                continue;
            }
            if lt >= flight {
                put(
                    f,
                    target(sc, i),
                    cell.ch,
                    flash.lerp(cell.fg, (lt - flight) / SWARM_LAND),
                );
                continue;
            }
            let (mut at, mut from) = (0.0, self.points[i][0]);
            for (k, &(_, _, ease)) in SWARM_LEGS.iter().enumerate() {
                let len = self.legs[i][k];
                let to = self.points[i].get(k + 1).copied().unwrap_or_else(|| target(sc, i));
                if lt < at + len {
                    let p = (lt - at) / len;
                    // tte flashes with the distance covered: base, flash, base.
                    let col = base.lerp(flash, (p * std::f32::consts::PI).sin());
                    put(f, lerp(from, to, ease(p)), cell.ch, col);
                    break;
                }
                at += len;
                from = to;
            }
        }
        done
    }
}
