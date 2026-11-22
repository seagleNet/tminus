//! Text effects in the spirit of terminaltexteffects (tte), which Omarchy's
//! screensaver runs over its logo. Every effect ends with the text at rest.
//!
//! Effects read the *current* target chars at draw time, so the clock keeps
//! ticking correctly while an effect is mid-animation.

mod light;
mod motion;

use rand::Rng;

use crate::{scene::Scene, term::Frame, theme::Rgb};

pub trait Effect {
    /// Draw the effect at `t` seconds after it started. Returns true when done.
    fn draw(&mut self, sc: &Scene, t: f32, f: &mut Frame) -> bool;

    /// True if it draws the whole nixie tubes itself, glow included (rather
    /// than playing the wires over the glass).
    fn keeps_tubes(&self) -> bool {
        false
    }
}

/// The available effects; clap derives the CLI names (`decrypt`, `rain`, ...).
#[derive(Clone, Copy, PartialEq, Eq, Debug, clap::ValueEnum)]
pub enum Kind {
    Decrypt,
    Rain,
    Beams,
    Slide,
    Swarm,
    /// Digital rain over the clock (light: still symbols, far less output)
    Matrix,
    /// The full-screen, constantly flickering version (much more output)
    MatrixHeavy,
    Burn,
    Print,
    Expand,
    Waves,
    Wipe,
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
    /// Nixie font only: every tube runs through all ten digits, like a nixie
    /// clock keeping its cathodes healthy
    Slotmachine,
    /// No effect at all
    #[value(name = "none")]
    Off,
}

