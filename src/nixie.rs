//! The nixie tube clock face (`--font nixie`): thin wire digits in glass tubes,
//! shaped after the IN-8-2 (a real 5, a comma at the bottom right).
//!
//! Each cell is a `▀` whose foreground is the upper pixel and whose
//! background is the lower one, so the tubes are a small true-color image:
//! anti-aliased wires, glow, unlit cathodes and glass. For effects the
//! target chars are the wires alone, as `▀▄█` (like the block font); the
//! image is drawn when the clock is at rest.

use std::{cell::RefCell, f32::consts::PI, rc::Rc};

use crate::{
    effects::Effect,
    font::{Glyphs, NO_OWNER},
    scene::{Scene, TICK_FX},
    term::{Cell, Frame},
    theme::Rgb,
    util::hash,
};

/// What separates hours, minutes and seconds.
#[derive(Clone, Copy, PartialEq, Eq, Debug, clap::ValueEnum)]
pub enum Separator {
    /// Two neon dots, like a colon
    Colon,
    /// One small square neon dot at the bottom, like a lamp between the tubes
    Dot,
    /// No separator, only a gap
    #[value(name = "none")]
    Off,
}

/// How small tubes (narrower than `SMALL_TUBE_W`) are drawn.
#[derive(Clone, Copy, PartialEq, Eq, Debug, clap::ValueEnum)]
pub enum SmallTubes {
    /// Octants in terminals known to draw them (Ghostty, Alacritty, foot, kitty), else half blocks
    Auto,
    /// Octants: thin wires and the glass outline at twice the resolution, no glow
    Octant,
    /// Half blocks like big tubes, with glow and unlit symbols
    Half,
}

impl SmallTubes {
    /// Whether small tubes use octants here.
    pub fn octants(self) -> bool {
        match self {
            SmallTubes::Octant => true,
            SmallTubes::Half => false,
            SmallTubes::Auto => {
                let var = |k: &str| std::env::var(k).unwrap_or_default();
                var("TERM_PROGRAM").eq_ignore_ascii_case("ghostty")
                    || var("TERM") == "xterm-ghostty"
                    || std::env::var_os("ALACRITTY_WINDOW_ID").is_some()
                    || std::env::var_os("KITTY_WINDOW_ID").is_some()
                    || var("TERM").starts_with("foot")
            }
        }
    }
}

/// The color of the lit digits.
#[derive(Clone, Copy, PartialEq, Eq, Debug, clap::ValueEnum)]
pub enum Tint {
    /// Neon orange, like a real tube
    Neon,
    /// The theme's colors, held still
    Theme,
    /// One theme color (its first) for all digits, no gradient
    ThemeSolid,
    /// The theme's colors drifting across the tubes like the other fonts
    /// (more CPU: every tube is drawn anew each frame)
    ThemeHeavy,
}

/// The parts of the look that can be switched (from the command line).
#[derive(Clone, Copy, PartialEq, Debug)]
#[allow(clippy::struct_excessive_bools, reason = "independent on/off CLI flags")]
pub struct Style {
    pub glass: bool,
    /// A tip on top of the glass, where the tube was sealed.
    pub glass_tip: bool,
    /// How strongly the unlit cathodes show, 0..=1.
    pub unlit: f32,
    /// Glow strength, 0..=1.
    pub glow: f32,
    /// Brightness of the lit digits and their light, 0..=1.
    pub brightness: f32,
    pub tint: Tint,
    pub separator: Separator,
    /// Small tubes drawn with octants (`SmallTubes::octants`).
    pub octants: bool,
    /// Separator lit only every other second.
    pub blink: bool,
    pub flicker: bool,
}

impl Default for Style {
    fn default() -> Self {
        Style {
            glass: true,
            glass_tip: false,
            unlit: 0.3,
            glow: 0.6,
            brightness: 1.0,
            tint: Tint::Neon,
            separator: Separator::Colon,
            octants: false,
            blink: false,
            flicker: false,
        }
    }
}

/// Digit box, in digit units: x 0..1, y 0..`H` (y down), the IN-8-2's 12 x 18 mm.
const H: f32 = 1.6;
/// Room around the digit box inside a tube (glass, glow).
const PAD: f32 = 0.32;
/// Wire width.
const STROKE: f32 = 0.085;
/// The comma cathode, at the bottom right of the digit.
const COMMA: (f32, f32) = (1.14, 1.55);
/// The colon's dots: their heights and radius.
const DOTS: [f32; 2] = [0.55, 1.05];
const DOT_R: f32 = 0.07;
/// The single dot: its height and half its side (it's a small square).
const DOT_Y: f32 = 1.5;
const DOT_HALF: f32 = 0.08;
/// Height of the tip on top of the glass (--glass-tip).
const TIP: f32 = 0.16;
/// How far the glow reaches.
const GLOW_R: f32 = 0.09;
/// Title tubes: the widest, and the narrowest in which letters still read.
const TITLE_TUBE_W: usize = 16;
const MIN_TITLE_TUBE_W: usize = 10;
/// Tubes narrower than this are small: with `Style::octants`, drawn in
/// octants (wires and glass outline only), which stay readable down to
/// `MIN_FINE_TUBE_W`.
const SMALL_TUBE_W: usize = 10;
const MIN_FINE_TUBE_W: usize = 5;
/// The symbols' height in small tubes, down to the comma's tail.
const FINE_BOX_H: f32 = H + 0.15;
/// Narrowest tube, in cells; below that the clock uses another font.
const MIN_TUBE_W: usize = 7;

impl Style {
    /// The narrowest tubes for the clock, and for a title.
    fn min_tube(self) -> usize {
        if self.octants { MIN_FINE_TUBE_W } else { MIN_TUBE_W }
    }

    fn min_title_tube(self) -> usize {
        if self.octants {
            MIN_FINE_TUBE_W
        } else {
            MIN_TITLE_TUBE_W
        }
    }

    /// Pixels per cell column for tubes `tube_w` wide: 2 drawn in octants.
    fn per_cell(self, tube_w: usize) -> usize {
        if self.octants && tube_w < SMALL_TUBE_W { 2 } else { 1 }
    }
}
/// Steps of the crossfade when a digit changes.
const FADE_STEPS: f32 = 8.0;
/// After an effect the glow comes back over this long.
pub const GLOW_FADE: f64 = 0.6;

const NEON: Rgb = Rgb::hex("#ff6e14");
const NEON_HOT: Rgb = Rgb::hex("#ffdcaa");
const UNLIT: Rgb = Rgb::hex("#5f5048");
const GLASS: Rgb = Rgb::hex("#464650");

type Pt = (f32, f32);

fn arc(c: Pt, r: Pt, a0: f32, a1: f32, n: usize) -> Vec<Pt> {
    (0..=n)
        .map(|i| {
            let a = (a0 + (a1 - a0) * i as f32 / n as f32) * PI / 180.0;
            (c.0 + r.0 * a.cos(), c.1 + r.1 * a.sin())
        })
        .collect()
}

fn bezier(p0: Pt, p1: Pt, p2: Pt, n: usize) -> Vec<Pt> {
    (0..=n)
        .map(|i| {
            let t = i as f32 / n as f32;
            let (a, b, c) = ((1.0 - t) * (1.0 - t), 2.0 * (1.0 - t) * t, t * t);
            (a * p0.0 + b * p1.0 + c * p2.0, a * p0.1 + b * p1.1 + c * p2.1)
        })
        .collect()
}

/// The wire of digit `d` as polylines (angles in degrees, y down).
fn strokes(d: u8) -> Vec<Vec<Pt>> {
    let six = || {
        vec![
            bezier((0.85, 0.02), (0.1, 0.25), (0.05, 1.17), 16),
            arc((0.5, 1.17), (0.45, 0.43), 0.0, 360.0, 48),
        ]
    };
    match d {
        0 => vec![arc((0.5, 0.8), (0.5, 0.8), 0.0, 360.0, 64)],
        1 => vec![vec![(0.5, 0.0), (0.5, H)]],
        2 => {
            let mut s = arc((0.5, 0.45), (0.45, 0.45), 180.0, 400.0, 32);
            s.extend([(0.0, H), (1.0, H)]);
            vec![s]
        }
        // Flat top, a diagonal to the middle, then the round bottom.
        3 => {
            let mut s = vec![(0.05, 0.0), (0.95, 0.0)];
            s.extend(arc((0.5, 1.16), (0.45, 0.45), -100.0, 160.0, 40));
            vec![s]
        }
        4 => vec![vec![(0.75, H), (0.75, 0.0), (0.0, 1.1), (1.0, 1.1)]],
        5 => {
            let mut s = vec![(0.92, 0.0), (0.1, 0.0), (0.06, 0.8)];
            s.extend(arc((0.5, 1.17), (0.45, 0.43), 215.0, 515.0, 40));
            vec![s]
        }
        6 => six(),
        7 => vec![vec![(0.0, 0.0), (1.0, 0.0), (0.3, H)]],
        8 => vec![
            arc((0.5, 0.4), (0.38, 0.38), 0.0, 360.0, 40),
            arc((0.5, 1.17), (0.45, 0.43), 0.0, 360.0, 48),
        ],
        // The 6 turned around.
        _ => six()
            .into_iter()
            .map(|s| s.into_iter().map(|(x, y)| (1.0 - x, H - y)).collect())
            .collect(),
    }
}

