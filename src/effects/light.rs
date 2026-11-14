//! Effects that work on the text in place: light, color, smoke, scans and
//! weather passing over it.
//!
//! Designs from `TerminalTextEffects` (MIT, © 2023 `ChrisBuilds`) and its Rust
//! port `ttfx` (MIT, © 2026 37signals) — see LICENSE. Timelines are shortened
//! so the clock stays readable.

use rand::{Rng, RngExt, seq::SliceRandom};

use super::{Effect, max, reveal};
use crate::{
    scene::Scene,
    term::Frame,
    theme::Rgb,
    util::{ease_in_out_sine, ease_out_cubic, hash, hash01, line},
};

const WHITE: Rgb = Rgb(255, 255, 255);

fn hexes(list: &[&str]) -> Vec<Rgb> {
    list.iter().map(|h| Rgb::hex(h)).collect()
}

/// Sample a color list as a loop.
fn cycle(colors: &[Rgb], u: f32) -> Rgb {
    let n = colors.len();
    let x = u.rem_euclid(1.0) * n as f32;
    let i = x as usize % n;
    colors[i].lerp(colors[(i + 1) % n], x.fract())
}

/// Sample a color list from first to last.
fn palette(colors: &[Rgb], u: f32) -> Rgb {
    let x = u.clamp(0.0, 1.0) * (colors.len() - 1) as f32;
    let i = (x as usize).min(colors.len() - 1);
    colors[i].lerp(colors[(i + 1).min(colors.len() - 1)], x.fract())
}

/// Distance of cell `i` from the canvas center, 0..1, with columns counted
/// half (cells are about twice as tall as wide).
fn radial(sc: &Scene, i: usize) -> f32 {
    let (r, c) = sc.rc(i);
    let dx = (c as f32 - sc.tw() as f32 / 2.0) / 2.0;
    let dy = r as f32 - sc.th() as f32 / 2.0;
    let far = ((sc.tw() as f32 / 4.0).hypot(sc.th() as f32 / 2.0)).max(1.0);
    dx.hypot(dy) / far
}

// --- colorshift ---------------------------------------------------------------

/// colorshift: a rainbow ripples out from the center, then the text settles
/// into its own colors.
pub struct ColorShift;

impl Effect for ColorShift {
    fn draw(&mut self, sc: &Scene, t: f32, f: &mut Frame) -> bool {
        const SHIFT: f32 = 2.4;
        const SETTLE: f32 = 0.6;
        let rainbow = hexes(&[
            "#e81416", "#ffa500", "#faeb36", "#79c314", "#487de7", "#4b369d", "#70369d",
        ]);
        for i in 0..sc.len() {
            let Some(cell) = sc.settled(i) else { continue };
            let wave = cycle(&rainbow, radial(sc, i) - t * 0.9);
            let (x, y) = sc.pos(i);
            let col = if t < SHIFT {
                wave
            } else {
                wave.lerp(cell.fg, (t - SHIFT) / SETTLE)
            };
            f.put(x, y, cell.ch, col);
        }
        t > SHIFT + SETTLE
    }
}

// --- highlight ----------------------------------------------------------------

/// highlight: a specular glint runs diagonally across the text.
pub struct Highlight;

impl Effect for Highlight {
    fn draw(&mut self, sc: &Scene, t: f32, f: &mut Frame) -> bool {
        const DUR: f32 = 1.3;
        const WIDTH: f32 = 8.0;
        let (tw, th) = (sc.tw() as f32, sc.th() as f32);
        // Bottom-left to top-right.
        let span = tw + 2.0 * th;
        let at = -WIDTH + (span + 2.0 * WIDTH) * ease_in_out_sine(t / DUR);
        for i in 0..sc.len() {
            let Some(cell) = sc.settled(i) else { continue };
            let (r, c) = sc.rc(i);
            let key = c as f32 + 2.0 * (th - 1.0 - r as f32);
            let glint = (1.0 - (key - at).abs() / WIDTH).max(0.0);
            let (x, y) = sc.pos(i);
            f.put(
                x,
                y,
                cell.ch,
                cell.fg.scale(1.0 + 0.75 * glint).lerp(WHITE, glint * 0.35),
            );
        }
        t > DUR
    }
}

