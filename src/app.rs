//! The running countdown: timing and sequence logic (`tick`, keys, remote
//! commands) and drawing it (`draw`). Headless mode uses only the former.

use std::time::Duration;

use crossterm::event::{KeyCode, KeyModifiers};
use rand::{RngExt, seq::IndexedRandom};

use crate::{
    cli::{
        Cli, DEMO_KEYS, KEYS, StatusAlign, StatusPos, StatusTime, TitleSize, format_clock, format_coarse, format_span,
        format_ticks, parse_duration,
    },
    clock::Timer,
    control::json_str,
    effects::{self, Moment, Pools},
    font,
    scene::{Scene, Target, wave},
    term::{Frame, str_width},
    theme::{Rgb, Theme},
};

/// With --effect-align, how long before a mark the effect for it is picked:
/// longer than any effect runs.
const ALIGN_LEAD: f64 = 5.7;

/// Seconds the background keeps flashing after zero (or a segment change).
const DONE_FLASH_FOR: f64 = 3.0;

/// One segment of the countdown (a plain timer is a single segment).
#[derive(Clone, Debug)]
pub struct Step {
    pub dur: Duration,
    pub label: Option<String>,
    /// Index into the theme list, if this segment has its own theme.
    pub theme: Option<usize>,
}

/// What happened when the current segment reached zero. `done` is the
/// label of the segment that ended, `next` the label of the one after it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// A segment ended and the next one started (or, with --confirm, waits).
    SegmentDone { done: Option<String>, next: Option<String> },
    /// The last segment ended.
    Completed { done: Option<String> },
}

/// An effect waiting to start so that it ends on a round number (--effect-align).
struct Pending {
    fx: Box<dyn effects::Effect>,
    /// Time left on the clock when it should end, in seconds.
    end_left: f64,
    /// How long it runs.
    dur: f64,
}

/// `--demo`: one effect at a time on a loop, chosen with the arrow keys.
struct Demo {
    /// Every effect (but `none`), in `--list-effects` order.
    kinds: Vec<effects::Kind>,
    /// Index into `kinds` of the effect being shown.
    pick: usize,
    /// Move on to the next effect after each one.
    auto: bool,
    /// The clock's normal duration, restored after a final-seconds preview.
    full: Duration,
}

impl Demo {
    /// Pause between replays of the effect.
    const GAP: f64 = 1.2;

    fn new(start: &[effects::Kind], full: Duration) -> Self {
        use clap::ValueEnum;
        let kinds: Vec<effects::Kind> = effects::Kind::value_variants()
            .iter()
            .copied()
            .filter(|k| *k != effects::Kind::Off)
            .collect();
        let pick = start
            .first()
            .and_then(|k| kinds.iter().position(|x| x == k))
            .unwrap_or(0);
        Demo {
            kinds,
            pick,
            auto: false,
            full,
        }
    }

    fn kind(&self) -> effects::Kind {
        self.kinds[self.pick]
    }

    /// The effect's CLI name.
    fn name(&self) -> String {
        use clap::ValueEnum;
        self.kind()
            .to_possible_value()
            .map(|v| v.get_name().to_string())
            .unwrap_or_default()
    }

    fn step(&mut self, forward: bool) {
        self.pick = cycle(self.pick, self.kinds.len(), forward);
    }
}

/// One step from `i` around a list of `len`, either way.
fn cycle(i: usize, len: usize, forward: bool) -> usize {
    if forward { (i + 1) % len } else { (i + len - 1) % len }
}

/// A random index below `len` other than `i` (if there is another).
fn random_other(rng: &mut impl rand::Rng, i: usize, len: usize) -> usize {
    if len < 2 {
        return i;
    }
    (i + rng.random_range(1..len)) % len
}

/// What a remote command asks of the main loop.
#[derive(Debug)]
pub enum Reply {
    Ok,
    Stop,
}

pub struct App<'a> {
    pub cli: &'a Cli,
    pools: Pools,
    themes: &'a [Theme],
    /// The terminal's own background, where the alert flash fades back to.
    pub term_bg: Option<Rgb>,
    /// Index into `themes` currently shown.
    theme: usize,
    /// The theme chosen by the user, shown by segments without their own.
    base_theme: usize,
    plan: Vec<Step>,
    seg: usize,
    timer: Timer,
    /// Time left and elapsed in the current segment, sampled once per frame.
    left: Duration,
    elapsed: Duration,
    target: Target,
    /// What the target was last built from: (text, title, width, height, large title).
    layout_key: Option<(String, Option<String>, i32, i32, bool)>,
    pub frame: Frame,
    /// The playing effect and the time it started.
    effect: Option<(Box<dyn effects::Effect>, f64)>,
    /// An aligned effect waiting for its moment.
    pending: Option<Pending>,
    /// With --effect-align: the mark (time left) the next effect should end
    /// on; `None` until a rhythm starts.
    next_mark: Option<f64>,
    last_effect: Option<effects::Kind>,
    next_effect_at: f64,
    /// Which moment the next effect is for (picks its pool).
    next_moment: Moment,
    /// When the current segment hit zero. On the last segment that means the
    /// whole countdown is done; earlier, it means waiting for --confirm.
    finished_at: Option<f64>,
    /// When the last automatic segment change happened (for its flash).
    switched_at: Option<f64>,
    title_large: bool,
    status_time: StatusTime,
    /// Whether the keybinding popup is open.
    help: bool,
    /// Set in `--demo` mode.
    demo: Option<Demo>,
}

impl<'a> App<'a> {
    pub fn new(cli: &'a Cli, pools: Pools, themes: &'a [Theme], theme: usize, plan: Vec<Step>, timer: Timer) -> Self {
        let demo = cli.demo.then(|| Demo::new(&cli.effect, plan[0].dur));
        let mut app = App {
            cli,
            pools,
            themes,
            term_bg: None,
            theme: plan[0].theme.unwrap_or(theme),
            base_theme: theme,
            plan,
            seg: 0,
            timer,
            left: Duration::ZERO,
            elapsed: Duration::ZERO,
            target: Target::empty(),
            layout_key: None,
            frame: Frame::new(0, 0),
            effect: None,
            pending: None,
            next_mark: None,
            last_effect: None,
            next_effect_at: 0.0, // play one right away
            next_moment: Moment::Intro,
            finished_at: None,
            switched_at: None,
            title_large: cli.title_size == TitleSize::Large,
            status_time: cli.status_time,
            help: false,
            demo,
        };
        app.sample();
        app
    }

