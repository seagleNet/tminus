//! Command-line interface: options, subcommands, and duration/clock text.

use std::{path::PathBuf, time::Duration};

use clap::{Parser, Subcommand};

use crate::{effects, font::FontKind};

/// Keybindings shown in the `h` popup. `--help` lists the same keys; a test
/// keeps the two in sync.
pub const KEYS: &[(&str, &str)] = &[
    ("space / p", "pause / resume"),
    ("e / enter", "play another effect now"),
    ("n / b", "next / previous segment of a sequence"),
    ("t", "next theme"),
    ("s", "switch title between large and small"),
    ("c", "cycle status line time: off / time of day / time left"),
    ("+ / -", "add / remove one minute"),
    ("r", "restart (the current segment)"),
    ("h / ?", "show / hide this list of keys (q and esc close it too)"),
    ("q / esc", "quit"),
];

/// Keys in `--demo` mode (keys not listed work as usual).
pub const DEMO_KEYS: &[(&str, &str)] = &[
    ("← / →", "previous / next theme"),
    ("↑ / ↓", "previous / next effect (plays at once)"),
    ("enter / e / space", "play the effect again"),
    ("a", "auto: move on to the next effect after each one"),
    ("f", "preview the final seconds and the flash at zero"),
    ("r", "random effect"),
    ("s", "switch title between large and small"),
    ("h / ?", "show / hide this list of keys (q and esc close it too)"),
    ("q / esc", "quit, printing the chosen --theme and --effect"),
];

#[derive(Clone, Copy, PartialEq, Eq, Debug, clap::ValueEnum)]
pub enum TitleSize {
    Large,
    Small,
}

/// Where the status line (`until ...`, `paused`, `time's up`) goes.
#[derive(Clone, Copy, PartialEq, Eq, Debug, clap::ValueEnum)]
pub enum StatusPos {
    /// Under the title (and progress bar)
    Below,
    /// Top edge of the screen
    Top,
    /// Bottom edge of the screen
    Bottom,
}

/// Plain-text time shown in the status line.
#[derive(Clone, Copy, PartialEq, Eq, Debug, clap::ValueEnum)]
pub enum StatusTime {
    /// None
    Off,
    /// Current time of day
    Now,
    /// Time remaining
    Left,
}

impl StatusTime {
    pub fn next(self) -> Self {
        match self {
            StatusTime::Off => StatusTime::Now,
            StatusTime::Now => StatusTime::Left,
            StatusTime::Left => StatusTime::Off,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, clap::ValueEnum)]
pub enum StatusAlign {
    Left,
    Center,
    Right,
}

/// A countdown timer for your terminal, styled after Omarchy's screensaver.
#[derive(Parser, Debug)]
#[allow(clippy::struct_excessive_bools, reason = "independent on/off CLI flags")]
#[command(
    version,
    // Config file values are passed as arguments ahead of the real ones; this
    // lets a later (command-line) occurrence replace the earlier one.
    args_override_self = true,
    args_conflicts_with_subcommands = true,
    subcommand_negates_reqs = true,
    after_help = "\
DURATION examples:  90   45s   5m   1h30m   \"1h 5m 10s\"   10:00   1:30:00

SEQUENCES: give several durations, each optionally with a label and a theme,
and `xN` to repeat what came before:
  t- 25m:work 5m:break x4          t- 25m:work@nord 5m:break@everforest
  t- @pomodoro                     (a preset from the config file)

KEYS:
  space / p    pause / resume
  e / enter    play another effect now
  n / b        next / previous segment of a sequence
  t            next theme
  s            switch title between large and small
  c            cycle status line time: off / time of day / time left
  + / -        add / remove one minute
  r            restart (the current segment)
  h / ?        show / hide this list of keys (q and esc close it too)
  q / esc      quit (exit code 1 if the timer hadn't finished)

REMOTE CONTROL: a running timer can be watched and steered from elsewhere,
e.g. a Waybar module or a keybinding:
  t- status --follow               JSON lines for a Waybar custom module
  t- ctl toggle                    pause/resume (also: add 5m, next, stop, ...)

CONFIG: defaults for any option can go in ~/.config/tminus/config.toml, keyed by
the long option name (theme = \"nord\", bar = true, effect = [\"matrix\"]).
Command-line options override it; every on/off option has an opposite for that
(--no-bar, --no-notify, --no-confirm, --bell, --tick-fx, ...).

When the timer runs out tminus rings the bell, keeps running and counts the
overtime. Pass --exit to quit as soon as it finishes, so you can chain it:
  t- 25m --exit && notify-send 'break time'"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,

    /// How long to count down for. Several make a sequence (see below)
    #[arg(
        value_name = "DURATION",
        required_unless_present_any = ["until", "list_effects", "list_themes", "demo"],
        conflicts_with = "until"
    )]
    pub durations: Vec<String>,

