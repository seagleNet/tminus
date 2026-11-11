//! Text effects in the spirit of terminaltexteffects (tte), which Omarchy's
//! screensaver runs over its logo. Every effect ends with the text at rest.
//!
//! Effects read the *current* target chars at draw time, so the clock keeps
//! ticking correctly while an effect is mid-animation.

mod light;
mod motion;

use rand::Rng;

use crate::{
    scene::Scene,
    term::Frame,
    theme::Rgb,
    util::{ease_in_out_cubic, ease_in_quad, ease_out_cubic, ease_out_expo, glyph, hash, hash01},
};

pub trait Effect {
    /// Draw the effect at `t` seconds after it started. Returns true when done.
    fn draw(&mut self, sc: &Scene, t: f32, f: &mut Frame) -> bool;
}

/// The available effects; clap derives the CLI names (`decrypt`, `rain`, ...).
#[derive(Clone, Copy, PartialEq, Eq, Debug, clap::ValueEnum)]
pub enum Kind {
    Decrypt,
    Rain,
    Beams,
    Slide,
    Swarm,
    /// Digital rain (light: sparser, trails don't flicker)
    Matrix,
    /// The full-screen, constantly flickering version (much more output)
    MatrixHeavy,
    Burn,
    Print,
    Expand,
    Waves,
    Wipe,
    // Ported from TerminalTextEffects via ttfx (see LICENSE).
    Binarypath,
    Blackhole,
    Bouncyballs,
    Bubbles,
    Colorshift,
    Crumble,
    Errorcorrect,
    Fireworks,
    Highlight,
    Laseretch,
    Middleout,
    Orbittingvolley,
    Overflow,
    Pour,
    Randomsequence,
    Rings,
    Scattered,
    Slice,
    Smoke,
    Spotlights,
    Spray,
    Sweep,
    Synthgrid,
    Thunderstorm,
    Unstable,
    Vhstape,
    /// No effect at all
    #[value(name = "none")]
    Off,
}

/// Effects picked at random when none are chosen: everything except the
/// output-heavy `matrix-heavy`, which has to be asked for.
pub fn default_pool() -> Vec<Kind> {
    use clap::ValueEnum;
    Kind::value_variants()
        .iter()
        .copied()
        .filter(|k| !matches!(k, Kind::MatrixHeavy | Kind::Off))
        .collect()
}

/// The effects to pick from given a user's choice: all (but matrix-heavy)
/// when nothing was chosen, none when `none` is among them.
pub fn pool(chosen: &[Kind]) -> Vec<Kind> {
    if chosen.is_empty() {
        default_pool()
    } else if chosen.contains(&Kind::Off) {
        vec![]
    } else {
        chosen.to_vec()
    }
}

impl Kind {
    /// Upper bound on how long the effect runs, in seconds. Used to skip
    /// effects that wouldn't finish before a short countdown's final seconds;
    /// the `effects_fit_their_durations` test keeps these honest.
    pub fn max_duration(self) -> f32 {
        match self {
            Kind::Off => 0.0,
            Kind::Expand | Kind::Highlight | Kind::Middleout => 1.5,
            Kind::Slide | Kind::Slice => 1.7,
            Kind::Wipe | Kind::Overflow | Kind::Scattered | Kind::Sweep | Kind::Randomsequence => 2.1,
            Kind::Swarm | Kind::Print => 2.6,
            Kind::Pour | Kind::Unstable => 2.8,
            Kind::Decrypt
            | Kind::Rain
            | Kind::Burn
            | Kind::Waves
            | Kind::Binarypath
            | Kind::Bouncyballs
            | Kind::Colorshift
            | Kind::Laseretch
            | Kind::Smoke
            | Kind::Spray => 3.1,
            Kind::Errorcorrect | Kind::Orbittingvolley | Kind::Synthgrid | Kind::Vhstape => 3.3,
            Kind::Beams => 3.4,
            Kind::Crumble | Kind::Spotlights => 3.5,
            Kind::Bubbles | Kind::Thunderstorm => 3.7,
            Kind::Blackhole | Kind::Rings | Kind::Fireworks => 4.1,
            // Raindrops take longer to clear a tall screen.
            Kind::Matrix | Kind::MatrixHeavy => 5.5,
        }
    }
}

