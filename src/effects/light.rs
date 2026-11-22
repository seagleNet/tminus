//! Effects that work on the text in place: light, color, smoke, scans and
//! weather passing over it.
//!
//! Designs from `TerminalTextEffects` (MIT, © 2023 `ChrisBuilds`) and its Rust
//! port `ttfx` (MIT, © 2026 37signals) — see LICENSE. Timelines are shortened
//! so the clock stays readable.

use rand::{
    Rng, RngExt,
    seq::{IndexedRandom, SliceRandom},
};

use super::{
    Effect, WHITE, hexes, max,
    motion::{Pos, ink},
    palette, reveal,
};
use crate::{
    scene::Scene,
    term::Frame,
    theme::{Rgb, cycle},
    util::{
        ease_in_out_circ, ease_in_out_quad, ease_in_out_sine, ease_inverse, ease_out_cubic, ease_out_sine, hash, hash01,
    },
};

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
        let rainbow = const {
            hexes([
                "#e81416", "#ffa500", "#faeb36", "#79c314", "#487de7", "#4b369d", "#70369d",
            ])
        };
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

/// highlight: a specular highlight runs diagonally across the text from the
/// bottom left: each character brightens, holds, and dims back.
pub struct Highlight;

impl Effect for Highlight {
    fn draw(&mut self, sc: &Scene, t: f32, f: &mut Frame) -> bool {
        // tte: diagonals paced in-out-circ; each character brightens over
        // 3 steps, holds 8, dims over 3 (2 frames a step).
        const SWEEP: f32 = 1.0;
        const UP: f32 = 6.0 / 60.0;
        const HOLD: f32 = 16.0 / 60.0;
        let th = sc.th();
        let span = (sc.tw() + th).saturating_sub(2).max(1) as f32;
        for i in 0..sc.len() {
            let Some(cell) = sc.settled(i) else { continue };
            let (r, c) = sc.rc(i);
            let at = SWEEP * ease_inverse(ease_in_out_circ, (c + th - 1 - r) as f32 / span);
            let lt = t - at;
            let bright = cell.fg.brightness(1.75);
            let col = if lt < 0.0 {
                cell.fg
            } else if lt < UP {
                cell.fg.lerp(bright, lt / UP)
            } else if lt < UP + HOLD {
                bright
            } else {
                bright.lerp(cell.fg, (lt - UP - HOLD) / UP)
            };
            let (x, y) = sc.pos(i);
            f.put(x, y, cell.ch, col);
        }
        t > SWEEP + 2.0 * UP + HOLD
    }
}

// --- randomsequence -----------------------------------------------------------

/// randomsequence: characters appear in random order, each fading in from
/// the background.
pub struct RandomSequence {
    appear: Vec<f32>,
}

pub fn randomsequence(sc: &Scene, rng: &mut impl Rng) -> Box<dyn Effect> {
    let mut order: Vec<usize> = (0..sc.len()).filter(|&i| sc.ch(i) != ' ').collect();
    order.shuffle(rng);
    let mut appear = vec![0.0; sc.len()];
    for (k, &i) in order.iter().enumerate() {
        appear[i] = k as f32 / order.len().max(1) as f32 * 1.4;
    }
    Box::new(RandomSequence { appear })
}

impl Effect for RandomSequence {
    fn draw(&mut self, sc: &Scene, t: f32, f: &mut Frame) -> bool {
        // tte: from the background to the final color, 7 steps of 8 frames.
        const FADE: f32 = 56.0 / 60.0;
        for (i, &appear) in self.appear.iter().enumerate() {
            if t >= appear {
                sc.draw_from(f, i, sc.theme.bg, (t - appear) / FADE);
            }
        }
        t > max(&self.appear) + FADE
    }
}

// --- sweep --------------------------------------------------------------------

/// sweep: a sweep from the right reveals the text in gray, each cell
/// shimmering through block shades of random grays; a sweep back from the
/// left shimmers in the gradient's colors and leaves the text in its own.
pub struct Sweep;

impl Effect for Sweep {
    fn draw(&mut self, sc: &Scene, t: f32, f: &mut Frame) -> bool {
        // tte: columns paced in-out-circ; each cell's shimmer is 4 symbols
        // of 5 frames, each in a random color.
        const PASS: f32 = 0.8;
        const SHIMMER: f32 = 20.0 / 60.0;
        const BAND: [char; 4] = ['█', '▓', '▒', '░'];
        let shades = const { hexes(["#A0A0A0", "#808080", "#404040", "#202020", "#101010"]) };
        let gray = const { Rgb::hex("#808080") };
        let tw = sc.tw();
        let span = tw.saturating_sub(1).max(1) as f32;
        for i in 0..sc.len() {
            let c = sc.rc(i).1;
            let (x, y) = sc.pos(i);
            let first = PASS * ease_inverse(ease_in_out_circ, (tw - 1 - c) as f32 / span);
            let second = PASS + PASS * ease_inverse(ease_in_out_circ, c as f32 / span);
            let (at, pass) = if t >= second { (second, 2) } else { (first, 1) };
            let lt = t - at;
            if lt < 0.0 {
                continue;
            }
            if lt < SHIMMER {
                let k = ((lt / SHIMMER * 4.0) as usize).min(3);
                let h = hash(i as u64 ^ (k as u64) << 32 ^ pass << 40);
                let col = if pass == 1 {
                    shades[(h % 5) as usize]
                } else {
                    sc.theme.gradient_at(hash01(h))
                };
                f.put(x, y, BAND[k], col);
            } else if let Some(cell) = sc.settled(i) {
                f.put(x, y, cell.ch, if pass == 1 { gray } else { cell.fg });
            }
        }
        t > 2.0 * PASS + SHIMMER
    }
}

// --- smoke --------------------------------------------------------------------

/// smoke: smoke floods the clock from a random spot along the paths of a
/// random maze, colorizing the gray text it passes.
pub struct Smoke {
    arrive: Vec<f32>,
}

const SMOKE_SPREAD: f32 = 1.6;
/// tte: 5 smoke symbols of 3 frames, then the paint gradient (5 frames a step).
const PUFF: f32 = 15.0 / 60.0;
const SMOKE_PAINT: f32 = 0.5;

pub fn smoke(sc: &Scene, rng: &mut impl Rng) -> Box<dyn Effect> {
    let (tw, n) = (sc.tw(), sc.len());
    // tte: a random spanning tree over the text's area, flooded breadth
    // first from a random cell, a layer a frame.
    let (_, links) = prims_simple(tw, n, rng.random_range(0..n.max(1)), rng);
    let mut depth = vec![usize::MAX; n];
    if n > 0 {
        let start = rng.random_range(0..n);
        depth[start] = 0;
        let mut queue = std::collections::VecDeque::from([start]);
        while let Some(i) = queue.pop_front() {
            for &j in &links[i] {
                if depth[j] == usize::MAX {
                    depth[j] = depth[i] + 1;
                    queue.push_back(j);
                }
            }
        }
    }
    let deepest = depth
        .iter()
        .copied()
        .filter(|&d| d != usize::MAX)
        .max()
        .unwrap_or(0)
        .max(1);
    // tte floods a layer a frame; at most SMOKE_SPREAD here.
    let step = (1.0 / 60.0f32).min(SMOKE_SPREAD / deepest as f32);
    let arrive = depth.iter().map(|&d| d.min(deepest) as f32 * step).collect();
    Box::new(Smoke { arrive })
}