/// The wire of symbol `c` (letters after the IN-15's: tall, full width,
/// upright sides), as polylines like `strokes`.
#[allow(clippy::too_many_lines, reason = "a table of glyphs")]
fn symbol_strokes(c: char) -> Vec<Vec<Pt>> {
    let o = || arc((0.5, 0.8), (0.5, 0.8), 0.0, 360.0, 64);
    let p = || {
        let mut s = vec![(0.05, H), (0.05, 0.0), (0.55, 0.0)];
        s.extend(arc((0.55, 0.42), (0.4, 0.42), -90.0, 90.0, 24));
        s.extend([(0.55, 0.84), (0.05, 0.84)]);
        vec![s]
    };
    let dot = |x: f32| vec![(x, 1.5), (x, 1.52)];
    let join = |mut a: Vec<Pt>, b: &[Pt]| {
        a.extend_from_slice(b);
        a
    };
    match c {
        'A' => vec![vec![(0.0, H), (0.5, 0.0), (1.0, H)], vec![(0.2, 1.0), (0.8, 1.0)]],
        'B' => vec![
            vec![(0.05, 0.0), (0.05, H)],
            join(
                join(
                    vec![(0.05, 0.0), (0.58, 0.0)],
                    &arc((0.58, 0.38), (0.35, 0.38), -90.0, 90.0, 24),
                ),
                &[(0.58, 0.76), (0.05, 0.76)],
            ),
            join(
                join(
                    vec![(0.05, 0.76), (0.6, 0.76)],
                    &arc((0.6, 1.18), (0.38, 0.42), -90.0, 90.0, 24),
                ),
                &[(0.6, H), (0.05, H)],
            ),
        ],
        'C' => vec![arc((0.55, 0.8), (0.5, 0.8), 315.0, 45.0, 48)],
        'D' => vec![
            vec![(0.05, 0.0), (0.05, H)],
            join(
                join(
                    vec![(0.05, 0.0), (0.4, 0.0)],
                    &arc((0.4, 0.8), (0.55, 0.8), -90.0, 90.0, 40),
                ),
                &[(0.4, H), (0.05, H)],
            ),
        ],
        'E' => vec![
            vec![(0.95, 0.0), (0.05, 0.0), (0.05, H), (0.95, H)],
            vec![(0.05, 0.78), (0.75, 0.78)],
        ],
        'F' => vec![
            vec![(0.95, 0.0), (0.05, 0.0), (0.05, H)],
            vec![(0.05, 0.78), (0.75, 0.78)],
        ],
        'G' => vec![join(
            arc((0.55, 0.8), (0.5, 0.8), 315.0, 30.0, 48),
            &[(0.98, 0.85), (0.55, 0.85)],
        )],
        'H' => vec![
            vec![(0.05, 0.0), (0.05, H)],
            vec![(0.95, 0.0), (0.95, H)],
            vec![(0.05, 0.78), (0.95, 0.78)],
        ],
        'I' => vec![
            vec![(0.5, 0.0), (0.5, H)],
            vec![(0.25, 0.0), (0.75, 0.0)],
            vec![(0.25, H), (0.75, H)],
        ],
        'J' => vec![join(
            vec![(0.8, 0.0), (0.8, 1.15)],
            &arc((0.45, 1.15), (0.35, 0.45), 0.0, 180.0, 24),
        )],
        'K' => vec![
            vec![(0.05, 0.0), (0.05, H)],
            vec![(0.95, 0.0), (0.05, 0.9)],
            vec![(0.35, 0.62), (0.95, H)],
        ],
        'L' => vec![vec![(0.05, 0.0), (0.05, H), (0.9, H)]],
        'M' => vec![vec![(0.05, H), (0.05, 0.0), (0.5, 0.9), (0.95, 0.0), (0.95, H)]],
        'N' => vec![vec![(0.05, H), (0.05, 0.0), (0.95, H), (0.95, 0.0)]],
        'O' => vec![o()],
        'P' => p(),
        'Q' => vec![o(), vec![(0.6, 1.2), (1.0, 1.65)]],
        'R' => {
            let mut r = p();
            r.push(vec![(0.45, 0.84), (0.95, H)]);
            r
        }
        'S' => vec![
            arc((0.5, 0.42), (0.43, 0.4), -30.0, -270.0, 32),
            arc((0.5, 1.2), (0.45, 0.4), -90.0, 150.0, 32),
        ],
        'T' => vec![vec![(0.0, 0.0), (1.0, 0.0)], vec![(0.5, 0.0), (0.5, H)]],
        'U' => vec![join(
            join(
                vec![(0.05, 0.0), (0.05, 1.1)],
                &arc((0.5, 1.1), (0.45, 0.5), 180.0, 0.0, 32),
            ),
            &[(0.95, 1.1), (0.95, 0.0)],
        )],
        'V' => vec![vec![(0.0, 0.0), (0.5, H), (1.0, 0.0)]],
        // The IN-15B's W: an M upside down.
        'W' => vec![vec![(0.05, 0.0), (0.05, H), (0.5, 0.75), (0.95, H), (0.95, 0.0)]],
        'X' => vec![vec![(0.0, 0.0), (1.0, H)], vec![(1.0, 0.0), (0.0, H)]],
        'Y' => vec![vec![(0.0, 0.0), (0.5, 0.8), (1.0, 0.0)], vec![(0.5, 0.8), (0.5, H)]],
        'Z' => vec![vec![(0.05, 0.0), (0.95, 0.0), (0.05, H), (0.95, H)]],
        'd' => vec![
            vec![(0.92, 0.0), (0.92, H)],
            arc((0.5, 1.1), (0.42, 0.5), 0.0, 360.0, 48),
        ],
        'h' => vec![
            vec![(0.08, 0.0), (0.08, H)],
            join(
                join(vec![(0.08, 1.0)], &arc((0.48, 1.0), (0.4, 0.4), 180.0, 360.0, 24)),
                &[(0.88, 1.0), (0.88, H)],
            ),
        ],
        'm' => vec![
            vec![(0.05, H), (0.05, 0.6)],
            join(
                arc((0.275, 0.95), (0.225, 0.35), 180.0, 360.0, 20),
                &[(0.5, 0.95), (0.5, H)],
            ),
            join(
                arc((0.725, 0.95), (0.225, 0.35), 180.0, 360.0, 20),
                &[(0.95, 0.95), (0.95, H)],
            ),
        ],
        '+' => vec![vec![(0.5, 0.45), (0.5, 1.15)], vec![(0.15, 0.8), (0.85, 0.8)]],
        '-' => vec![vec![(0.2, 0.8), (0.8, 0.8)]],
        '!' => vec![vec![(0.5, 0.0), (0.5, 1.15)], dot(0.5)],
        '?' => vec![
            join(
                arc((0.5, 0.4), (0.4, 0.38), 200.0, 450.0, 32),
                &[(0.5, 0.78), (0.5, 1.15)],
            ),
            dot(0.5),
        ],
        '/' => vec![vec![(0.9, 0.0), (0.1, H)]],
        '\'' => vec![vec![(0.5, 0.0), (0.5, 0.4)]],
        ',' => vec![vec![(0.55, 1.35), (0.55, 1.5), (0.4, 1.75)]],
        '.' => vec![dot(0.5)],
        _ => vec![],
    }
}

/// A tube's set of cathodes, stacked like the digits 0-9 are: the digits
/// (and the comma), or a group of letters or signs.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Group {
    Digits,
    Letters(u8),
    /// `d`, `h`, `m` after a number: `2d 03:04:05`, `1h 05m`.
    Units,
    Signs,
}

/// The symbols of the groups other than the digits, in cathode order.
const LETTER_GROUPS: [&str; 3] = ["ABCDEFGHIJK", "LMNOPQRSTUV", "WXYZ"];
const UNITS: &str = "dhm";
const SIGNS: &str = "+-!?/',.";

impl Group {
    fn symbols(self) -> &'static str {
        match self {
            Group::Digits => "0123456789",
            Group::Letters(n) => LETTER_GROUPS[usize::from(n)],
            Group::Units => UNITS,
            Group::Signs => SIGNS,
        }
    }

    /// The wires of cathode `k`.
    fn strokes(self, k: u8) -> Vec<Vec<Pt>> {
        match self {
            Group::Digits => strokes(k),
            g => g
                .symbols()
                .chars()
                .nth(usize::from(k))
                .map_or_else(Vec::new, symbol_strokes),
        }
    }
}

/// The tube and cathode for `c`; `after_digit` makes `d`, `h`, `m` units.
fn cathode(c: char, after_digit: bool) -> Option<(Group, u8)> {
    if let Some(d) = c.to_digit(10) {
        return Some((Group::Digits, d as u8));
    }
    let find = |g: Group, c: char| g.symbols().find(c).map(|k| (g, k as u8));
    if after_digit && let Some(found) = find(Group::Units, c) {
        return Some(found);
    }
    let up = c.to_ascii_uppercase();
    (0..3u8)
        .find_map(|n| find(Group::Letters(n), up))
        .or_else(|| find(Group::Signs, c))
}

fn seg_dist(p: Pt, a: Pt, b: Pt) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let l2 = dx * dx + dy * dy;
    let t = if l2 == 0.0 {
        0.0
    } else {
        (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / l2).clamp(0.0, 1.0)
    };
    (p.0 - a.0 - t * dx).hypot(p.1 - a.1 - t * dy)
}

/// Move vertical and horizontal wires onto pixel centers. Between two
/// pixels a thin wire lights both at half strength and never gets the hot
/// core that curves (which cross pixel centers somewhere) have.
fn snap(line: &[Pt], s: f32, v0: f32) -> Vec<Pt> {
    let center = |u: f32, off: f32| (((u + off) * s).floor() + 0.5) / s - off;
    let mut out = line.to_vec();
    #[allow(clippy::float_cmp, reason = "exactly straight as written in `strokes`")]
    for i in 0..line.len().saturating_sub(1) {
        let (a, b) = (line[i], line[i + 1]);
        if a.0 == b.0 {
            out[i].0 = center(a.0, PAD);
            out[i + 1].0 = out[i].0;
        }
        if a.1 == b.1 {
            out[i].1 = center(a.1, v0);
            out[i + 1].1 = out[i].1;
        }
    }
    out
}

/// Glow allowed at pixel `p` of `n` (`s` pixels per unit): none at the edges.
fn edge_fade(p: usize, n: usize, s: f32) -> f32 {
    let d = (p.min(n - 1 - p) as f32 + 0.5) / s;
    (d / 0.25).clamp(0.0, 1.0)
}

/// What each cathode (ten digits, comma) does to one pixel when fully lit:
/// its wire's coverage, its coverage as an unlit cathode, its glow and its
/// light on the glass.
#[derive(Clone, Copy)]
struct Shade {
    core: [f32; 11],
    unlit: [f32; 11],
    halo: [f32; 11],
    near: [f32; 11],
}

/// Signed distance from `p` to a rect centered at `c` with half sizes `half`
/// and corner radius `r`.
fn rounded_rect(p: Pt, c: Pt, half: Pt, r: f32) -> f32 {
    let q = ((p.0 - c.0).abs() - half.0 + r, (p.1 - c.1).abs() - half.1 + r);
    q.0.max(0.0).hypot(q.1.max(0.0)) + q.0.max(q.1).min(0.0) - r
}

/// Signed distance from `p` to a glass tip: a nub `len` tall and `r` round
/// standing on (`x`, `top`).
fn tip(p: Pt, x: f32, top: f32, len: f32, r: f32) -> f32 {
    seg_dist(p, (x, top - len + r), (x, top)) - r
}

/// Index of the comma in a pixel's distances (after the ten digits).
const COMMA_I: usize = 10;