/// The effect names for `--list-effects`, one per line. In a terminal, ones
/// with a note (`matrix`, `matrix-heavy`, `slotmachine`, `none`) get it beside them, dimmed;
/// piped, it's plain names for scripts.
pub fn list(color: bool) -> String {
    use clap::ValueEnum;
    use std::fmt::Write;
    let values: Vec<clap::builder::PossibleValue> = Kind::value_variants()
        .iter()
        .filter_map(ValueEnum::to_possible_value)
        .collect();
    let width = values.iter().map(|v| v.get_name().len()).max().unwrap_or(0);
    let mut out = String::new();
    for v in &values {
        let name = v.get_name();
        // Writing to a String can't fail.
        let _ = match v.get_help().filter(|_| color) {
            Some(help) => writeln!(out, "{name:width$}  \x1b[2m{help}\x1b[0m"),
            None => writeln!(out, "{name}"),
        };
    }
    out
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
            Kind::Slotmachine => crate::nixie::SLOT_MAX,
            Kind::Expand => 1.2,
            Kind::Highlight => 1.5,
            Kind::Middleout => 1.8,
            Kind::Slice => 1.7,
            Kind::Wipe => 1.9,
            // Print: typing only (at most 1.8s, then the last character's
            // shades). The head's returns (`light::PRINT_RETURNS`) come on
            // top, and the final seconds may cut them off: the rows are
            // readable by then.
            Kind::Overflow | Kind::Scattered | Kind::Sweep | Kind::Slide | Kind::Print => 2.1,
            Kind::Randomsequence => 2.4,
            Kind::Pour | Kind::Rain => 2.8,
            Kind::Waves => 2.9,
            Kind::Burn | Kind::Bouncyballs | Kind::Colorshift | Kind::Laseretch | Kind::Smoke | Kind::Spray => 3.1,
            Kind::Orbittingvolley | Kind::Synthgrid | Kind::Vhstape => 3.3,
            Kind::Binarypath | Kind::Errorcorrect => 3.4,
            // Beams: its closing wipe takes longer on wide clocks (nixie tubes).
            Kind::Beams | Kind::Decrypt | Kind::Crumble | Kind::Spotlights => 3.5,
            Kind::Unstable => 3.6,
            Kind::Bubbles | Kind::Thunderstorm => 3.7,
            Kind::Swarm => 4.3,
            Kind::Blackhole | Kind::Rings | Kind::Fireworks => 4.1,
            // Rain, a full screen of symbols, then the text resolving out of it.
            Kind::Matrix | Kind::MatrixHeavy => 4.4,
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
    match kind {
        Kind::Off => Box::new(Settled),
        Kind::Slotmachine => crate::nixie::slot_machine(),
        Kind::Beams => light::beams(sc, rng),
        Kind::Binarypath => motion::binarypath(sc, rng),
        Kind::Blackhole => motion::blackhole(sc, rng),
        Kind::Bouncyballs => motion::bouncyballs(sc, rng),
        Kind::Bubbles => motion::bubbles(sc, rng),
        Kind::Burn => light::burn(sc, rng),
        Kind::Colorshift => Box::new(light::ColorShift),
        Kind::Crumble => motion::crumble(sc, rng),
        Kind::Decrypt => light::decrypt(sc, rng),
        Kind::Errorcorrect => motion::errorcorrect(sc, rng),
        Kind::Expand => motion::expand(sc, rng),
        Kind::Fireworks => motion::fireworks(sc, rng),
        Kind::Highlight => Box::new(light::Highlight),
        Kind::Laseretch => light::laseretch(sc, rng),
        Kind::Matrix => light::matrix(sc, rng, false),
        Kind::MatrixHeavy => light::matrix(sc, rng, true),
        Kind::Middleout => motion::middleout(sc, rng),
        Kind::Orbittingvolley => motion::orbittingvolley(sc, rng),
        Kind::Overflow => light::overflow(sc, rng),
        Kind::Pour => motion::pour(sc, rng),
        Kind::Print => light::print(sc, rng),
        Kind::Rain => motion::rain(sc, rng),
        Kind::Randomsequence => light::randomsequence(sc, rng),
        Kind::Rings => motion::rings(sc, rng),
        Kind::Scattered => motion::scattered(sc, rng),
        Kind::Slice => motion::slice(sc, rng),
        Kind::Slide => motion::slide(sc, rng),
        Kind::Smoke => light::smoke(sc, rng),
        Kind::Spotlights => light::spotlights(sc, rng),
        Kind::Spray => motion::spray(sc, rng),
        Kind::Swarm => motion::swarm(sc, rng),
        Kind::Sweep => Box::new(light::Sweep),
        Kind::Synthgrid => light::synthgrid(sc, rng),
        Kind::Thunderstorm => light::thunderstorm(sc, rng),
        Kind::Unstable => motion::unstable(sc, rng),
        Kind::Vhstape => light::vhstape(sc, rng),
        Kind::Waves => Box::new(light::Waves),
        Kind::Wipe => light::wipe(sc, rng),
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

const WHITE: Rgb = Rgb(255, 255, 255);

/// Colors from `#rrggbb` strings. Use it as `const { hexes([...]) }`, so a
/// typo fails the build, and nothing is parsed per frame.
const fn hexes<const N: usize>(list: [&str; N]) -> [Rgb; N] {
    let mut out = [Rgb(0, 0, 0); N];
    let mut i = 0;
    while i < N {
        out[i] = Rgb::hex(list[i]);
        i += 1;
    }
    out
}

/// Pick from a color list by a fraction in 0..1, first to last, blending neighbours.
fn palette(colors: &[Rgb], u: f32) -> Rgb {
    let x = u.clamp(0.0, 1.0) * (colors.len() - 1) as f32;
    let i = (x as usize).min(colors.len() - 1);
    colors[i].lerp(colors[(i + 1).min(colors.len() - 1)], x.fract())
}

fn max(v: &[f32]) -> f32 {
    v.iter().copied().fold(0.0, f32::max)
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
        let nixie = crate::nixie::glyphs("24:59", 120, 36, crate::nixie::Style::default(), false).expect("fits");
        let layouts = [
            (font::shadow("24:59"), 120, 36),
            (font::block("1:24:59", 3), 220, 64),
            (nixie, 120, 36),
        ];
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
                rest: themes[0].bg,
                glow_k: 1.0,
                blink_dark: false,
                pulse: 0.0,
            };
            for &kind in Kind::value_variants() {
                for seed in 0..3 {
                    let mut fx = make(kind, &sc, &mut StdRng::seed_from_u64(seed));
                    let mut frame = Frame::new(sw as u16, sh as u16);
                    let mut t = 0.0;
                    // Print's head returns don't count (see `max_duration`).
                    let allowed = kind.max_duration() + if kind == Kind::Print { light::PRINT_RETURNS } else { 0.0 };
                    while !fx.draw(&sc, t, &mut frame) {
                        t += 1.0 / 60.0;
                        assert!(t <= allowed, "{kind:?} ran past its max_duration on {sw}x{sh}");
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
        let nixie = |t, w, h| crate::nixie::glyphs(t, w, h, crate::nixie::Style::default(), false).expect("fits");
        let themes = theme::builtin();
        let layouts = [
            (font::shadow("24:59"), 80, 24),
            (font::shadow("24:59"), 120, 36),
            (font::shadow("1:24:59"), 200, 50),
            (font::block("1:24:59", 3), 220, 64),
            (font::block("59", 6), 120, 60),
            (nixie("24:59", 120, 36), 120, 36),
            (nixie("1:24:59", 220, 64), 220, 64),
            (nixie("59", 80, 24), 80, 24),
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
                        nixie: g.nixie.clone(),
                        nixie_title: g.nixie_title.clone(),
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
                    rest: themes[0].bg,
                    glow_k: 1.0,
                    blink_dark: false,
                    pulse: 0.0,
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
            (
                crate::nixie::glyphs("12:34.5", 120, 40, crate::nixie::Style::default(), false).expect("fits"),
                120,
                40,
            ),
            // `--done-text ""`: nothing to show.
            (font::plain(""), 80, 24),
            // A terminal that reports no size yet.
            (font::plain("0:01"), 0, 0),
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
                        rest: themes[0].bg,
                        glow_k: 1.0,
                        blink_dark: false,
                        pulse: 0.0,
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
