//! Effects where characters travel into place.
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
    util::{
        ease_in_expo, ease_in_out_back, ease_in_out_expo, ease_in_out_sine, ease_in_quad, ease_out_bounce,
        ease_out_expo, ease_out_sine, hash,
    },
};

type Pos = (f32, f32);

const WHITE: Rgb = Rgb(255, 255, 255);

/// Indices of the cells that currently show something.
fn ink(sc: &Scene) -> Vec<usize> {
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

fn random_screen_pos(sc: &Scene, rng: &mut impl Rng) -> Pos {
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

fn hexes(list: &[&str]) -> Vec<Rgb> {
    list.iter().map(|h| Rgb::hex(h)).collect()
}

/// Pick from a color list by a fraction in 0..1, blending neighbours.
fn palette(colors: &[Rgb], u: f32) -> Rgb {
    let x = u.clamp(0.0, 1.0) * (colors.len() - 1) as f32;
    let i = (x as usize).min(colors.len() - 1);
    colors[i].lerp(colors[(i + 1).min(colors.len() - 1)], x.fract())
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
            end,
        }
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
            if p < 1.0 {
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

/// scattered: characters start anywhere and swing into place.
pub fn scattered(sc: &Scene, rng: &mut impl Rng) -> Box<dyn Effect> {
    let n = sc.len();
    let start = (0..n).map(|_| random_screen_pos(sc, rng)).collect();
    let color = (0..n).map(|_| sc.theme.gradient_at(rng.random())).collect();
    Box::new(Travel::new(start, vec![0.0; n], vec![1.6; n], ease_in_out_back, color))
}

/// spray: characters are sprayed from the right edge, one after another.
pub fn spray(sc: &Scene, rng: &mut impl Rng) -> Box<dyn Effect> {
    let n = sc.len();
    let source = ((sc.sw - 1) as f32, center(sc).1);
    let mut order: Vec<usize> = (0..n).collect();
    order.shuffle(rng);
    let mut delay = vec![0.0; n];
    for (rank, &i) in order.iter().enumerate() {
        delay[i] = rank as f32 / n.max(1) as f32 * 1.4;
    }
    let dur = (0..n).map(|_| rng.random_range(0.6..1.1)).collect();
    Box::new(Travel::new(vec![source; n], delay, dur, ease_out_expo, vec![WHITE; n]))
}

/// slice: the left half slides down from above, the right half up from below.
pub fn slice(sc: &Scene, _rng: &mut impl Rng) -> Box<dyn Effect> {
    let n = sc.len();
    let start = (0..n)
        .map(|i| {
            let (x, y) = target(sc, i);
            let from_top = sc.rc(i).1 < sc.tw() / 2;
            (x, if from_top { y - sc.sh as f32 } else { y + sc.sh as f32 })
        })
        .collect();
    let color = (0..n).map(|_| sc.theme.flash).collect();
    Box::new(Travel::new(start, vec![0.0; n], vec![1.2; n], ease_in_out_expo, color))
}

/// pour: rows are poured in from the top, bottom row first, each row back
/// and forth.
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
    let start = (0..n).map(|i| (target(sc, i).0, -1.0)).collect();
    let dur = (0..n).map(|_| rng.random_range(0.4..0.65)).collect();
    Box::new(Travel::new(start, delay, dur, ease_in_quad, vec![WHITE; n]))
}

/// bouncyballs: characters drop from above as balls and bounce into place.
pub fn bouncyballs(sc: &Scene, rng: &mut impl Rng) -> Box<dyn Effect> {
    let n = sc.len();
    let colors = hexes(&["#d1f4a5", "#96e2a4", "#5acda9"]);
    let th = sc.th() as f32;
    let start = (0..n)
        .map(|i| (target(sc, i).0, -1.0 - rng.random_range(0.0..4.0)))
        .collect();
    let delay = (0..n)
        .map(|i| (th - 1.0 - sc.rc(i).0 as f32) / th * 1.1 + rng.random_range(0.0..0.35))
        .collect();
    let color = (0..n).map(|_| colors[rng.random_range(0..colors.len())]).collect();
    let mut fx = Travel::new(start, delay, vec![1.2; n], ease_out_bounce, color);
    fx.glyphs = Some(
        (0..n)
            .map(|_| ['*', 'o', 'O', '0', '.'][rng.random_range(0..5)])
            .collect(),
    );
    Box::new(fx)
}

// --- middleout ---------------------------------------------------------------

/// middleout: everything gathers in the middle, spreads into a row, then
/// expands vertically into place.
pub struct MiddleOut;

impl Effect for MiddleOut {
    fn draw(&mut self, sc: &Scene, t: f32, f: &mut Frame) -> bool {
        const ROW: f32 = 0.7;
        const OUT: f32 = 0.7;
        let c = center(sc);
        for i in 0..sc.len() {
            let Some(cell) = sc.settled(i) else { continue };
            let (tx, ty) = target(sc, i);
            let (pos, col) = if t < ROW {
                (lerp(c, (tx, c.1), ease_in_out_sine(t / ROW)), WHITE)
            } else {
                let p = (t - ROW) / OUT;
                (lerp((tx, c.1), (tx, ty), ease_in_out_sine(p)), WHITE.lerp(cell.fg, p))
            };
            put(f, pos, cell.ch, col);
        }
        t > ROW + OUT
    }
}

// --- binarypath ----------------------------------------------------------------

/// binarypath: each character travels as its binary code along a right-angled
/// path, then decodes into place.
pub struct BinaryPath {
    start: Vec<(i32, i32)>,
    horizontal_first: Vec<bool>,
    delay: Vec<f32>,
    speed: Vec<f32>,
    end: f32,
}

pub fn binarypath(sc: &Scene, rng: &mut impl Rng) -> Box<dyn Effect> {
    let n = sc.len();
    let start: Vec<(i32, i32)> = (0..n)
        .map(|_| {
            let p = random_edge_pos(sc, rng);
            (p.0 as i32, p.1 as i32)
        })
        .collect();
    let delay: Vec<f32> = (0..n).map(|_| rng.random_range(0.0..1.3)).collect();
    // Paths take at most ~1.2s, however long.
    let speed: Vec<f32> = (0..n)
        .map(|i| {
            let (tx, ty) = sc.pos(i);
            let len = (tx - start[i].0).abs() + (ty - start[i].1).abs();
            (len as f32 / 1.2).max(30.0)
        })
        .collect();
    let end = (0..n)
        .map(|i| {
            let (tx, ty) = sc.pos(i);
            delay[i] + ((tx - start[i].0).abs() + (ty - start[i].1).abs()) as f32 / speed[i]
        })
        .fold(0.0, f32::max);
    let horizontal_first = (0..n).map(|_| rng.random()).collect();
    Box::new(BinaryPath {
        start,
        horizontal_first,
        delay,
        speed,
        end,
    })
}

impl BinaryPath {
    /// Point `s` cells along the L-shaped path from start to `to`.
    fn along(&self, i: usize, to: (i32, i32), s: i32) -> (i32, i32) {
        let (x0, y0) = self.start[i];
        let (dx, dy) = (to.0 - x0, to.1 - y0);
        if self.horizontal_first[i] {
            if s <= dx.abs() {
                (x0 + dx.signum() * s, y0)
            } else {
                (to.0, y0 + dy.signum() * (s - dx.abs()))
            }
        } else if s <= dy.abs() {
            (x0, y0 + dy.signum() * s)
        } else {
            (x0 + dx.signum() * (s - dy.abs()), to.1)
        }
    }
}

impl Effect for BinaryPath {
    fn draw(&mut self, sc: &Scene, t: f32, f: &mut Frame) -> bool {
        let greens = hexes(&["#044E29", "#157e38", "#45bf55", "#95ed87"]);
        for i in 0..sc.len() {
            let ch = sc.ch(i);
            if ch == ' ' || t < self.delay[i] {
                continue;
            }
            let to = sc.pos(i);
            let len = (to.0 - self.start[i].0).abs() + (to.1 - self.start[i].1).abs();
            let s = ((t - self.delay[i]) * self.speed[i]) as i32;
            if s >= len {
                let since = (t - self.delay[i]) - len as f32 / self.speed[i];
                sc.draw_from(f, i, greens[3], since / 0.4);
                continue;
            }
            let bits = format!("{:08b}", u32::from(ch) & 0xff);
            for (k, b) in bits.chars().enumerate() {
                let back = s - k as i32;
                if back < 0 {
                    break;
                }
                let (x, y) = self.along(i, to, back);
                f.put(x, y, b, palette(&greens, 1.0 - k as f32 / 8.0));
            }
        }
        t > self.end + 0.4
    }
}

// --- errorcorrect -------------------------------------------------------------

/// errorcorrect: some characters start swapped and in error-red; pair by
/// pair they trade places, turning green, then settle.
pub struct ErrorCorrect {
    /// For each cell, the cell it is swapped with (or itself) and when.
    partner: Vec<usize>,
    swap_at: Vec<f32>,
    end: f32,
}

const SWAP: f32 = 0.45;

pub fn errorcorrect(sc: &Scene, rng: &mut impl Rng) -> Box<dyn Effect> {
    let mut cells = ink(sc);
    cells.shuffle(rng);
    let pairs = ((cells.len() as f32 * 0.1 / 2.0).ceil() as usize)
        .min(cells.len() / 2)
        .max(1);
    let mut partner: Vec<usize> = (0..sc.len()).collect();
    let mut swap_at = vec![0.0; sc.len()];
    for k in 0..pairs.min(cells.len() / 2) {
        let (a, b) = (cells[2 * k], cells[2 * k + 1]);
        partner[a] = b;
        partner[b] = a;
        swap_at[a] = 0.3 + k as f32 * (1.8 / pairs as f32);
        swap_at[b] = swap_at[a];
    }
    let end = max(&swap_at) + SWAP + 0.6;
    Box::new(ErrorCorrect { partner, swap_at, end })
}

impl Effect for ErrorCorrect {
    fn draw(&mut self, sc: &Scene, t: f32, f: &mut Frame) -> bool {
        let (error, correct) = (Rgb::hex("#e74c3c"), Rgb::hex("#45bf55"));
        for i in 0..sc.len() {
            let Some(cell) = sc.settled(i) else { continue };
            let j = self.partner[i];
            if j == i {
                sc.draw_from(f, i, cell.fg, 1.0);
                continue;
            }
            let lt = t - self.swap_at[i];
            if lt < 0.0 {
                put(f, target(sc, j), cell.ch, error);
            } else if lt < SWAP {
                let p = lt / SWAP;
                put(
                    f,
                    lerp(target(sc, j), target(sc, i), ease_in_out_sine(p)),
                    '▓',
                    error.lerp(correct, p),
                );
            } else {
                sc.draw_from(f, i, correct, (lt - SWAP) / 0.5);
            }
        }
        t > self.end
    }
}

// --- unstable ---------------------------------------------------------------

/// unstable: characters appear jumbled and shaking, burst to the screen
/// edges, then reassemble in order.
pub struct Unstable {
    jumbled: Vec<Pos>,
    edge: Vec<Pos>,
}

pub fn unstable(sc: &Scene, rng: &mut impl Rng) -> Box<dyn Effect> {
    let cells = ink(sc);
    let mut spots: Vec<Pos> = cells.iter().map(|&i| target(sc, i)).collect();
    spots.shuffle(rng);
    let mut jumbled: Vec<Pos> = (0..sc.len()).map(|i| target(sc, i)).collect();
    for (k, &i) in cells.iter().enumerate() {
        jumbled[i] = spots[k];
    }
    let edge = (0..sc.len()).map(|_| random_edge_pos(sc, rng)).collect();
    Box::new(Unstable { jumbled, edge })
}

impl Effect for Unstable {
    fn draw(&mut self, sc: &Scene, t: f32, f: &mut Frame) -> bool {
        const RUMBLE: f32 = 1.0;
        const BURST: f32 = 0.6;
        const HOLD: f32 = 0.2;
        const BACK: f32 = 0.9;
        let orange = Rgb::hex("#ff9200");
        for i in 0..sc.len() {
            let Some(cell) = sc.settled(i) else { continue };
            let (pos, col) = if t < RUMBLE {
                let k = (t * 20.0) as u64;
                let jx = (hash(i as u64 ^ k << 20) % 3) as f32 - 1.0;
                let jy = (hash(i as u64 ^ k << 24 ^ 7) % 3) as f32 - 1.0;
                let shake = t / RUMBLE;
                let p = self.jumbled[i];
                ((p.0 + jx * shake, p.1 + jy * shake * 0.5), cell.fg.lerp(orange, shake))
            } else if t < RUMBLE + BURST {
                (
                    lerp(self.jumbled[i], self.edge[i], ease_out_expo((t - RUMBLE) / BURST)),
                    orange,
                )
            } else if t < RUMBLE + BURST + HOLD {
                (self.edge[i], orange)
            } else {
                let p = (t - RUMBLE - BURST - HOLD) / BACK;
                (
                    lerp(self.edge[i], target(sc, i), ease_out_expo(p)),
                    orange.lerp(cell.fg, p),
                )
            };
            put(f, pos, cell.ch, col);
        }
        t > RUMBLE + BURST + HOLD + BACK
    }
}

// --- crumble ------------------------------------------------------------------

/// crumble: the text weakens, characters crumble to dust at the bottom, the
/// dust is vacuumed up, and the text re-forms with a flash.
pub struct Crumble {
    fall_at: Vec<f32>,
    top_x: Vec<f32>,
}

pub fn crumble(sc: &Scene, rng: &mut impl Rng) -> Box<dyn Effect> {
    let n = sc.len();
    Box::new(Crumble {
        fall_at: (0..n).map(|_| rng.random_range(0.2..1.2)).collect(),
        top_x: (0..n).map(|i| target(sc, i).0 + rng.random_range(-6.0..6.0)).collect(),
    })
}

impl Effect for Crumble {
    fn draw(&mut self, sc: &Scene, t: f32, f: &mut Frame) -> bool {
        const FALL: f32 = 0.55;
        const SUCK_AT: f32 = 1.9;
        const SUCK: f32 = 0.5;
        const FORM_AT: f32 = 2.5;
        const FORM: f32 = 0.8;
        let gray = Rgb::hex("#808080");
        let bottom = (sc.sh - 1) as f32;
        for i in 0..sc.len() {
            let Some(cell) = sc.settled(i) else { continue };
            let (tx, ty) = target(sc, i);
            let weak = cell.fg.lerp(gray, 0.6).scale(0.6);
            let dust = ['.', ',', '\''][(hash(i as u64) % 3) as usize];
            if t < self.fall_at[i] {
                put(f, (tx, ty), cell.ch, weak);
            } else if t < self.fall_at[i] + FALL {
                let p = (t - self.fall_at[i]) / FALL;
                let ch = if p < 0.3 { cell.ch } else { dust };
                put(f, (tx, ty + (bottom - ty) * ease_in_quad(p)), ch, WHITE.lerp(weak, p));
            } else if t < SUCK_AT {
                put(f, (tx, bottom), dust, weak);
            } else if t < SUCK_AT + SUCK {
                let p = ease_in_expo((t - SUCK_AT) / SUCK);
                put(f, lerp((tx, bottom), (self.top_x[i], 0.0), p), dust, weak);
            } else if t < FORM_AT {
                put(f, (self.top_x[i], 0.0), dust, weak);
            } else {
                let p = ((t - FORM_AT) / FORM).min(1.0);
                put(
                    f,
                    lerp((self.top_x[i], 0.0), (tx, ty), ease_out_expo(p)),
                    cell.ch,
                    WHITE.lerp(cell.fg, p),
                );
            }
        }
        t > FORM_AT + FORM
    }
}

// --- fireworks ----------------------------------------------------------------

/// fireworks: shells launch from the bottom, burst into characters, and the
/// sparks fall into place.
pub struct Fireworks {
    /// Per cell: which shell it belongs to.
    shell: Vec<usize>,
    burst_dir: Vec<Pos>,
    shells: Vec<Shell>,
}

struct Shell {
    launch_at: f32,
    from: Pos,
    burst: Pos,
    color: Rgb,
}

const LAUNCH: f32 = 0.6;
const BURST: f32 = 0.35;
const DROP: f32 = 0.8;

pub fn fireworks(sc: &Scene, rng: &mut impl Rng) -> Box<dyn Effect> {
    let colors = hexes(&["#88F7E2", "#44D492", "#F5EB67", "#FFA15C", "#FA233E"]);
    let mut cells = ink(sc);
    cells.sort_by_key(|&i| (sc.rc(i).1, sc.rc(i).0));
    let per = (cells.len() as f32 * 0.05).ceil().max(3.0) as usize;
    let groups: Vec<&[usize]> = cells.chunks(per).collect();
    let mut shell = vec![usize::MAX; sc.len()];
    let mut shells = Vec::new();
    for (k, g) in groups.iter().enumerate() {
        let cx = g.iter().map(|&i| target(sc, i).0).sum::<f32>() / g.len() as f32;
        let top = g.iter().map(|&i| target(sc, i).1).fold(f32::MAX, f32::min);
        for &i in *g {
            shell[i] = k;
        }
        shells.push(Shell {
            launch_at: k as f32 * (2.2 / groups.len().max(1) as f32),
            from: (cx + rng.random_range(-3.0..3.0), sc.sh as f32),
            burst: (cx, (top - rng.random_range(1.0..4.0)).max(1.0)),
            color: colors[rng.random_range(0..colors.len())],
        });
    }
    let radius = (sc.tw() as f32 * 0.2).clamp(4.0, 14.0);
    let burst_dir = (0..sc.len())
        .map(|_| {
            let a: f32 = rng.random_range(0.0..std::f32::consts::TAU);
            let r = radius * rng.random_range(0.4..1.0);
            (a.cos() * r, a.sin() * r * 0.5)
        })
        .collect();
    Box::new(Fireworks {
        shell,
        burst_dir,
        shells,
    })
}

impl Effect for Fireworks {
    fn draw(&mut self, sc: &Scene, t: f32, f: &mut Frame) -> bool {
        for s in &self.shells {
            let lt = t - s.launch_at;
            if (0.0..LAUNCH).contains(&lt) {
                put(f, lerp(s.from, s.burst, ease_out_sine(lt / LAUNCH)), 'o', s.color);
            }
        }
        for i in 0..sc.len() {
            let Some(cell) = sc.settled(i) else { continue };
            let Some(s) = self.shells.get(self.shell[i]) else {
                // Not part of a shell (e.g. a digit that just appeared).
                sc.draw_from(f, i, cell.fg, 1.0);
                continue;
            };
            let lt = t - s.launch_at - LAUNCH;
            if lt < 0.0 {
                continue;
            }
            let spark = (s.burst.0 + self.burst_dir[i].0, s.burst.1 + self.burst_dir[i].1);
            if lt < BURST {
                put(f, lerp(s.burst, spark, ease_out_expo(lt / BURST)), cell.ch, s.color);
            } else {
                let p = ((lt - BURST) / DROP).min(1.0);
                put(
                    f,
                    lerp(spark, target(sc, i), ease_in_quad(p)),
                    cell.ch,
                    s.color.lerp(cell.fg, p),
                );
            }
        }
        let last = self.shells.iter().map(|s| s.launch_at).fold(0.0, f32::max);
        t > last + LAUNCH + BURST + DROP
    }
}

// --- bubbles ------------------------------------------------------------------

/// bubbles: characters gather into bubbles that float down and pop, each
/// releasing its characters into place.
pub struct Bubbles {
    group: Vec<usize>,
    /// Per cell: angle around its bubble.
    angle: Vec<f32>,
    list: Vec<Bubble>,
}

struct Bubble {
    start_at: f32,
    x: f32,
    pop_y: f32,
    radius: f32,
    color: Rgb,
}

const FLOAT: f32 = 1.3;
const POP: f32 = 0.15;
const RELEASE: f32 = 0.5;

pub fn bubbles(sc: &Scene, rng: &mut impl Rng) -> Box<dyn Effect> {
    let colors = hexes(&["#d33aff", "#7395c4", "#43c2a7", "#02ff7f"]);
    let mut cells = ink(sc);
    cells.shuffle(rng);
    let mut bubble = vec![usize::MAX; sc.len()];
    let mut angle = vec![0.0; sc.len()];
    let mut bubbles = Vec::new();
    let mut rest = &cells[..];
    while !rest.is_empty() {
        let take = rng.random_range(5..=12).min(rest.len());
        let (group, more) = rest.split_at(take);
        rest = more;
        let k = bubbles.len();
        for (j, &i) in group.iter().enumerate() {
            bubble[i] = k;
            angle[i] = j as f32 / group.len() as f32 * std::f32::consts::TAU;
        }
        let x0 = sc.ox as f32;
        bubbles.push(Bubble {
            start_at: 0.0,
            x: rng.random_range(x0..x0 + sc.tw().max(1) as f32),
            pop_y: group.iter().map(|&i| target(sc, i).1).fold(0.0, f32::max),
            radius: (group.len() as f32 / 4.0).max(1.5),
            color: colors[rng.random_range(0..colors.len())],
        });
    }
    let n = bubbles.len().max(1) as f32;
    for (k, b) in bubbles.iter_mut().enumerate() {
        b.start_at = k as f32 * (1.6 / n);
    }
    Box::new(Bubbles {
        group: bubble,
        angle,
        list: bubbles,
    })
}

impl Effect for Bubbles {
    fn draw(&mut self, sc: &Scene, t: f32, f: &mut Frame) -> bool {
        for i in 0..sc.len() {
            let Some(cell) = sc.settled(i) else { continue };
            let Some(b) = self.list.get(self.group[i]) else {
                sc.draw_from(f, i, cell.fg, 1.0);
                continue;
            };
            let lt = t - b.start_at;
            if lt < 0.0 {
                continue;
            }
            let center = (
                b.x,
                -b.radius - 1.0 + (b.pop_y + b.radius + 1.0) * ease_in_out_sine((lt / FLOAT).min(1.0)),
            );
            let a = self.angle[i] + lt * 1.5;
            let on_ring = (center.0 + a.cos() * b.radius * 2.0, center.1 + a.sin() * b.radius);
            if lt < FLOAT {
                // A sheen: the top of the bubble is lighter.
                let col = b.color.lerp(WHITE, (0.5 - a.sin() * 0.5) * 0.4);
                put(f, on_ring, cell.ch, col);
            } else if lt < FLOAT + POP {
                put(f, on_ring, '*', WHITE);
            } else {
                let p = ((lt - FLOAT - POP) / RELEASE).min(1.0);
                put(
                    f,
                    lerp(on_ring, target(sc, i), ease_out_expo(p)),
                    cell.ch,
                    b.color.lerp(cell.fg, p),
                );
            }
        }
        let last = self.list.iter().map(|b| b.start_at).fold(0.0, f32::max);
        t > last + FLOAT + POP + RELEASE
    }
}

// --- orbittingvolley ----------------------------------------------------------

/// orbittingvolley: four launchers circle the clock, firing volleys of
/// characters that build it from the center out.
pub struct OrbittingVolley {
    fire_at: Vec<f32>,
    launcher: Vec<usize>,
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
    let last_fire = max(&fire_at);
    Box::new(OrbittingVolley {
        fire_at,
        launcher,
        last_fire,
    })
}

impl OrbittingVolley {
    /// Position of launcher `k` at time `t`, travelling around a box just
    /// outside the clock.
    fn launcher_pos(sc: &Scene, k: usize, t: f32) -> Pos {
        let (x0, y0) = ((sc.ox - 4).max(0) as f32, (sc.oy - 2).max(0) as f32);
        let x1 = ((sc.ox + sc.tw() as i32 + 3).min(sc.sw - 1)) as f32;
        let y1 = ((sc.oy + sc.th() as i32 + 1).min(sc.sh - 1)) as f32;
        let (w, h) = (x1 - x0, y1 - y0);
        let per = 2.0 * (w + h);
        let mut d = ((t * 0.12 + k as f32 * 0.25).fract()) * per;
        if d < w {
            return (x0 + d, y0);
        }
        d -= w;
        if d < h {
            return (x1, y0 + d);
        }
        d -= h;
        if d < w {
            return (x1 - d, y1);
        }
        (x0, y1 - (d - w))
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
            let p = lt / VOLLEY_FLIGHT;
            if p < 1.0 {
                let from = Self::launcher_pos(sc, k, self.fire_at[i]);
                let col = sc.theme.gradient_at(k as f32 * 0.25).lerp(cell.fg, p);
                put(f, lerp(from, target(sc, i), ease_out_sine(p)), cell.ch, col);
            } else {
                sc.draw_from(f, i, sc.theme.flash, (p - 1.0) * VOLLEY_FLIGHT / 0.3);
            }
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
        t > self.last_fire + VOLLEY_FLIGHT + 0.3
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
    let gap = (sc.th() as f32 * 0.35).max(1.5);
    let (mut ring_of, mut angle) = (vec![0; sc.len()], vec![0.0; sc.len()]);
    let mut radii = Vec::new();
    let mut rest = &cells[..];
    while !rest.is_empty() {
        let r = gap * (radii.len() + 1) as f32;
        // Rough circumference in cells (x counts double: cells are tall).
        let cap = ((std::f32::consts::TAU * r * 1.5) as usize).max(6);
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
        let colors = hexes(&["#ab48ff", "#e7b2b2", "#fffebd"]);
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
    star_color: Vec<Rgb>,
    swallow_at: Vec<f32>,
}

pub fn blackhole(sc: &Scene, rng: &mut impl Rng) -> Box<dyn Effect> {
    let colors = hexes(&["#ffcc0d", "#ff7326", "#ff194d"]);
    let n = sc.len();
    Box::new(Blackhole {
        star: (0..n).map(|_| random_screen_pos(sc, rng)).collect(),
        star_glyph: (0..n)
            .map(|_| ['*', '\'', '`', '¤', '•', '°', '·'][rng.random_range(0..7)])
            .collect(),
        star_color: (0..n).map(|_| colors[rng.random_range(0..colors.len())]).collect(),
        swallow_at: (0..n).map(|_| rng.random_range(1.0..1.9)).collect(),
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
                    self.star_color[k % self.star_color.len().max(1)],
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
                put(f, self.star[i], self.star_glyph[i], self.star_color[i].scale(twinkle));
            } else if t < self.swallow_at[i] + SWALLOW {
                let p = ease_in_expo((t - self.swallow_at[i]) / SWALLOW);
                put(f, lerp(self.star[i], c, p), self.star_glyph[i], self.star_color[i]);
            } else if t >= EXPLODE_AT {
                let p = ((t - EXPLODE_AT) / EXPLODE).min(1.0);
                let col = self.star_color[i].lerp(cell.fg, p);
                put(f, lerp(c, target(sc, i), ease_out_expo(p)), cell.ch, col);
            }
        }
        t > EXPLODE_AT + EXPLODE
    }
}