/// Per-pixel distances for one tube size, computed once and shared.
pub struct Field {
    /// Pixels per digit unit.
    s: f32,
    /// Pixels across and down one tube.
    w: usize,
    h: usize,
    /// Top of the digit box, in units from the tube's top edge.
    v0: f32,
    /// Each group's cathodes, worked out when first needed.
    groups: RefCell<Vec<(Group, Rc<Cathodes>)>>,
    /// For each pixel: signed distance to the glass outline (< 0 inside).
    glass: Vec<f32>,
    /// For each pixel: how much glow it may show, fading out towards the
    /// tube's edges so the glow doesn't end in a visible box.
    edge: Vec<f32>,
    /// For each pixel: how much of the reflection streak inside the glass it shows.
    streak: Vec<f32>,
    /// Whether the glass has a tip on top.
    tip: bool,
    /// Pixels a cell is wide (`Style::per_cell`): 2 for octants.
    m: usize,
    /// The cells each slot was last drawn as, and what from (see
    /// `draw_lamps`), by the layout's row and the slot.
    cache: RefCell<Vec<((usize, usize), Drawn)>>,
}

/// A group's cathodes in one tube size.
struct Cathodes {
    /// For each pixel: the distance to each cathode's wire (digits: and the
    /// comma); unused ones are far away.
    dist: Vec<[f32; 11]>,
    /// For each pixel: what each cathode does to it, from `dist` (fixed for
    /// a tube size, so frames only weigh these by how lit each cathode is).
    shade: Vec<Shade>,
}

impl Field {
    fn new(tube_w: usize, rows: usize, with_tip: bool, m: usize) -> Field {
        let s = (tube_w * m) as f32 / (1.0 + 2.0 * PAD);
        let (w, h) = (tube_w * m, rows * 2 * m);
        let v0 = (h as f32 / s - H) / 2.0;
        // The glass: a rounded rect half a pixel inside the tube's cells,
        // lower by the tip's height if it has one.
        let inset = 0.6 / s;
        let (top, bottom) = (
            H / 2.0 - h as f32 / s / 2.0 + inset,
            H / 2.0 + h as f32 / s / 2.0 - inset,
        );
        let top = if with_tip { top + TIP } else { top };
        let (hw, hh) = (0.5 + PAD - inset, (bottom - top) / 2.0);
        let cy = top.midpoint(bottom);
        let mut glass = Vec::with_capacity(w * h);
        let mut edge = Vec::with_capacity(w * h);
        let mut streak = Vec::with_capacity(w * h);
        for py in 0..h {
            for px in 0..w {
                let p = ((px as f32 + 0.5) / s - PAD, (py as f32 + 0.5) / s - v0);
                let mut sd = rounded_rect(p, (0.5, cy), (hw, hh), 0.45);
                if with_tip {
                    sd = sd.min(tip(p, 0.5, top + 0.05, TIP + 0.05, 0.035));
                }
                glass.push(sd);
                edge.push(edge_fade(px, w, s) * edge_fade(py, h, s));
                // A thin band just inside the glass down both sides, fading
                // towards the top and bottom.
                let band = cover((sd + 0.1).abs(), 0.04_f32.max(1.0 / s), 1.0 / s);
                let side = ((0.5 - p.0).abs() - 0.3).clamp(0.0, 0.2) / 0.2;
                let along = (1.0 - (p.1 - H / 2.0).abs() / (H / 2.0 + 0.1)).clamp(0.0, 1.0);
                streak.push(band * side * along * along);
            }
        }
        Field {
            s,
            w,
            h,
            v0,
            groups: RefCell::default(),
            glass,
            edge,
            streak,
            tip: with_tip,
            m,
            cache: RefCell::default(),
        }
    }

    /// The shared field for this tube size (a few sizes are kept: the
    /// clock's and the title's).
    fn get(tube_w: usize, rows: usize, tip: bool, m: usize) -> Rc<Field> {
        thread_local! {
            static CACHE: RefCell<Vec<Rc<Field>>> = const { RefCell::new(Vec::new()) };
        }
        CACHE.with_borrow_mut(|c| {
            if let Some(f) = c
                .iter()
                .find(|f| f.w == tube_w * m && f.h == rows * 2 * m && f.tip == tip)
            {
                return f.clone();
            }
            let f = Rc::new(Field::new(tube_w, rows, tip, m));
            if c.len() >= 4 {
                c.remove(0);
            }
            c.push(f.clone());
            f
        })
    }

    /// The cathodes of `group` in this tube size.
    fn cathodes(&self, group: Group) -> Rc<Cathodes> {
        if let Some((_, c)) = self.groups.borrow().iter().find(|(g, _)| *g == group) {
            return c.clone();
        }
        let (s, v0) = (self.s, self.v0);
        // Small tubes in octants have no glass: the symbols take the whole
        // tube, but a pixel or two at its edges. Their box reaches down to
        // the comma (`FINE_BOX_H`).
        let fit = if self.m == 2 {
            let (w, h) = (self.w as f32, self.h as f32);
            let (x0, x1) = (1.5 / s - PAD, (w - 1.5) / s - PAD);
            let (y0, y1) = (2.5 / s - v0, (h - 2.5) / s - v0);
            let k = ((x1 - x0) / 1.0).min((y1 - y0) / FINE_BOX_H);
            Some((x0.midpoint(x1), y0.midpoint(y1), k))
        } else {
            None
        };
        let shrink = |(x, y): Pt| match fit {
            Some((cx, cy, k)) => (cx + (x - 0.5) * k, cy + (y - FINE_BOX_H * 0.5) * k),
            None => (x, y),
        };
        let segs: Vec<Vec<(Pt, Pt)>> = (0..group.symbols().len() as u8)
            .map(|k| {
                group
                    .strokes(k)
                    .iter()
                    .map(|l| snap(&l.iter().copied().map(shrink).collect::<Vec<_>>(), s, v0))
                    .flat_map(|l| l.windows(2).map(|p| (p[0], p[1])).collect::<Vec<_>>())
                    .collect()
            })
            .collect();
        let mut dist = Vec::with_capacity(self.w * self.h);
        let mut shade = Vec::with_capacity(self.w * self.h);
        for py in 0..self.h {
            for px in 0..self.w {
                let p = ((px as f32 + 0.5) / s - PAD, (py as f32 + 0.5) / s - v0);
                let mut d = [f32::INFINITY; 11];
                for (k, segs) in segs.iter().enumerate() {
                    d[k] = segs
                        .iter()
                        .map(|&(a, b)| seg_dist(p, a, b))
                        .fold(f32::INFINITY, f32::min);
                }
                if group == Group::Digits {
                    let comma = shrink(COMMA);
                    d[COMMA_I] = (p.0 - comma.0).hypot(p.1 - comma.1) - DOT_R / 2.0;
                }
                dist.push(d);
                shade.push(Shade {
                    // At least a pixel wide, or small tubes' wires come out dim and uneven.
                    core: d.map(|d| cover(d, STROKE.max(1.0 / s), 1.0 / s)),
                    unlit: d.map(|d| cover(d, STROKE * 0.6, 1.0 / s)),
                    halo: d.map(|d| (-d / GLOW_R).exp()),
                    near: d.map(|d| (-d / 0.35).exp()),
                });
            }
        }
        let c = Rc::new(Cathodes { dist, shade });
        self.groups.borrow_mut().push((group, c.clone()));
        c
    }

    /// Signed distance from pixel (`px`, `py`) of a separator slot `w`
    /// pixels wide to the glass of the dot's bulb: a small tube standing
    /// on the digit tubes' baseline, around the dot.
    fn bulb_dist(&self, w: usize, px: usize, py: usize) -> f32 {
        let p = (
            (px as f32 + 0.5 - w as f32 / 2.0) / self.s,
            (py as f32 + 0.5) / self.s - self.v0,
        );
        let inset = 0.6 / self.s;
        let (top, bottom) = (DOT_Y - 0.4, H / 2.0 + self.h as f32 / self.s / 2.0 - inset);
        let (hw, hh) = (w as f32 / self.s / 2.0 - inset, (bottom - top) / 2.0);
        let sd = rounded_rect(p, (0.0, top.midpoint(bottom)), (hw, hh), hw.min(0.2));
        if self.tip {
            sd.min(tip(p, 0.0, top + 0.03, 0.13, 0.03))
        } else {
            sd
        }
    }

    /// Signed distance from pixel (`px`, `py`) of a separator slot `w`
    /// pixels wide to its dots.
    fn colon_dist(&self, sep: Separator, w: usize, px: usize, py: usize) -> f32 {
        let p = (
            (px as f32 + 0.5 - w as f32 / 2.0) / self.s,
            (py as f32 + 0.5) / self.s - self.v0,
        );
        if sep == Separator::Dot {
            let q = (p.0.abs() - DOT_HALF, (p.1 - DOT_Y).abs() - DOT_HALF);
            return q.0.max(0.0).hypot(q.1.max(0.0)) + q.0.max(q.1).min(0.0);
        }
        DOTS.iter()
            .map(|&y| p.0.hypot(p.1 - y) - DOT_R)
            .fold(f32::INFINITY, f32::min)
    }
}

/// What lights up in a slot.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lamp {
    /// Cathode `k` of a tube with `group`'s symbols, and (digits) the comma.
    Tube {
        group: Group,
        k: u8,
        comma: bool,
    },
    /// A tube for a character none has: glass and unlit symbols only.
    Blank,
    Colon,
}

#[derive(Clone, Copy, Debug)]
pub struct Slot {
    /// First column, row (of a line of tubes) and width, in cells.
    pub x: usize,
    pub y: usize,
    pub w: usize,
    pub lamp: Lamp,
    /// What it showed before the last change, and when that was.
    pub prev: Option<Lamp>,
    pub changed: f64,
    /// The input character it was drawn from (for the tick fade).
    pub owner: usize,
}

/// The tubes of the clock face, part of its `Glyphs`.
#[derive(Clone)]
pub struct Layout {
    pub slots: Vec<Slot>,
    /// Columns before the first tube (when stacked over a wider title), and
    /// rows above it (the title's tubes, under the clock).
    pub dx: usize,
    pub dy: usize,
    pub rows: usize,
    pub style: Style,
    field: Rc<Field>,
}

impl Layout {
    /// Carry the tick fade over from the layout shown before. Only slots
    /// drawn from input characters before `fx_until` fade.
    pub fn follow(&mut self, old: &Layout, now: f64, fx_until: usize) {
        if old.slots.len() != self.slots.len() {
            return;
        }
        for (new, old) in self.slots.iter_mut().zip(&old.slots) {
            if new.lamp == old.lamp {
                new.prev = old.prev;
                new.changed = old.changed;
            } else if new.owner < fx_until {
                new.prev = Some(old.lamp);
                new.changed = now;
            }
        }
    }
}