impl Effect for Smoke {
    fn draw(&mut self, sc: &Scene, t: f32, f: &mut Frame) -> bool {
        const PUFFS: [char; 5] = ['░', '▒', '▓', '▒', '░'];
        let gray = const { Rgb::hex("#7A7A7A") };
        // tte's smoke gradient: dark gray, white, then the final gradient
        // backwards; the paint runs through the final gradient to the color.
        let mut smoke: Vec<Rgb> = vec![const { Rgb::hex("#242424") }, WHITE];
        smoke.extend(sc.theme.gradient.iter().rev());
        for (i, &arrive) in self.arrive.iter().enumerate() {
            let (x, y) = sc.pos(i);
            let lt = t - arrive;
            if lt < 0.0 {
                if let Some(cell) = sc.settled(i) {
                    f.put(x, y, cell.ch, gray);
                }
            } else if lt < PUFF {
                // The smoke covers every cell of the clock, blank or not.
                let p = lt / PUFF;
                f.put(x, y, PUFFS[((p * 5.0) as usize).min(4)], palette(&smoke, p));
            } else if let Some(cell) = sc.settled(i) {
                let mut paint = sc.theme.gradient.clone();
                paint.push(cell.fg);
                f.put(x, y, cell.ch, palette(&paint, (lt - PUFF) / SMOKE_PAINT));
            }
        }
        t > max(&self.arrive) + PUFF + SMOKE_PAINT
    }
}

// --- spotlights ---------------------------------------------------------------

/// spotlights: three spotlights sweep in from off screen along curves,
/// searching the dark text, converge in the middle and widen until all of
/// it is lit.
pub struct Spotlights {
    /// Per light: its waypoints on the screen, each with a curve's control
    /// point (the first is where it starts, off screen).
    paths: Vec<Vec<(Pos, Pos)>>,
}

const SEARCH: f32 = 2.0;
const GATHER: f32 = 0.5;
const WIDEN: f32 = 0.8;

/// The area the spotlights search: the clock and half its size more on
/// every side, as (left, top, width, height).
type Area = (f32, f32, f32, f32);

/// A random point in `area`.
fn in_area(a: Area, rng: &mut impl Rng) -> Pos {
    (a.0 + rng.random::<f32>() * a.2, a.1 + rng.random::<f32>() * a.3)
}

/// A random point just outside `area`.
fn around_area(a: Area, rng: &mut impl Rng) -> Pos {
    let (x, y) = in_area(a, rng);
    match rng.random_range(0..4) {
        0 => (x, a.1 - 1.0),
        1 => (x, a.1 + a.3),
        2 => (a.0 - 1.0, y),
        _ => (a.0 + a.2, y),
    }
}

pub fn spotlights(sc: &Scene, rng: &mut impl Rng) -> Box<dyn Effect> {
    // tte: each light spawns off its canvas and searches along 10 curved
    // legs for 550 frames at 0.35-0.75 cells a frame. To keep that pace in
    // 2s, the lights search around the clock (an area half its size wider
    // on every side), spawn just outside that area, and take 2 legs.
    let (tw, th) = (sc.tw() as f32, sc.th() as f32);
    let area = (
        sc.ox as f32 - tw / 2.0,
        sc.oy as f32 - th / 2.0,
        2.0 * tw.max(1.0),
        2.0 * th.max(1.0),
    );
    let paths = (0..3)
        .map(|_| {
            let mut path = vec![(around_area(area, rng), (0.0, 0.0))];
            for _ in 0..2 {
                path.push((in_area(area, rng), around_area(area, rng)));
            }
            path
        })
        .collect();
    Box::new(Spotlights { paths })
}

impl Effect for Spotlights {
    fn draw(&mut self, sc: &Scene, t: f32, f: &mut Frame) -> bool {
        let center = (sc.ox as f32 + sc.tw() as f32 / 2.0, sc.oy as f32 + sc.th() as f32 / 2.0);
        // tte: the beam is min(width, height) / 2 of the text, dimming over
        // its outer 30%.
        let beam = (sc.tw().min(sc.th()) as f32 / 2.0).max(1.0);
        let reach = (sc.tw() as f32 / 2.0).hypot(sc.th() as f32) + 1.0;
        let lights: Vec<(f32, f32, f32)> = self
            .paths
            .iter()
            .map(|path| {
                let legs = (path.len() - 1) as f32;
                let s = (t / SEARCH).clamp(0.0, 1.0) * legs;
                let k = (s as usize).min(path.len() - 2);
                let ((a, _), (b, control)) = (path[k], path[k + 1]);
                let p = ease_in_out_quad(s - k as f32);
                let q = |a: f32, c: f32, b: f32| (1.0 - p).powi(2) * a + 2.0 * (1.0 - p) * p * c + p * p * b;
                let searching = (q(a.0, control.0, b.0), q(a.1, control.1, b.1));
                let g = ease_in_out_sine((t - SEARCH) / GATHER);
                let pos = (
                    searching.0 + (center.0 - searching.0) * g,
                    searching.1 + (center.1 - searching.1) * g,
                );
                let widen = ease_out_cubic((t - SEARCH - GATHER) / WIDEN);
                (pos.0, pos.1, beam + (reach - beam).max(0.0) * widen)
            })
            .collect();
        for i in 0..sc.len() {
            let Some(cell) = sc.settled(i) else { continue };
            let (x, y) = sc.pos(i);
            let lit = lights
                .iter()
                .map(|&(lx, ly, radius)| {
                    let d = ((x as f32 - lx) / 2.0).hypot(y as f32 - ly) / radius;
                    ((1.0 - d) / 0.3).clamp(0.0, 1.0)
                })
                .fold(0.0, f32::max);
            f.put(x, y, cell.ch, cell.fg.scale(0.12 + 0.88 * lit));
        }
        t > SEARCH + GATHER + WIDEN
    }
}

// --- laseretch ----------------------------------------------------------------

/// laseretch: a laser etches the text along a random maze path, its beam
/// running up and to the right to the top of the screen; each character
/// glows yellow, cools to orange and into its color, and throws a spark
/// that falls to the bottom of the screen.
pub struct LaserEtch {
    order: Vec<usize>,
    etch_at: Vec<f32>,
    /// Per cell: where its spark lands (column offset) and the height of
    /// its curve's control point (row offset).
    spark: Vec<(f32, f32)>,
}