    // --- state ------------------------------------------------------------

    fn is_last(&self) -> bool {
        self.seg + 1 >= self.plan.len()
    }

    /// The whole countdown (every segment) has run out.
    pub fn completed(&self) -> bool {
        self.finished_at.is_some() && self.is_last()
    }

    /// Between segments, waiting for the user (--confirm).
    fn waiting(&self) -> bool {
        self.finished_at.is_some() && !self.is_last()
    }

    fn title(&self) -> Option<&str> {
        self.plan[self.seg].label.as_deref().or(self.cli.title.as_deref())
    }

    /// Read the clock; everything in a frame works from this one sample.
    fn sample(&mut self) {
        self.elapsed = self.timer.elapsed();
        self.left = self.timer.total.saturating_sub(self.elapsed);
    }

    /// Advance time: sample the clock and handle the current segment reaching zero.
    pub fn tick(&mut self, now: f64) -> Option<Event> {
        self.sample();
        if let (Some(demo), Some(at)) = (&self.demo, self.finished_at)
            && now - at > DONE_FLASH_FOR + 1.0
        {
            self.timer = Timer::new(demo.full);
            self.finished_at = None;
            self.next_effect_at = now;
            self.sample();
            return None;
        }
        if self.finished_at.is_some() || !self.left.is_zero() {
            return None;
        }
        self.next_moment = Moment::Zero;
        let done = self.plan[self.seg].label.clone();
        if self.is_last() {
            self.finished_at = Some(now);
            self.next_effect_at = now;
            return Some(Event::Completed { done });
        }
        let next = self.plan[self.seg + 1].label.clone();
        if self.cli.confirm {
            self.finished_at = Some(now);
            self.next_effect_at = now;
        } else {
            self.switched_at = Some(now);
            self.go_to(self.seg + 1, now);
        }
        Some(Event::SegmentDone { done, next })
    }

    /// Start segment `seg` from its beginning.
    fn go_to(&mut self, seg: usize, now: f64) {
        self.seg = seg.min(self.plan.len() - 1);
        self.timer = Timer::new(self.plan[self.seg].dur);
        self.finished_at = None;
        self.theme = self.plan[self.seg].theme.unwrap_or(self.base_theme);
        self.effect = None;
        self.pending = None;
        self.next_mark = None;
        self.next_effect_at = now;
        self.next_moment = Moment::Zero;
        self.sample();
    }

    fn next(&mut self, now: f64) -> bool {
        if self.is_last() {
            return false;
        }
        self.go_to(self.seg + 1, now);
        true
    }

    fn back(&mut self, now: f64) {
        self.go_to(self.seg.saturating_sub(1), now);
    }

    fn restart(&mut self, now: f64) {
        self.pending = None;
        self.next_mark = None;
        self.timer = Timer::new(self.timer.total);
        self.finished_at = None;
        self.next_effect_at = now;
        self.next_moment = Moment::Intro;
        self.sample();
    }

    fn add(&mut self, d: Duration) {
        self.timer.total += d;
        self.finished_at = None;
        self.sample();
    }

    fn sub(&mut self, d: Duration) {
        self.timer.total = self.timer.total.saturating_sub(d);
        self.sample();
    }

    fn set_paused(&mut self, paused: bool) {
        if self.finished_at.is_none() && self.timer.paused() != paused {
            self.timer.toggle();
        }
    }

    /// Handle a key press. Returns true if the user asked to quit.
    pub fn on_key(&mut self, code: KeyCode, mods: KeyModifiers, now: f64) -> bool {
        if self.demo.is_some() && !self.help && self.on_demo_key(code, now) {
            return false;
        }
        match code {
            KeyCode::Char('c') if mods.contains(KeyModifiers::CONTROL) => return true,
            KeyCode::Char('h' | '?') => self.help = !self.help,
            // q and Esc close the popup first; with no popup they quit.
            KeyCode::Char('q') | KeyCode::Esc if self.help => self.help = false,
            KeyCode::Char('q') | KeyCode::Esc => return true,
            // While waiting between segments, these move on.
            KeyCode::Char(' ' | 'p' | 'n') | KeyCode::Enter if self.waiting() => {
                self.next(now);
            }
            KeyCode::Char(' ' | 'p') => self.set_paused(!self.timer.paused()),
            KeyCode::Char('n') => {
                self.next(now);
            }
            KeyCode::Char('b') => self.back(now),
            KeyCode::Char('e') | KeyCode::Enter => self.effect_now(now),
            KeyCode::Char('t') => {
                self.base_theme = (self.theme + 1) % self.themes.len();
                self.theme = self.base_theme;
            }
            KeyCode::Char('s') => self.title_large = !self.title_large,
            KeyCode::Char('c') => self.status_time = self.status_time.next(),
            KeyCode::Char('+' | '=') => self.add(Duration::from_secs(60)),
            KeyCode::Char('-' | '_') => self.sub(Duration::from_secs(60)),
            KeyCode::Char('r') => self.restart(now),
            _ => {}
        }
        false
    }

    /// Keys of `--demo` mode. Returns false for keys it leaves to `on_key`.
    fn on_demo_key(&mut self, code: KeyCode, now: f64) -> bool {
        let Some(demo) = &mut self.demo else { return false };
        match code {
            KeyCode::Left | KeyCode::Right => {
                self.base_theme = cycle(self.theme, self.themes.len(), code == KeyCode::Right);
                self.theme = self.base_theme;
                return true;
            }
            KeyCode::Up | KeyCode::Down => demo.step(code == KeyCode::Down),
            KeyCode::Char('a') => {
                demo.auto = !demo.auto;
                return true;
            }
            KeyCode::Char('r') => demo.pick = random_other(&mut rand::rng(), demo.pick, demo.kinds.len()),
            KeyCode::Char('f') => {
                // Count down through the final seconds from just before them.
                self.timer = Timer::new(Duration::from_secs_f64(self.final_window() + 2.0));
                self.finished_at = None;
                self.sample();
            }
            KeyCode::Char('e' | ' ') | KeyCode::Enter => {}
            _ => return false,
        }
        // Show the (new) effect right away, from the start.
        self.effect = None;
        self.effect_now(now);
        true
    }

    /// In `--demo` mode: the chosen theme and effect, as their CLI names.
    pub fn demo_choice(&self) -> Option<(String, String)> {
        let demo = self.demo.as_ref()?;
        Some((self.themes[self.theme].name.clone(), demo.name()))
    }