/// True if every char of `text` is in the nixie font.
/// Anything else gets a blank tube; only control characters and the like don't.
pub fn supports(text: &str) -> bool {
    !text.is_empty() && text.chars().all(|c| !c.is_control())
}

/// The clock face for `text`, as large as fits (like the block font, a good
/// chunk of `max_w` x `max_h`), or `None` if even small tubes don't fit.
/// With `wrap` (a done-text) it may go on several lines, broken at spaces,
/// when that gives bigger tubes.
pub fn glyphs(text: &str, max_w: usize, max_h: usize, style: Style, wrap: bool) -> Option<Glyphs> {
    if !supports(text) {
        return None;
    }
    let width = |line: &str, tube_w: usize| slots(line, tube_w, style.separator).last().map_or(0, |s| s.x + s.w);
    let fits = |lines: &[Line], tube_w: usize, w: usize, h: usize| {
        let n = lines.len();
        lines.iter().all(|(_, l)| width(l, tube_w) <= w) && n * tube_rows(tube_w) + n - 1 <= h
    };
    let ways = if wrap {
        wrappings(text)
    } else {
        vec![vec![(0, text.to_string())]]
    };
    // The biggest tubes over all ways to break the lines (fewer lines on a tie).
    let mut best: Option<(&[Line], usize)> = None;
    for lines in &ways {
        // Several lines may take more of the height: they're all there is to see.
        let h = if lines.len() > 1 { max_h * 4 / 5 } else { max_h * 3 / 5 };
        let mut tube_w = style.min_tube();
        while fits(lines, tube_w + 1, max_w * 4 / 5, h) {
            tube_w += 1;
        }
        if fits(lines, tube_w, max_w * 4 / 5, h) && best.is_none_or(|(_, w)| tube_w > w) {
            best = Some((lines, tube_w));
        }
    }
    // Nothing fits that comfortably: the smallest tubes, if they fit at all.
    let (lines, tube_w) = best.or_else(|| {
        ways.iter()
            .find(|l| fits(l, style.min_tube(), max_w, max_h))
            .map(|l| (l.as_slice(), style.min_tube()))
    })?;
    Some(sized(lines, tube_w, style))
}

/// A line of text and the index of its first character in the whole.
type Line = (usize, String);

/// Ways to break `text` at spaces into 1-4 lines of about equal length.
fn wrappings(text: &str) -> Vec<Vec<Line>> {
    let mut words: Vec<(usize, &str)> = vec![];
    let mut start = None;
    for (n, c) in text.chars().chain([' ']).enumerate() {
        match (c == ' ', start) {
            (false, None) => start = Some(n),
            (true, Some(s)) => {
                words.push((s, text.get(char_byte(text, s)..char_byte(text, n)).unwrap_or("")));
                start = None;
            }
            _ => {}
        }
    }
    let total: usize = words.iter().map(|(_, w)| w.chars().count() + 1).sum();
    let mut ways: Vec<Vec<Line>> = vec![];
    for n in 1..=words.len().min(4) {
        let target = total.div_ceil(n);
        let mut lines: Vec<Line> = vec![];
        for &(at, word) in &words {
            match lines.last_mut() {
                Some((_, line)) if line.chars().count() + 1 + word.chars().count() <= target => {
                    line.push(' ');
                    line.push_str(word);
                }
                _ => lines.push((at, word.to_string())),
            }
        }
        if !ways.contains(&lines) {
            ways.push(lines);
        }
    }
    ways
}

/// Byte offset of char `n` in `s` (its length past the end).
fn char_byte(s: &str, n: usize) -> usize {
    s.char_indices().nth(n).map_or(s.len(), |(b, _)| b)
}

/// Rows of a tube `tube_w` cells wide.
fn tube_rows(tube_w: usize) -> usize {
    let s = tube_w as f32 / (1.0 + 2.0 * PAD);
    ((H + 2.0 * PAD) * s / 2.0).ceil() as usize
}

/// Tubes for the title: about a twelfth of the screen wide, at most
/// `TITLE_TUBE_W`, narrower to fit `max_w`; `None` below `MIN_TITLE_TUBE_W`,
/// where letters stop being readable.
pub fn title(text: &str, max_w: usize, style: Style) -> Option<Glyphs> {
    if !supports(text) {
        return None;
    }
    let min = style.min_title_tube();
    (min..=(max_w / 12).clamp(min, TITLE_TUBE_W))
        .rev()
        .map(|w| sized(&[(0, text.to_string())], w, style))
        .find(|g| g.w <= max_w)
}

/// `lines` (which `supports`) in tubes `tube_w` cells wide, each line
/// centered, a row apart.
fn sized(lines: &[Line], tube_w: usize, style: Style) -> Glyphs {
    let rows = tube_rows(tube_w);
    let width = |slots: &[Slot]| slots.last().map_or(0, |s| s.x + s.w);
    let per_line: Vec<Vec<Slot>> = lines.iter().map(|(_, l)| slots(l, tube_w, style.separator)).collect();
    let w = per_line.iter().map(|s| width(s)).max().unwrap_or(0);
    let h = lines.len() * (rows + 1) - 1;
    let mut slots = vec![];
    for (n, (line, &(at, _))) in per_line.iter().zip(lines).enumerate() {
        let dx = (w - width(line)) / 2;
        slots.extend(line.iter().map(|s| Slot {
            x: s.x + dx,
            y: n * (rows + 1),
            owner: s.owner + at,
            ..*s
        }));
    }
    let m = style.per_cell(tube_w);
    let field = Field::get(tube_w, rows, style.glass_tip, m);
    let mut chars = vec![' '; w * h];
    let mut owner = vec![NO_OWNER; w * h];
    // The wires as half blocks: a pixel is on where the wire passes through it.
    let thr = (STROKE / 2.0).max(0.6 / field.s);
    for slot in &slots {
        for py in 0..rows * 2 * m {
            for px in 0..slot.w * m {
                let on = match slot.lamp {
                    Lamp::Tube { group, k, comma } => {
                        let d = &field.cathodes(group).dist[py * field.w + px];
                        d[usize::from(k)] < thr || comma && d[COMMA_I] < thr
                    }
                    Lamp::Colon => field.colon_dist(style.separator, slot.w * m, px, py) < thr,
                    Lamp::Blank => false,
                };
                if on {
                    // Effects play the wires as half blocks, whatever the pixels.
                    let half_row = py / m;
                    let i = (slot.y + half_row / 2) * w + slot.x + px / m;
                    chars[i] = match (chars[i], half_row % 2) {
                        (' ', 0) => '▀',
                        (' ', _) => '▄',
                        ('▀', 0) | ('▄', 1) => chars[i],
                        _ => '█',
                    };
                    owner[i] = slot.owner;
                }
            }
        }
    }
    Glyphs {
        w,
        h,
        chars,
        owner,
        nixie: Some(Layout {
            slots,
            dx: 0,
            dy: 0,
            rows,
            style,
            field,
        }),
        nixie_title: None,
    }
}

/// Lay out `text` in tubes `tube_w` cells wide.
fn slots(text: &str, tube_w: usize, sep: Separator) -> Vec<Slot> {
    let s = tube_w as f32 / (1.0 + 2.0 * PAD);
    let cells = |u: f32| ((u * s).round() as usize).max(1);
    let gap = cells(0.12);
    let mut out: Vec<Slot> = vec![];
    let mut x = 0;
    let mut after_digit = false;
    for (n, c) in text.chars().enumerate() {
        let tube = cathode(c, after_digit);
        let prev_digit = after_digit;
        after_digit = c.is_ascii_digit();
        let (w, lamp) = match c {
            ':' => match sep {
                Separator::Colon => (cells(0.45), Some(Lamp::Colon)),
                Separator::Dot => (cells(0.5), Some(Lamp::Colon)),
                Separator::Off => (cells(0.25), None),
            },
            '.' if prev_digit => {
                // The comma cathode of the tube before.
                if let Some(Slot {
                    lamp:
                        Lamp::Tube {
                            group: Group::Digits,
                            comma,
                            ..
                        },
                    ..
                }) = out.last_mut()
                {
                    *comma = true;
                }
                continue;
            }
            ' ' => (cells(0.5), None),
            // A symbol, or a blank tube for a character no tube has.
            _ => (
                tube_w,
                Some(tube.map_or(Lamp::Blank, |(group, k)| Lamp::Tube { group, k, comma: false })),
            ),
        };
        if n > 0 {
            x += gap;
        }
        if let Some(lamp) = lamp {
            out.push(Slot {
                x,
                y: 0,
                w,
                lamp,
                prev: None,
                changed: f64::NEG_INFINITY,
                owner: n,
            });
        }
        x += w;
    }
    out
}

/// The colors of a lit lamp: wire, its hot core, glow.
#[derive(Clone, Copy, PartialEq)]
struct Colors {
    wire: Rgb,
    hot: Rgb,
    glow: Rgb,
    /// Glass and unlit cathodes, to suit the background (darker on a light one).
    glass: Rgb,
    unlit: Rgb,
    /// A light background: glow tints it rather than adding light (which
    /// would only wash out to white).
    light: bool,
}

/// How much each cathode (ten digits, comma) is lit, 0..=1.
type Lit = [f32; 11];

/// Light `lamp`'s cathodes by `k`. Added up, so a cathode lit both before
/// and after a change (the comma) stays at full strength through the fade.
fn lit_of(lamp: Option<Lamp>, k: f32, lit: &mut Lit) {
    if let Some(Lamp::Tube { k: digit, comma, .. }) = lamp {
        lit[usize::from(digit)] = (lit[usize::from(digit)] + k).min(1.0);
        if comma {
            lit[COMMA_I] = (lit[COMMA_I] + k).min(1.0);
        }
    }
}

/// Anti-aliased coverage of a line `w` wide at distance `d`, pixels `px` big.
fn cover(d: f32, w: f32, px: f32) -> f32 {
    (0.5 + (w / 2.0 - d) / px).clamp(0.0, 1.0)
}

fn mix(a: [f32; 3], b: Rgb, k: f32) -> [f32; 3] {
    let b = [f32::from(b.0), f32::from(b.1), f32::from(b.2)];
    [
        a[0] + (b[0] - a[0]) * k,
        a[1] + (b[1] - a[1]) * k,
        a[2] + (b[2] - a[2]) * k,
    ]
}

fn to_rgb(c: [f32; 3]) -> Rgb {
    let ch = |v: f32| v.round().clamp(0.0, 255.0) as u8;
    Rgb(ch(c[0]), ch(c[1]), ch(c[2]))
}