    /// Count down to a time or date instead: 17:30, tomorrow 9:00, fri 17:00,
    /// 2026-12-24 or 2026-12-24 18:00 (24h, local time)
    #[arg(short, long, value_name = "TIME")]
    pub until: Option<String>,

    /// Text shown under the clock (a segment's own label takes precedence)
    #[arg(short, long)]
    pub title: Option<String>,

    /// Title size: large block letters or a small line of text (toggle with s)
    #[arg(long, value_enum, default_value_t = TitleSize::Large, value_name = "SIZE")]
    pub title_size: TitleSize,

    /// Effects to use, comma-separated [default: all but matrix-heavy, picked at random]
    // `Set` (not the default `Append`) so a command-line --effect replaces the config's list.
    #[arg(short, long, value_enum, value_delimiter = ',', value_name = "NAMES", action = clap::ArgAction::Set)]
    pub effect: Vec<effects::Kind>,

    /// Effects for when the timer starts [default: as --effect]
    #[arg(long, value_enum, value_delimiter = ',', value_name = "NAMES", action = clap::ArgAction::Set)]
    pub intro_effect: Vec<effects::Kind>,

    /// Effects at zero and when a sequence moves on [default: as --effect]
    #[arg(long, value_enum, value_delimiter = ',', value_name = "NAMES", action = clap::ArgAction::Set)]
    pub zero_effect: Vec<effects::Kind>,

    /// Time effects to finish exactly as the clock reaches a multiple of SECS,
    /// e.g. 10 → on :50, :40, ...; 30 → on :30 and :00. --every is then rounded
    /// up to a multiple of SECS
    #[arg(long, value_name = "SECS", value_parser = clap::value_parser!(u64).range(1..))]
    pub effect_align: Option<u64>,

    /// Seconds between effect replays, like the screensaver cycling (0 = intro only;
    /// with --effect-align, rounded up to a multiple of it)
    #[arg(long, default_value_t = 30.0, value_name = "SECS")]
    pub every: f64,

    /// Color theme, built in or defined in the config (see --list-themes)
    /// [default: the active Omarchy theme, else tokyo-night]
    #[arg(long)]
    pub theme: Option<String>,

    /// Font for the clock face
    #[arg(short, long, value_enum, default_value_t = FontKind::Auto)]
    pub font: FontKind,

    /// Don't scramble digits as they tick
    #[arg(long, overrides_with = "tick_fx")]
    pub no_tick_fx: bool,
    #[arg(long, hide = true, overrides_with = "no_tick_fx")]
    pub tick_fx: bool,

    /// Don't ring the terminal bell at zero
    #[arg(long, overrides_with = "bell")]
    pub no_bell: bool,
    #[arg(long, hide = true, overrides_with = "no_bell")]
    pub bell: bool,

    /// Send a desktop notification (notify-send) at zero
    #[arg(long, overrides_with = "no_notify")]
    pub notify: bool,
    #[arg(long, hide = true, overrides_with = "notify")]
    pub no_notify: bool,

    // Doc comments are --help text here, where backticks would show literally.
    #[allow(clippy::doc_markdown)]
    /// Run a shell command at zero and at each segment change. It gets
    /// TMINUS_EVENT (segment or done), TMINUS_LABEL, TMINUS_NEXT and TMINUS_TITLE
    #[arg(long, value_name = "CMD")]
    pub exec: Option<String>,