/// Which moment an effect is for; each can use its own set of effects.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Moment {
    /// When the timer starts.
    Intro,
    /// Periodic replays, `e`, and re-layouts.
    Replay,
    /// At zero, and when a sequence moves to another segment.
    Zero,
}

/// Effect choices for each moment.
pub struct Pools {
    pub intro: Vec<Kind>,
    pub replay: Vec<Kind>,
    pub zero: Vec<Kind>,
}

impl Pools {
    pub fn get(&self, m: Moment) -> &[Kind] {
        match m {
            Moment::Intro => &self.intro,
            Moment::Replay => &self.replay,
            Moment::Zero => &self.zero,
        }
    }
}

/// Build effect `kind` from a fixed random seed, so it can be built again
/// identically (see `measure`).
pub fn make_seeded(kind: Kind, sc: &Scene, seed: u64) -> Box<dyn Effect> {
    use rand::SeedableRng;
    make(kind, sc, &mut rand::rngs::StdRng::seed_from_u64(seed))
}

/// Exactly how long `make_seeded(kind, sc, seed)` runs on this scene, found
/// by playing a copy off-screen.
pub fn measure(kind: Kind, sc: &Scene, seed: u64) -> f32 {
    const STEP: f32 = 1.0 / 120.0;
    let mut fx = make_seeded(kind, sc, seed);
    let mut scratch = Frame::new(sc.sw.max(0) as u16, sc.sh.max(0) as u16);
    let mut t = 0.0;
    while !fx.draw(sc, t, &mut scratch) && t < 15.0 {
        scratch.clear();
        t += STEP;
    }
    t
}

pub fn make(kind: Kind, sc: &Scene, rng: &mut impl Rng) -> Box<dyn Effect> {
    let n = sc.len();
    macro_rules! rand_vec {
        ($lo:expr, $hi:expr) => {
            (0..n).map(|_| rng.random_range($lo..$hi)).collect::<Vec<f32>>()
        };
    }
    match kind {
        Kind::Rain => Box::new(Rain {
            delay: rand_vec!(0.0, 1.4),
            dur: rand_vec!(0.45, 0.8),
            start: rand_vec!(1.0, 6.0),
        }),
        Kind::Beams => Box::new(Beams::new(sc, rng)),
        Kind::Slide => Box::new(Slide {
            from_left: rng.random(),
        }),
        Kind::Swarm => {
            let (sw, sh) = (sc.sw as f32, sc.sh as f32);
            let start = (0..n)
                .map(|_| (rng.random_range(0.0..sw), rng.random_range(0.0..sh)))
                .collect();
            let hue = rand_vec!(0.0, 1.0);
            Box::new(Swarm {
                start,
                delay: rand_vec!(0.0, 0.5),
                dur: rand_vec!(1.0, 1.6),
                hue,
            })
        }
        Kind::Matrix | Kind::MatrixHeavy => {
            let lock = rand_vec!(0.9, 2.6);
            Box::new(Matrix {
                drops: vec![],
                heavy: kind == Kind::MatrixHeavy,
                last_lock: max(&lock),
                lock,
                last: 0.0,
                seed: rng.random(),
            })
        }
        Kind::Burn => Box::new(Burn {
            jitter: rand_vec!(0.0, 0.35),
        }),
        Kind::Print => Box::new(Print),
        Kind::Expand => Box::new(Expand {
            delay: rand_vec!(0.0, 0.25),
        }),
        Kind::Waves => Box::new(Waves),
        Kind::Wipe => Box::new(Wipe {
            dir: rng.random_range(0..4),
        }),
        Kind::Off => Box::new(Settled),
        Kind::Binarypath => motion::binarypath(sc, rng),
        Kind::Blackhole => motion::blackhole(sc, rng),
        Kind::Bouncyballs => motion::bouncyballs(sc, rng),
        Kind::Bubbles => motion::bubbles(sc, rng),
        Kind::Colorshift => Box::new(light::ColorShift),
        Kind::Crumble => motion::crumble(sc, rng),
        Kind::Errorcorrect => motion::errorcorrect(sc, rng),
        Kind::Fireworks => motion::fireworks(sc, rng),
        Kind::Highlight => Box::new(light::Highlight),
        Kind::Laseretch => light::laseretch(sc, rng),
        Kind::Middleout => Box::new(motion::MiddleOut),
        Kind::Orbittingvolley => motion::orbittingvolley(sc, rng),
        Kind::Overflow => light::overflow(sc, rng),
        Kind::Pour => motion::pour(sc, rng),
        Kind::Randomsequence => light::randomsequence(sc, rng),
        Kind::Rings => motion::rings(sc, rng),
        Kind::Scattered => motion::scattered(sc, rng),
        Kind::Slice => motion::slice(sc, rng),
        Kind::Smoke => light::smoke(sc, rng),
        Kind::Spotlights => light::spotlights(sc, rng),
        Kind::Spray => motion::spray(sc, rng),
        Kind::Sweep => Box::new(light::Sweep),
        Kind::Synthgrid => light::synthgrid(sc, rng),
        Kind::Thunderstorm => light::thunderstorm(sc, rng),
        Kind::Unstable => motion::unstable(sc, rng),
        Kind::Vhstape => light::vhstape(sc, rng),

        Kind::Decrypt => {
            let resolve = rand_vec!(0.7, 2.3);
            Box::new(Decrypt {
                appear: rand_vec!(0.0, 0.5),
                end: max(&resolve) + 0.5,
                resolve,
                seed: rng.random(),
            })
        }
    }
}