    /// Play an effect as soon as possible (aligned: on the next reachable mark).
    fn effect_now(&mut self, now: f64) {
        self.next_effect_at = now;
        self.next_mark = None;
    }

    /// Run a command from `t- ctl` (see `control`).
    pub fn remote(&mut self, line: &str, now: f64) -> Result<Reply, String> {
        let mut words = line.split_whitespace();
        let cmd = words.next().unwrap_or_default();
        let arg = words.collect::<Vec<_>>().join(" ");
        let dur = || parse_duration(&arg).map_err(|e| format!("`{cmd}` needs a duration: {e}"));
        match cmd {
            "pause" => self.set_paused(true),
            "resume" | "toggle" | "next" if self.waiting() => {
                self.next(now);
            }
            "resume" => self.set_paused(false),
            "toggle" => self.set_paused(!self.timer.paused()),
            "add" => self.add(dur()?),
            "sub" => self.sub(dur()?),
            "restart" => self.restart(now),
            "next" => {
                if !self.next(now) {
                    return Err("already on the last segment".to_string());
                }
            }
            "back" => self.back(now),
            "effect" => self.effect_now(now),
            "stop" | "quit" => return Ok(Reply::Stop),
            "" => return Err("empty command".to_string()),
            other => {
                return Err(format!(
                    "unknown command `{other}` (try pause, resume, toggle, add 5m, sub 1m, restart, next, back, effect, stop)"
                ));
            }
        }
        self.sample();
        Ok(Reply::Ok)
    }

    /// Whether anything on screen moves fast enough to need the full frame rate.
    pub fn animating(&self, now: f64) -> bool {
        let flashing = |at: Option<f64>| at.is_some_and(|at| now - at < DONE_FLASH_FOR);
        self.effect.is_some()
            || self.cli.decimals > 0
            || (!self.cli.no_tick_fx && self.target.ticking(now))
            || (self.cli.alert > 0.0 && self.urgency() > 0.0)
            || flashing(self.finished_at)
            || flashing(self.switched_at)
    }

    fn hour_width(&self) -> usize {
        if self.cli.pad_hours { 2 } else { 1 }
    }

    fn clock_text(&self, secs: u64) -> String {
        format_clock(secs, (secs >= 3600).then_some(self.hour_width()))
    }

    /// Whole seconds left, rounded up like the big clock.
    fn left_secs(&self) -> u64 {
        self.left.as_secs() + u64::from(self.left.subsec_nanos() > 0)
    }

    fn overtime_secs(&self) -> u64 {
        self.elapsed.saturating_sub(self.timer.total).as_secs()
    }

    /// Length of the final-seconds window, in which no big effects run.
    fn final_window(&self) -> f64 {
        const DEFAULT_WINDOW: f64 = 10.0;
        if self.cli.alert > 0.0 {
            self.cli.alert
        } else {
            DEFAULT_WINDOW
        }
    }

    /// True while counting down through the final seconds window.
    fn in_final_seconds(&self) -> bool {
        self.finished_at.is_none() && self.left.as_secs_f64() <= self.final_window()
    }

    /// Effects that may play next: from the pool for the coming moment, only
    /// ones that finish before the final seconds (so short timers get short
    /// effects, or none), and not the same one twice in a row if avoidable.
    fn effect_choices(&self, cap: f32) -> Vec<effects::Kind> {
        let budget = if self.finished_at.is_some() || self.timer.paused() {
            f32::INFINITY
        } else {
            (self.left.as_secs_f64() - self.final_window()) as f32
        }
        .min(cap);
        if let Some(demo) = &self.demo {
            let kind = demo.kind();
            return if kind.max_duration() <= budget {
                vec![kind]
            } else {
                vec![]
            };
        }
        let fits: Vec<effects::Kind> = self
            .pools
            .get(self.next_moment)
            .iter()
            .copied()
            .filter(|k| k.max_duration() <= budget)
            .collect();
        if fits.len() > 1 {
            fits.into_iter().filter(|k| Some(*k) != self.last_effect).collect()
        } else {
            fits
        }
    }

    /// 0 outside the alert window, rising to 1 at zero.
    fn urgency(&self) -> f32 {
        let window = self.cli.alert;
        if window <= 0.0 || self.finished_at.is_some() {
            return 0.0;
        }
        let left = self.left.as_secs_f64();
        if left > window {
            0.0
        } else {
            (1.0 - left / window) as f32
        }
    }

    // --- remote status ----------------------------------------------------

    /// One line of JSON in Waybar's custom-module format.
    /// The state (`running`, `paused`, `alert`, `waiting`, `done`) and the
    /// clock as the bar shows it (`18:07`, or `+00:12` of overtime).
    fn state_and_clock(&self) -> (&'static str, String) {
        if self.completed() {
            ("done", format!("+{}", self.clock_text(self.overtime_secs())))
        } else if self.waiting() {
            ("waiting", format!("+{}", self.clock_text(self.overtime_secs())))
        } else if self.timer.paused() {
            ("paused", self.clock_text(self.left_secs()))
        } else if self.urgency() > 0.0 || (self.cli.alert <= 0.0 && self.in_final_seconds()) {
            ("alert", self.clock_text(self.left_secs()))
        } else {
            ("running", self.clock_text(self.left_secs()))
        }
    }

    pub fn status_json(&self) -> String {
        let (class, clock) = self.state_and_clock();
        let text = match self.plan[self.seg].label.as_deref() {
            Some(label) => format!("{label} {clock}"),
            None => clock,
        };
        let mut tip: Vec<String> = vec![];
        if let Some(t) = self.title() {
            tip.push(t.to_string());
        }
        if self.plan.len() > 1 {
            tip.push(format!("segment {}/{}", self.seg + 1, self.plan.len()));
        }
        tip.extend(self.upcoming_text());
        tip.extend(self.state_text());
        let pct = if self.timer.total.is_zero() {
            100
        } else {
            ((self.elapsed.as_secs_f64() / self.timer.total.as_secs_f64()).min(1.0) * 100.0).round() as u32
        };
        format!(
            r#"{{"text":{},"tooltip":{},"class":{},"alt":{},"percentage":{pct}}}"#,
            json_str(&text),
            json_str(&tip.join("\n")),
            json_str(class),
            json_str(class),
        )
    }