/// One pixel of a tube: `None` where nothing shows (the background stays).
#[allow(clippy::too_many_arguments, reason = "one pixel's inputs, called in a hot loop")]
fn tube_pixel(
    style: &Style,
    field: &Field,
    cath: &Cathodes,
    j: usize,
    rest: Rgb,
    lit: &Lit,
    c: Colors,
    glow: f32,
) -> Option<Rgb> {
    let px = 1.0 / field.s;
    let (sh, glass) = (&cath.shade[j], field.glass[j]);
    let mut col = [f32::from(rest.0), f32::from(rest.1), f32::from(rest.2)];
    let mut shown = false;
    if style.glass {
        if glass > px {
            return None;
        }
        let near = (0..11).map(|k| lit[k] * sh.near[k]).fold(0.0, f32::max);
        let on = lit.iter().copied().fold(0.0, f32::max);
        col = glass_shade(col, glass, px, near, field.streak[j] * on, c, glow);
        shown = true;
    }
    let glow = glow * field.edge[j];
    if style.unlit > 0.0 {
        let m = (0..11).map(|k| sh.unlit[k] * (1.0 - lit[k])).fold(0.0, f32::max) * style.unlit;
        col = mix(col, c.unlit, m);
        shown |= m > 0.01;
    }
    let g: f32 = (0..11).map(|k| lit[k] * sh.halo[k]).sum::<f32>() * glow;
    col = glow_onto(col, c, g);
    shown |= g * 255.0 > 2.0;
    let core = (0..11).map(|k| lit[k] * sh.core[k]).fold(0.0, f32::max);
    if core > 0.0 {
        col = mix(col, c.wire.lerp(c.hot, core * 0.7), core);
        shown = true;
    }
    shown.then(|| to_rgb(col))
}

/// A colon pixel: its dots and their glow.
/// Glass over a pixel `sd` from its outline (< 0 inside): a faint tint, the
/// outline, and the lamp's light on it: on the rim by `near` (how close the
/// lit cathode is), and as a reflection streak by `streak`.
/// Glow `g` from a lit cathode onto `col`.
fn glow_onto(col: [f32; 3], c: Colors, g: f32) -> [f32; 3] {
    if c.light {
        return mix(col, c.wire, (g * 0.5).min(0.6));
    }
    let gc = [f32::from(c.glow.0), f32::from(c.glow.1), f32::from(c.glow.2)];
    [col[0] + gc[0] * g, col[1] + gc[1] * g, col[2] + gc[2] * g]
}

fn glass_shade(col: [f32; 3], sd: f32, px: f32, near: f32, streak: f32, c: Colors, glow: f32) -> [f32; 3] {
    let col = if sd < 0.0 { mix(col, c.glass, 0.08) } else { col };
    // At least a pixel, and thicker on big tubes (small fonts) so it stays visible.
    let rim = cover(sd.abs(), (px * 0.9).max(0.025), px);
    let col = mix(col, c.glass, rim * 0.8);
    let k = glow * (rim * near * 1.6 + streak * 0.35);
    mix(col, c.hot.lerp(c.glow, 0.4), k.min(1.0))
}

/// Where a separator pixel is: distance to its dots, the glass of the dot's
/// bulb (signed distance, with glass and `--separator dot`), and how much glow
/// it may show (none at the slot's edges).
struct SepPx {
    d: f32,
    bulb: Option<f32>,
    edge: f32,
}

/// A separator pixel: its dots and their glow, and the dot's bulb.
fn colon_pixel(px: f32, rest: Rgb, at: &SepPx, k: f32, c: Colors, glow: f32) -> Option<Rgb> {
    let mut col = [f32::from(rest.0), f32::from(rest.1), f32::from(rest.2)];
    let mut shown = false;
    if let Some(sd) = at.bulb {
        if sd > px {
            return None;
        }
        col = glass_shade(col, sd, px, k * (-at.d.max(0.0) / 0.35).exp(), 0.0, c, glow);
        shown = true;
    }
    let g = k * (-at.d.max(0.0) / (GLOW_R * 0.8)).exp() * glow * at.edge;
    col = glow_onto(col, c, g);
    let core = k * (0.5 - at.d / px).clamp(0.0, 1.0);
    col = mix(col, c.wire.lerp(c.hot, core * 0.7), core);
    (shown || g * 255.0 > 2.0 || core > 0.0).then(|| to_rgb(col))
}

/// Brightness of slot `n` with --flicker. Like a real tube it's mostly
/// steady: now and then (about every 20 s per tube) a short burst of one to
/// three quick dips, 60 ms each. Steady frames are drawn from the cache.
fn flicker(n: usize, now: f64) -> f32 {
    const STEP: f64 = 0.06;
    let seed = (n as u64) << 40;
    // One chance a second, at a random moment in it.
    let second = now.max(0.0).floor() as u64;
    let h = hash(seed ^ second);
    if !h.is_multiple_of(20) {
        return 1.0;
    }
    let start = (h >> 8) % 12;
    let len = 1 + (h >> 16) % 3;
    let step = (now.fract() / STEP).floor() as u64;
    if !(start..start + len).contains(&step) {
        return 1.0;
    }
    [0.92, 0.84, 0.9, 0.78][(hash(seed ^ second ^ step) % 4) as usize]
}
/// Draw the tubes. Without `lit`, only what stays while an effect plays the
/// wires: glass and the unlit cathodes.
pub fn draw(sc: &Scene, nx: &Layout, f: &mut Frame, lit: bool) {
    draw_lamps(sc, nx, f, lit, |_, slot| {
        let k = if sc.tick_fx {
            // In steps: frames between them are the same, so cached and not resent.
            let k = ((sc.now - slot.changed) / TICK_FX).clamp(0.0, 1.0) as f32;
            (k * FADE_STEPS).round() / FADE_STEPS
        } else {
            1.0
        };
        let mut on: Lit = [0.0; 11];
        if lit {
            lit_of(Some(slot.lamp), k, &mut on);
            // A change to another tube's symbols (a letter after a digit) just switches.
            let same = |p: &Lamp| matches!((p, slot.lamp), (Lamp::Tube { group: a, .. }, Lamp::Tube { group: b, .. }) if *a == b);
            lit_of(slot.prev.filter(same), 1.0 - k, &mut on);
        }
        on
    });
}

/// Draw the tubes with the cathodes `on(slot index, slot)` lights; colons
/// light with `colon_lit`.
fn draw_lamps(sc: &Scene, nx: &Layout, f: &mut Frame, colon_lit: bool, on: impl Fn(usize, &Slot) -> Lit) {
    let field = &nx.field;
    let style = &nx.style;
    let glow = style.glow * 1.4 * sc.glow_k * (1.0 + 2.0 * sc.pulse);
    let colon_lit = colon_lit && !(style.blink && sc.blink_dark);
    // A tube's cells only change when what's lit, its colors or the
    // background do: reuse them otherwise. Not with drifting colors.
    // One color for every cell: work it out once per tube.
    let uniform = matches!(style.tint, Tint::Neon | Tint::ThemeSolid);
    let cached = style.tint != Tint::ThemeHeavy;
    let mut cache = field.cache.borrow_mut();
    for (n, slot) in nx.slots.iter().enumerate() {
        let on = on(n, slot);
        let bright = if style.flicker { flicker(n, sc.now) } else { 1.0 };
        let first = (nx.dy + slot.y) * sc.tw() + nx.dx + slot.x;
        let group = match slot.lamp {
            Lamp::Tube { group, .. } => Some(group),
            Lamp::Blank => Some(Group::Signs),
            Lamp::Colon => None,
        };
        let key = SlotKey {
            w: slot.w,
            group,
            on,
            colon_lit,
            colors: sc.nixie_colors(first, bright),
            rest: sc.rest,
            glow,
            // Theme colors vary across the canvas: where the tube is in it, and which theme.
            at: (first, sc.tw(), sc.th()),
            theme: sc.theme.name.clone(),
        };
        let put = |f: &mut Frame, cells: &[Option<Cell>]| {
            for (k, &cell) in cells.iter().enumerate() {
                let (x, y) = sc.pos(k / slot.w * sc.tw() + first + k % slot.w);
                f.set(x, y, cell);
            }
        };
        let at = cache.iter().position(|(at, _)| *at == (nx.dy, n));
        if let Some((_, (_, cells))) = at.map(|i| &cache[i]).filter(|(_, (k, _))| cached && *k == key) {
            put(f, cells);
            continue;
        }
        let cath = group.map(|g| field.cathodes(g));
        let cells = if field.m == 2 {
            fine_cells(nx, slot, cath.as_deref(), &on, colon_lit, key.colors)
        } else {
            half_cells(
                sc,
                nx,
                slot,
                cath.as_deref(),
                &on,
                colon_lit,
                glow,
                bright,
                uniform.then_some(key.colors),
            )
        };
        put(f, &cells);
        if cached {
            match at {
                Some(i) => cache[i].1 = (key, cells),
                None => cache.push(((nx.dy, n), (key, cells))),
            }
        }
    }
}

/// A tube's cells in half blocks: each cell's upper and lower pixel in its
/// own color. `colors`: one set for the whole tube (else per cell).
#[allow(clippy::too_many_arguments, reason = "a tube's drawing inputs")]
fn half_cells(
    sc: &Scene,
    nx: &Layout,
    slot: &Slot,
    cath: Option<&Cathodes>,
    on: &Lit,
    colon_lit: bool,
    glow: f32,
    bright: f32,
    colors: Option<Colors>,
) -> Vec<Option<Cell>> {
    let (field, style) = (&nx.field, &nx.style);
    let px = 1.0 / field.s;
    let first = (nx.dy + slot.y) * sc.tw() + nx.dx + slot.x;
    let mut cells = Vec::with_capacity(nx.rows * slot.w);
    for row in 0..nx.rows {
        for col in 0..slot.w {
            let i = row * sc.tw() + first + col;
            let colors = colors.unwrap_or_else(|| sc.nixie_colors(i, bright));
            // The cell's upper (`y` 0) or lower pixel.
            let pixel = |y: usize| {
                let (x, y) = (col, row * 2 + y);
                match slot.lamp {
                    Lamp::Tube { .. } | Lamp::Blank => {
                        let cath = cath.expect("a tube has cathodes");
                        tube_pixel(style, field, cath, y * field.w + x, sc.rest, on, colors, glow)
                    }
                    Lamp::Colon => {
                        let k = if colon_lit { 1.0 } else { 0.0 };
                        let w = slot.w;
                        let at = SepPx {
                            d: field.colon_dist(style.separator, w, x, y),
                            bulb: (style.glass && style.separator == Separator::Dot).then(|| field.bulb_dist(w, x, y)),
                            edge: edge_fade(x, w, field.s) * edge_fade(y, field.h, field.s),
                        };
                        colon_pixel(px, sc.rest, &at, k, colors, glow)
                    }
                }
            };
            cells.push(match (pixel(0), pixel(1)) {
                (Some(t), b) => Some(Cell {
                    ch: '▀', fg: t, bg: b
                }),
                (None, Some(b)) => Some(Cell {
                    ch: '▄',
                    fg: b,
                    bg: None,
                }),
                (None, None) => None,
            });
        }
    }
    cells
}