/// Not an effect: draws the text at rest and is done at once.
struct Settled;

impl Effect for Settled {
    fn draw(&mut self, sc: &Scene, _t: f32, f: &mut Frame) -> bool {
        sc.draw_settled(f);
        true
    }
}

/// Per-row stagger: `gap` seconds per row, but at most ~0.5s over all rows,
/// so a tall (block-font) clock doesn't make the effect drag on.
fn row_gap(sc: &Scene, gap: f32) -> f32 {
    gap.min(0.5 / sc.th().max(1) as f32)
}

/// Fade from the theme flash color into the settled color after a reveal.
fn reveal(sc: &Scene, f: &mut Frame, i: usize, since: f32, fade: f32) {
    sc.draw_from(f, i, sc.theme.flash, since / fade);
}

fn max(v: &[f32]) -> f32 {
    v.iter().copied().fold(0.0, f32::max)
}

// --- decrypt: scrambled ciphertext that resolves cell by cell -------------

struct Decrypt {
    appear: Vec<f32>,
    resolve: Vec<f32>,
    end: f32,
    seed: u64,
}

impl Effect for Decrypt {
    fn draw(&mut self, sc: &Scene, t: f32, f: &mut Frame) -> bool {
        for i in 0..sc.len() {
            if sc.ch(i) == ' ' || t < self.appear[i] {
                continue;
            }
            if t < self.resolve[i] {
                let k = (t * 14.0) as u64;
                let g = glyph(self.seed ^ (i as u64) << 20 ^ k);
                let col = sc.theme.gradient_at(hash01(self.seed ^ i as u64)).scale(0.55);
                let (x, y) = sc.pos(i);
                f.put(x, y, g, col);
            } else {
                reveal(sc, f, i, t - self.resolve[i], 0.5);
            }
        }
        t > self.end
    }
}

// --- rain: characters fall from the top into place ------------------------

struct Rain {
    delay: Vec<f32>,
    dur: Vec<f32>,
    start: Vec<f32>,
}

impl Effect for Rain {
    fn draw(&mut self, sc: &Scene, t: f32, f: &mut Frame) -> bool {
        let mut done = true;
        for i in 0..sc.len() {
            let ch = sc.ch(i);
            let lt = t - self.delay[i];
            if lt < self.dur[i] + 0.4 {
                done = false;
            }
            if ch == ' ' || lt < 0.0 {
                continue;
            }
            let (x, ty) = sc.pos(i);
            let p = lt / self.dur[i];
            if p < 1.0 {
                let y0 = -self.start[i];
                let y = y0 + (ty as f32 - y0) * ease_in_quad(p);
                let col = sc.theme.gradient_at(i as f32 * 0.013).lerp(sc.theme.flash, 0.4);
                f.put(x, y.round() as i32, ch, col);
            } else {
                reveal(sc, f, i, lt - self.dur[i], 0.4);
            }
        }
        done
    }
}