const ETCH: f32 = 2.0;
/// tte: the spark cools through 4 colors of 7 frames and falls at 0.3 cells
/// a frame (capped here); a character spawns yellow, cools to orange in 8
/// steps of 3 frames, then to its color in 8 more.
const SPARK: f32 = 0.6;
const ETCH_COOL: f32 = 0.4;

pub fn laseretch(sc: &Scene, rng: &mut impl Rng) -> Box<dyn Effect> {
    let (tw, n) = (sc.tw(), sc.len());
    // tte's "algorithm" pattern: a recursive backtracker over the text area
    // from a random cell; blank cells are skipped.
    let mut order = Vec::with_capacity(n);
    if n > 0 {
        let neighbors = |i: usize| {
            let (r, c) = (i / tw, i % tw);
            [
                (r > 0).then(|| i - tw),
                (c + 1 < tw).then_some(i + 1),
                (i + tw < n).then_some(i + tw),
                (c > 0).then(|| i - 1),
            ]
            .into_iter()
            .flatten()
        };
        let mut seen = vec![false; n];
        let first = rng.random_range(0..n);
        seen[first] = true;
        let mut stack = vec![first];
        order.push(first);
        while let Some(&current) = stack.last() {
            let unvisited: Vec<usize> = neighbors(current).filter(|&j| !seen[j]).collect();
            if let Some(&next) = unvisited.choose(rng) {
                seen[next] = true;
                order.push(next);
                stack.push(next);
            } else {
                stack.pop();
            }
        }
        order.retain(|&i| sc.ch(i) != ' ');
    }
    let mut etch_at = vec![0.0; n];
    // tte etches a character every 2 frames; all of them within ETCH here.
    let step = (2.0 / 60.0f32).min(ETCH / order.len().max(1) as f32);
    for (k, &i) in order.iter().enumerate() {
        etch_at[i] = k as f32 * step;
    }
    let spark = (0..n)
        .map(|_| (rng.random_range(-20.0..=20.0), rng.random_range(-10.0..=20.0)))
        .collect();
    Box::new(LaserEtch { order, etch_at, spark })
}

impl Effect for LaserEtch {
    fn draw(&mut self, sc: &Scene, t: f32, f: &mut Frame) -> bool {
        let cool = const { hexes(["#ffe680", "#ff7b00"]) };
        let laser = const { hexes(["#ffffff", "#376cff"]) };
        let sparks = const { hexes(["#ffffff", "#ffe680", "#ff7b00", "#1a0900"]) };
        let bottom = (sc.sh - 1).max(0) as f32;
        let mut etching = None;
        for &i in &self.order {
            let Some(cell) = sc.settled(i) else { continue };
            let age = t - self.etch_at[i];
            if age < 0.0 {
                continue;
            }
            etching = Some(i);
            let (x, y) = sc.pos(i);
            let glow = if age < ETCH_COOL {
                palette(&cool, age / ETCH_COOL)
            } else {
                cool[1].lerp(cell.fg, (age - ETCH_COOL) / ETCH_COOL)
            };
            f.put(x, y, cell.ch, glow);
            // Its spark falls along a curve to the bottom of the screen.
            if age < SPARK {
                let (dx, lift) = self.spark[i];
                let p = ease_out_sine(age / SPARK);
                let (x0, y0) = (x as f32, y as f32);
                let (x1, y1) = (x0 + dx, bottom);
                let (cx, cy) = (x1, y0 + lift);
                let q = |a: f32, c: f32, b: f32| (1.0 - p).powi(2) * a + 2.0 * (1.0 - p) * p * c + p * p * b;
                f.put(
                    q(x0, cx, x1).round() as i32,
                    q(y0, cy, y1).round() as i32,
                    '*',
                    palette(&sparks, age / SPARK),
                );
            }
        }
        // The beam: from the character being etched, up and to the right
        // to the top of the screen, `*` at the tip.
        if let Some(i) = etching.filter(|_| t < max(&self.etch_at) + 1.0 / 30.0) {
            let (x, y) = sc.pos(i);
            for k in 0..=y.max(0) {
                let ch = if k == 0 { '*' } else { '/' };
                f.put(x + k, y - k, ch, palette(&laser, k as f32 / (y.max(1)) as f32));
            }
        }
        t > max(&self.etch_at) + SPARK.max(2.0 * ETCH_COOL)
    }
}

// --- overflow -----------------------------------------------------------------

/// overflow: jumbled copies of the text's rows scroll up the whole screen
/// from the bottom, overflowing, followed by the screen itself, blank rows
/// and all, which stops with the text in place.
pub struct Overflow {
    /// Source row for each line of the scroll; `None` marks the real rows.
    lines: Vec<Option<usize>>,
}

const SCROLL: f32 = 2.0;

pub fn overflow(sc: &Scene, rng: &mut impl Rng) -> Box<dyn Effect> {
    let th = sc.th().max(1);
    // tte: 2-4 cycles of the rows, shuffled each time; more on a tall
    // screen, so the jumbled rows fill it before the gap and the text.
    let mut lines = vec![];
    let mut cycles: u32 = rng.random_range(2..=4);
    while cycles > 0 || lines.len() < sc.sh.max(0) as usize {
        let mut rows: Vec<usize> = (0..th).collect();
        rows.shuffle(rng);
        lines.extend(rows.into_iter().map(Some));
        cycles = cycles.saturating_sub(1);
    }
    lines.extend(std::iter::repeat_n(None, th));
    Box::new(Overflow { lines })
}

impl Effect for Overflow {
    fn draw(&mut self, sc: &Scene, t: f32, f: &mut Frame) -> bool {
        let colors = const { hexes(["#f2ebc0", "#8dbfb3", "#f2ebc0"]) };
        let (tw, th) = (sc.tw(), sc.th());
        let jumbled = self.lines.len() - th;
        // tte scrolls the jumbled rows and then every row of the screen (its
        // canvas: blank rows above the text, the text, blank rows below) in
        // at the bottom, at about 40 rows a second, until the screen is
        // back in place. Line `k` of that stack, counted from its top:
        let above = sc.oy.max(0) as usize;
        let total = jumbled + sc.sh.max(0) as usize;
        let stop = total.saturating_sub(1) as f32;
        let offset = (t * 40.0f32.max(stop / SCROLL)).min(stop);
        for (n, line) in self.lines.iter().enumerate() {
            // The real rows sit `above` blank rows further down the stack.
            let k = if line.is_some() { n } else { n + above };
            let y = (sc.sh - 1) as f32 - (offset - k as f32);
            if y >= sc.sh as f32 {
                break;
            }
            let y = y.round() as i32;
            if let Some(src) = line {
                let col = palette(&colors, k as f32 / self.lines.len() as f32);
                for c in 0..tw {
                    f.put(sc.ox + c as i32, y, sc.ch(src * tw + c), col);
                }
            } else {
                let real = n - jumbled;
                for c in 0..tw {
                    if let Some(cell) = sc.settled(real * tw + c) {
                        f.put(sc.ox + c as i32, y, cell.ch, cell.fg);
                    }
                }
            }
        }
        t >= stop / 40.0f32.max(stop / SCROLL)
    }
}