    /// Play a sound file at zero and at each segment change (with pw-play, paplay, aplay or afplay)
    #[arg(long, value_name = "FILE")]
    pub sound: Option<PathBuf>,

    /// Show TEXT in big letters instead of 00:00 when the countdown finishes
    #[arg(long, value_name = "TEXT")]
    pub done_text: Option<String>,

    /// Quit as soon as the countdown (or the whole sequence) finishes
    #[arg(long, overrides_with = "no_exit")]
    pub exit: bool,
    #[arg(long, hide = true, overrides_with = "exit")]
    pub no_exit: bool,

    /// In a sequence, wait for a key (n, space or enter) before starting the next segment
    #[arg(long, overrides_with = "no_confirm")]
    pub confirm: bool,
    #[arg(long, hide = true, overrides_with = "confirm")]
    pub no_confirm: bool,

    /// Show fractions of a second: 1 = tenths, 2 = hundredths
    #[arg(short, long, default_value_t = 0, value_name = "N", value_parser = clap::value_parser!(u8).range(0..=2))]
    pub decimals: u8,

    /// Pad hours to two digits (01:30:00 instead of 1:30:00)
    #[arg(long, overrides_with = "no_pad_hours")]
    pub pad_hours: bool,
    #[arg(long, hide = true, overrides_with = "pad_hours")]
    pub no_pad_hours: bool,

    /// Leave the seconds off while an hour or more remains (H:MM)
    #[arg(long, overrides_with = "no_hide_seconds")]
    pub hide_seconds: bool,
    #[arg(long, hide = true, overrides_with = "hide_seconds")]
    pub no_hide_seconds: bool,

    /// Clock size for the block font, in pixels per dot (implies --font block)
    /// [default: as large as fits]
    #[arg(long, value_name = "N", value_parser = clap::value_parser!(u8).range(1..=40))]
    pub scale: Option<u8>,

    /// Flash the background during the last SECS seconds (0 = off). Effect replays
    /// also stop in this window (the last 10s when 0) so the seconds stay readable
    #[arg(long, default_value_t = 10.0, value_name = "SECS")]
    pub alert: f64,

    /// Where to show the status line (until/paused/time's up)
    #[arg(long, value_enum, default_value_t = StatusPos::Below, value_name = "POS")]
    pub status_pos: StatusPos,

    /// How to align the status line: to the clock's edges when below it, else to the screen's
    #[arg(long, value_enum, default_value_t = StatusAlign::Center, value_name = "ALIGN")]
    pub status_align: StatusAlign,

    /// Plain-text time in the status line: time of day or time left (cycle with c)
    #[arg(long, value_enum, default_value_t = StatusTime::Off, value_name = "WHAT")]
    pub status_time: StatusTime,

    /// Show a progress bar under the clock
    #[arg(short, long, overrides_with = "no_bar")]
    pub bar: bool,
    #[arg(long, hide = true, overrides_with = "bar")]
    pub no_bar: bool,

    /// Make the progress bar cover the whole sequence instead of the current segment
    #[arg(long, overrides_with = "no_bar_total")]
    pub bar_total: bool,
    #[arg(long, hide = true, overrides_with = "bar_total")]
    pub no_bar_total: bool,

    /// Name for remote control (`t- ctl --name NAME ...`) [default: tminus]
    #[arg(long)]
    pub name: Option<String>,

    /// Run without a screen, e.g. only shown in Waybar via `t- status --follow`
    #[arg(long, overrides_with = "no_headless")]
    pub headless: bool,
    #[arg(long, hide = true, overrides_with = "headless")]
    pub no_headless: bool,

    /// Frames per second while animating
    #[arg(long, default_value_t = 60, value_parser = clap::value_parser!(u32).range(5..=240))]
    pub fps: u32,

    /// Config file to read [default: ~/.config/tminus/config.toml]
    #[arg(long, value_name = "PATH")]
    pub config: Option<PathBuf>,

    /// Ignore the config file
    #[arg(long, conflicts_with = "config")]
    pub no_config: bool,

    /// List the available effects and exit
    #[arg(long)]
    pub list_effects: bool,