// --- beams: light beams sweep rows, revealing text; columns glow after ----

const TRAIL: i32 = 10;

struct Beams {
    row_delay: Vec<f32>,
    row_ltr: Vec<bool>,
    col_delay: Vec<(usize, f32)>,
    speed: f32,
    vspeed: f32,
}

impl Beams {
    fn new(sc: &Scene, rng: &mut impl Rng) -> Self {
        let th = sc.th();
        let mut order: Vec<usize> = (0..th).collect();
        for i in (1..th).rev() {
            order.swap(i, rng.random_range(0..=i));
        }
        // Rows start 0.12s apart, but a tall clock still gets all its beams out within ~1.2s.
        let row_gap = (1.2 / th.max(1) as f32).min(0.12);
        let mut row_delay = vec![0.0; th];
        for (k, &r) in order.iter().enumerate() {
            row_delay[r] = k as f32 * row_gap;
        }
        let rows_end = th as f32 * row_gap + 0.5;
        let col_delay = (0..sc.tw())
            .step_by(3)
            .map(|c| (c, rows_end + rng.random_range(0.0..0.8)))
            .collect();
        Beams {
            row_delay,
            row_ltr: (0..th).map(|_| rng.random()).collect(),
            col_delay,
            speed: (sc.sw as f32 + 2.0 * TRAIL as f32) / 0.8,
            vspeed: (sc.sh as f32 + 2.0 * TRAIL as f32) / 0.7,
        }
    }

    fn head(&self, sc: &Scene, r: usize, t: f32) -> f32 {
        let d = (t - self.row_delay[r]) * self.speed - TRAIL as f32;
        if self.row_ltr[r] { d } else { sc.sw as f32 - 1.0 - d }
    }
}

impl Effect for Beams {
    fn draw(&mut self, sc: &Scene, t: f32, f: &mut Frame) -> bool {
        let th = sc.theme;
        let row_time = (sc.sw as f32 + 2.0 * TRAIL as f32) / self.speed;
        // Text cells revealed behind each row beam.
        for i in 0..sc.len() {
            let (r, _) = sc.rc(i);
            let (x, _) = sc.pos(i);
            let dist = (x as f32 + TRAIL as f32) / self.speed;
            let dist = if self.row_ltr[r] {
                dist
            } else {
                (sc.sw as f32 - 1.0 - x as f32 + TRAIL as f32) / self.speed
            };
            let at = self.row_delay[r] + dist;
            if t >= at {
                reveal(sc, f, i, t - at, 0.7);
            }
        }
        // Row beams.
        for r in 0..sc.th() {
            if t < self.row_delay[r] || t > self.row_delay[r] + row_time {
                continue;
            }
            let h = self.head(sc, r, t).round() as i32;
            let dir = if self.row_ltr[r] { -1 } else { 1 };
            let y = sc.oy + r as i32;
            for k in 0..TRAIL {
                let col = th
                    .flash
                    .lerp(th.gradient_at(r as f32 * 0.07), k as f32 / 4.0)
                    .scale(1.0 - k as f32 / TRAIL as f32);
                f.put(h + k * dir, y, if k == 0 { '█' } else { '▂' }, col);
            }
        }
        // Column beams: decorative glow passes.
        let mut last = 0.0f32;
        for &(c, d) in &self.col_delay {
            let col_time = (sc.sh as f32 + 2.0 * TRAIL as f32) / self.vspeed;
            last = last.max(d + col_time);
            if t < d || t > d + col_time {
                continue;
            }
            let h = ((t - d) * self.vspeed - TRAIL as f32).round() as i32;
            let x = sc.ox + c as i32;
            for k in 0..TRAIL / 2 {
                let col = th
                    .flash
                    .lerp(th.gradient_at(c as f32 * 0.02), k as f32 / 3.0)
                    .scale(1.0 - k as f32 / (TRAIL / 2) as f32);
                f.put(x, h - k, '▎', col);
            }
        }
        t > last.max(max(&self.row_delay) + row_time + 0.7)
    }
}