// --- randomsequence -----------------------------------------------------------

/// randomsequence: characters appear one at a time in random order.
pub struct RandomSequence {
    appear: Vec<f32>,
}

pub fn randomsequence(sc: &Scene, rng: &mut impl Rng) -> Box<dyn Effect> {
    let mut order: Vec<usize> = (0..sc.len()).filter(|&i| sc.ch(i) != ' ').collect();
    order.shuffle(rng);
    let mut appear = vec![0.0; sc.len()];
    for (k, &i) in order.iter().enumerate() {
        appear[i] = k as f32 / order.len().max(1) as f32 * 1.6;
    }
    Box::new(RandomSequence { appear })
}

impl Effect for RandomSequence {
    fn draw(&mut self, sc: &Scene, t: f32, f: &mut Frame) -> bool {
        for (i, &appear) in self.appear.iter().enumerate() {
            if t >= appear {
                reveal(sc, f, i, t - appear, 0.3);
            }
        }
        t > max(&self.appear) + 0.3
    }
}

// --- sweep --------------------------------------------------------------------

/// sweep: a shaded band sweeps right-to-left revealing the text uncolored,
/// then left-to-right coloring it.
pub struct Sweep;

impl Effect for Sweep {
    fn draw(&mut self, sc: &Scene, t: f32, f: &mut Frame) -> bool {
        const PASS: f32 = 0.8;
        const SECOND: f32 = 0.9;
        const BAND: [char; 4] = ['█', '▓', '▒', '░'];
        let tw = sc.tw() as f32;
        let gray = Rgb::hex("#808080");
        let travel = |p: f32| -4.0 + (tw + 8.0) * ease_in_out_sine(p);
        let first = tw - travel(t / PASS); // right to left
        let second = travel((t - SECOND) / PASS); // left to right
        for i in 0..sc.len() {
            let c = sc.rc(i).1 as f32;
            let (x, y) = sc.pos(i);
            let band = |edge: f32, behind_is_right: bool| {
                let d = if behind_is_right { c - edge } else { edge - c };
                (0.0..4.0).contains(&d).then(|| BAND[d as usize])
            };
            if t >= SECOND {
                if let Some(ch) = band(second, false) {
                    f.put(x, y, ch, sc.theme.flash);
                } else if c < second {
                    sc.draw_from(f, i, sc.theme.flash, (second - c) / 6.0);
                } else if let Some(cell) = sc.settled(i) {
                    f.put(x, y, cell.ch, gray);
                }
            } else if let Some(ch) = band(first, true) {
                f.put(x, y, ch, gray.lerp(WHITE, 0.4));
            } else if c > first
                && let Some(cell) = sc.settled(i)
            {
                f.put(x, y, cell.ch, gray);
            }
        }
        t > SECOND + PASS
    }
}

// --- smoke --------------------------------------------------------------------

/// smoke: smoke spreads from a point across the clock, coloring the text
/// as it passes.
pub struct Smoke {
    arrive: Vec<f32>,
}

const PUFF: f32 = 0.5;

pub fn smoke(sc: &Scene, rng: &mut impl Rng) -> Box<dyn Effect> {
    let (r0, c0) = (rng.random_range(0..sc.th().max(1)), rng.random_range(0..sc.tw().max(1)));
    let dist = |i: usize| {
        let (r, c) = sc.rc(i);
        (r as f32 - r0 as f32).abs() + (c as f32 - c0 as f32).abs() / 2.0
    };
    let far = (0..sc.len()).map(dist).fold(1.0, f32::max);
    let arrive = (0..sc.len())
        .map(|i| 0.1 + dist(i) / far * 1.8 + rng.random_range(0.0..0.15))
        .collect();
    Box::new(Smoke { arrive })
}

