mod app;
mod cli;
mod clock;
mod config;
mod control;
mod effects;
mod font;
mod plan;
mod scene;
mod term;
mod theme;
mod util;

use std::{
    collections::HashMap,
    ffi::OsString,
    io::{IsTerminal, Write},
    process::ExitCode,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use clap::{CommandFactory, Parser};
use crossterm::{
    event::{self, Event as TermEvent, KeyEventKind},
    terminal,
};

use crate::{
    app::{App, Event, Reply, Step},
    cli::{Cli, Command},
    clock::Timer,
    config::Config,
    control::Server,
    term::{Input, Term},
};

/// Input events handled per frame at most (a backstop against input floods).
const MAX_EVENTS_PER_FRAME: usize = 64;
/// Frame interval when nothing is animating (the gradient drift is slow).
const IDLE_FRAME: Duration = Duration::from_millis(50);
/// How often headless mode wakes up.
const HEADLESS_TICK: Duration = Duration::from_millis(100);

fn fail(msg: &str) -> ExitCode {
    eprintln!("tminus: {msg}");
    ExitCode::from(2)
}

/// Parse the command line, with config file values as defaults underneath it.
/// `t- status`, `t- pause -n tea`, ...: a `t- ctl` command typed without
/// the `ctl` (0.1/0.2 had a top-level `status`). None of these words is a
/// duration, so this never catches a real timer.
fn ctl_hint(argv: &[OsString]) -> Option<String> {
    let word = argv.get(1)?.to_str()?;
    let ctl = Cli::command();
    ctl.find_subcommand("ctl")?.find_subcommand(word)?;
    let typed: Vec<String> = argv[1..].iter().map(|a| a.to_string_lossy().into_owned()).collect();
    Some(format!(
        "`{word}` is a `t- ctl` command: try `t- ctl {}`",
        typed.join(" ")
    ))
}

fn parse_cli() -> Result<(Cli, Config), ExitCode> {
    let argv: Vec<OsString> = std::env::args_os().collect();
    if let Some(hint) = ctl_hint(&argv) {
        return Err(fail(&hint));
    }
    // Parse once without the config: handles --help, finds --config/--no-config,
    // and means any error in the second parse must come from the config.
    let cli = Cli::parse_from(&argv);
    // Subcommands (status, ctl) talk to a running timer; option defaults don't apply.
    if cli.no_config || cli.command.is_some() {
        return Ok((cli, Config::default()));
    }
    let Some((path, required)) = cli
        .config
        .clone()
        .map(|p| (p, true))
        .or_else(|| config::default_path().map(|p| (p, false)))
    else {
        return Ok((cli, Config::default()));
    };
    let in_config = |e: &dyn std::fmt::Display| fail(&format!("in config {}: {e}", path.display()));
    let mut config = config::load(&path, &Cli::command(), required).map_err(|e| in_config(&e))?;
    if config.args.is_empty() {
        return Ok((cli, config));
    }
    let from_config = std::mem::take(&mut config.args);
    let merged = argv[..1]
        .iter()
        .cloned()
        .chain(from_config)
        .chain(argv[1..].iter().cloned());
    let cli =
        Cli::try_parse_from(merged).map_err(|e| in_config(&e.to_string().trim_start_matches("error: ").trim_end()))?;
    Ok((cli, config))
}

fn main() -> ExitCode {
    // `COMPLETE=<shell> t-` prints the shell's completion hook, and the hook
    // calls back in here on every Tab; either way this exits before the
    // config is read or anything else happens.
    clap_complete::CompleteEnv::with_factory(Cli::command)
        .bin(BIN)
        .complete();
    let (cli, config) = match parse_cli() {
        Ok(parsed) => parsed,
        Err(code) => return code,
    };

    if let Some(command) = &cli.command {
        return run_command(command);
    }

    let mut themes = theme::builtin();
    // Real colors of the installed Omarchy themes, then config themes on top.
    theme::merge(&mut themes, theme::from_omarchy());
    theme::merge(&mut themes, config.themes);
    let default_theme = theme::detect_omarchy(&themes).unwrap_or(0);

    if cli.list_effects {
        return print_out(effects::list(theme::color_output()).as_bytes());
    }
    if cli.list_themes {
        return print_out(theme::list(&themes, default_theme).as_bytes());
    }

    let find_theme = |name: &str| {
        theme::find(&themes, name).ok_or_else(|| {
            let names: Vec<&str> = themes.iter().map(|t| t.name.as_str()).collect();
            format!("unknown theme {name:?} (available: {})", names.join(", "))
        })
    };
    let theme = match cli.theme.as_deref().map(find_theme).transpose() {
        Ok(t) => t.unwrap_or(default_theme),
        Err(e) => return fail(&e),
    };

    let built = build_plan(&cli, &config.presets, &find_theme);
    let (mut steps, timer) = match built {
        Ok(b) => b,
        Err(e) => return fail(&e),
    };
    if cli.demo {
        // Only the first segment's clock is shown.
        steps.truncate(1);
    }

    // Intro and at-zero effects fall back to --effect when not given.
    let pool_or_default =
        |chosen: &[effects::Kind]| effects::pool(if chosen.is_empty() { &cli.effect } else { chosen });
    let pools = effects::Pools {
        intro: pool_or_default(&cli.intro_effect),
        replay: effects::pool(&cli.effect),
        zero: pool_or_default(&cli.zero_effect),
    };

    if let Some(f) = &cli.sound
        && !f.is_file()
    {
        return fail(&format!("sound file {} not found", f.display()));
    }

    // Every timer has its own name, so `t- ctl --name` can reach each one.
    // Unnamed ones get a random free one (`eager-comet`); a name that's taken
    // is refused rather than shadowing the running timer.
    let name = match &cli.name {
        _ if cli.demo => None,
        Some(n) => match control::running_pid(n) {
            Some(pid) => {
                return fail(&format!(
                    "a timer called `{n}` is already running (pid {pid}); stop it with \
                     `t- ctl stop -n {n}`, or pick another --name"
                ));
            }
            None => Some(n.clone()),
        },
        // Named after the preset if one was used (`t- @pomodoro` → `pomodoro`).
        None => Some(control::free_name(&mut rand::rng(), first_preset(&cli.durations))),
    };
    // Remote control is best effort for the TUI, but the point of --headless.
    let server = match name.as_deref().map(Server::start) {
        Some(Ok(s)) => Some(s),
        Some(Err(e)) if cli.headless => return fail(&format!("can't open the control socket: {e}")),
        None | Some(Err(_)) => None,
    };
    // A headless timer has no screen to show its generated name on.
    if cli.headless
        && cli.name.is_none()
        && let Some(n) = &name
        && std::io::stderr().is_terminal()
    {
        eprintln!("tminus: started as {n} (t- ctl list, t- ctl ... -n {n})");
    }

    let mut app = App::new(&cli, pools, &themes, theme, steps, timer);
    let result = if cli.headless && !cli.demo {
        run_headless(&cli, app, server)
    } else {
        let prev_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            term::restore();
            prev_hook(info);
        }));
        let r = run(&cli, &mut app, server);
        term::restore();
        if let (Ok(_), Some((theme, effect))) = (&r, app.demo_choice()) {
            return print_out(format!("--theme {theme} --effect {effect}\n").as_bytes());
        }
        r
    };
    match result {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(e) => fail(&e.to_string()),
    }
}