// --- slide: rows slide in from alternating sides ---------------------------

struct Slide {
    from_left: bool,
}

impl Effect for Slide {
    fn draw(&mut self, sc: &Scene, t: f32, f: &mut Frame) -> bool {
        const DUR: f32 = 1.0;
        for i in 0..sc.len() {
            let Some(cell) = sc.settled(i) else { continue };
            let (r, _) = sc.rc(i);
            let p = (t - r as f32 * row_gap(sc, 0.07)) / DUR;
            if p <= 0.0 {
                continue;
            }
            let left = (r % 2 == 0) == self.from_left;
            let start = if left {
                -(sc.ox + sc.tw() as i32) - 2
            } else {
                sc.sw - sc.ox + 2
            };
            let off = (start as f32 * (1.0 - ease_out_cubic(p))).round() as i32;
            let (x, y) = sc.pos(i);
            let col = sc.theme.flash.lerp(cell.fg, p);
            f.put(x + off, y, cell.ch, col);
        }
        t > sc.th() as f32 * row_gap(sc, 0.07) + DUR
    }
}

// --- swarm: characters scattered across the screen converge ---------------

struct Swarm {
    start: Vec<(f32, f32)>,
    delay: Vec<f32>,
    dur: Vec<f32>,
    hue: Vec<f32>,
}

impl Effect for Swarm {
    fn draw(&mut self, sc: &Scene, t: f32, f: &mut Frame) -> bool {
        let mut done = true;
        for i in 0..sc.len() {
            let p = ((t - self.delay[i]) / self.dur[i]).max(0.0);
            if p < 1.2 {
                done = false;
            }
            let Some(cell) = sc.settled(i) else { continue };
            let (tx, ty) = sc.pos(i);
            let (sx, sy) = self.start[i];
            let e = ease_in_out_cubic(p);
            let x = sx + (tx as f32 - sx) * e;
            let y = sy + (ty as f32 - sy) * e;
            let col = sc
                .theme
                .gradient_at(self.hue[i])
                .lerp(cell.fg, ((p - 0.6) / 0.6).max(0.0));
            f.put(x.round() as i32, y.round() as i32, cell.ch, col);
        }
        done
    }
}

// --- matrix: digital rain that locks the text in as it falls --------------

struct Drop {
    x: i32,
    y: f32,
    speed: f32,
    len: i32,
    seed: u64,
}

struct Matrix {
    drops: Vec<Drop>,
    /// Dense rain whose trails flicker every frame; light mode is sparser with
    /// still trails, which writes several times less to the terminal.
    heavy: bool,
    lock: Vec<f32>,
    last_lock: f32,
    last: f32,
    seed: u64,
}

const RAIN_FOR: f32 = 2.3;

impl Effect for Matrix {
    fn draw(&mut self, sc: &Scene, t: f32, f: &mut Frame) -> bool {
        let dt = (t - self.last).clamp(0.0, 0.1);
        self.last = t;
        if t < RAIN_FOR {
            // Spawn rate scales with screen width.
            let n = sc.sw as f32 * dt * if self.heavy { 1.3 } else { 0.55 };
            let k = n as usize + usize::from(hash01(self.seed ^ (t * 1000.0) as u64) < n.fract());
            for j in 0..k {
                let s = hash(self.seed ^ ((t * 1000.0) as u64) << 8 ^ j as u64);
                self.drops.push(Drop {
                    x: (s % sc.sw.max(1) as u64) as i32,
                    y: -1.0,
                    speed: 26.0 + hash01(s ^ 1) * 28.0,
                    len: 5 + (hash(s ^ 2) % 14) as i32,
                    seed: s,
                });
            }
        }
        let green = Rgb::hex("#00ff41");
        let dark = Rgb::hex("#003b10");
        let head = Rgb::hex("#d0ffd8");
        for d in &mut self.drops {
            d.y += d.speed * dt;
            let hy = d.y.floor() as i32;
            for k in 0..d.len {
                let y = hy - k;
                // Heavy: every trail cell keeps changing. Light: only the head
                // flickers, and trail colors fade in a few steps instead of smoothly.
                let churn = if self.heavy || k == 0 {
                    (t * (6.0 + k as f32)) as u64
                } else {
                    0
                };
                let g = glyph(d.seed ^ (y as u64) << 16 ^ churn);
                let fade = k as f32 / d.len as f32;
                let fade = if self.heavy { fade } else { (fade * 4.0).ceil() / 4.0 };
                let col = if k == 0 { head } else { green.lerp(dark, fade) };
                f.put(d.x, y, g, col);
            }
        }
        self.drops.retain(|d| (d.y as i32 - d.len) < sc.sh);
        for i in 0..sc.len() {
            if t >= self.lock[i] {
                sc.draw_from(f, i, head, (t - self.lock[i]) / 0.6);
            }
        }
        t > RAIN_FOR && self.drops.is_empty() && t > self.last_lock + 0.6
    }
}