// --- synthgrid ----------------------------------------------------------------

/// synthgrid: a neon grid grows over the screen; the blocks holding text
/// fill with flickering shades that resolve into it, a tenth at a time;
/// the grid then retracts.
pub struct SynthGrid {
    /// Screen rows of the horizontal lines and columns of the vertical ones.
    rows: Vec<i32>,
    columns: Vec<i32>,
    /// Per cell: when its block starts filling, and how long it flickers.
    fill: Vec<(f32, f32)>,
}

const GROW: f32 = 0.7;
const FILL_END: f32 = 2.5;
const RETRACT: f32 = 0.6;

/// tte's `find_even_gap`: of the gaps above 4 that split `dimension - 2`
/// evenly (or nearly), the one closest to a fifth of it.
fn find_even_gap(dimension: i32) -> i32 {
    let d = dimension - 2;
    if d <= 0 {
        return 0;
    }
    let target = d / 5;
    (5..=d)
        .rev()
        .filter(|i| d % i <= 1)
        .min_by_key(|g| (g - target).abs())
        .unwrap_or(4)
}

pub fn synthgrid(sc: &Scene, rng: &mut impl Rng) -> Box<dyn Effect> {
    let (w, h) = (sc.sw, sc.sh);
    // tte: the gap follows the longer side; rows are half as far apart.
    let (row_gap, column_gap) = if h > 2 * w {
        let g = find_even_gap(h) + 1;
        (g, g * 2)
    } else {
        let g = find_even_gap(w) + 1;
        (g / 2, g)
    };
    // The border, then lines every gap from the bottom left, not within 2
    // of the far edge.
    let mut rows = vec![0, h - 1];
    let mut y = h - 1 - row_gap.max(1);
    while y > 0 {
        if y >= 2 {
            rows.push(y);
        }
        y -= row_gap.max(1);
    }
    let mut columns = vec![0, w - 1];
    let mut x = column_gap.max(1);
    while x < w - 1 {
        if w - 1 - x >= 2 {
            columns.push(x);
        }
        x += column_gap.max(1);
    }
    rows.sort_unstable();
    columns.sort_unstable();
    // The blocks with text, filled in random order (tte: a tenth active at once).
    let block = |i: usize| {
        let (x, y) = sc.pos(i);
        (columns.partition_point(|&c| c <= x), rows.partition_point(|&r| r <= y))
    };
    let mut blocks: Vec<(usize, usize)> = ink(sc).into_iter().map(block).collect();
    blocks.sort_unstable();
    blocks.dedup();
    blocks.shuffle(rng);
    let span = FILL_END - GROW - 0.5;
    let fill = (0..sc.len())
        .map(|i| {
            let rank = blocks.iter().position(|&b| b == block(i)).unwrap_or(0);
            let start = GROW + rank as f32 / blocks.len().max(1) as f32 * span;
            // tte: 15-30 frames of flicker.
            (start, rng.random_range(15.0..=30.0) / 60.0)
        })
        .collect();
    Box::new(SynthGrid { rows, columns, fill })
}