    /// List the available themes and exit
    #[arg(long)]
    pub list_themes: bool,

    /// Browse themes and effects with a live preview (arrow keys; q prints
    /// the choice). The clock shows DURATION, else 25:00
    #[arg(long, conflicts_with = "until")]
    pub demo: bool,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Print a running timer's state as JSON, in Waybar's custom-module format
    Status {
        /// Keep printing a line whenever it changes (and wait for a timer if none runs)
        #[arg(long)]
        follow: bool,
        /// Which timer, if several run [default: the most recently started]
        #[arg(long)]
        name: Option<String>,
    },
    /// Control a running timer
    Ctl {
        /// Which timer, if several run [default: the most recently started]
        #[arg(long, global = true)]
        name: Option<String>,
        #[command(subcommand)]
        command: CtlCommand,
    },
    /// Print shell completions, e.g. `t- completions fish > ~/.config/fish/completions/t-.fish`
    Completions {
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },
    /// Print the man page, e.g. `t- man > ~/.local/share/man/man1/t-.1`
    Man,
}

/// What `t- ctl` asks a running timer to do.
#[derive(Subcommand, Debug, PartialEq, Eq)]
pub enum CtlCommand {
    /// Pause the countdown
    Pause,
    /// Resume the countdown (or start the next segment when waiting)
    Resume,
    /// Pause or resume
    Toggle,
    /// Add time, e.g. `t- ctl add 5m`
    Add {
        /// How much, e.g. 5m, 90 or 1h30m
        #[arg(required = true, num_args = 1.., value_name = "DURATION")]
        duration: Vec<String>,
    },
    /// Remove time, e.g. `t- ctl sub 1m`
    Sub {
        /// How much, e.g. 5m, 90 or 1h30m
        #[arg(required = true, num_args = 1.., value_name = "DURATION")]
        duration: Vec<String>,
    },
    /// Restart the current segment
    Restart,
    /// Go to the next segment of a sequence
    Next,
    /// Go back to the previous segment of a sequence
    Back,
    /// Play an effect now
    Effect,
    /// Quit the timer
    #[command(alias = "quit")]
    Stop,
}

impl CtlCommand {
    /// The line sent over the control socket (see `App::remote`). Durations
    /// are checked here, so a typo is reported without touching the timer.
    pub fn line(&self) -> Result<String, String> {
        let with_duration = |cmd: &str, words: &[String]| {
            let d = words.join(" ");
            parse_duration(&d).map(|_| format!("{cmd} {d}"))
        };
        Ok(match self {
            CtlCommand::Pause => "pause".into(),
            CtlCommand::Resume => "resume".into(),
            CtlCommand::Toggle => "toggle".into(),
            CtlCommand::Add { duration } => with_duration("add", duration)?,
            CtlCommand::Sub { duration } => with_duration("sub", duration)?,
            CtlCommand::Restart => "restart".into(),
            CtlCommand::Next => "next".into(),
            CtlCommand::Back => "back".into(),
            CtlCommand::Effect => "effect".into(),
            CtlCommand::Stop => "stop".into(),
        })
    }
}