// --- burn: text ignites bottom-up then cools to its colors ----------------

struct Burn {
    jitter: Vec<f32>,
}

const FIRE: &[Rgb] = &[
    Rgb::hex("#2a0000"),
    Rgb::hex("#8b0000"),
    Rgb::hex("#ff4500"),
    Rgb::hex("#ff8c00"),
    Rgb::hex("#ffd27a"),
    Rgb::hex("#fffbe6"),
];

impl Effect for Burn {
    fn draw(&mut self, sc: &Scene, t: f32, f: &mut Frame) -> bool {
        const BURN: f32 = 0.6;
        const COOL: f32 = 0.6;
        let mut done = true;
        for i in 0..sc.len() {
            let (r, c) = sc.rc(i);
            let at =
                (sc.th() - 1 - r) as f32 / sc.th() as f32 * 1.0 + c as f32 / sc.tw() as f32 * 0.25 + self.jitter[i];
            let lt = t - at;
            if lt < BURN + COOL {
                done = false;
            }
            let Some(cell) = sc.settled(i) else { continue };
            if lt < 0.0 {
                continue;
            }
            let (x, y) = sc.pos(i);
            if lt < BURN {
                let p = lt / BURN;
                let fx = p * (FIRE.len() - 1) as f32;
                let k = fx as usize;
                let col = FIRE[k].lerp(FIRE[(k + 1).min(FIRE.len() - 1)], fx.fract());
                let ch = ['░', '▒', '▓', '█'][((p * 4.0) as usize).min(3)];
                f.put(x, y, ch, col);
                // A flicker of smoke above freshly lit cells.
                if p < 0.5 && hash01(i as u64 ^ (t * 20.0) as u64) < 0.3 {
                    f.put(x, y - 1, '·', sc.theme.dim);
                }
            } else {
                f.put(x, y, cell.ch, Rgb::hex("#ff8c00").lerp(cell.fg, (lt - BURN) / COOL));
            }
        }
        done
    }
}

// --- print: typewriter, row by row, with a block cursor -------------------

struct Print;

impl Effect for Print {
    fn draw(&mut self, sc: &Scene, t: f32, f: &mut Frame) -> bool {
        let row_dur = (1.8 / sc.th() as f32).min(0.4);
        let cps = sc.tw() as f32 / row_dur;
        let total = sc.th() as f32 * row_dur;
        for i in 0..sc.len() {
            let (r, c) = sc.rc(i);
            let at = r as f32 * row_dur + c as f32 / cps;
            if t >= at {
                reveal(sc, f, i, t - at, 0.5);
            }
        }
        if t < total {
            let r = (t / row_dur) as usize;
            let c = ((t - r as f32 * row_dur) * cps) as i32;
            f.put(sc.ox + c, sc.oy + r as i32, '█', sc.theme.flash);
        }
        t > total + 0.5
    }
}

// --- expand: everything bursts out from the center ------------------------

struct Expand {
    delay: Vec<f32>,
}