    /// The reply to `info` (for `t- ctl list`): mode, state, clock, title and
    /// sequence position, tab-separated (tabs and newlines in the title
    /// become spaces).
    pub fn info_line(&self, headless: bool) -> String {
        let (class, clock) = self.state_and_clock();
        let title = self.title().unwrap_or_default().replace(['\t', '\n', '\r'], " ");
        let seg = if self.plan.len() > 1 {
            format!("{}/{}", self.seg + 1, self.plan.len())
        } else {
            String::new()
        };
        let mode = if headless { "headless" } else { "window" };
        format!("{mode}\t{class}\t{clock}\t{title}\t{seg}")
    }

    /// Words describing the state: `paused`, `until 17:30`, `time's up +00:12`, ...
    fn state_text(&self) -> Option<String> {
        if self.completed() {
            let done = self.cli.done_text.as_deref().unwrap_or("time's up");
            return Some(format!("{done}  +{}", self.clock_text(self.overtime_secs())));
        }
        if self.waiting() {
            let next = self.plan[self.seg + 1].label.as_deref().unwrap_or("next");
            return Some(format!(
                "time's up  +{}  ·  n: start {next}",
                self.clock_text(self.overtime_secs())
            ));
        }
        match (&self.cli.until, self.timer.synced(), self.timer.paused()) {
            (Some(u), true, _) => Some(format!("until {u}")),
            // A --until timer that was paused (or restarted) is now a plain
            // countdown; say so, since it no longer ends at the time asked for.
            (Some(u), false, true) => Some(format!("paused  ·  no longer synced to {u}")),
            (Some(u), false, false) => {
                let fmt = if u.matches(':').count() == 2 {
                    "%H:%M:%S"
                } else {
                    "%H:%M"
                };
                let ends = chrono::Local::now() + self.left;
                Some(format!("now ends {} (was {u})", ends.format(fmt)))
            }
            (None, _, true) => Some("paused".to_string()),
            (None, _, false) => None,
        }
    }

    /// Progress through a sequence: `● ● ◐ ○` for short ones, `3/40` for long.
    fn sequence_text(&self) -> Option<String> {
        let n = self.plan.len();
        if n < 2 {
            return None;
        }
        if n > 12 {
            return Some(format!("{}/{n}", self.seg + 1));
        }
        let dots: Vec<&str> = (0..n)
            .map(|i| match i.cmp(&self.seg) {
                std::cmp::Ordering::Less => "●",
                std::cmp::Ordering::Equal if self.finished_at.is_some() => "●",
                std::cmp::Ordering::Equal => "◐",
                std::cmp::Ordering::Greater => "○",
            })
            .collect();
        Some(dots.join(" "))
    }

    /// What comes after this segment: its label (if any), length and theme.
    /// `None` on the last segment, and while waiting (the status line already
    /// says what `n` starts then).
    fn upcoming(&self) -> Option<(Option<&str>, String, usize)> {
        if self.is_last() || self.waiting() {
            return None;
        }
        let next = &self.plan[self.seg + 1];
        let theme = next.theme.unwrap_or(self.base_theme);
        Some((next.label.as_deref(), format_span(next.dur.as_secs()), theme))
    }

    /// `next: break 5m` (or `next: 5m` without a label).
    fn upcoming_text(&self) -> Option<String> {
        let (label, span, _) = self.upcoming()?;
        Some(match label {
            Some(l) => format!("next: {l} {span}"),
            None => format!("next: {span}"),
        })
    }

    // --- drawing ----------------------------------------------------------

    pub fn resize(&mut self, w: u16, h: u16) {
        self.frame = Frame::new(w, h);
        self.target = Target::empty(); // force relayout
    }

    /// Time left in units of 10^-decimals seconds, rounded up so the display
    /// hits zero exactly at the deadline.
    fn remaining_ticks(&self) -> u64 {
        let unit = 1_000_000_000 / 10u128.pow(u32::from(self.cli.decimals));
        self.left.as_nanos().div_ceil(unit) as u64
    }

    /// Rebuild the clock + title canvas if anything it depends on changed.
    /// Returns true on a relayout (the canvas changed size).
    fn layout(&mut self, now: f64) -> bool {
        let cli = self.cli;
        let text = self.clock_face();
        let (sw, sh) = (self.frame.w, self.frame.h);
        let key = (text, self.title().map(str::to_string), sw, sh, self.title_large);
        if self.layout_key.as_ref() == Some(&key) && self.target.w > 0 {
            return false;
        }
        let text = &key.0;
        // Fraction digits change too fast for the tick scramble.
        let fx_until = text.find('.').unwrap_or(usize::MAX);
        let title = key.1.as_deref().map(|t| self.title_glyphs(t));
        let title_h = title.as_ref().map_or(0, |g| g.h as i32 + 1);
        let edges = self.edge_rows().0 + self.edge_rows().1;
        let glyphs = font::render(
            text,
            cli.font,
            (sw - 2).max(1) as usize,
            (sh - self.footer_height() - edges - title_h - 2).max(1) as usize,
            cli.scale.map(usize::from),
        );
        let (glyphs, title_row) = match &title {
            Some(t) => font::stack(&glyphs, 1, t),
            None => (glyphs, usize::MAX),
        };
        self.target.title_row = title_row;
        let relayout = self.target.update(glyphs, now, fx_until);
        self.layout_key = Some(key);
        relayout
    }

    /// The text of the big clock: the time left, or --done-text once finished.
    fn clock_face(&self) -> String {
        if self.completed()
            && let Some(text) = &self.cli.done_text
        {
            return text.clone();
        }
        if self.cli.hide_seconds && self.left_secs() >= 3600 {
            return format_coarse(self.left_secs());
        }
        let ticks = self.remaining_ticks();
        let secs = ticks / 10u64.pow(u32::from(self.cli.decimals));
        let hours = secs >= 3600 || self.timer.total.as_secs() >= 3600 && self.finished_at.is_none();
        format_ticks(ticks, self.cli.decimals, hours.then_some(self.hour_width()))
    }