pub fn parse_duration(s: &str) -> Result<Duration, String> {
    let s = s.trim();
    let bad = || format!("can't understand duration {s:?} (try 90, 5m, 1h30m or 10:00)");
    let secs_f64 = |n: f64| Duration::try_from_secs_f64(n).map_err(|_| bad());
    if s.contains(':') {
        let parts: Vec<u64> = s
            .split(':')
            .map(|p| p.parse().map_err(|_| bad()))
            .collect::<Result<_, _>>()?;
        let secs = match parts.as_slice() {
            [m, s] => m.checked_mul(60).and_then(|m| m.checked_add(*s)),
            [h, m, s] => h
                .checked_mul(3600)
                .and_then(|h| h.checked_add(m.checked_mul(60)?))
                .and_then(|hm| hm.checked_add(*s)),
            _ => None,
        };
        return secs.map(Duration::from_secs).ok_or_else(bad);
    }
    if let Ok(n) = s.parse::<f64>() {
        return secs_f64(n);
    }
    let mut total = 0.0;
    let mut chars = s.chars().peekable();
    let mut any = false;
    while chars.peek().is_some() {
        while chars.next_if(|c| c.is_whitespace()).is_some() {}
        let num: String = std::iter::from_fn(|| chars.next_if(|c| c.is_ascii_digit() || *c == '.')).collect();
        while chars.next_if(|c| c.is_whitespace()).is_some() {}
        let unit: String = std::iter::from_fn(|| chars.next_if(|c| c.is_alphabetic())).collect();
        if num.is_empty() && unit.is_empty() {
            break;
        }
        let n: f64 = num.parse().map_err(|_| bad())?;
        let mult = match unit.to_ascii_lowercase().as_str() {
            "d" | "day" | "days" => 86400.0,
            "h" | "hr" | "hrs" | "hour" | "hours" => 3600.0,
            "m" | "min" | "mins" | "minute" | "minutes" => 60.0,
            "s" | "sec" | "secs" | "second" | "seconds" | "" => 1.0,
            _ => return Err(bad()),
        };
        total += n * mult;
        any = true;
    }
    if !any {
        return Err(bad());
    }
    secs_f64(total)
}

/// Format `ticks` (units of 10^-decimals seconds) as a clock, e.g. `05:30.25`.
pub fn format_ticks(ticks: u64, decimals: u8, hours: Option<usize>) -> String {
    let scale = 10u64.pow(u32::from(decimals));
    let clock = format_clock(ticks / scale, hours);
    if decimals == 0 {
        clock
    } else {
        format!("{clock}.{:0w$}", ticks % scale, w = usize::from(decimals))
    }
}

/// Format whole seconds. `hours` is the minimum width of the hours field,
/// or `None` to leave hours out (MM:SS). A day or more gets a days field:
/// `2d 03:04:05`.
pub fn format_clock(secs: u64, hours: Option<usize>) -> String {
    let (d, h, m, s) = (secs / 86_400, secs / 3600 % 24, secs / 60 % 60, secs % 60);
    if d > 0 {
        return format!("{d}d {h:02}:{m:02}:{s:02}");
    }
    let h = secs / 3600;
    match hours {
        Some(w) => format!("{h:0w$}:{m:02}:{s:02}"),
        None => format!("{m:02}:{s:02}"),
    }
}