impl Effect for Expand {
    fn draw(&mut self, sc: &Scene, t: f32, f: &mut Frame) -> bool {
        const DUR: f32 = 1.1;
        let cx = sc.ox as f32 + sc.tw() as f32 / 2.0;
        let cy = sc.oy as f32 + sc.th() as f32 / 2.0;
        for i in 0..sc.len() {
            let Some(cell) = sc.settled(i) else { continue };
            let p = (t - self.delay[i]) / DUR;
            if p < 0.0 {
                continue;
            }
            let (tx, ty) = sc.pos(i);
            let e = ease_out_expo(p);
            let x = cx + (tx as f32 - cx) * e;
            let y = cy + (ty as f32 - cy) * e;
            f.put(
                x.round() as i32,
                y.round() as i32,
                cell.ch,
                sc.theme.flash.lerp(cell.fg, p),
            );
        }
        t > DUR + 0.3
    }
}

// --- waves: a wave of block heights washes across, then settles ----------

struct Waves;

const WAVE: &[char] = &['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█', '▇', '▆', '▅', '▄', '▃', '▂'];

impl Effect for Waves {
    fn draw(&mut self, sc: &Scene, t: f32, f: &mut Frame) -> bool {
        for i in 0..sc.len() {
            if sc.ch(i) == ' ' {
                continue;
            }
            let (r, c) = sc.rc(i);
            let u = c as f32 / sc.tw() as f32;
            let enter = u * 0.6;
            let settle = 1.2 + u * 1.0 + r as f32 * row_gap(sc, 0.03);
            if t < enter {
                continue;
            }
            if t < settle {
                // f32 rem_euclid can round up to exactly len, so wrap again as an integer.
                let k =
                    (t * 16.0 - c as f32 * 0.5 - r as f32 * 0.4).rem_euclid(WAVE.len() as f32) as usize % WAVE.len();
                let (x, y) = sc.pos(i);
                f.put(x, y, WAVE[k], sc.theme.gradient_at(u * 0.8 + t * 0.4));
            } else {
                reveal(sc, f, i, t - settle, 0.4);
            }
        }
        t > 2.2 + sc.th() as f32 * row_gap(sc, 0.03) + 0.4
    }
}

// --- wipe: a shaded diagonal edge sweeps the text in ----------------------

struct Wipe {
    dir: u8,
}