impl Effect for SynthGrid {
    fn draw(&mut self, sc: &Scene, t: f32, f: &mut Frame) -> bool {
        const SHADES: [char; 3] = ['░', '▒', '▓'];
        let grid = const { hexes(["#CC00CC", "#ffffff"]) };
        let text = const { hexes(["#8A008A", "#00D1FF", "#FFFFFF"]) };
        let (w, h) = (sc.sw, sc.sh);
        // The grid grows from the left and from the bottom, then retracts.
        let reach = if t < FILL_END {
            ease_out_cubic(t / GROW)
        } else {
            1.0 - ease_out_cubic((t - FILL_END) / RETRACT)
        };
        let shade = |x: i32, y: i32| palette(&grid, (x + h - 1 - y) as f32 / (w + h).max(1) as f32);
        let across = (reach * w as f32) as i32;
        let up = (reach * h as f32) as i32;
        for &y in &self.rows {
            for x in 0..across {
                let ch = if self.columns.binary_search(&x).is_ok() {
                    '┼'
                } else {
                    '─'
                };
                f.put(x, y, ch, shade(x, y));
            }
        }
        for &x in &self.columns {
            for y in (h - up..h).filter(|y| self.rows.binary_search(y).is_err()) {
                f.put(x, y, '│', shade(x, y));
            }
        }
        // The text on top, block by block.
        for (i, &(start, flicker)) in self.fill.iter().enumerate() {
            let Some(cell) = sc.settled(i) else { continue };
            let lt = t - start;
            if lt < 0.0 {
                continue;
            }
            let (x, y) = sc.pos(i);
            if lt < flicker {
                let h = hash(i as u64 ^ ((t * 30.0) as u64) << 24);
                f.put(x, y, SHADES[(h % 3) as usize], palette(&text, hash01(h ^ 1)));
            } else {
                f.put(x, y, cell.ch, cell.fg);
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
        let bolt = const { Rgb::hex("#68A3E8") };
        let glow = const { Rgb::hex("#EF5411") };
        let spark = const { Rgb::hex("#ff4d00") };
        let rain = const { Rgb::hex("#aaaaff") };
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
        let colors = const { hexes(["#ffffff", "#ff0000", "#00ff00", "#0000ff", "#ffffff"]) };
        let noise = const { hexes(["#1e1e1f", "#3c3b3d", "#6d6c70", "#a2a1a6", "#cbc9cf"]) };
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

// --- wipe ---------------------------------------------------------------------

/// wipe: a diagonal wipe from the top left reveals the text, each character
/// shifting from the gradient's first color to its own.
pub struct Wipe {
    /// When each diagonal (row + column) appears; tte paces them in-out-circ.
    at: Vec<f32>,
}

const WIPE: f32 = 1.2;
const WIPE_FADE: f32 = 0.6;

pub fn wipe(sc: &Scene, _rng: &mut impl Rng) -> Box<dyn Effect> {
    let span = (sc.tw() + sc.th()).saturating_sub(2).max(1);
    let at = (0..=span)
        .map(|d| WIPE * ease_inverse(ease_in_out_circ, d as f32 / span as f32))
        .collect();
    Box::new(Wipe { at })
}

impl Effect for Wipe {
    fn draw(&mut self, sc: &Scene, t: f32, f: &mut Frame) -> bool {
        let first = sc.theme.gradient_at(0.0);
        for i in 0..sc.len() {
            let Some(cell) = sc.settled(i) else { continue };
            let (r, c) = sc.rc(i);
            let at = self.at[(r + c).min(self.at.len() - 1)];
            if t >= at {
                let (x, y) = sc.pos(i);
                f.put(x, y, cell.ch, first.lerp(cell.fg, (t - at) / WIPE_FADE));
            }
        }
        t > WIPE + WIPE_FADE
    }
}

// --- beams --------------------------------------------------------------------

/// beams: beams race along every row and column of the screen in random
/// groups, leaving the text dimmed, then a diagonal wipe brightens it.
pub struct Beams {
    /// Per screen row, then per screen column: when its beam starts, its
    /// speed (cells a second) and whether it runs backwards.
    rows: Vec<(f32, f32, bool)>,
    columns: Vec<(f32, f32, bool)>,
    /// When the final wipe starts, and how many diagonals it reaches a second.
    wipe_at: f32,
    wipe_rate: f32,
}

const BEAM: f32 = 16.0 / 60.0;
const BEAM_FADE: f32 = 20.0 / 60.0;
const BEAM_BRIGHTEN: f32 = 0.5;

pub fn beams(sc: &Scene, rng: &mut impl Rng) -> Box<dyn Effect> {
    let (sw, sh) = (sc.sw.max(0) as usize, sc.sh.max(0) as usize);
    // Every row and column of the screen (tte's canvas), in random order.
    let mut groups: Vec<(bool, usize)> = (0..sh).map(|y| (true, y)).chain((0..sw).map(|x| (false, x))).collect();
    groups.shuffle(rng);
    let mut rows = vec![(0.0, 1.0, false); sh];
    let mut columns = vec![(0.0, 1.0, false); sw];
    // tte releases 1-5 beams every 6 frames; bigger batches keep a big
    // screen's beams within about 1.2s.
    let batch_min = groups.len().div_ceil(12).max(1);
    let (mut released, mut at, mut last) = (0, 0.0, 0.0f32);
    while released < groups.len() {
        let batch = rng.random_range(1..=5).max(batch_min);
        for &(is_row, k) in groups.iter().skip(released).take(batch) {
            // tte: 1.5-6 cells a frame along a row, 0.9-1.5 down a column;
            // at least fast enough to cross the screen in 0.6s.
            let (len, speed) = if is_row {
                (sw, rng.random_range(90.0..360.0f32).max(sw as f32 / 0.6))
            } else {
                (sh, rng.random_range(54.0..90.0f32).max(sh as f32 / 0.6))
            };
            let beam = (at, speed, rng.random());
            if is_row {
                rows[k] = beam;
            } else {
                columns[k] = beam;
            }
            last = last.max(at + len as f32 / speed);
        }
        released += batch;
        at += 0.1;
    }
    let span = (sc.tw() + sc.th()).max(1) as f32;
    Box::new(Beams {
        rows,
        columns,
        wipe_at: last + BEAM + BEAM_FADE,
        // tte wipes 3 diagonals a frame; at most 0.6s on a big clock.
        wipe_rate: (3.0 * 60.0f32).max(span / 0.6),
    })
}

impl Effect for Beams {
    fn draw(&mut self, sc: &Scene, t: f32, f: &mut Frame) -> bool {
        const ROW: [char; 3] = ['▂', '▁', '_'];
        const COLUMN: [char; 4] = ['▌', '▍', '▎', '▏'];
        let (white, cyan, purple) = const { (Rgb::hex("#ffffff"), Rgb::hex("#00D1FF"), Rgb::hex("#8A008A")) };
        let (sw, sh) = (self.columns.len(), self.rows.len());
        // When the beam along a line passes cell `pos` of it.
        let hit = |(start, speed, reverse): (f32, f32, bool), pos: usize, len: usize| {
            start + if reverse { len - 1 - pos } else { pos } as f32 / speed
        };
        for y in 0..sh {
            for x in 0..sw {
                // The latest beam to have passed, row or column.
                let latest = [(hit(self.rows[y], x, sw), true), (hit(self.columns[x], y, sh), false)]
                    .into_iter()
                    .filter(|&(h, _)| h <= t)
                    .max_by(|a, b| a.0.total_cmp(&b.0));
                let Some((h, along_row)) = latest else { continue };
                let since = t - h;
                let (sx, sy) = (x as i32, y as i32);
                if since < BEAM {
                    let p = since / BEAM;
                    // tte's beam gradient: white to cyan in 2 steps, cyan to purple in 6.
                    let col = if p < 0.25 {
                        white.lerp(cyan, p / 0.25)
                    } else {
                        cyan.lerp(purple, (p - 0.25) / 0.75)
                    };
                    let symbols: &[char] = if along_row { &ROW } else { &COLUMN };
                    f.put(
                        sx,
                        sy,
                        symbols[((p * symbols.len() as f32) as usize).min(symbols.len() - 1)],
                        col,
                    );
                    continue;
                }
                // Past the beam: the clock's characters dim, until the wipe.
                let (c, r) = (sx - sc.ox, sy - sc.oy);
                if !(0..sc.tw() as i32).contains(&c) || !(0..sc.th() as i32).contains(&r) {
                    continue;
                }
                let Some(cell) = sc.settled(r as usize * sc.tw() + c as usize) else {
                    continue;
                };
                let dim = cell.fg.brightness(0.3);
                let wiped = self.wipe_at + (r + c) as f32 / self.wipe_rate;
                let col = if t >= wiped {
                    dim.lerp(cell.fg, (t - wiped) / BEAM_BRIGHTEN)
                } else {
                    cell.fg.lerp(dim, (since - BEAM) / BEAM_FADE)
                };
                f.put(sx, sy, cell.ch, col);
            }
        }
        t > self.wipe_at + (sc.tw() + sc.th()) as f32 / self.wipe_rate + BEAM_BRIGHTEN
    }
}

/// tte's `PrimsSimple` over the canvas grid: a random spanning tree grown
/// from `first`, linking one neighbour a step. Returns the cells in the
/// order they're linked, and each cell's linked neighbours.
fn prims_simple(tw: usize, n: usize, first: usize, rng: &mut impl Rng) -> (Vec<usize>, Vec<Vec<usize>>) {
    let mut order = Vec::with_capacity(n);
    let mut links = vec![vec![]; n];
    if n == 0 {
        return (order, links);
    }
    let neighbors = |i: usize| {
        let (r, c) = (i / tw, i % tw);
        [
            (r > 0).then(|| i - tw),
            (c + 1 < tw).then_some(i + 1),
            (i + tw < n).then_some(i + tw),
            (c > 0).then(|| i - 1),
        ]
        .into_iter()
        .flatten()
    };
    let mut linked = vec![false; n];
    linked[first] = true;
    order.push(first);
    let mut edge = vec![first];
    while !edge.is_empty() {
        let current = edge.remove(rng.random_range(0..edge.len()));
        let mut unlinked: Vec<usize> = neighbors(current).filter(|&j| !linked[j]).collect();
        if unlinked.is_empty() {
            continue;
        }
        let next = unlinked.remove(rng.random_range(0..unlinked.len()));
        linked[next] = true;
        links[current].push(next);
        links[next].push(current);
        order.push(next);
        if !unlinked.is_empty() {
            edge.push(current);
        }
        if neighbors(next).any(|j| !linked[j]) {
            edge.push(next);
        }
    }
    (order, links)
}

// --- burn ---------------------------------------------------------------------

/// burn: the gray text catches fire from a random spot, the fire spreading
/// cell to cell; each character burns through, cools into its color, and
/// may send up a wisp of smoke.
pub struct Burn {
    ignite: Vec<f32>,
    /// Smoke from a burnt cell: where it drifts (column offset), its symbol.
    smoke: Vec<Option<(f32, char)>>,
}

const IGNITE: f32 = 1.5;
const BURN: f32 = 0.6;
const BURN_COOL: f32 = 0.5;
const BURN_SMOKE: f32 = 0.9;

pub fn burn(sc: &Scene, rng: &mut impl Rng) -> Box<dyn Effect> {
    let (tw, n) = (sc.tw(), sc.len());
    // tte's spread: PrimsSimple from a random character in the middle of
    // the text.
    let middle: Vec<usize> = (0..n)
        .filter(|&i| sc.ch(i) != ' ' && (tw / 4..tw - tw / 4).contains(&(i % tw)))
        .collect();
    let first = middle
        .choose(rng)
        .copied()
        .unwrap_or_else(|| rng.random_range(0..n.max(1)));
    let (order, _) = prims_simple(tw, n, first, rng);
    let mut ignite = vec![0.0; n];
    for (rank, &i) in order.iter().enumerate() {
        ignite[i] = rank as f32 / n.max(1) as f32 * IGNITE;
    }
    let smoke = (0..n)
        .map(|_| {
            rng.random_bool(0.5).then(|| {
                (
                    rng.random_range(-4.0..=4.0),
                    ['.', ',', '\'', '`', '#', '*'][rng.random_range(0..6)],
                )
            })
        })
        .collect();
    Box::new(Burn { ignite, smoke })
}

impl Effect for Burn {
    fn draw(&mut self, sc: &Scene, t: f32, f: &mut Frame) -> bool {
        const SYMBOLS: [char; 9] = ['\'', '.', '▖', '▙', '█', '▜', '▀', '▝', '.'];
        let gray = const { Rgb::hex("#837373") };
        let fire = const { hexes(["#ffffff", "#fff75d", "#fe650d", "#8A003C", "#510100"]) };
        let (smoke_dark, smoke_light) = const { (Rgb::hex("#504F4F"), Rgb::hex("#C7C7C7")) };
        for (i, &ignite) in self.ignite.iter().enumerate() {
            let Some(cell) = sc.settled(i) else { continue };
            let lt = t - ignite;
            let (x, y) = sc.pos(i);
            if lt < 0.0 {
                f.put(x, y, cell.ch, gray);
            } else if lt < BURN {
                let p = lt / BURN;
                f.put(x, y, SYMBOLS[((p * 9.0) as usize).min(8)], palette(&fire, p));
            } else {
                f.put(x, y, cell.ch, fire[4].lerp(cell.fg, (lt - BURN) / BURN_COOL));
            }
        }
        // Smoke on top: it rises to the top of the screen (30 rows a second
        // in tte) and lightens as it goes.
        for (i, (&ignite, &smoke)) in self.ignite.iter().zip(&self.smoke).enumerate() {
            let (Some((dx, ch)), Some(_)) = (smoke, sc.settled(i)) else {
                continue;
            };
            let lt = t - ignite - BURN;
            if !(0.0..BURN_SMOKE).contains(&lt) {
                continue;
            }
            let (x, y) = sc.pos(i);
            let rise = (lt * 30.0 / y.max(1) as f32).min(1.0);
            let sx = x as f32 + dx * rise;
            let sy = y as f32 * (1.0 - rise);
            f.put(
                sx.round() as i32,
                sy.round() as i32,
                ch,
                smoke_dark.lerp(smoke_light, lt / BURN_SMOKE),
            );
        }
        t > IGNITE + BURN + BURN_COOL.max(BURN_SMOKE)
    }
}

// --- decrypt ------------------------------------------------------------------

/// decrypt: green ciphertext is typed out, flickers fast, then slowly, and
/// each character is discovered, fading from white into its color.
pub struct Decrypt {
    typed: Vec<f32>,
    green: Vec<Rgb>,
    /// How long each character's slow flipping holds each symbol.
    holds: Vec<Vec<f32>>,
    /// When the decrypting starts, after the typing.
    start: f32,
    seed: u64,
}

const DECRYPT_TYPE_ANIM: f32 = 8.0 / 60.0;
const DECRYPT_FAST: f32 = 0.7;
const DECRYPT_FOUND: f32 = 0.5;

pub fn decrypt(sc: &Scene, rng: &mut impl Rng) -> Box<dyn Effect> {
    let greens = const { hexes(["#008000", "#00cb00", "#00ff00"]) };
    let n = sc.len();
    let order: Vec<usize> = (0..n).filter(|&i| sc.ch(i) != ' ').collect();
    // tte types about 90 characters a second; at most 0.8s here.
    let typing = (order.len() as f32 / 90.0).min(0.8);
    let mut typed = vec![0.0; n];
    for (rank, &i) in order.iter().enumerate() {
        typed[i] = rank as f32 / order.len().max(1) as f32 * typing;
    }
    let holds = (0..n)
        .map(|_| {
            // tte: 1-15 symbols, mostly short holds, 30% long ones; at most 1.3s here.
            let mut left = 1.3f32;
            (0..rng.random_range(1..=6))
                .map(|_| {
                    let hold = if rng.random_bool(0.3) {
                        rng.random_range(0.3f32..0.5)
                    } else {
                        rng.random_range(0.05..0.1)
                    }
                    .min(left);
                    left -= hold;
                    hold
                })
                .collect()
        })
        .collect();
    Box::new(Decrypt {
        typed,
        green: (0..n).map(|_| greens[rng.random_range(0..3)]).collect(),
        holds,
        start: typing + DECRYPT_TYPE_ANIM,
        seed: rng.random(),
    })
}

/// tte's ciphertext: printable ASCII, block elements, box drawing and Latin letters.
fn cipher(h: u64) -> char {
    const RANGES: [(u32, u32); 4] = [(33, 94), (0x2588, 24), (0x2500, 127), (174, 278)];
    let mut k = (hash(h) % 523) as u32;
    for (from, len) in RANGES {
        if k < len {
            return char::from_u32(from + k).unwrap_or('?');
        }
        k -= len;
    }
    '?'
}

impl Effect for Decrypt {
    fn draw(&mut self, sc: &Scene, t: f32, f: &mut Frame) -> bool {
        const TYPING: [char; 4] = ['▉', '▓', '▒', '░'];
        let mut end = self.start + DECRYPT_FAST;
        for i in 0..sc.len() {
            let slow: f32 = self.holds[i].iter().sum();
            end = end.max(self.start + DECRYPT_FAST + slow + DECRYPT_FOUND);
            let Some(cell) = sc.settled(i) else { continue };
            let lt = t - self.typed[i];
            if lt < 0.0 {
                continue;
            }
            let (x, y) = sc.pos(i);
            let id = self.seed ^ (i as u64) << 16;
            let (ch, col) = if t < self.start {
                let k = (lt / DECRYPT_TYPE_ANIM * 4.0) as usize;
                (TYPING.get(k).copied().unwrap_or_else(|| cipher(id)), self.green[i])
            } else if t - self.start < DECRYPT_FAST {
                (cipher(id ^ ((t - self.start) * 30.0) as u64), self.green[i])
            } else {
                let d = t - self.start - DECRYPT_FAST;
                if d < slow {
                    let mut at = 0.0;
                    let k = self.holds[i].iter().take_while(|&&h| {
                        at += h;
                        at <= d
                    });
                    (cipher(id ^ (k.count() as u64 + 1) << 40), self.green[i])
                } else {
                    (cell.ch, WHITE.lerp(cell.fg, (d - slow) / DECRYPT_FOUND))
                }
            };
            f.put(x, y, ch, col);
        }
        t > end
    }
}

// --- print --------------------------------------------------------------------

/// print: each row is typed on the bottom line, every character passing
/// through block shades from white to its color; at the end of a row the
/// paper feeds up a line, and the print head appears and slides back to
/// where the next row starts, then disappears.
pub struct Print {
    /// Per row: when typing starts, and its first and last non-blank
    /// columns (`None`: a blank row).
    rows: Vec<(f32, Option<(usize, usize)>)>,
    /// Characters typed a second.
    rate: f32,
    /// Per row: how long the head takes back to the next row (0: no return).
    returns: Vec<f32>,
}

const PRINT_SHADE: f32 = 15.0 / 60.0;
/// The head's returns together take at most this long. They don't count
/// towards `Kind::max_duration`: the rows are readable by then.
pub const PRINT_RETURNS: f32 = 2.0;

pub fn print(sc: &Scene, _rng: &mut impl Rng) -> Box<dyn Effect> {
    let (tw, th) = (sc.tw(), sc.th());
    let extents: Vec<Option<(usize, usize)>> = (0..th)
        .map(|r| {
            let ink = |c: &usize| sc.ch(r * tw + c) != ' ';
            Some(((0..tw).find(ink)?, (0..tw).rev().find(ink)?))
        })
        .collect();
    let total: usize = extents.iter().flatten().map(|(a, b)| b - a + 1).sum();
    // tte types 2 characters a frame; faster on a big clock so it stays short.
    let rate = 120.0f32.max(total as f32 / 1.8);
    // tte's head returns at 1.5 cells a frame, eased, between two rows with
    // text; all of them capped together on a tall clock.
    let mut returns: Vec<f32> = (0..th)
        .map(|r| match (extents[r], extents.get(r + 1).copied().flatten()) {
            (Some((_, last)), Some((first, _))) => (last.abs_diff(first) as f32 / 90.0).max(0.1),
            _ => 0.0,
        })
        .collect();
    let sum: f32 = returns.iter().sum();
    if sum > PRINT_RETURNS {
        for d in &mut returns {
            *d *= PRINT_RETURNS / sum;
        }
    }
    let mut at = 0.0;
    let rows = extents
        .into_iter()
        .zip(&returns)
        .map(|(extent, back)| {
            let start = at;
            // A blank row is one keystroke in tte, then the paper feeds.
            at += extent.map_or(1, |(a, b)| b - a + 1) as f32 / rate + back;
            (start, extent)
        })
        .collect();
    Box::new(Print { rows, rate, returns })
}

impl Print {
    /// When row `r` is typed to its end (and the paper feeds).
    fn done(&self, r: usize) -> f32 {
        let (start, extent) = self.rows[r];
        start + extent.map_or(1, |(a, b)| b - a + 1) as f32 / self.rate
    }
}

impl Effect for Print {
    fn draw(&mut self, sc: &Scene, t: f32, f: &mut Frame) -> bool {
        const SHADES: [char; 4] = ['█', '▓', '▒', '░'];
        let (tw, th) = (sc.tw(), self.rows.len());
        let bottom = sc.oy + th as i32 - 1;
        // tte: each finished row but the last feeds the paper up a line.
        let feeds = |r: usize| (r..th.saturating_sub(1)).filter(|&k| self.done(k) <= t).count() as i32;
        for (r, &(start, extent)) in self.rows.iter().enumerate() {
            let Some((first, last)) = extent else { continue };
            let y = bottom - feeds(r);
            for c in first..=last {
                let lt = t - start - (c - first) as f32 / self.rate;
                if lt < 0.0 {
                    break;
                }
                let i = r * tw + c;
                let x = sc.pos(i).0;
                let p = lt / PRINT_SHADE;
                let fg = sc.settled(i).map_or(sc.theme.dim, |cell| cell.fg);
                let col = WHITE.lerp(fg, p);
                match SHADES.get((p * 5.0) as usize) {
                    Some(&shade) => f.put(x, y, shade, col),
                    None => {
                        if let Some(cell) = sc.settled(i) {
                            f.put(x, y, cell.ch, col);
                        }
                    }
                }
            }
        }
        // The head shows only on its way back, on the bottom line, from the
        // end of the row just typed to the start of the next one.
        for r in 0..th {
            let (done, back) = (self.done(r), self.returns[r]);
            if back <= 0.0 || !(done..done + back).contains(&t) {
                continue;
            }
            let (Some((_, from)), Some((to, _))) = (self.rows[r].1, self.rows[r + 1].1) else {
                continue;
            };
            let p = ease_in_out_quad((t - done) / back);
            let x = from as f32 + (to as f32 - from as f32) * p;
            f.put(sc.ox + x.round() as i32, bottom, '█', WHITE);
        }
        let end = th.checked_sub(1).map_or(0.0, |r| self.done(r));
        t > end + PRINT_SHADE
    }
}

// --- waves --------------------------------------------------------------------

/// waves: column by column, waves of rising and falling blocks wash through
/// the text in tte's yellow, orange and blue, then settle into its colors.
pub struct Waves;

const WAVE_COLUMNS: f32 = 0.8;
const WAVE_TIME: f32 = 1.4;
const WAVE_COUNT: f32 = 3.0;
const WAVE_FADE: f32 = 0.6;

impl Effect for Waves {
    fn draw(&mut self, sc: &Scene, t: f32, f: &mut Frame) -> bool {
        const SYMBOLS: [char; 15] = [
            '▁', '▂', '▃', '▄', '▅', '▆', '▇', '█', '▇', '▆', '▅', '▄', '▃', '▂', '▁',
        ];
        let colors = const { hexes(["#f0ff65", "#ffb102", "#31a0d4", "#ffb102", "#f0ff65"]) };
        let tw = sc.tw().max(1) as f32;
        for i in 0..sc.len() {
            let Some(cell) = sc.settled(i) else { continue };
            let c = sc.rc(i).1 as f32;
            // tte starts a column a frame; spread over at most WAVE_COLUMNS.
            let lt = t - (c / 60.0).min(c / tw * WAVE_COLUMNS);
            if lt < 0.0 {
                continue;
            }
            let (x, y) = sc.pos(i);
            if lt < WAVE_TIME {
                let k = (ease_in_out_sine(lt / WAVE_TIME) * WAVE_COUNT).fract();
                f.put(x, y, SYMBOLS[((k * 15.0) as usize).min(14)], palette(&colors, k));
            } else {
                f.put(x, y, cell.ch, colors[4].lerp(cell.fg, (lt - WAVE_TIME) / WAVE_FADE));
            }
        }
        t > WAVE_COLUMNS + WAVE_TIME + WAVE_FADE
    }
}

// --- matrix -------------------------------------------------------------------

/// matrix: digital rain falls over the screen, then the columns fill up with
/// symbols and the text resolves out of them as the rest drops away.
/// `matrix-heavy` rains denser with ever-changing symbols; the light one
/// keeps its symbols still, writing far less.
pub struct Matrix {
    heavy: bool,
    drops: Vec<RainDrop>,
    last: f32,
    seed: u64,
    /// Per column of the area: when it starts filling, and rows a second.
    fill: Vec<(f32, f32)>,
    resolve_at: f32,
}

struct RainDrop {
    x: i32,
    y: f32,
    speed: f32,
    len: i32,
    seed: u64,
}

const MATRIX_RAIN: f32 = 1.6;
const MATRIX_HOLD: f32 = 0.3;
const MATRIX_RESOLVE: f32 = 1.0;
const MATRIX_SETTLE: f32 = 0.4;
const MATRIX_SYMBOLS: [char; 50] = [
    '2', '5', '9', '8', 'Z', '*', ')', ':', '.', '"', '=', '+', '-', '¦', '|', '_', 'ｦ', 'ｱ', 'ｳ', 'ｴ', 'ｵ', 'ｶ', 'ｷ',
    'ｹ', 'ｺ', 'ｻ', 'ｼ', 'ｽ', 'ｾ', 'ｿ', 'ﾀ', 'ﾂ', 'ﾃ', 'ﾅ', 'ﾆ', 'ﾇ', 'ﾈ', 'ﾊ', 'ﾋ', 'ﾎ', 'ﾏ', 'ﾐ', 'ﾑ', 'ﾒ', 'ﾓ', 'ﾔ',
    'ﾕ', 'ﾗ', 'ﾘ', 'ﾜ',
];

pub fn matrix(sc: &Scene, rng: &mut impl Rng, heavy: bool) -> Box<dyn Effect> {
    let (x0, x1, y0, y1) = (0, sc.sw, 0, sc.sh);
    let h = (y1 - y0).max(1) as f32;
    // tte fills a column a row every 1-5 frames; faster on a tall area.
    let fill: Vec<(f32, f32)> = (x0..x1)
        .map(|_| {
            (
                MATRIX_RAIN + rng.random_range(0.0..0.3),
                rng.random_range(12.0f32..60.0).max(h / 0.7),
            )
        })
        .collect();
    let filled_by = fill.iter().map(|&(s, v)| s + h / v).fold(MATRIX_RAIN, f32::max);
    Box::new(Matrix {
        heavy,
        drops: vec![],
        last: 0.0,
        seed: rng.random(),
        fill,
        resolve_at: filled_by + MATRIX_HOLD,
    })
}

impl Effect for Matrix {
    fn draw(&mut self, sc: &Scene, t: f32, f: &mut Frame) -> bool {
        let (light, dark, head) = const { (Rgb::hex("#92be92"), Rgb::hex("#185318"), Rgb::hex("#dbffdb")) };
        let rain_color = |h: u64| light.lerp(dark, hash01(h));
        let dt = (t - self.last).clamp(0.0, 0.1);
        self.last = t;
        let (x0, x1, y0, y1) = (0, sc.sw, 0, sc.sh);
        let width = (x1 - x0).max(0);
        if t < MATRIX_RAIN && width > 0 {
            let n = width as f32 * dt * if self.heavy { 1.3 } else { 0.55 };
            let k = n as usize + usize::from(hash01(self.seed ^ (t * 1000.0) as u64) < n.fract());
            for j in 0..k {
                let s = hash(self.seed ^ ((t * 1000.0) as u64) << 8 ^ j as u64);
                self.drops.push(RainDrop {
                    x: x0 + (s % width as u64) as i32,
                    y: y0 as f32 - 1.0,
                    speed: 15.0 + hash01(s ^ 1) * 25.0,
                    len: 4 + (hash(s ^ 2) % 12) as i32,
                    seed: s,
                });
            }
        }
        if t < self.resolve_at {
            for d in &mut self.drops {
                d.y += d.speed * dt;
                let hy = d.y.floor() as i32;
                for k in 0..d.len {
                    let y = hy - k;
                    if y < y0 || y >= y1 {
                        continue;
                    }
                    // Heavy: every trail cell keeps changing; light: only the head.
                    let churn = if self.heavy || k == 0 {
                        (t * (6.0 + k as f32)) as u64
                    } else {
                        0
                    };
                    let sym = MATRIX_SYMBOLS[(hash(d.seed ^ (y as u64) << 16 ^ churn) % 50) as usize];
                    let col = if k == 0 { head } else { rain_color(d.seed ^ y as u64) };
                    f.put(d.x, y, sym, col);
                }
            }
            self.drops.retain(|d| (d.y as i32 - d.len) < y1);
        }
        for (cx, &(start, speed)) in self.fill.iter().enumerate() {
            if t < start {
                continue;
            }
            let x = x0 + cx as i32;
            let filled = ((t - start) * speed) as i32;
            for y in y0..y1.min(y0 + filled + 1) {
                let id = self.seed ^ (x as u64) << 20 ^ y as u64;
                let resolve = self.resolve_at + hash01(id ^ 0x5eed) * MATRIX_RESOLVE;
                let (c, r) = (x - sc.ox, y - sc.oy);
                let cell = ((0..sc.tw() as i32).contains(&c) && (0..sc.th() as i32).contains(&r))
                    .then(|| r as usize * sc.tw() + c as usize)
                    .and_then(|i| sc.settled(i));
                if t >= resolve {
                    // The text stays and takes its colors; the rest drops away.
                    if let Some(cell) = cell {
                        f.put(x, y, cell.ch, head.lerp(cell.fg, (t - resolve) / MATRIX_SETTLE));
                    }
                    continue;
                }
                let churn = if self.heavy { (t * 8.0) as u64 } else { 0 };
                let sym = MATRIX_SYMBOLS[(hash(id ^ churn << 32) % 50) as usize];
                let col = if y == y0 + filled { head } else { rain_color(id) };
                f.put(x, y, sym, col);
            }
        }
        t > self.resolve_at + MATRIX_RESOLVE + MATRIX_SETTLE
    }
}