/// The binary's name, as completions and the man page should call it.
const BIN: &str = "t-";

/// Subcommands: talk to a running timer, or print completions / the man page.
fn run_command(command: &Command) -> ExitCode {
    match command {
        Command::Completions { shell } => {
            // The same hook `COMPLETE=<shell> t-` prints, for installing as a file.
            let name = shell.to_string();
            let shells = clap_complete::env::Shells::builtins();
            let Some(completer) = shells.completer(&name) else {
                return fail(&format!("no completions for {name}"));
            };
            let mut buf = Vec::new();
            if let Err(e) = completer.write_registration("COMPLETE", BIN, BIN, BIN, &mut buf) {
                return fail(&e.to_string());
            }
            if name == "zsh" {
                // Installed as an autoloaded `_t-` file (in $fpath), the first
                // Tab only runs this file: complete right away, not just on the
                // next Tab. Sourcing it (`source <(COMPLETE=zsh t-)`) is unaffected.
                buf.extend_from_slice(
                    b"\nif [[ \"${funcstack[1]}\" == _t- ]]; then\n    _clap_dynamic_completer_t_ \"$@\"\nfi\n",
                );
            }
            print_out(&buf)
        }
        Command::Man => {
            let mut buf = Vec::new();
            match clap_mangen::Man::new(Cli::command().name(BIN)).render(&mut buf) {
                Ok(()) => print_out(&buf),
                Err(e) => fail(&e.to_string()),
            }
        }
        Command::Ctl { name, command } => match control::ctl(name.as_deref(), command) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("tminus: {e}");
                ExitCode::from(1)
            }
        },
    }
}

/// Write generated output; a reader that stops early (`| head`) is fine.
fn print_out(bytes: &[u8]) -> ExitCode {
    match std::io::stdout().write_all(bytes) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => ExitCode::SUCCESS,
        Err(e) => fail(&e.to_string()),
    }
}

/// The first `@preset` among the sequence tokens, if any.
fn first_preset(tokens: &[String]) -> Option<&str> {
    tokens
        .iter()
        .find_map(|t| t.trim().strip_prefix('@'))
        .filter(|p| !p.is_empty())
}