/// The character for each octant pattern: bit `k` set lights pixel `k`
/// of the cell's 2x4, row by row. `\0`: the middle quarter of one side,
/// whose character the tested terminals weren't checked with (`octant`
/// widens it).
#[rustfmt::skip]
const OCTANTS: [char; 256] = [
    '\u{20}', '\u{1cea8}', '\u{1ceab}', '\u{1fb82}', '\u{1cd00}', '\u{2598}', '\u{1cd01}', '\u{1cd02}',
    '\u{1cd03}', '\u{1cd04}', '\u{259d}', '\u{1cd05}', '\u{1cd06}', '\u{1cd07}', '\u{1cd08}', '\u{2580}',
    '\u{1cd09}', '\u{1cd0a}', '\u{1cd0b}', '\u{1cd0c}', '\0', '\u{1cd0d}', '\u{1cd0e}', '\u{1cd0f}',
    '\u{1cd10}', '\u{1cd11}', '\u{1cd12}', '\u{1cd13}', '\u{1cd14}', '\u{1cd15}', '\u{1cd16}', '\u{1cd17}',
    '\u{1cd18}', '\u{1cd19}', '\u{1cd1a}', '\u{1cd1b}', '\u{1cd1c}', '\u{1cd1d}', '\u{1cd1e}', '\u{1cd1f}',
    '\0', '\u{1cd20}', '\u{1cd21}', '\u{1cd22}', '\u{1cd23}', '\u{1cd24}', '\u{1cd25}', '\u{1cd26}',
    '\u{1cd27}', '\u{1cd28}', '\u{1cd29}', '\u{1cd2a}', '\u{1cd2b}', '\u{1cd2c}', '\u{1cd2d}', '\u{1cd2e}',
    '\u{1cd2f}', '\u{1cd30}', '\u{1cd31}', '\u{1cd32}', '\u{1cd33}', '\u{1cd34}', '\u{1cd35}', '\u{1fb85}',
    '\u{1cea3}', '\u{1cd36}', '\u{1cd37}', '\u{1cd38}', '\u{1cd39}', '\u{1cd3a}', '\u{1cd3b}', '\u{1cd3c}',
    '\u{1cd3d}', '\u{1cd3e}', '\u{1cd3f}', '\u{1cd40}', '\u{1cd41}', '\u{1cd42}', '\u{1cd43}', '\u{1cd44}',
    '\u{2596}', '\u{1cd45}', '\u{1cd46}', '\u{1cd47}', '\u{1cd48}', '\u{258c}', '\u{1cd49}', '\u{1cd4a}',
    '\u{1cd4b}', '\u{1cd4c}', '\u{259e}', '\u{1cd4d}', '\u{1cd4e}', '\u{1cd4f}', '\u{1cd50}', '\u{259b}',
    '\u{1cd51}', '\u{1cd52}', '\u{1cd53}', '\u{1cd54}', '\u{1cd55}', '\u{1cd56}', '\u{1cd57}', '\u{1cd58}',
    '\u{1cd59}', '\u{1cd5a}', '\u{1cd5b}', '\u{1cd5c}', '\u{1cd5d}', '\u{1cd5e}', '\u{1cd5f}', '\u{1cd60}',
    '\u{1cd61}', '\u{1cd62}', '\u{1cd63}', '\u{1cd64}', '\u{1cd65}', '\u{1cd66}', '\u{1cd67}', '\u{1cd68}',
    '\u{1cd69}', '\u{1cd6a}', '\u{1cd6b}', '\u{1cd6c}', '\u{1cd6d}', '\u{1cd6e}', '\u{1cd6f}', '\u{1cd70}',
    '\u{1cea0}', '\u{1cd71}', '\u{1cd72}', '\u{1cd73}', '\u{1cd74}', '\u{1cd75}', '\u{1cd76}', '\u{1cd77}',
    '\u{1cd78}', '\u{1cd79}', '\u{1cd7a}', '\u{1cd7b}', '\u{1cd7c}', '\u{1cd7d}', '\u{1cd7e}', '\u{1cd7f}',
    '\u{1cd80}', '\u{1cd81}', '\u{1cd82}', '\u{1cd83}', '\u{1cd84}', '\u{1cd85}', '\u{1cd86}', '\u{1cd87}',
    '\u{1cd88}', '\u{1cd89}', '\u{1cd8a}', '\u{1cd8b}', '\u{1cd8c}', '\u{1cd8d}', '\u{1cd8e}', '\u{1cd8f}',
    '\u{2597}', '\u{1cd90}', '\u{1cd91}', '\u{1cd92}', '\u{1cd93}', '\u{259a}', '\u{1cd94}', '\u{1cd95}',
    '\u{1cd96}', '\u{1cd97}', '\u{2590}', '\u{1cd98}', '\u{1cd99}', '\u{1cd9a}', '\u{1cd9b}', '\u{259c}',
    '\u{1cd9c}', '\u{1cd9d}', '\u{1cd9e}', '\u{1cd9f}', '\u{1cda0}', '\u{1cda1}', '\u{1cda2}', '\u{1cda3}',
    '\u{1cda4}', '\u{1cda5}', '\u{1cda6}', '\u{1cda7}', '\u{1cda8}', '\u{1cda9}', '\u{1cdaa}', '\u{1cdab}',
    '\u{2582}', '\u{1cdac}', '\u{1cdad}', '\u{1cdae}', '\u{1cdaf}', '\u{1cdb0}', '\u{1cdb1}', '\u{1cdb2}',
    '\u{1cdb3}', '\u{1cdb4}', '\u{1cdb5}', '\u{1cdb6}', '\u{1cdb7}', '\u{1cdb8}', '\u{1cdb9}', '\u{1cdba}',
    '\u{1cdbb}', '\u{1cdbc}', '\u{1cdbd}', '\u{1cdbe}', '\u{1cdbf}', '\u{1cdc0}', '\u{1cdc1}', '\u{1cdc2}',
    '\u{1cdc3}', '\u{1cdc4}', '\u{1cdc5}', '\u{1cdc6}', '\u{1cdc7}', '\u{1cdc8}', '\u{1cdc9}', '\u{1cdca}',
    '\u{1cdcb}', '\u{1cdcc}', '\u{1cdcd}', '\u{1cdce}', '\u{1cdcf}', '\u{1cdd0}', '\u{1cdd1}', '\u{1cdd2}',
    '\u{1cdd3}', '\u{1cdd4}', '\u{1cdd5}', '\u{1cdd6}', '\u{1cdd7}', '\u{1cdd8}', '\u{1cdd9}', '\u{1cdda}',
    '\u{2584}', '\u{1cddb}', '\u{1cddc}', '\u{1cddd}', '\u{1cdde}', '\u{2599}', '\u{1cddf}', '\u{1cde0}',
    '\u{1cde1}', '\u{1cde2}', '\u{259f}', '\u{1cde3}', '\u{2586}', '\u{1cde4}', '\u{1cde5}', '\u{2588}',
];

/// A small tube's cells in octants: only its lit wires, from the shapes
/// alone (a pixel is lit or not), so a pulse or flash changes the color,
/// never the pattern. No glass, glow or unlit symbols: next to a wire they
/// need a third color, and at cell size glow and unlit symbols only smudge.
fn fine_cells(
    nx: &Layout,
    slot: &Slot,
    cath: Option<&Cathodes>,
    on: &Lit,
    colon_lit: bool,
    c: Colors,
) -> Vec<Option<Cell>> {
    let (field, style) = (&nx.field, &nx.style);
    // Within half a pixel: a centered wire lights one pixel, not two.
    let thr = (STROKE / 2.0).max(0.5 / field.s);
    let w = slot.w * 2;
    let lit = |x: usize, y: usize| match (slot.lamp, cath) {
        (Lamp::Colon, _) => colon_lit && field.colon_dist(style.separator, w, x, y) < thr,
        // Mid-fade the new symbol takes over (no blending in two colors).
        (_, Some(cath)) => (0..11).any(|k| on[k] >= 0.5 && cath.dist[y * field.w + x][k] < thr),
        (_, None) => false,
    };
    let wire = c.wire.lerp(c.hot, 0.35);
    let mut cells = Vec::with_capacity(nx.rows * slot.w);
    for row in 0..nx.rows {
        for col in 0..slot.w {
            let bits = (0..8)
                .filter(|&k| lit(col * 2 + k % 2, row * 4 + k / 2))
                .fold(0usize, |m, k| m | 1 << k);
            cells.push((bits != 0).then(|| Cell {
                ch: octant(bits),
                fg: wire,
                bg: None,
            }));
        }
    }
    cells
}

/// The octant character for pixels `bits`, widened by a pixel where that
/// pattern has none.
fn octant(mut bits: usize) -> char {
    let mut k = 0;
    while OCTANTS[bits] == '\0' {
        bits |= 1 << k;
        k += 1;
    }
    OCTANTS[bits]
}

/// A tube's cells and what they were drawn from.
type Drawn = (SlotKey, Vec<Option<Cell>>);

/// What a tube's cells were drawn from (colors held still): the same key
/// gives the same cells.
#[derive(Clone, PartialEq)]
struct SlotKey {
    w: usize,
    /// The tube's symbols (`None`: a separator).
    group: Option<Group>,
    on: Lit,
    colon_lit: bool,
    colors: Colors,
    rest: Rgb,
    glow: f32,
    at: (usize, usize, usize),
    theme: String,
}

/// Each tube in turn runs through all ten digits and back to the time, like
/// a nixie clock's cathode poisoning prevention ("slot machine").
struct SlotMachine;

/// How long each digit shows, and the delay from one tube to the next.
const SLOT_STEP: f32 = 0.07;
const SLOT_STAGGER: f32 = 0.06;

/// How long `slotmachine` runs at most: ten steps plus the stagger across
/// the tubes (squeezed to fit many tubes).
pub const SLOT_MAX: f32 = 10.5 * SLOT_STEP + 0.45;

/// Delay between tubes, squeezed so many tubes still fit `SLOT_MAX`.
fn slot_stagger(tubes: usize) -> f32 {
    SLOT_STAGGER.min(0.45 / tubes.saturating_sub(1).max(1) as f32)
}