    pub fn draw(&mut self, now: f64, rng: &mut impl rand::Rng) {
        let cli = self.cli;
        if self.layout(now) {
            // New layout: restart with a fresh effect.
            self.effect = None;
            self.pending = None;
            self.next_mark = None;
            self.next_effect_at = now;
        }

        // Clock and title form one canvas so effects animate both; the bar and
        // status line sit below it.
        let (sw, sh) = (self.frame.w, self.frame.h);
        let (tw, th) = (self.target.w as i32, self.target.h as i32);
        let ox = (sw - tw) / 2;
        let (top, bottom) = self.edge_rows();
        let oy = top + (sh - top - bottom - th - self.footer_height()) / 2;

        let sc = Scene {
            t: &self.target,
            ox,
            oy,
            sw,
            sh,
            theme: &self.themes[self.theme],
            now,
            paused: self.timer.paused(),
            done: self.finished_at.is_some(),
            tick_fx: !cli.no_tick_fx,
            urgency: self.urgency(),
        };

        // Keep the final seconds readable: no big effects (tick scramble stays).
        if self.in_final_seconds() {
            self.effect = None;
            self.pending = None;
        } else if let Some(p) = self.pending.take() {
            // Start once the time left to the round number is the effect's length.
            let lead = self.left.as_secs_f64() - p.end_left;
            if lead <= p.dur {
                self.effect = Some((p.fx, now - (p.dur - lead.max(0.0))));
            } else {
                self.pending = Some(p);
            }
        } else if self.effect.is_none() {
            let aligned = cli
                .effect_align
                .filter(|_| self.finished_at.is_none() && self.demo.is_none());
            let left = self.left.as_secs_f64();
            // In an aligned rhythm, pick shortly before the next mark; otherwise when due.
            let due = match (aligned, self.next_mark) {
                (Some(_), Some(mark)) => left <= mark + ALIGN_LEAD,
                _ => now >= self.next_effect_at,
            };
            if due {
                let cap = match (aligned, self.next_mark) {
                    (Some(_), Some(mark)) => (left - mark) as f32,
                    _ => f32::INFINITY,
                };
                let kind = self.effect_choices(cap).choose(rng).copied();
                self.next_moment = Moment::Replay;
                match (kind, aligned) {
                    (Some(kind), Some(unit)) => {
                        // Measure it, then end it on the mark (or, starting a
                        // rhythm, on the latest round number it can reach).
                        self.last_effect = Some(kind);
                        let seed: u64 = rng.random();
                        let dur = f64::from(effects::measure(kind, &sc, seed));
                        let unit = unit as f64;
                        let end_left = self.next_mark.unwrap_or(((left - dur) / unit).floor() * unit);
                        if end_left >= self.final_window() && left - dur >= end_left - 1e-6 {
                            let fx = effects::make_seeded(kind, &sc, seed);
                            self.pending = Some(Pending { fx, end_left, dur });
                        }
                        self.next_mark = self.mark_after(end_left);
                        if self.next_mark.is_none() {
                            self.next_effect_at = f64::INFINITY;
                        }
                    }
                    (Some(kind), None) => {
                        self.last_effect = Some(kind);
                        self.effect = Some((effects::make(kind, &sc, rng), now));
                    }
                    // Nothing fits before this mark: keep the rhythm, try the next one.
                    (None, Some(_)) if self.next_mark.is_some() => {
                        self.next_mark = self.next_mark.and_then(|m| self.mark_after(m));
                    }
                    // `none` for this moment, or nothing short enough: wait for the next replay.
                    (None, _) => self.next_effect_at = self.next_replay(now),
                }
            }
        }

        self.frame.clear();
        let done = if let Some((fx, start)) = &mut self.effect {
            fx.draw(&sc, (now - *start) as f32, &mut self.frame)
        } else {
            sc.draw_settled(&mut self.frame);
            false
        };
        if done {
            self.effect = None;
            self.next_effect_at = self.next_replay(now);
            if let Some(demo) = self.demo.as_mut().filter(|d| d.auto) {
                demo.step(true);
            }
        }

        self.frame.bg = self.alert_bg(now);
        self.draw_footer(now, oy + th + 1, ox, tw);
        self.draw_demo_bar();
        if self.help {
            self.draw_help();
        }
    }

    /// When the next periodic effect is due.
    fn next_replay(&self, now: f64) -> f64 {
        if self.demo.is_some() {
            return now + Demo::GAP;
        }
        let every = if self.finished_at.is_some() {
            6.0
        } else {
            self.cli.every
        };
        if every > 0.0 { now + every } else { f64::INFINITY }
    }

    /// With --effect-align: the mark (time left, in seconds) one step after
    /// `mark`, where the step is --every rounded up to a multiple of the
    /// alignment. `None` without replays or once it would fall in the final seconds.
    fn mark_after(&self, mark: f64) -> Option<f64> {
        let unit = self.cli.effect_align? as f64;
        if self.cli.every <= 0.0 {
            return None;
        }
        let step = (self.cli.every / unit).ceil().max(1.0) * unit;
        Some(mark - step).filter(|&m| m >= self.final_window())
    }

    /// Background for the alert flash: a pulse on every second tick in the
    /// final window, then rapid flashing for a few seconds after zero or a
    /// segment change.
    fn alert_bg(&self, now: f64) -> Option<Rgb> {
        if self.cli.alert <= 0.0 || self.timer.paused() {
            return None;
        }
        let theme = &self.themes[self.theme];
        let recent = |at: Option<f64>| at.filter(|at| now - at < DONE_FLASH_FOR);
        let (since_pulse, strength) = match recent(self.finished_at).or(recent(self.switched_at)) {
            Some(at) => ((((now - at) * 2.0).fract() / 2.0) as f32, 0.75),
            None if self.finished_at.is_some() => return None,
            None => {
                let urgency = self.urgency();
                if urgency <= 0.0 {
                    return None;
                }
                // Seconds since the displayed second last ticked over.
                ((1.0 - self.left.as_secs_f64().fract()) as f32, 0.25 + 0.4 * urgency)
            }
        };
        let k = (1.0 - since_pulse * 2.5).max(0.0);
        // Fade back to the terminal's real background (if it told us), so the
        // switch to "no fill" at the end of the pulse doesn't show.
        let rest = self.term_bg.unwrap_or(theme.bg);
        (k > 0.0).then(|| rest.lerp(theme.alert, k * k * strength))
    }

    /// Fraction of the progress bar to fill: this segment, or with
    /// --bar-total the whole sequence.
    fn progress(&self) -> f32 {
        let part = |done: Duration, total: Duration| {
            if total.is_zero() {
                1.0
            } else {
                (done.as_secs_f64() / total.as_secs_f64()).min(1.0)
            }
        };
        if !self.cli.bar_total || self.plan.len() < 2 {
            return part(self.elapsed, self.timer.total) as f32;
        }
        let before: Duration = self.plan[..self.seg].iter().map(|s| s.dur).sum();
        let after: Duration = self.plan[self.seg + 1..].iter().map(|s| s.dur).sum();
        let total = before + self.timer.total + after;
        part(before + self.elapsed.min(self.timer.total), total) as f32
    }