/// The plan: one or more segments, and the timer for the first.
fn build_plan(
    cli: &Cli,
    presets: &HashMap<String, Vec<String>>,
    find_theme: &dyn Fn(&str) -> Result<usize, String>,
) -> Result<(Vec<Step>, Timer), String> {
    if let Some(u) = &cli.until {
        let timer = Timer::until(clock::parse_until(u)?);
        return Ok((
            vec![Step {
                dur: timer.total,
                label: None,
                theme: None,
            }],
            timer,
        ));
    }
    let demo_default = ["25m".to_string()];
    let durations = if cli.demo && cli.durations.is_empty() {
        &demo_default[..]
    } else {
        &cli.durations[..]
    };
    let steps = plan::parse(durations, presets)?
        .into_iter()
        .map(|s| {
            let theme = s.theme.as_deref().map(find_theme).transpose()?;
            Ok(Step {
                dur: s.dur,
                label: s.label,
                theme,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let first = steps.first().ok_or("nothing to count down")?.dur;
    Ok((steps, Timer::new(first)))
}

/// Quit cleanly when killed (restoring the terminal), not just on `q`.
fn watch_signals() -> std::io::Result<Arc<AtomicBool>> {
    let signalled = Arc::new(AtomicBool::new(false));
    for sig in [
        signal_hook::consts::SIGTERM,
        signal_hook::consts::SIGHUP,
        signal_hook::consts::SIGINT,
    ] {
        signal_hook::flag::register(sig, Arc::clone(&signalled))?;
    }
    Ok(signalled)
}

/// Answer remote-control requests. Returns true if asked to stop.
fn serve(server: &mut Option<Server>, app: &mut App, now: f64) -> bool {
    let Some(srv) = server else { return false };
    for mut req in srv.accept() {
        let reply = match req.line.as_str() {
            "status" => app.status_json(),
            "info" => app.info_line(app.cli.headless),
            "follow" => {
                let status = app.status_json();
                srv.follow(req.stream, &status);
                continue;
            }
            line => match app.remote(line, now) {
                Ok(Reply::Ok) => "ok".to_string(),
                Ok(Reply::Stop) => {
                    let _ = writeln!(req.stream, "ok");
                    return true;
                }
                Err(e) => format!("error: {e}"),
            },
        };
        let _ = writeln!(req.stream, "{reply}");
    }
    srv.publish(&app.status_json());
    false
}

/// Bell, notification, sound and command when a segment or the whole
/// countdown ends.
fn announce(cli: &Cli, event: &Event, term: Option<&mut Term>) -> std::io::Result<()> {
    if !cli.no_bell
        && let Some(term) = term
    {
        term.bell()?;
    }
    let (kind, done, next) = match event {
        Event::SegmentDone { done, next } => ("segment", done, next.as_deref()),
        Event::Completed { done } => ("done", done, None),
    };
    if cli.notify {
        let msg = match event {
            Event::SegmentDone { next, .. } => {
                format!("Time's up. Next: {}", next.as_deref().unwrap_or("next segment"))
            }
            Event::Completed { .. } => cli.title.clone().unwrap_or_else(|| "Time's up!".into()),
        };
        notify(&msg);
    }
    if let Some(file) = &cli.sound {
        play_sound(file);
    }
    if let Some(cmd) = &cli.exec {
        let env = [
            ("TMINUS_EVENT", kind),
            ("TMINUS_LABEL", done.as_deref().unwrap_or_default()),
            ("TMINUS_NEXT", next.unwrap_or_default()),
            ("TMINUS_TITLE", cli.title.as_deref().unwrap_or_default()),
        ];
        let mut c = std::process::Command::new("sh");
        c.args(["-c", cmd]).envs(env);
        run_detached(c);
    }
    Ok(())
}

/// Start `cmd` now, without letting it draw over the screen, and reap it on
/// a background thread. Starting it right away matters: with --exit we may
/// quit a moment later, and a started process outlives us.
fn run_detached(mut cmd: std::process::Command) -> bool {
    use std::process::Stdio;
    let Ok(mut child) = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };
    std::thread::spawn(move || child.wait());
    true
}

/// Play a sound file with the first audio player that's installed.
fn play_sound(file: &std::path::Path) {
    for player in ["pw-play", "paplay", "aplay", "afplay"] {
        let mut c = std::process::Command::new(player);
        c.arg(file);
        if run_detached(c) {
            return;
        }
    }
}

/// Runs the TUI. Returns Ok(true) if the countdown completed.
fn run(cli: &Cli, app: &mut App, mut server: Option<Server>) -> std::io::Result<bool> {
    let signalled = watch_signals()?;
    let mut term =
        Term::enter().map_err(|e| std::io::Error::new(e.kind(), format!("needs to run in a terminal ({e})")))?;
    app.term_bg = term.background();
    let mut rng = rand::rng();
    let epoch = Instant::now();
    let frame_dur = Duration::from_secs_f64(1.0 / f64::from(cli.fps));
    let (w, h) = terminal::size()?;
    app.resize(w, h);

    loop {
        let tick = Instant::now();
        let now = epoch.elapsed().as_secs_f64();
        if signalled.load(Ordering::Relaxed) {
            return Ok(app.completed());
        }

        // Input. crossterm's reader is only used when our own poll says there
        // is real input, since it spins forever on a hung-up terminal.
        match term.wait_input(Duration::ZERO)? {
            Input::HungUp => return Ok(app.completed()),
            Input::None => {}
            Input::Ready => {
                for _ in 0..MAX_EVENTS_PER_FRAME {
                    if !event::poll(Duration::ZERO)? {
                        break;
                    }
                    if let TermEvent::Key(k) = event::read()?
                        && k.kind != KeyEventKind::Release
                        && app.on_key(k.code, k.modifiers, now)
                    {
                        return Ok(app.completed());
                    }
                }
            }
        }
        // Resizes are picked up by checking the size, not via crossterm's reader.
        let (w, h) = terminal::size()?;
        if (i32::from(w), i32::from(h)) != (app.frame.w, app.frame.h) {
            app.resize(w, h);
            term.invalidate();
        }

        if serve(&mut server, app, now) {
            return Ok(app.completed());
        }
        if let Some(event) = app.tick(now)
            && !cli.demo
        {
            announce(cli, &event, Some(&mut term))?;
            if matches!(event, Event::Completed { .. }) && cli.exit {
                return Ok(true);
            }
        }

        app.draw(now, &mut rng);
        term.present(&app.frame)?;

        let budget = if app.animating(now) {
            frame_dur
        } else {
            frame_dur.max(IDLE_FRAME)
        };
        if let Some(wait) = budget.checked_sub(tick.elapsed())
            && term.wait_input(wait)? == Input::HungUp
        {
            return Ok(app.completed());
        }
    }
}

/// No screen: keep time, answer remote control, and announce the end.
fn run_headless(cli: &Cli, mut app: App, mut server: Option<Server>) -> std::io::Result<bool> {
    let signalled = watch_signals()?;
    let epoch = Instant::now();
    loop {
        let now = epoch.elapsed().as_secs_f64();
        if signalled.load(Ordering::Relaxed) || serve(&mut server, &mut app, now) {
            return Ok(app.completed());
        }
        if let Some(event) = app.tick(now) {
            announce(cli, &event, None)?;
            if matches!(event, Event::Completed { .. }) && cli.exit {
                return Ok(true);
            }
        }
        std::thread::sleep(HEADLESS_TICK);
    }
}

/// Fire-and-forget desktop notification.
fn notify(msg: &str) {
    let mut c = std::process::Command::new("notify-send");
    c.args(["-u", "critical", "tminus", msg]);
    run_detached(c);
}

#[cfg(test)]
mod tests {
    #[test]
    fn ctl_commands_without_ctl_get_a_hint() {
        let hint = |args: &[&str]| {
            let argv: Vec<std::ffi::OsString> = ["t-"].iter().chain(args).map(Into::into).collect();
            super::ctl_hint(&argv)
        };
        assert_eq!(
            hint(&["status", "--follow"]).as_deref(),
            Some("`status` is a `t- ctl` command: try `t- ctl status --follow`")
        );
        assert!(
            hint(&["pause", "-n", "tea"])
                .unwrap()
                .ends_with("`t- ctl pause -n tea`")
        );
        // Durations, options first, and other subcommands are left alone.
        assert_eq!(hint(&["5m"]), None);
        assert_eq!(hint(&["--no-config", "status"]), None);
        assert_eq!(hint(&["ctl", "status"]), None);
        assert_eq!(hint(&["help"]), None);
        assert_eq!(hint(&[]), None);
    }

    #[test]
    fn wide_chars_take_two_cells() {
        let g = crate::font::plain("a☕b");
        assert_eq!(g.w, 4);
        assert_eq!(g.chars[2], crate::term::WIDE_TAIL);
        assert_eq!(crate::term::str_width("café ☕"), 7);
    }

    #[test]
    fn fonts_are_rectangular() {
        use crate::font;
        for text in ["00:00", "1:23:45", "98:76", "12:34.56"] {
            for g in [font::shadow(text), font::block(text, 3), font::plain(text)] {
                assert_eq!(g.chars.len(), g.w * g.h);
                assert_eq!(g.owner.len(), g.w * g.h);
            }
        }
        // Digits are fixed-width so the clock doesn't wobble.
        assert_eq!(font::shadow("11:11").w, font::shadow("00:00").w);
    }
}