fn slot_duration(tubes: usize) -> f32 {
    // Half a step on the time, so the last tube shows it before the end.
    tubes.saturating_sub(1) as f32 * slot_stagger(tubes) + 10.5 * SLOT_STEP
}

/// The digit a tube shows `t` seconds into its run: 0 to 9, then `digit`.
fn slot_digit(t: f32, digit: u8) -> u8 {
    let step = (t / SLOT_STEP).floor();
    if (0.0..10.0).contains(&step) { step as u8 } else { digit }
}

pub fn slot_machine() -> Box<dyn Effect> {
    Box::new(SlotMachine)
}

impl Effect for SlotMachine {
    fn draw(&mut self, sc: &Scene, t: f32, f: &mut Frame) -> bool {
        let Some(nx) = &sc.t.nixie else {
            // Only the nixie font has digits to cycle.
            sc.draw_settled(f);
            return true;
        };
        let is_digit = |s: &Slot| {
            matches!(
                s.lamp,
                Lamp::Tube {
                    group: Group::Digits,
                    ..
                }
            )
        };
        let tubes = nx.slots.iter().filter(|s| is_digit(s)).count();
        let stagger = slot_stagger(tubes);
        let mut rank = vec![0; nx.slots.len()];
        let mut r = 0;
        for (n, s) in nx.slots.iter().enumerate() {
            rank[n] = r;
            r += usize::from(is_digit(s));
        }
        draw_lamps(sc, nx, f, true, |n, slot| {
            let mut on: Lit = [0.0; 11];
            if let Lamp::Tube { group, k, comma } = slot.lamp {
                let k = if group == Group::Digits {
                    slot_digit(t - rank[n] as f32 * stagger, k)
                } else {
                    k
                };
                lit_of(Some(Lamp::Tube { group, k, comma }), 1.0, &mut on);
            }
            on
        });
        sc.draw_title(f);
        t >= slot_duration(tubes)
    }

    fn keeps_tubes(&self) -> bool {
        true
    }
}