    /// Progress bar and status line, starting at row `y` (after a blank row).
    /// Both are centered on the canvas (`ox`, `tw`), not the screen, so they
    /// round the same way as the clock and title above them.
    fn draw_footer(&mut self, now: f64, y: i32, ox: i32, tw: i32) {
        let theme = &self.themes[self.theme];
        let sw = self.frame.w;
        // One blank row between blocks: canvas, [bar], status.
        let status_row = if self.cli.bar { y + 2 } else { y };
        if self.cli.bar {
            let bw = tw.max(20).min(sw - 4);
            let filled = (self.progress() * bw as f32).round() as i32;
            let bx = ox + (tw - bw) / 2;
            let drift = (now * 0.025).fract() as f32;
            for i in 0..bw {
                let (ch, col) = if i < filled {
                    ('━', theme.gradient_at(i as f32 / bw as f32 * 0.55 + drift))
                } else {
                    ('─', theme.dim.scale(0.6))
                };
                self.frame.put(bx + i, y, ch, col);
            }
        }

        let time = match self.status_time {
            StatusTime::Off => None,
            StatusTime::Now => Some(chrono::Local::now().format("%H:%M").to_string()),
            // After zero the overtime already says it all.
            StatusTime::Left if self.finished_at.is_some() => None,
            StatusTime::Left => Some(format!("{} left", self.clock_text(self.left_secs()))),
        };
        let col = if self.finished_at.is_some() {
            theme.flash.lerp(theme.dim, 0.5 + 0.5 * wave(now, 5.0))
        } else {
            theme.dim
        };
        // Pieces of the status line, each with its color: the dots, then what
        // comes next with its name in the next segment's theme color so it
        // stands out (and shows a theme change coming), then the state.
        let mut parts: Vec<Vec<(String, Rgb)>> = [time, self.sequence_text()]
            .into_iter()
            .flatten()
            .map(|t| vec![(t, col)])
            .collect();
        if let Some((label, span, next_theme)) = self.upcoming() {
            let accent = self.themes[next_theme].gradient_at(0.0);
            parts.push(match label {
                Some(l) => vec![
                    ("next: ".into(), col),
                    (l.to_string(), accent),
                    (format!(" {span}"), col),
                ],
                None => vec![("next: ".into(), col), (span, accent)],
            });
        }
        parts.extend(self.state_text().map(|t| vec![(t, col)]));
        if parts.is_empty() {
            return;
        }
        let spans: Vec<(String, Rgb)> = parts
            .into_iter()
            .enumerate()
            .flat_map(|(i, part)| {
                let sep = (i > 0).then(|| ("  ·  ".to_string(), col));
                sep.into_iter().chain(part)
            })
            .collect();
        let w: i32 = spans.iter().map(|(t, _)| str_width(t)).sum();
        // Span to align within, and the row: the clock's edges below it, or
        // the screen's (with a small margin) at the top/bottom.
        let (sh, margin) = (self.frame.h, 2);
        let (left, right, row) = match self.cli.status_pos {
            StatusPos::Below => (ox, ox + tw, status_row),
            StatusPos::Top => (margin, sw - margin, 1),
            StatusPos::Bottom => (margin, sw - margin, sh - 2 - self.edge_rows().1 + 2),
        };
        let mut x = match self.cli.status_align {
            StatusAlign::Left => left,
            StatusAlign::Center => left + (right - left - w) / 2,
            StatusAlign::Right => right - w,
        };
        for (text, c) in spans {
            self.frame.text(x, row, &text, c);
            x += str_width(&text);
        }
    }

    /// The keybinding popup: a bordered box centered on the screen, drawn over
    /// whatever is underneath (its blank interior hides the clock behind it).
    fn draw_help(&mut self) {
        let theme = &self.themes[self.theme];
        let keys = self.keys();
        let key_w = keys.iter().map(|(k, _)| str_width(k)).max().unwrap_or(0);
        let desc_w = keys.iter().map(|(_, d)| str_width(d)).max().unwrap_or(0);
        // Border + 2 padding on each side, 3 between columns; title row + gap + keys.
        let (w, h) = (key_w + desc_w + 3 + 6, keys.len() as i32 + 4);
        if w > self.frame.w || h > self.frame.h {
            self.draw_help_compact(key_w);
            return;
        }
        let (x0, y0) = ((self.frame.w - w) / 2, (self.frame.h - h) / 2);
        let border = theme.dim;
        for y in y0..y0 + h {
            for x in x0..x0 + w {
                let ch = match (x - x0, y - y0) {
                    (0, 0) => '╭',
                    (dx, 0) if dx == w - 1 => '╮',
                    (0, dy) if dy == h - 1 => '╰',
                    (dx, dy) if dx == w - 1 && dy == h - 1 => '╯',
                    (_, 0) => '─',
                    (_, dy) if dy == h - 1 => '─',
                    (0, _) => '│',
                    (dx, _) if dx == w - 1 => '│',
                    _ => ' ',
                };
                self.frame.put(x, y, ch, border);
            }
        }
        let title = " keys ";
        self.frame.text(x0 + (w - str_width(title)) / 2, y0, title, theme.flash);
        for (i, (key, desc)) in keys.iter().enumerate() {
            let y = y0 + 2 + i as i32;
            let col = theme.gradient_at(i as f32 / keys.len() as f32 * 0.5);
            self.frame.text(x0 + 3, y, key, col);
            self.frame
                .text(x0 + 3 + key_w + 3, y, desc, theme.flash.lerp(theme.dim, 0.35));
        }
    }

    /// Popup fallback for screens too small for the box: no border, keys kept
    /// whole, descriptions cut to fit.
    fn draw_help_compact(&mut self, key_w: i32) {
        let theme = &self.themes[self.theme];
        let (sw, sh) = (self.frame.w, self.frame.h);
        let keys = self.keys();
        let n = keys.len() as i32;
        let y0 = ((sh - n) / 2).max(0);
        for y in y0..(y0 + n).min(sh) {
            for x in 0..sw {
                self.frame.put(x, y, ' ', theme.dim);
            }
        }
        let desc_x = 1 + key_w + 2;
        let room = (sw - desc_x).max(0) as usize;
        for (i, (key, desc)) in keys.iter().enumerate() {
            let y = y0 + i as i32;
            self.frame.text(1, y, key, theme.gradient_at(i as f32 / n as f32 * 0.5));
            let desc: String = if desc.chars().count() > room {
                desc.chars().take(room.saturating_sub(1)).chain(['…']).collect()
            } else {
                (*desc).to_string()
            };
            self.frame.text(desc_x, y, &desc, theme.flash.lerp(theme.dim, 0.35));
        }
    }