/// Without seconds, for long countdowns: `1h 05m`, `2d 03h 04m`. Minutes are
/// rounded up, like the full clock rounds up seconds.
pub fn format_coarse(secs: u64) -> String {
    let mins = secs.div_ceil(60);
    let (d, h, m) = (mins / 1440, mins / 60 % 24, mins % 60);
    if d > 0 {
        format!("{d}d {h:02}h {m:02}m")
    } else {
        format!("{h}h {m:02}m")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn durations() {
        let d = |s| parse_duration(s).unwrap().as_secs();
        assert_eq!(d("90"), 90);
        assert_eq!(d("45s"), 45);
        assert_eq!(d("5m"), 300);
        assert_eq!(d("1h30m"), 5400);
        assert_eq!(d("1h 5m 10s"), 3910);
        assert_eq!(d("2 minutes"), 120);
        assert_eq!(d("10:00"), 600);
        assert_eq!(d("1:30:00"), 5400);
        assert!(parse_duration("soon").is_err());
        assert!(parse_duration("5x").is_err());
    }

    #[test]
    fn absurd_durations_are_errors_not_panics() {
        for s in [
            "1e400",
            "99999999999999999999h",
            "-5",
            "NaN",
            "inf",
            "18446744073709551615:59",
        ] {
            assert!(parse_duration(s).is_err(), "{s} should be rejected");
        }
    }

    #[test]
    fn clock_format() {
        assert_eq!(format_clock(65, None), "01:05");
        assert_eq!(format_clock(3725, Some(1)), "1:02:05");
        assert_eq!(format_clock(3725, Some(2)), "01:02:05");
        assert_eq!(format_clock(43_200, Some(2)), "12:00:00");
        assert_eq!(format_ticks(6_507, 2, None), "01:05.07");
        assert_eq!(format_ticks(651, 1, None), "01:05.1");
        assert_eq!(format_ticks(65, 0, None), "01:05");
        assert_eq!(format_ticks(372_512, 2, Some(2)), "01:02:05.12");
        assert_eq!(format_clock(2 * 86_400 + 3 * 3600 + 4 * 60 + 5, Some(1)), "2d 03:04:05");
        assert_eq!(format_clock(86_400, None), "1d 00:00:00");
        assert_eq!(format_coarse(3600), "1h 00m");
        assert_eq!(format_coarse(3601), "1h 01m");
        assert_eq!(format_coarse(2 * 86_400 + 3 * 3600 + 4 * 60), "2d 03h 04m");
    }

    #[test]
    fn command_line_overrides_config() {
        // Config values come first in argv; later (command-line) ones must win.
        let cli = Cli::try_parse_from([
            "t-",
            "--theme",
            "nord",
            "--effect",
            "matrix,burn",
            "--decimals",
            "2",
            "5m",
            "--theme",
            "gruvbox",
            "--effect",
            "rain",
        ])
        .unwrap();
        assert_eq!(cli.theme.as_deref(), Some("gruvbox"));
        assert_eq!(cli.effect, [effects::Kind::Rain]);
        assert_eq!(cli.decimals, 2);
    }

    #[test]
    fn subcommands_and_sequences_parse() {
        let cli = Cli::try_parse_from(["t-", "25m:work", "5m:break", "x4"]).unwrap();
        assert_eq!(cli.durations, ["25m:work", "5m:break", "x4"]);
        let ctl = |args: &[&str]| match Cli::try_parse_from([&["t-", "ctl"], args].concat()).map(|c| c.command) {
            Ok(Some(Command::Ctl { name, command })) => Ok((name, command.line())),
            Ok(other) => panic!("{other:?}"),
            Err(e) => Err(e.kind()),
        };
        assert_eq!(ctl(&["add", "5m"]), Ok((None, Ok("add 5m".into()))));
        assert_eq!(ctl(&["add", "1h", "5m"]), Ok((None, Ok("add 1h 5m".into()))));
        assert!(matches!(ctl(&["sub", "soon"]), Ok((None, Err(_)))));
        // --name before or after the command; `quit` is `stop`.
        let tea = Some("tea".to_string());
        assert_eq!(
            ctl(&["--name", "tea", "toggle"]),
            Ok((tea.clone(), Ok("toggle".into())))
        );
        assert_eq!(ctl(&["quit", "--name", "tea"]), Ok((tea, Ok("stop".into()))));
        assert_eq!(ctl(&["dance"]), Err(clap::error::ErrorKind::InvalidSubcommand));
        assert_eq!(ctl(&["add"]), Err(clap::error::ErrorKind::MissingRequiredArgument));
        let cli = Cli::try_parse_from(["t-", "status", "--follow"]).unwrap();
        assert!(matches!(cli.command, Some(Command::Status { follow: true, .. })));
        let cli = Cli::try_parse_from(["t-", "completions", "fish"]).unwrap();
        assert!(matches!(cli.command, Some(Command::Completions { .. })));
    }

    #[test]
    fn negations_override_config_switches() {
        // As with config values: the switch comes first, its opposite later.
        let cli = Cli::try_parse_from([
            "t-",
            "--bar",
            "--notify",
            "--no-bell",
            "5m",
            "--no-bar",
            "--no-notify",
            "--bell",
        ])
        .unwrap();
        assert!(!cli.bar && !cli.notify && !cli.no_bell);
        let cli = Cli::try_parse_from(["t-", "--no-bar", "5m", "--bar"]).unwrap();
        assert!(cli.bar);
    }

    #[test]
    fn help_text_lists_every_key() {
        let help = Cli::command().render_long_help().to_string();
        for (key, desc) in KEYS {
            let first = desc.split(['(', ':']).next().unwrap().trim();
            assert!(
                help.contains(key) && help.contains(first),
                "--help is missing {key:?} ({desc:?})"
            );
        }
    }

    #[test]
    fn cli_is_valid() {
        Cli::command().debug_assert();
    }
}