impl Effect for Smoke {
    fn draw(&mut self, sc: &Scene, t: f32, f: &mut Frame) -> bool {
        const PUFFS: [char; 5] = ['░', '▒', '▓', '▒', '░'];
        let (start, dark) = (Rgb::hex("#7A7A7A"), Rgb::hex("#242424"));
        for (i, &arrive) in self.arrive.iter().enumerate() {
            let (x, y) = sc.pos(i);
            let lt = t - arrive;
            if lt < 0.0 {
                if let Some(cell) = sc.settled(i) {
                    f.put(x, y, cell.ch, start);
                }
            } else if lt < PUFF {
                // The smoke covers every cell of the clock, blank or not.
                let p = lt / PUFF;
                let k = ((p * PUFFS.len() as f32) as usize).min(PUFFS.len() - 1);
                f.put(x, y, PUFFS[k], dark.lerp(WHITE, 1.0 - (p - 0.5).abs() * 2.0));
            } else {
                reveal(sc, f, i, lt - PUFF, 0.3);
            }
        }
        t > max(&self.arrive) + PUFF + 0.3
    }
}

// --- spotlights ---------------------------------------------------------------

/// spotlights: spotlights search the dark text, then gather in the center
/// and widen to light it all.
pub struct Spotlights {
    /// Waypoints for each light, in canvas coordinates.
    paths: Vec<Vec<(f32, f32)>>,
}

const SEARCH: f32 = 2.0;
const GATHER: f32 = 0.5;
const WIDEN: f32 = 0.8;

pub fn spotlights(sc: &Scene, rng: &mut impl Rng) -> Box<dyn Effect> {
    let (tw, th) = (sc.tw().max(1) as f32, sc.th().max(1) as f32);
    let paths = (0..3)
        .map(|_| {
            (0..4)
                .map(|_| (rng.random_range(0.0..tw), rng.random_range(0.0..th)))
                .collect()
        })
        .collect();
    Box::new(Spotlights { paths })
}

impl Effect for Spotlights {
    fn draw(&mut self, sc: &Scene, t: f32, f: &mut Frame) -> bool {
        let (tw, th) = (sc.tw() as f32, sc.th() as f32);
        let center = (tw / 2.0, th / 2.0);
        let beam = (tw / 2.0).hypot(th).max(4.0) / 4.0;
        let lights: Vec<(f32, f32, f32)> = self
            .paths
            .iter()
            .map(|path| {
                let legs = (path.len() - 1) as f32;
                let s = (t / SEARCH).min(1.0) * legs;
                let k = (s as usize).min(path.len() - 2);
                let (a, b) = (path[k], path[k + 1]);
                let p = ease_in_out_sine(s - k as f32);
                let searching = (a.0 + (b.0 - a.0) * p, a.1 + (b.1 - a.1) * p);
                let g = ease_in_out_sine((t - SEARCH) / GATHER);
                let pos = (
                    searching.0 + (center.0 - searching.0) * g,
                    searching.1 + (center.1 - searching.1) * g,
                );
                let widen = ease_out_cubic((t - SEARCH - GATHER) / WIDEN);
                (pos.0, pos.1, beam * (1.0 + widen * 4.0))
            })
            .collect();
        for i in 0..sc.len() {
            let Some(cell) = sc.settled(i) else { continue };
            let (r, c) = sc.rc(i);
            let lit = lights
                .iter()
                .map(|&(lx, ly, radius)| {
                    let d = ((c as f32 - lx) / 2.0).hypot(r as f32 - ly) / radius;
                    ((1.0 - d) / 0.3).clamp(0.0, 1.0)
                })
                .fold(0.0, f32::max);
            let (x, y) = sc.pos(i);
            f.put(x, y, cell.ch, cell.fg.scale(0.12 + 0.88 * lit));
        }
        t > SEARCH + GATHER + WIDEN
    }
}

// --- laseretch ----------------------------------------------------------------