    /// The keys the help popup lists.
    fn keys(&self) -> &'static [(&'static str, &'static str)] {
        if self.demo.is_some() { DEMO_KEYS } else { KEYS }
    }

    /// `--demo`'s control line at the bottom: theme and effect with their
    /// keys, shortened to what fits.
    fn draw_demo_bar(&mut self) {
        let Some(demo) = &self.demo else { return };
        let theme = &self.themes[self.theme];
        let (key, label, value) = (theme.flash, theme.dim, theme.gradient_at(0.0));
        let effect = demo.name();
        let count = |i: usize, n: usize| format!(" {}/{n}", i + 1);
        let mut parts: Vec<(String, Rgb)> = vec![
            ("← → ".into(), key),
            ("theme ".into(), label),
            (theme.name.clone(), value),
            (count(self.theme, self.themes.len()), label),
            ("    ↑ ↓ ".into(), key),
            ("effect ".into(), label),
            (effect, theme.gradient_at(0.3)),
            (count(demo.pick, demo.kinds.len()), label),
        ];
        let hints: [(String, Rgb); 8] = [
            ("    enter ".into(), key),
            ("replay  ".into(), label),
            ("a ".into(), key),
            (format!("auto {}  ", if demo.auto { "on" } else { "off" }), label),
            ("f ".into(), key),
            ("final seconds  ".into(), label),
            ("r ".into(), key),
            ("random effect  ·  h keys".into(), label),
        ];
        let width = |p: &[(String, Rgb)]| p.iter().map(|(t, _)| str_width(t)).sum::<i32>();
        let sw = self.frame.w;
        if width(&parts) + width(&hints) <= sw - 2 {
            parts.extend(hints);
        } else if width(&parts) > sw - 2 {
            // Just the names.
            parts = vec![parts[2].clone(), ("  ·  ".into(), label), parts[6].clone()];
        }
        let mut x = (sw - width(&parts)) / 2;
        let y = self.frame.h - 2;
        for (text, col) in parts {
            self.frame.text(x, y, &text, col);
            x += str_width(&text);
        }
    }

    /// Rows kept free at the (top, bottom) screen edge for an edge status
    /// line and `--demo`'s control line.
    fn edge_rows(&self) -> (i32, i32) {
        let demo = if self.demo.is_some() { 2 } else { 0 };
        match self.cli.status_pos {
            StatusPos::Below => (0, demo),
            StatusPos::Top => (2, demo),
            StatusPos::Bottom => (0, 2 + demo),
        }
    }

    /// Rows below the canvas: bar, and the status line when it sits below.
    /// Its row is always reserved so the clock doesn't jump when it appears.
    fn footer_height(&self) -> i32 {
        let status = if self.cli.status_pos == StatusPos::Below { 2 } else { 0 };
        i32::from(self.cli.bar) * 2 + status
    }

    /// The title in the 3-row block font when large (and it has only
    /// characters the font knows and fits), else as a line of text.
    fn title_glyphs(&self, title: &str) -> font::Glyphs {
        if self.title_large && font::block_supports(title) && font::block_width(title, 1) as i32 <= self.frame.w - 4 {
            font::block(title, 1)
        } else {
            font::plain(title)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    fn app_with<'a>(cli: &'a Cli, themes: &'a [Theme], mins: &[u64]) -> App<'a> {
        let plan: Vec<Step> = mins
            .iter()
            .enumerate()
            .map(|(i, m)| Step {
                dur: Duration::from_secs(m * 60),
                label: Some(format!("s{i}")),
                theme: None,
            })
            .collect();
        let timer = Timer::new(plan[0].dur);
        let one = || vec![effects::Kind::Decrypt];
        App::new(
            cli,
            Pools {
                intro: one(),
                replay: one(),
                zero: one(),
            },
            themes,
            0,
            plan,
            timer,
        )
    }

    #[test]
    fn sequence_advances_and_completes() {
        let cli = Cli::try_parse_from(["t-", "1m"]).unwrap();
        let themes = crate::theme::builtin();
        let mut app = app_with(&cli, &themes, &[1, 2]);
        assert_eq!(app.tick(0.0), None);
        assert_eq!(app.upcoming_text().as_deref(), Some("next: s1 2m"));
        assert!(app.status_json().contains(r"segment 1/2\nnext: s1 2m"));
        assert_eq!(app.info_line(true), "headless\trunning\t01:00\ts0\t1/2");
        app.remote("sub 5m", 0.0).ok();
        assert_eq!(
            app.tick(1.0),
            Some(Event::SegmentDone {
                done: Some("s0".into()),
                next: Some("s1".into())
            })
        );
        assert_eq!((app.seg, app.title()), (1, Some("s1")));
        // Nothing comes after the last segment.
        assert_eq!(app.upcoming_text(), None);
        assert!(!app.completed());
        app.remote("sub 5m", 2.0).ok();
        assert_eq!(
            app.tick(3.0),
            Some(Event::Completed {
                done: Some("s1".into())
            })
        );
        assert!(app.completed());
        assert!(app.status_json().contains(r#""class":"done""#));
    }

    #[test]
    fn confirm_waits_for_next() {
        let cli = Cli::try_parse_from(["t-", "1m", "--confirm"]).unwrap();
        let themes = crate::theme::builtin();
        let mut app = app_with(&cli, &themes, &[1, 1]);
        app.remote("sub 5m", 0.0).ok();
        assert!(matches!(app.tick(1.0), Some(Event::SegmentDone { .. })));
        assert!(app.waiting() && app.seg == 0);
        assert_eq!(app.upcoming_text(), None, "the waiting line already names it");
        assert!(app.status_json().contains(r#""class":"waiting""#));
        app.remote("toggle", 2.0).ok();
        assert!(!app.waiting() && app.seg == 1);
    }

    #[test]
    fn quit_keys_close_the_help_popup_first() {
        let cli = Cli::try_parse_from(["t-", "1m"]).unwrap();
        let themes = crate::theme::builtin();
        let mut app = app_with(&cli, &themes, &[1]);
        let none = KeyModifiers::NONE;
        for quit in [KeyCode::Char('q'), KeyCode::Esc] {
            assert!(!app.on_key(KeyCode::Char('h'), none, 0.0));
            assert!(!app.on_key(quit, none, 0.0), "{quit:?} should only close the popup");
            assert!(!app.help);
        }
        assert!(
            app.on_key(KeyCode::Char('q'), none, 0.0),
            "q quits once the popup is closed"
        );
    }

    #[test]
    fn short_timers_get_short_effects_or_none() {
        use effects::Kind;
        let cli = Cli::try_parse_from(["t-", "1m"]).unwrap();
        let themes = crate::theme::builtin();
        let all = || vec![Kind::Highlight, Kind::Decrypt, Kind::Blackhole, Kind::Matrix];
        let with_secs = |secs: u64| {
            let plan = vec![Step {
                dur: Duration::from_secs(secs),
                label: None,
                theme: None,
            }];
            let timer = Timer::new(plan[0].dur);
            App::new(
                &cli,
                Pools {
                    intro: all(),
                    replay: all(),
                    zero: all(),
                },
                &themes,
                0,
                plan,
                timer,
            )
        };
        // The final 10 seconds are effect-free, so a 10s timer gets none at all...
        assert_eq!(with_secs(10).effect_choices(f32::INFINITY), []);
        // ...12s leaves ~2s: only the short highlight fits...
        assert_eq!(with_secs(12).effect_choices(f32::INFINITY), [Kind::Highlight]);
        // ...14s leaves ~4s: everything but the long ones.
        assert_eq!(
            with_secs(14).effect_choices(f32::INFINITY),
            [Kind::Highlight, Kind::Decrypt]
        );
        assert_eq!(with_secs(60).effect_choices(f32::INFINITY).len(), 4);
    }

    #[test]
    fn aligned_effects_step_by_every_rounded_up() {
        let themes = crate::theme::builtin();
        let mark_after = |args: &[&str], mark: f64| {
            let cli = Cli::try_parse_from([&["t-", "1h"], args].concat()).unwrap();
            app_with(&cli, &themes, &[60]).mark_after(mark)
        };
        assert_eq!(
            mark_after(&["--effect-align", "10", "--every", "15"], 1500.0),
            Some(1480.0)
        );
        assert_eq!(
            mark_after(&["--effect-align", "30", "--every", "10"], 1500.0),
            Some(1470.0)
        );
        assert_eq!(
            mark_after(&["--effect-align", "30", "--every", "30"], 1500.0),
            Some(1470.0)
        );
        assert_eq!(mark_after(&["--effect-align", "5", "--every", "12"], 40.0), Some(25.0));
        // No replays, or a mark inside the final seconds: no next mark.
        assert_eq!(mark_after(&["--effect-align", "10", "--every", "0"], 1500.0), None);
        assert_eq!(mark_after(&["--effect-align", "10", "--every", "15"], 25.0), None);
        assert_eq!(mark_after(&["--every", "15"], 1500.0), None);
    }

    #[test]
    fn demo_keys_pick_theme_and_effect() {
        use effects::Kind;
        let cli = Cli::try_parse_from(["t-", "--demo", "--effect", "rain"]).unwrap();
        let themes = crate::theme::builtin();
        let mut app = app_with(&cli, &themes, &[25]);
        let key = |app: &mut App, code| assert!(!app.on_key(code, KeyModifiers::NONE, 1.0));
        let kinds = app.demo.as_ref().unwrap().kinds.len();
        assert_eq!(app.demo.as_ref().unwrap().kind(), Kind::Rain, "starts on --effect");
        assert!(!app.demo.as_ref().unwrap().kinds.contains(&Kind::Off));

        // Arrows wrap around both lists.
        key(&mut app, KeyCode::Left);
        assert_eq!(app.theme, themes.len() - 1);
        key(&mut app, KeyCode::Right);
        key(&mut app, KeyCode::Right);
        assert_eq!(app.theme, 1);
        key(&mut app, KeyCode::Down);
        assert_eq!(app.demo.as_ref().unwrap().kind(), Kind::Beams);
        for _ in 0..kinds {
            key(&mut app, KeyCode::Up);
        }
        assert_eq!(app.demo.as_ref().unwrap().kind(), Kind::Beams);
        assert_eq!(app.effect_choices(f32::INFINITY), [Kind::Beams]);
        assert_eq!(app.demo_choice(), Some((themes[1].name.clone(), "beams".to_string())));

        // r picks another effect and keeps the theme.
        key(&mut app, KeyCode::Char('r'));
        assert_eq!(app.theme, 1);
        assert_ne!(app.demo.as_ref().unwrap().pick, 2);
        app.demo.as_mut().unwrap().pick = 2;

        // f jumps to just before the final seconds; after zero it goes back.
        key(&mut app, KeyCode::Char('f'));
        assert_eq!(app.timer.total, Duration::from_secs(12));
        app.timer = Timer::new(Duration::ZERO);
        assert!(matches!(app.tick(2.0), Some(Event::Completed { .. })));
        assert_eq!(app.tick(2.0 + DONE_FLASH_FOR + 1.5), None);
        assert_eq!((app.timer.total, app.finished_at), (Duration::from_secs(25 * 60), None));

        // Other keys work as usual; q still quits.
        key(&mut app, KeyCode::Char('h'));
        assert!(app.help);
        key(&mut app, KeyCode::Char('q'));
        assert!(app.on_key(KeyCode::Char('q'), KeyModifiers::NONE, 1.0));
    }

    #[test]
    fn remote_commands() {
        let cli = Cli::try_parse_from(["t-", "1m"]).unwrap();
        let themes = crate::theme::builtin();
        let mut app = app_with(&cli, &themes, &[1]);
        app.remote("pause", 0.0).ok();
        assert!(app.timer.paused());
        assert!(app.status_json().contains(r#""class":"paused""#));
        app.remote("add 2m", 0.0).ok();
        assert_eq!(app.timer.total, Duration::from_secs(180));
        assert!(matches!(app.remote("stop", 0.0), Ok(Reply::Stop)));
        assert!(app.remote("next", 0.0).unwrap_err().contains("last segment"));
        assert!(app.remote("add soon", 0.0).unwrap_err().contains("needs a duration"));
        assert!(app.remote("dance", 0.0).unwrap_err().contains("unknown command"));
    }
}