impl Scene<'_> {
    /// Wire, core and glow colors of the tube at cell `i`, `flicker` dimmed.
    fn nixie_colors(&self, i: usize, flicker: f32) -> Colors {
        let base = self.wire_base(i);
        let hot = match self.t.nixie_look.map(|s| s.tint) {
            Some(Tint::Theme | Tint::ThemeHeavy | Tint::ThemeSolid) => base.lerp(Rgb(255, 255, 255), 0.6),
            _ => self.tint(NEON_HOT),
        };
        // The alert pulse (with this font, the tubes flare instead of the background).
        let k = self.brightness() * flicker * (1.0 + 0.5 * self.pulse);
        let wire = base.scale(k);
        let Rgb(r, g, b) = self.rest;
        let light = f32::from(r) * 0.3 + f32::from(g) * 0.6 + f32::from(b) * 0.1 > 128.0;
        Colors {
            wire,
            // A white-hot core would vanish on a light background: deepen it instead.
            hot: if light { wire.brightness(0.7) } else { hot.scale(k) },
            glow: wire.scale(0.55),
            glass: if light {
                self.rest.lerp(Rgb(40, 40, 48), 0.45)
            } else {
                GLASS
            },
            unlit: if light {
                self.rest.lerp(Rgb(70, 55, 48), 0.35)
            } else {
                UNLIT
            },
            light,
        }
    }

    /// The color of lit wire at cell `i` (also what effects play the wires in).
    pub fn wire_color(&self, i: usize) -> Rgb {
        self.wire_base(i).scale(self.brightness())
    }

    /// Lit wire before --brightness.
    fn wire_base(&self, i: usize) -> Rgb {
        match self.t.nixie_look.map(|s| s.tint) {
            Some(Tint::Neon) => self.tint(NEON),
            Some(Tint::Theme) => self.gradient_color(i, false),
            Some(Tint::ThemeSolid) => self.tint(self.theme.gradient.first().copied().unwrap_or(self.theme.flash)),
            _ => self.gradient_color(i, true),
        }
    }

    fn brightness(&self) -> f32 {
        self.t.nixie_look.map_or(1.0, |s| s.brightness)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn digit(k: u8, comma: bool) -> Lamp {
        Lamp::Tube {
            group: Group::Digits,
            k,
            comma,
        }
    }

    #[test]
    fn lays_out_tubes_and_wires() {
        let g = glyphs("12:34.5", 200, 60, Style::default(), false).expect("fits");
        let nx = g.nixie.as_ref().expect("nixie layout");
        let digits: Vec<_> = nx.slots.iter().map(|s| s.lamp).collect();
        assert_eq!(digits.len(), 6, "five tubes and a colon: {digits:?}");
        assert_eq!(digits[2], Lamp::Colon);
        // The '.' lights the comma of the tube before it.
        assert_eq!(digits[4], digit(4, true));
        assert!(g.chars.iter().any(|&c| c != ' '), "wires drawn");
        // Tubes don't overlap.
        assert!(nx.slots.windows(2).all(|p| p[0].x + p[0].w < p[1].x));
        // Nothing fits in a tiny space, and some characters have no tube.
        assert!(glyphs("12:34", 20, 5, Style::default(), false).is_none());
        // A character no tube has gets a blank one.
        let g = glyphs("50€", 200, 60, Style::default(), false).expect("fits");
        assert_eq!(g.nixie.expect("nixie layout").slots[2].lamp, Lamp::Blank);
        // No colon: a gap instead.
        let g = glyphs(
            "12:34",
            200,
            60,
            Style {
                separator: Separator::Off,
                ..Style::default()
            },
            false,
        )
        .expect("fits");
        assert_eq!(g.nixie.expect("nixie layout").slots.len(), 4);
        // One dot: a lamp low between the tubes, lit only near the bottom.
        let g = glyphs(
            "12:34",
            200,
            60,
            Style {
                separator: Separator::Dot,
                ..Style::default()
            },
            false,
        )
        .expect("fits");
        let nx = g.nixie.as_ref().expect("nixie layout");
        let dot = nx.slots[2];
        assert_eq!(dot.lamp, Lamp::Colon);
        let rows: Vec<usize> = (0..g.h)
            .filter(|r| (dot.x..dot.x + dot.w).any(|c| g.chars[r * g.w + c] != ' '))
            .collect();
        assert!(
            !rows.is_empty() && rows.iter().all(|&r| r >= g.h * 3 / 4),
            "dot rows {rows:?} of {}",
            g.h
        );
    }

    #[test]
    fn letters_go_in_symbol_tubes() {
        let lamps = |t: &str| -> Vec<(Group, char)> {
            let g = glyphs(t, 400, 60, Style::default(), false).expect("fits");
            g.nixie
                .expect("nixie layout")
                .slots
                .iter()
                .filter_map(|s| match s.lamp {
                    Lamp::Tube { group, k, .. } => Some((group, group.symbols().chars().nth(usize::from(k))?)),
                    Lamp::Colon | Lamp::Blank => None,
                })
                .collect()
        };
        // Units after a number; the same letters elsewhere are capitals.
        assert_eq!(
            lamps("2d 1h"),
            [
                (Group::Digits, '2'),
                (Group::Units, 'd'),
                (Group::Digits, '1'),
                (Group::Units, 'h')
            ]
        );
        assert_eq!(
            lamps("dam!"),
            [
                (Group::Letters(0), 'D'),
                (Group::Letters(0), 'A'),
                (Group::Letters(1), 'M'),
                (Group::Signs, '!')
            ]
        );
        assert_eq!(lamps("ZOO")[0], (Group::Letters(2), 'Z'));
        // Every symbol has a wire, and each group fits a tube's eleven cathodes.
        for g in [
            Group::Letters(0),
            Group::Letters(1),
            Group::Letters(2),
            Group::Units,
            Group::Signs,
        ] {
            assert!(g.symbols().len() <= 11);
            for k in 0..g.symbols().len() as u8 {
                assert!(!g.strokes(k).is_empty(), "{g:?} {k}");
            }
        }
    }

    #[test]
    fn long_text_wraps_into_bigger_tubes() {
        let text = "break time, now, get coffee!";
        let style = Style::default();
        // On one line it only fits in small tubes, if at all.
        let one = glyphs(text, 279, 70, style, false);
        let wrapped = glyphs(text, 279, 70, style, true).expect("fits wrapped");
        let nx = wrapped.nixie.as_ref().expect("nixie layout");
        let lines: std::collections::BTreeSet<usize> = nx.slots.iter().map(|s| s.y).collect();
        assert!(lines.len() > 1, "wrapped onto {} lines", lines.len());
        let tube = |g: &Glyphs| g.nixie.as_ref().map_or(0, |n| n.slots[0].w);
        assert!(
            one.as_ref().is_none_or(|g| tube(g) < tube(&wrapped)),
            "bigger tubes when wrapped"
        );
        assert!(wrapped.w <= 279 && wrapped.h <= 70);
        // Commas and a period after a word are signs; after a digit, the comma cathode.
        let signs = |t: &str| {
            glyphs(t, 400, 60, style, false)
                .expect("fits")
                .nixie
                .expect("nixie layout")
                .slots
                .iter()
                .filter(|s| {
                    matches!(
                        s.lamp,
                        Lamp::Tube {
                            group: Group::Signs,
                            ..
                        }
                    )
                })
                .count()
        };
        assert_eq!(signs("NOW, GO."), 2);
        assert_eq!(signs("12.5"), 0);
        // The clock itself never wraps.
        let clock = glyphs("2d 03:04:05", 279, 70, style, false).expect("fits");
        assert!(clock.nixie.expect("nixie layout").slots.iter().all(|s| s.y == 0));
    }

    #[test]
    fn a_fallback_font_keeps_the_tube_color() {
        let themes = crate::theme::builtin();
        let mut target = crate::scene::Target::empty();
        target.update(crate::font::block("WAIT", 1), 0.0, usize::MAX);
        target.nixie_look = Some(Style::default());
        let sc = Scene {
            t: &target,
            ox: 0,
            oy: 0,
            sw: 80,
            sh: 24,
            theme: &themes[0],
            now: 0.0,
            paused: false,
            done: false,
            tick_fx: false,
            urgency: 0.0,
            rest: themes[0].bg,
            glow_k: 1.0,
            blink_dark: false,
            pulse: 0.0,
        };
        let i = (0..sc.len()).find(|&i| sc.ch(i) != ' ').expect("a lit cell");
        assert_eq!(sc.settled(i).expect("drawn").fg, NEON);
        // Done, the tubes flare in their own color, never fade toward the flash color.
        for now in [0.0, 0.1, 0.2, 0.3] {
            let done = Scene { done: true, now, ..sc };
            let Rgb(r, g, b) = done.settled(i).expect("drawn").fg;
            // Scaled (a flare clips red, nudging it a little toward yellow),
            // but still saturated orange: a pale flash color would lift blue.
            assert!(r > 180 && g < 170 && b < 40, "orange at {now}: {r},{g},{b}");
        }
    }

    #[test]
    fn title_tubes_sit_under_the_clock() {
        let clock = glyphs("12:34", 200, 40, Style::default(), false).expect("fits");
        assert!(title("deep work", 136, Style::default()).is_some(), "a two-word title");
        let small = title("focus", 200, Style::default()).expect("fits");
        let (g, by) = crate::font::stack(&clock, 1, &small);
        let nt = g.nixie_title.expect("title tubes");
        assert_eq!(nt.dy, by);
        assert_eq!(nt.slots.len(), 5);
        assert!(title("a much too long title for this", 40, Style::default()).is_none());
    }

    #[test]
    fn every_digit_has_its_own_wire() {
        let field = Field::new(14, 10, false, 1);
        for a in 0..10 {
            let on = |d: usize| {
                field
                    .cathodes(Group::Digits)
                    .dist
                    .iter()
                    .filter(|p| p[d] < STROKE)
                    .count()
            };
            assert!(on(a) > 10, "digit {a} barely drawn");
            for b in 0..a {
                let differ = field
                    .cathodes(Group::Digits)
                    .dist
                    .iter()
                    .filter(|p| (p[a] < STROKE) != (p[b] < STROKE))
                    .count();
                assert!(differ > 10, "digits {a} and {b} look alike");
            }
        }
    }

    #[test]
    fn straight_wires_sit_on_pixels() {
        // The 4's upright and crossbar get full-strength pixels at any size.
        for tube_w in MIN_TUBE_W..40 {
            let rows = ((H + 2.0 * PAD) * tube_w as f32 / (1.0 + 2.0 * PAD) / 2.0).ceil() as usize;
            let f = Field::new(tube_w, rows, false, 1);
            let full = f.cathodes(Group::Digits).dist.iter().filter(|p| p[4] < 1e-3).count();
            assert!(
                full as f32 > 1.5 * f.s,
                "tube {tube_w}: only {full} pixels on the 4's wires"
            );
        }
    }

    #[test]
    fn small_tubes_keep_bright_wires() {
        // A sub-pixel wire spreads over half-lit pixels (down to 0.58 at 8
        // cells); a curve between pixel centers still loses a little.
        for tube_w in MIN_TUBE_W..40 {
            let f = Field::new(tube_w, tube_rows(tube_w), false, 1);
            let cath = f.cathodes(Group::Digits);
            for k in 0..10 {
                let peak = cath.shade.iter().map(|s| s.core[k]).fold(0.0, f32::max);
                assert!(peak > 0.85, "tube {tube_w}: digit {k} peaks at {peak}");
            }
        }
    }

    #[test]
    fn slot_machine_runs_every_tube_through_all_digits() {
        for tubes in 1..=9 {
            let (stagger, end) = (slot_stagger(tubes), slot_duration(tubes));
            assert!(end <= SLOT_MAX, "{tubes} tubes take {end}s");
            for rank in 0..tubes {
                let mut seen = [false; 10];
                let mut t = 0.0;
                while t < end {
                    seen[slot_digit(t - rank as f32 * stagger, 7) as usize] = true;
                    t += 1.0 / 120.0;
                }
                assert!(seen.iter().all(|&s| s), "tube {rank} of {tubes} skipped a digit");
                assert_eq!(slot_digit(end - rank as f32 * stagger, 7), 7, "back to the time");
            }
        }
    }

    #[test]
    fn a_shared_cathode_stays_lit_through_the_fade() {
        let (old, new) = (digit(8, true), digit(7, true));
        for k in [0.0, 0.3, 0.5, 0.8, 1.0] {
            let mut lit: Lit = [0.0; 11];
            lit_of(Some(new), k, &mut lit);
            lit_of(Some(old), 1.0 - k, &mut lit);
            assert!(
                (lit[COMMA_I] - 1.0).abs() < 1e-6,
                "comma at {} with k = {k}",
                lit[COMMA_I]
            );
            assert!((lit[7] - k).abs() < 1e-6 && (lit[8] - (1.0 - k)).abs() < 1e-6);
        }
    }

    #[test]
    fn cached_tubes_draw_the_same_and_follow_changes() {
        for tint in [Tint::Neon, Tint::Theme, Tint::ThemeSolid] {
            cache_follows_changes(tint);
        }
    }

    fn cache_follows_changes(tint: Tint) {
        let themes = crate::theme::builtin();
        let style = Style {
            blink: true,
            tint,
            ..Style::default()
        };
        let mut target = crate::scene::Target::empty();
        target.update(glyphs("12:34", 120, 36, style, false).expect("fits"), 0.0, usize::MAX);
        let mut sc = Scene {
            t: &target,
            ox: 2,
            oy: 2,
            sw: 120,
            sh: 36,
            theme: &themes[0],
            now: 10.0,
            paused: false,
            done: false,
            tick_fx: true,
            urgency: 0.0,
            rest: themes[0].bg,
            glow_k: 1.0,
            blink_dark: false,
            pulse: 0.0,
        };
        let frame = |sc: &Scene| {
            let mut f = Frame::new(120, 36);
            sc.draw_settled(&mut f);
            f
        };
        let first = frame(&sc);
        assert!(first == frame(&sc), "drawn again from the cache");
        sc.blink_dark = true;
        let dark = frame(&sc);
        assert!(dark != first, "the colon went dark");
        sc.blink_dark = false;
        sc.ox = 3;
        let moved = frame(&sc);
        assert!(moved != first && moved != dark, "drawn where the clock is now");
        sc.paused = true;
        let paused = frame(&sc);
        assert!(paused != moved, "dimmed while paused");
        sc.paused = false;
        sc.theme = &themes[1];
        if tint != Tint::Neon {
            assert!(frame(&sc) != moved, "another theme's colors");
        }
    }

    #[test]
    fn colors_suit_light_and_dark_backgrounds() {
        let themes = crate::theme::builtin();
        let mut target = crate::scene::Target::empty();
        target.update(
            glyphs("12:34", 120, 36, Style::default(), false).expect("fits"),
            0.0,
            usize::MAX,
        );
        let lum = |c: Rgb| u32::from(c.0) + u32::from(c.1) + u32::from(c.2);
        for (rest, light) in [(Rgb(12, 12, 12), false), (Rgb(250, 248, 240), true)] {
            let sc = Scene {
                t: &target,
                ox: 0,
                oy: 0,
                sw: 120,
                sh: 36,
                theme: &themes[0],
                now: 0.0,
                paused: false,
                done: false,
                tick_fx: true,
                urgency: 0.0,
                rest,
                glow_k: 1.0,
                blink_dark: false,
                pulse: 0.0,
            };
            let c = sc.nixie_colors(0, 1.0);
            assert_eq!(c.light, light);
            // The glass stands out from the background either way, and the
            // core stays visible: brighter than the wire on dark, deeper on light.
            assert_eq!(lum(c.glass) < lum(rest), light, "glass {:?} on {rest:?}", c.glass);
            assert_eq!(lum(c.hot) < lum(c.wire), light, "core {:?}, wire {:?}", c.hot, c.wire);
        }
    }

    #[test]
    fn flicker_is_rare_and_brief() {
        // A minute at 60 fps for 8 tubes: steady nearly all the time, with a
        // few bursts, each over in a fraction of a second.
        let mut dim = 0;
        let mut bursts = 0;
        for n in 0..8 {
            let mut was = 1.0;
            for f in 0..3600 {
                let b = flicker(n, 100.0 + f64::from(f) / 60.0);
                assert!((0.78..=1.0).contains(&b));
                if b < 1.0 {
                    dim += 1;
                    bursts += usize::from(was >= 1.0);
                }
                was = b;
            }
        }
        assert!((5..60).contains(&bursts), "{bursts} bursts in 8 tube-minutes");
        assert!(dim < 8 * 3600 / 50, "dim in {dim} of {} frames", 8 * 3600);
    }

    #[test]
    fn small_tubes_in_octants_keep_their_pattern() {
        let style = Style {
            octants: true,
            ..Style::default()
        };
        // In a small window the tubes are small, so in octants; big ones aren't.
        let g = glyphs("00:14", 50, 20, style, false).expect("fits");
        assert_eq!(g.nixie.as_ref().expect("nixie layout").field.m, 2);
        let big = glyphs("00:14", 200, 60, style, false).expect("fits");
        assert_eq!(big.nixie.expect("nixie layout").field.m, 1);
        // A title fits a 66-column window in small tubes.
        assert!(title("deep work", 62, style).is_some(), "the title fits in small tubes");
        assert!(
            title("deep work", 62, Style::default()).is_none(),
            "but not in half-block ones"
        );
        // A pulse changes the colors, never which characters are drawn.
        let themes = crate::theme::builtin();
        let mut target = crate::scene::Target::empty();
        target.update(g, 0.0, usize::MAX);
        let frame = |pulse: f32| {
            let sc = Scene {
                t: &target,
                ox: 0,
                oy: 0,
                sw: 66,
                sh: 38,
                theme: &themes[0],
                now: 10.0,
                paused: false,
                done: false,
                tick_fx: true,
                urgency: 0.6,
                rest: themes[0].bg,
                glow_k: 1.0,
                blink_dark: false,
                pulse,
            };
            let mut f = Frame::new(66, 38);
            sc.draw_settled(&mut f);
            f
        };
        let (calm, flare) = (frame(0.0), frame(1.0));
        assert!(calm != flare, "the flare shows");
        assert_eq!(calm.chars(), flare.chars(), "the same pattern");
    }

    #[test]
    fn a_change_fades_over() {
        let old = glyphs("12", 200, 60, Style::default(), false)
            .and_then(|g| g.nixie)
            .expect("fits");
        let mut new = glyphs("13", 200, 60, Style::default(), false)
            .and_then(|g| g.nixie)
            .expect("fits");
        new.follow(&old, 5.0, usize::MAX);
        assert_eq!(new.slots[0].prev, None);
        assert_eq!(new.slots[1].prev, Some(digit(2, false)));
        assert!((new.slots[1].changed - 5.0).abs() < f64::EPSILON);
        // Past `fx_until` (fractions): no fade.
        let mut new = glyphs("13", 200, 60, Style::default(), false)
            .and_then(|g| g.nixie)
            .expect("fits");
        new.follow(&old, 5.0, 1);
        assert_eq!(new.slots[1].prev, None);
    }
}