/// laseretch: a laser from above etches the characters one by one, throwing
/// sparks; each etched character cools from yellow to its color.
pub struct LaserEtch {
    order: Vec<usize>,
    etch_at: Vec<f32>,
}

const ETCH: f32 = 2.0;

pub fn laseretch(sc: &Scene, _rng: &mut impl Rng) -> Box<dyn Effect> {
    // Etch row by row, snaking back and forth.
    let (tw, th) = (sc.tw(), sc.th());
    let mut order = Vec::new();
    for r in 0..th {
        let cols: Vec<usize> = if r % 2 == 0 {
            (0..tw).collect()
        } else {
            (0..tw).rev().collect()
        };
        order.extend(cols.into_iter().map(|c| r * tw + c).filter(|&i| sc.ch(i) != ' '));
    }
    let mut etch_at = vec![0.0; sc.len()];
    for (k, &i) in order.iter().enumerate() {
        etch_at[i] = k as f32 / order.len().max(1) as f32 * ETCH;
    }
    Box::new(LaserEtch { order, etch_at })
}

impl Effect for LaserEtch {
    fn draw(&mut self, sc: &Scene, t: f32, f: &mut Frame) -> bool {
        let cool = hexes(&["#ffe680", "#ff7b00"]);
        let laser = hexes(&["#ffffff", "#376cff"]);
        let sparks = hexes(&["#ffffff", "#ffe680", "#ff7b00", "#1a0900"]);
        for (i, &etch_at) in self.etch_at.iter().enumerate() {
            let Some(cell) = sc.settled(i) else { continue };
            let age = t - etch_at;
            if age < 0.0 {
                continue;
            }
            let (x, y) = sc.pos(i);
            let cooling = if age < 0.3 {
                palette(&cool, age / 0.3)
            } else {
                cool[1].lerp(cell.fg, (age - 0.3) / 0.4)
            };
            f.put(x, y, cell.ch, cooling);
            // Two sparks per etched character, thrown out and falling.
            if age < 0.6 {
                for s in 0..2u64 {
                    let h = hash(i as u64 * 2 + s);
                    let vx = (hash01(h) - 0.5) * 14.0;
                    let vy = -2.0 - hash01(h ^ 9) * 6.0;
                    let (sx, sy) = (x as f32 + vx * age, y as f32 + vy * age + 25.0 * age * age);
                    let ch = ['*', '.', '\''][(h % 3) as usize];
                    f.put(sx.round() as i32, sy.round() as i32, ch, palette(&sparks, age / 0.6));
                }
            }
        }
        // The beam, from above the clock to the character being etched.
        if t < ETCH {
            let k = ((t / ETCH) * self.order.len() as f32) as usize;
            if let Some(&i) = self.order.get(k) {
                let to = sc.pos(i);
                let from = (sc.ox + sc.tw() as i32 / 2, (sc.oy - 6).max(0));
                let path = line(from, to);
                let n = path.len().max(2) - 1;
                for (j, &(x, y)) in path.iter().enumerate() {
                    let (dx, dy) = (to.0 - from.0, to.1 - from.1);
                    let ch = if dx.abs() > 2 * dy.abs() {
                        '─'
                    } else if dy.abs() > 2 * dx.abs() {
                        '│'
                    } else if (dx > 0) == (dy > 0) {
                        '\\'
                    } else {
                        '/'
                    };
                    f.put(x, y, ch, palette(&laser, 1.0 - j as f32 / n as f32));
                }
            }
        }
        t > ETCH + 0.8
    }
}

// --- overflow -----------------------------------------------------------------

/// overflow: rows of the text scroll past in a jumble, overflowing, until
/// the real rows arrive in order.
pub struct Overflow {
    /// Source row for each line of the scroll; `None` marks the real rows.
    lines: Vec<Option<usize>>,
}

const SCROLL: f32 = 2.0;