impl Effect for Wipe {
    fn draw(&mut self, sc: &Scene, t: f32, f: &mut Frame) -> bool {
        const DUR: f32 = 1.4;
        const EDGE: f32 = 0.18;
        let span = sc.tw() as f32 + 2.0 * sc.th() as f32;
        for i in 0..sc.len() {
            let Some(cell) = sc.settled(i) else { continue };
            let (r, c) = sc.rc(i);
            let (c, r) = (c as f32, r as f32);
            let key = match self.dir {
                0 => c + 2.0 * r,
                1 => (sc.tw() as f32 - c) + 2.0 * r,
                2 => c + 2.0 * (sc.th() as f32 - r),
                _ => (sc.tw() as f32 - c) + 2.0 * (sc.th() as f32 - r),
            } / span;
            let at = key * DUR;
            let (x, y) = sc.pos(i);
            if t >= at {
                sc.draw_from(f, i, sc.theme.flash, (t - at) / 0.5);
            } else if t >= at - EDGE {
                let p = (t - (at - EDGE)) / EDGE;
                f.put(
                    x,
                    y,
                    ['░', '▒', '▓'][((p * 3.0) as usize).min(2)],
                    sc.theme.flash.lerp(cell.fg, 0.4),
                );
            }
        }
        t > DUR + 0.6
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{font, scene::Target, theme};
    use clap::ValueEnum;
    use rand::{SeedableRng, rngs::StdRng};

    /// Effects hide the clock while they run, so they must stay short, and
    /// within the `max_duration` the picker relies on — on a typical screen
    /// and on a large one with a big block-font clock.
    #[test]
    fn effects_fit_their_durations() {
        const SHORT: f32 = 5.0;
        let themes = theme::builtin();
        let layouts = [(font::shadow("24:59"), 120, 36), (font::block("1:24:59", 3), 220, 64)];
        let mut report = vec![];
        for (g, sw, sh) in layouts {
            let mut target = Target::empty();
            target.update(g, 0.0, usize::MAX);
            let sc = Scene {
                t: &target,
                ox: (sw - target.w as i32) / 2,
                oy: (sh - target.h as i32) / 2,
                sw,
                sh,
                theme: &themes[0],
                now: 0.0,
                paused: false,
                done: false,
                tick_fx: true,
                urgency: 0.0,
            };
            for &kind in Kind::value_variants() {
                for seed in 0..3 {
                    let mut fx = make(kind, &sc, &mut StdRng::seed_from_u64(seed));
                    let mut frame = Frame::new(sw as u16, sh as u16);
                    let mut t = 0.0;
                    while !fx.draw(&sc, t, &mut frame) {
                        t += 1.0 / 60.0;
                        assert!(
                            t <= kind.max_duration(),
                            "{kind:?} ran past its max_duration on {sw}x{sh}"
                        );
                    }
                    if sw == 120 && seed == 0 {
                        assert!(
                            t < SHORT || matches!(kind, Kind::Matrix | Kind::MatrixHeavy),
                            "{kind:?} too long"
                        );
                        report.push(format!("{kind:?} {t:.1}s"));
                    }
                }
            }
        }
        println!("{}", report.join(", "));
    }

    /// Prints each effect's longest run over a range of layouts, for tuning
    /// `max_duration`: `cargo test measure_durations -- --ignored --nocapture`.
    #[test]
    #[ignore = "a measuring tool, not a check"]
    fn measure_durations() {
        let themes = theme::builtin();
        let layouts = [
            (font::shadow("24:59"), 80, 24),
            (font::shadow("24:59"), 120, 36),
            (font::shadow("1:24:59"), 200, 50),
            (font::block("1:24:59", 3), 220, 64),
            (font::block("59", 6), 120, 60),
        ];
        for &kind in Kind::value_variants() {
            let mut worst: f32 = 0.0;
            for (g, sw, sh) in &layouts {
                let mut target = Target::empty();
                target.update(
                    font::Glyphs {
                        w: g.w,
                        h: g.h,
                        chars: g.chars.clone(),
                        owner: g.owner.clone(),
                    },
                    0.0,
                    usize::MAX,
                );
                let sc = Scene {
                    t: &target,
                    ox: (sw - target.w as i32) / 2,
                    oy: (sh - target.h as i32) / 2,
                    sw: *sw,
                    sh: *sh,
                    theme: &themes[0],
                    now: 0.0,
                    paused: false,
                    done: false,
                    tick_fx: true,
                    urgency: 0.0,
                };
                for seed in 0..5 {
                    let mut fx = make(kind, &sc, &mut StdRng::seed_from_u64(seed));
                    let mut frame = Frame::new(*sw as u16, *sh as u16);
                    let mut t = 0.0;
                    while !fx.draw(&sc, t, &mut frame) && t < 30.0 {
                        t += 1.0 / 60.0;
                    }
                    worst = worst.max(t);
                }
            }
            println!("{kind:?} {worst:.2} (table {})", kind.max_duration());
        }
    }

    /// Every effect must settle: run each one frame by frame and require it to
    /// report done within a generous bound, for a few layouts and seeds.
    #[test]
    fn every_effect_finishes() {
        const LIMIT: f32 = 15.0;
        let themes = theme::builtin();
        let layouts = [
            (font::shadow("12:34"), 80, 24),
            (font::plain("0:01"), 10, 3),
            (font::block("59:59", 3), 120, 40),
        ];
        for (g, sw, sh) in layouts {
            let mut target = Target::empty();
            target.update(g, 0.0, usize::MAX);
            let ox = (sw - target.w as i32) / 2;
            let oy = (sh - target.h as i32) / 2;
            for &kind in Kind::value_variants() {
                for seed in 0..3 {
                    let sc = Scene {
                        t: &target,
                        ox,
                        oy,
                        sw,
                        sh,
                        theme: &themes[0],
                        now: 0.0,
                        paused: false,
                        done: false,
                        tick_fx: true,
                        urgency: 0.0,
                    };
                    let mut rng = StdRng::seed_from_u64(seed);
                    let mut fx = make(kind, &sc, &mut rng);
                    let mut frame = Frame::new(sw as u16, sh as u16);
                    let mut t = 0.0;
                    while !fx.draw(&sc, t, &mut frame) {
                        t += 1.0 / 60.0;
                        assert!(t < LIMIT, "{kind:?} still running after {LIMIT}s on {sw}x{sh}");
                    }
                }
            }
        }
    }
}