pub fn overflow(sc: &Scene, rng: &mut impl Rng) -> Box<dyn Effect> {
    let th = sc.th().max(1);
    let cycles = rng.random_range(2..=4);
    let mut lines: Vec<Option<usize>> = (0..cycles * th).map(|_| Some(rng.random_range(0..th))).collect();
    lines.extend(std::iter::repeat_n(None, th));
    Box::new(Overflow { lines })
}

impl Effect for Overflow {
    fn draw(&mut self, sc: &Scene, t: f32, f: &mut Frame) -> bool {
        let colors = hexes(&["#f2ebc0", "#8dbfb3", "#f2ebc0"]);
        let (tw, th) = (sc.tw(), sc.th());
        let total = self.lines.len() - th;
        let offset = (ease_out_cubic(t / SCROLL) * total as f32).round() as usize;
        for row in 0..th {
            let line = offset + row;
            if let Some(src) = self.lines.get(line).copied().flatten() {
                let col = palette(&colors, line as f32 / self.lines.len() as f32);
                for c in 0..tw {
                    let ch = sc.ch(src * tw + c);
                    f.put(sc.ox + c as i32, sc.oy + row as i32, ch, col);
                }
            } else {
                // A real row: the one that belongs `line - total` rows down.
                let real = line - total;
                for c in 0..tw {
                    let i = real * tw + c;
                    if let Some(cell) = sc.settled(i) {
                        f.put(sc.ox + c as i32, sc.oy + row as i32, cell.ch, cell.fg);
                    }
                }
            }
        }
        t > SCROLL
    }
}

// --- synthgrid ----------------------------------------------------------------

/// synthgrid: a neon grid grows over the clock; its blocks fill with
/// dissolving shades that resolve into the text; the grid then retracts.
pub struct SynthGrid {
    block_w: usize,
    block_h: usize,
    /// When each block (row-major) starts dissolving into text.
    fill_at: Vec<f32>,
}

const GROW: f32 = 0.7;
const FILL_END: f32 = 2.5;
const DISSOLVE: f32 = 0.4;
const RETRACT: f32 = 0.6;

pub fn synthgrid(sc: &Scene, rng: &mut impl Rng) -> Box<dyn Effect> {
    let block_w = (sc.tw() / 6).max(4);
    let block_h = (sc.th() / 2).max(2);
    let blocks = sc.tw().div_ceil(block_w) * sc.th().div_ceil(block_h);
    let fill_at = (0..blocks)
        .map(|_| rng.random_range(GROW..FILL_END - DISSOLVE))
        .collect();
    Box::new(SynthGrid {
        block_w,
        block_h,
        fill_at,
    })
}

impl Effect for SynthGrid {
    fn draw(&mut self, sc: &Scene, t: f32, f: &mut Frame) -> bool {
        const SHADES: [char; 3] = ['░', '▒', '▓'];
        let grid = hexes(&["#CC00CC", "#ffffff"]);
        let text = hexes(&["#8A008A", "#00D1FF", "#FFFFFF"]);
        let (tw, th) = (sc.tw(), sc.th());
        let per_row = tw.div_ceil(self.block_w);
        // The text, block by block.
        for i in 0..sc.len() {
            let (r, c) = sc.rc(i);
            let lt = t - self.fill_at[(r / self.block_h) * per_row + c / self.block_w];
            if lt < 0.0 {
                continue;
            }
            let (x, y) = sc.pos(i);
            if lt < DISSOLVE {
                if sc.ch(i) != ' ' {
                    let shade = SHADES[(hash(i as u64 ^ (t * 15.0) as u64) % 3) as usize];
                    f.put(x, y, shade, palette(&text, r as f32 / th.max(1) as f32));
                }
            } else {
                reveal(sc, f, i, lt - DISSOLVE, 0.3);
            }
        }
        // The grid, growing from the center then retracting, drawn on blank cells only.
        let reach = if t < FILL_END {
            ease_out_cubic(t / GROW)
        } else {
            1.0 - ease_out_cubic((t - FILL_END) / RETRACT)
        };
        let (cx, cy) = (tw as f32 / 2.0, th as f32 / 2.0);
        for r in 0..=th {
            for c in 0..=tw {
                // Block edges, plus the outer border (without doubling a block
                // edge that falls right next to it).
                let on_row = (r % self.block_h == 0 && r + 1 < th) || r == th;
                let on_col = (c % self.block_w == 0 && c + 1 < tw) || c == tw;
                if !(on_row || on_col) {
                    continue;
                }
                let d = ((c as f32 - cx) / cx.max(1.0))
                    .abs()
                    .max(((r as f32 - cy) / cy.max(1.0)).abs());
                if d > reach {
                    continue;
                }
                let i = r * tw + c;
                if r < th && c < tw && sc.ch(i) != ' ' && t > GROW {
                    continue;
                }
                let ch = match (on_row, on_col) {
                    (true, true) => '┼',
                    (true, false) => '─',
                    _ => '│',
                };
                let col = palette(&grid, (r + c) as f32 / (tw + th).max(1) as f32);
                f.put(sc.ox + c as i32, sc.oy + r as i32, ch, col);
            }
        }
        t > FILL_END + RETRACT
    }
}

// --- thunderstorm -------------------------------------------------------------

/// thunderstorm: rain falls over the darkened text, lightning strikes, the
/// text glows and sparks fly, then the storm passes.
pub struct Thunderstorm {
    strikes: Vec<Strike>,
}

struct Strike {
    at: f32,
    path: Vec<(i32, i32, char)>,
}

const STORM: f32 = 3.0;
const CLEAR: f32 = 0.6;

pub fn thunderstorm(sc: &Scene, rng: &mut impl Rng) -> Box<dyn Effect> {
    let strikes = (0..3)
        .map(|k| {
            let mut x = rng.random_range(sc.ox..=(sc.ox + sc.tw() as i32).max(sc.ox));
            let mut path = Vec::new();
            for y in 0..sc.sh {
                let step = rng.random_range(-1..=1);
                let ch = match step {
                    -1 => '/',
                    1 => '\\',
                    _ => '│',
                };
                path.push((x, y, ch));
                x += step;
            }
            Strike {
                at: 0.4 + k as f32 * 0.85 + rng.random_range(0.0..0.3),
                path,
            }
        })
        .collect();
    Box::new(Thunderstorm { strikes })
}

impl Effect for Thunderstorm {
    fn draw(&mut self, sc: &Scene, t: f32, f: &mut Frame) -> bool {
        let bolt = Rgb::hex("#68A3E8");
        let glow = Rgb::hex("#EF5411");
        let spark = Rgb::hex("#ff4d00");
        let rain = Rgb::hex("#5d7a99");
        // How strongly the text glows after the latest strike.
        let since = self
            .strikes
            .iter()
            .filter(|s| t >= s.at)
            .map(|s| t - s.at)
            .fold(f32::INFINITY, f32::min);
        let glowing = if since.is_finite() {
            (1.0 - since / 0.5).max(0.0)
        } else {
            0.0
        };
        let clear = ((t - STORM) / CLEAR).clamp(0.0, 1.0);
        for i in 0..sc.len() {
            let Some(cell) = sc.settled(i) else { continue };
            let (x, y) = sc.pos(i);
            let dark = cell.fg.scale(0.3).lerp(glow, glowing);
            f.put(x, y, cell.ch, dark.lerp(cell.fg, clear));
        }
        // Rain: slanted drops, thinning out as the storm passes.
        if t < STORM {
            let drops = (sc.sw * sc.sh / 30).max(1) as u64;
            let density = 1.0 - (t / STORM).powi(4);
            for d in 0..drops {
                if hash01(d ^ 0xabc) > density {
                    continue;
                }
                let h = hash(d);
                let fall = (h % 1000) as f32 / 1000.0 * (sc.sh + 8) as f32 + t * 28.0;
                let y = fall.rem_euclid((sc.sh + 8) as f32) - 4.0;
                let x = ((h >> 20) % sc.sw.max(1) as u64) as f32 + y * 0.5;
                let ch = ['\\', '.', ','][((h >> 40) % 3) as usize];
                f.put(x.rem_euclid(sc.sw as f32) as i32, y as i32, ch, rain);
            }
        }
        // Lightning, flickering, then sparks where it hits.
        for s in &self.strikes {
            let lt = t - s.at;
            if (0.0..0.3).contains(&lt) && ((lt * 25.0) as u32) % 3 != 2 {
                for &(x, y, ch) in &s.path {
                    f.put(x, y, ch, bolt.lerp(WHITE, 0.3));
                }
            }
            if let Some(&(bx, by, _)) = s.path.last()
                && (0.0..0.7).contains(&lt)
            {
                for k in 0..10u64 {
                    let h = hash(k ^ (s.at * 1000.0) as u64);
                    let vx = (hash01(h) - 0.5) * 24.0;
                    let vy = -4.0 - hash01(h ^ 3) * 10.0;
                    let sx = bx as f32 + vx * lt;
                    let sy = by as f32 + vy * lt + 20.0 * lt * lt;
                    f.put(
                        sx as i32,
                        sy as i32,
                        ['*', '.', '\''][(h % 3) as usize],
                        spark.lerp(rain, lt / 0.7),
                    );
                }
            }
        }
        t > STORM + CLEAR
    }
}

// --- vhstape ------------------------------------------------------------------

/// vhstape: lines glitch sideways in RGB, noise speckles the picture, a
/// distortion wave rolls down, and finally each line is redrawn clean.
pub struct VhsTape {
    seed: u64,
}

pub fn vhstape(_sc: &Scene, rng: &mut impl Rng) -> Box<dyn Effect> {
    Box::new(VhsTape { seed: rng.random() })
}

impl Effect for VhsTape {
    fn draw(&mut self, sc: &Scene, t: f32, f: &mut Frame) -> bool {
        const GLITCH: f32 = 2.2;
        const REDRAW: f32 = 0.6;
        let colors = hexes(&["#ffffff", "#ff0000", "#00ff00", "#0000ff", "#ffffff"]);
        let noise = hexes(&["#1e1e1f", "#3c3b3d", "#6d6c70", "#a2a1a6", "#cbc9cf"]);
        let (tw, th) = (sc.tw(), sc.th());
        let slot = (t / 0.08) as u64;
        // A wave of distortion rolling down once.
        let wave_row = (t - 0.5) / 1.0 * th as f32;
        for r in 0..th {
            let redraw_at = GLITCH + r as f32 / th.max(1) as f32 * REDRAW;
            let h = hash(self.seed ^ (r as u64) << 32 ^ slot);
            let in_wave = (r as f32 - wave_row).abs() < 1.5;
            let (shift, tint) = if t < GLITCH && (hash01(h) < 0.06 || in_wave) {
                let amount = if in_wave { 3 } else { (h >> 8) as i32 % 7 - 3 };
                (amount, Some(colors[(h >> 16) as usize % colors.len()]))
            } else {
                (0, None)
            };
            for c in 0..tw {
                let i = r * tw + c;
                let (x, y) = sc.pos(i);
                if t >= redraw_at {
                    if t < redraw_at + 0.08 {
                        f.put(x, y, '█', WHITE);
                    } else {
                        reveal(sc, f, i, t - redraw_at - 0.08, 0.25);
                    }
                    continue;
                }
                let speck = hash(self.seed ^ i as u64 ^ slot << 20);
                if hash01(speck) < 0.012 {
                    f.put(
                        x,
                        y,
                        ['█', '▓', '▒', '░'][(speck >> 8) as usize % 4],
                        noise[(speck >> 12) as usize % 5],
                    );
                } else if let Some(cell) = sc.settled(i) {
                    f.put(x + shift, y, cell.ch, tint.map_or(cell.fg, |c| cell.fg.lerp(c, 0.7)));
                }
            }
        }
        t > GLITCH + REDRAW + 0.35
    }
}
