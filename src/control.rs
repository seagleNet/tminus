//! Remote control over a Unix socket, for Waybar modules and keybindings.
//!
//! Every running timer listens on `$XDG_RUNTIME_DIR/tminus/<name>.<pid>.sock`
//! (a private, per-user directory). The protocol is one line of text per
//! connection: `status`, `follow` (stream status lines), `info` (one
//! tab-separated line for `t- ctl list`), or a command such as `toggle` or
//! `add 5m`; the reply is a JSON status line, the info line, `ok` or `error: ...`.

use std::{
    fmt::Write as _,
    io::{self, BufRead, BufReader, Write},
    os::unix::{
        fs::PermissionsExt,
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
    time::Duration,
};

/// Status line printed when no timer is running; empty text hides a Waybar module.
pub const IDLE_JSON: &str = r#"{"text":"","tooltip":"no timer running","class":"idle"}"#;

pub fn socket_dir() -> PathBuf {
    // Socket paths are limited to ~108 bytes; leave room for `<name>.<pid>.sock`.
    const MAX_DIR_LEN: usize = 60;
    std::env::var_os("XDG_RUNTIME_DIR")
        .filter(|v| !v.is_empty())
        .map(|dir| PathBuf::from(dir).join("tminus"))
        .filter(|p| p.as_os_str().len() <= MAX_DIR_LEN)
        // SAFETY: getuid has no preconditions and cannot fail.
        .unwrap_or_else(|| PathBuf::from("/tmp").join(format!("tminus-{}", unsafe { libc::getuid() })))
}

/// Words for the names of unnamed timers (`eager-comet`), like Docker's
/// container names, with a launch-countdown theme.
const ADJECTIVES: &[&str] = &[
    "agile", "amber", "bold", "brave", "bright", "brisk", "calm", "clever", "cosmic", "crisp", "daring", "eager",
    "early", "fearless", "fiery", "gentle", "golden", "grand", "happy", "keen", "lively", "lucid", "lunar", "mellow",
    "mighty", "nimble", "noble", "patient", "polar", "proud", "quick", "quiet", "radiant", "rapid", "ready", "serene",
    "sharp", "silent", "silver", "solar", "steady", "stellar", "swift", "tidy", "vivid", "warm", "wise", "zesty",
];
const NOUNS: &[&str] = &[
    "apogee",
    "asteroid",
    "aurora",
    "beacon",
    "booster",
    "capsule",
    "comet",
    "cosmos",
    "countdown",
    "eclipse",
    "equinox",
    "galaxy",
    "gantry",
    "horizon",
    "ignition",
    "lander",
    "liftoff",
    "meteor",
    "module",
    "moon",
    "nebula",
    "nova",
    "orbit",
    "payload",
    "perigee",
    "planet",
    "probe",
    "pulsar",
    "quasar",
    "rocket",
    "rover",
    "satellite",
    "shuttle",
    "signal",
    "solstice",
    "star",
    "station",
    "sunrise",
    "telescope",
    "thruster",
    "trajectory",
    "vector",
    "voyager",
    "zenith",
    "launchpad",
    "starlight",
    "spacewalk",
    "stardust",
];

/// A name no running timer has: `base` itself if given and free (a preset's
/// name, e.g. `pomodoro`), else `base` with a random adjective
/// (`eager-pomodoro`); without `base`, adjective and noun (`eager-comet`).
pub fn free_name(rng: &mut impl rand::Rng, base: Option<&str>) -> String {
    pick_name(rng, base, |name| running_pid(name).is_some())
}

/// `free_name` with the "is this name in use?" check passed in.
fn pick_name(rng: &mut impl rand::Rng, base: Option<&str>, taken: impl Fn(&str) -> bool) -> String {
    use rand::seq::IndexedRandom;
    let base = base.map(sanitize);
    if let Some(b) = &base
        && !taken(b)
    {
        return b.clone();
    }
    let mut pick = || {
        let adjective = ADJECTIVES.choose(rng).copied().unwrap_or("quick");
        let noun = match &base {
            Some(b) => b.as_str(),
            None => NOUNS.choose(rng).copied().unwrap_or("timer"),
        };
        format!("{adjective}-{noun}")
    };
    // Enough combinations that 20 tries only fail with dozens of timers running.
    for _ in 0..20 {
        let name = pick();
        if !taken(&name) {
            return name;
        }
    }
    format!("{}-{}", pick(), std::process::id())
}

/// The PID of a running timer called `name` (compared like socket names, so
/// `Tea` is `tea`). Sockets of timers that died don't count and are removed.
pub fn running_pid(name: &str) -> Option<u32> {
    sockets(Some(name)).into_iter().find_map(|path| {
        let (_, pid) = sock_name_and_pid(&path)?;
        if UnixStream::connect(&path).is_ok() {
            Some(pid)
        } else {
            let _ = std::fs::remove_file(&path);
            None
        }
    })
}

/// Socket names only use a safe subset of characters.
pub fn sanitize(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    if s.is_empty() { "tminus".into() } else { s }
}

/// Quote `s` as a JSON string.
pub fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// One request from a client: its command line and the connection to answer on.
pub struct Request {
    pub line: String,
    pub stream: UnixStream,
}

pub struct Server {
    listener: UnixListener,
    path: PathBuf,
    followers: Vec<UnixStream>,
    last_sent: String,
}

impl Server {
    pub fn start(name: &str) -> io::Result<Server> {
        let dir = socket_dir();
        std::fs::create_dir_all(&dir)?;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
        let path = dir.join(format!("{}.{}.sock", sanitize(name), std::process::id()));
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path)?;
        listener.set_nonblocking(true)?;
        Ok(Server {
            listener,
            path,
            followers: vec![],
            last_sent: String::new(),
        })
    }

    /// Accept pending connections and read each one's request line.
    pub fn accept(&mut self) -> Vec<Request> {
        let mut out = vec![];
        while let Ok((stream, _)) = self.listener.accept() {
            // Clients send their line right away; don't let a silent one stall us.
            let _ = stream.set_nonblocking(false);
            let _ = stream.set_read_timeout(Some(Duration::from_millis(100)));
            let _ = stream.set_write_timeout(Some(Duration::from_millis(100)));
            let mut line = String::new();
            if let Ok(reader) = stream.try_clone()
                && BufReader::new(reader).read_line(&mut line).is_ok()
            {
                out.push(Request {
                    line: line.trim().to_string(),
                    stream,
                });
            }
        }
        out
    }

    /// Keep `stream` and send it every status change from now on.
    pub fn follow(&mut self, mut stream: UnixStream, status: &str) {
        if writeln!(stream, "{status}").is_ok() {
            self.followers.push(stream);
        }
    }

    /// Send `status` to followers if it changed; drops ones that went away.
    pub fn publish(&mut self, status: &str) {
        if status == self.last_sent {
            return;
        }
        self.last_sent = status.to_string();
        self.followers.retain_mut(|s| writeln!(s, "{status}").is_ok());
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Connect to a running timer: the one called `name`, else the newest.
/// Sockets left behind by timers that died are removed along the way.
/// Timer sockets (optionally only those named `name`), newest first.
fn sockets(name: Option<&str>) -> Vec<PathBuf> {
    let want = name.map(sanitize);
    let Ok(dir) = std::fs::read_dir(socket_dir()) else {
        return vec![];
    };
    let mut socks: Vec<(std::time::SystemTime, PathBuf)> = dir
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            let Some((sock_name, _pid)) = sock_name_and_pid(p) else {
                return false;
            };
            want.as_deref().is_none_or(|w| w == sock_name)
        })
        .filter_map(|p| Some((p.metadata().ok()?.modified().ok()?, p)))
        .collect();
    socks.sort_by_key(|(modified, _)| std::cmp::Reverse(*modified));
    socks.into_iter().map(|(_, path)| path).collect()
}

/// Connect to the newest live timer (named `name`, if given). Sockets left
/// behind by timers that died are removed on the way.
pub fn connect(name: Option<&str>) -> Option<UnixStream> {
    for path in sockets(name) {
        match UnixStream::connect(&path) {
            Ok(s) => return Some(s),
            Err(_) => {
                let _ = std::fs::remove_file(&path);
            }
        }
    }
    None
}

fn sock_name_and_pid(p: &Path) -> Option<(String, u32)> {
    let stem = p.file_name()?.to_str()?.strip_suffix(".sock")?;
    let (name, pid) = stem.rsplit_once('.')?;
    Some((name.to_string(), pid.parse().ok()?))
}

/// Send one request and return the first reply line.
pub fn request(stream: &mut UnixStream, line: &str) -> io::Result<String> {
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    writeln!(stream, "{line}")?;
    let mut reply = String::new();
    BufReader::new(stream.try_clone()?).read_line(&mut reply)?;
    // Only the line ending: `info` replies can end in an empty, tab-separated field.
    Ok(reply.trim_end_matches(['\n', '\r']).to_string())
}

/// `t- ctl status [--follow]`: print status JSON. Following never ends on its
/// own: it reconnects whenever a timer (re)appears, printing the idle line
/// in between, which is what a Waybar `exec` module wants.
pub fn status(name: Option<&str>, follow: bool) -> io::Result<()> {
    let mut out = io::stdout().lock();
    if !follow {
        let line = match connect(name) {
            Some(mut s) => request(&mut s, "status")?,
            None => IDLE_JSON.to_string(),
        };
        return writeln!(out, "{line}");
    }
    let mut last = String::new();
    let mut emit = |line: &str| -> io::Result<()> {
        if line != last {
            writeln!(out, "{line}")?;
            out.flush()?;
            last = line.to_string();
        }
        Ok(())
    };
    loop {
        if let Some(mut s) = connect(name) {
            s.set_read_timeout(None)?;
            writeln!(s, "follow")?;
            for line in BufReader::new(s).lines() {
                let Ok(line) = line else { break };
                emit(&line)?;
            }
        }
        emit(IDLE_JSON)?;
        std::thread::sleep(Duration::from_secs(1));
    }
}

/// One running timer, as `t- ctl list` shows it.
#[derive(Debug, PartialEq, Eq)]
struct Listed {
    name: String,
    pid: u32,
    /// From the timer's `info` reply; `None` for a timer too old to know it.
    info: Option<[String; 5]>,
}

impl Listed {
    /// Mode, state, clock, title and sequence position, `?` when unknown.
    fn fields(&self) -> [&str; 5] {
        match &self.info {
            Some([mode, state, clock, title, seg]) => [mode, state, clock, title, seg],
            None => ["?", "?", "", "(older t-, try t- ctl status)", ""],
        }
    }
}

/// Parse an `info` reply (see `App::info_line`).
fn parse_info(line: &str) -> Option<[String; 5]> {
    if line.starts_with("error: ") {
        return None;
    }
    let fields: Vec<String> = line.split('\t').map(str::to_string).collect();
    fields.try_into().ok()
}

/// Every live timer (named `name`, if given), newest first.
fn running(name: Option<&str>) -> Vec<Listed> {
    sockets(name)
        .into_iter()
        .filter_map(|path| {
            let (name, pid) = sock_name_and_pid(&path)?;
            let Ok(mut s) = UnixStream::connect(&path) else {
                let _ = std::fs::remove_file(&path);
                return None;
            };
            let info = request(&mut s, "info").ok().as_deref().and_then(parse_info);
            Some(Listed { name, pid, info })
        })
        .collect()
}

/// Names of the running timers, newest first, each with a short description
/// (`headless, paused 02:59 · tea`), for shell completion of `t- ctl -n`.
pub fn names_for_completion() -> Vec<(String, String)> {
    running(None)
        .into_iter()
        .map(|t| {
            let [mode, state, clock, title, seg] = t.fields();
            let mut help = format!("{mode}, {state} {clock}").trim_end().to_string();
            for extra in [title, seg] {
                if !extra.is_empty() {
                    help += " · ";
                    help += extra;
                }
            }
            (t.name, help)
        })
        .collect()
}

/// `t- ctl list [--json]`: a table (or JSON array) of the running timers.
/// The newest is marked: it's the one `t- ctl` acts on without `--name`.
fn list(name: Option<&str>, json: bool) -> io::Result<()> {
    let timers = running(name);
    let mut out = io::stdout().lock();
    if json {
        let items: Vec<String> = timers
            .iter()
            .enumerate()
            .map(|(i, t)| {
                let [mode, state, clock, title, seg] = t.fields();
                format!(
                    r#"{{"name":{},"pid":{},"mode":{},"state":{},"clock":{},"title":{},"segment":{},"default":{}}}"#,
                    json_str(&t.name),
                    t.pid,
                    json_str(mode),
                    json_str(state),
                    json_str(clock),
                    json_str(title),
                    json_str(seg),
                    i == 0
                )
            })
            .collect();
        return writeln!(out, "[{}]", items.join(","));
    }
    if timers.is_empty() {
        return writeln!(out, "no running timer");
    }
    let header = ["NAME", "PID", "MODE", "STATE", "CLOCK", "TITLE"];
    let rows: Vec<[String; 6]> = timers
        .iter()
        .map(|t| {
            let [mode, state, clock, title, seg] = t.fields();
            let title = match (title, seg) {
                (t, "") => t.to_string(),
                ("", s) => s.to_string(),
                (t, s) => format!("{t} · {s}"),
            };
            [
                t.name.clone(),
                t.pid.to_string(),
                mode.into(),
                state.into(),
                clock.into(),
                title,
            ]
        })
        .collect();
    let width = |col: usize| {
        rows.iter()
            .map(|r| r[col].chars().count())
            .chain([header[col].len()])
            .max()
            .unwrap_or(0)
    };
    let widths: Vec<usize> = (0..header.len()).map(width).collect();
    let line = |marker: &str, cells: &[&str]| {
        let mut l = String::from(marker);
        for (i, cell) in cells.iter().enumerate() {
            if i + 1 == cells.len() {
                l += cell;
            } else {
                let _ = write!(l, "{cell:w$}  ", w = widths[i]);
            }
        }
        l.trim_end().to_string()
    };
    writeln!(out, "{}", line("  ", &header))?;
    for (i, r) in rows.iter().enumerate() {
        let cells: Vec<&str> = r.iter().map(String::as_str).collect();
        writeln!(out, "{}", line(if i == 0 { "* " } else { "  " }, &cells))?;
    }
    Ok(())
}

/// `t- ctl pause|resume|stop --all`: send `line` to every running timer and
/// say which ones it reached (`done` is the past tense: `paused`).
fn on_all(line: &str, done: &str) -> io::Result<()> {
    let mut out = io::stdout().lock();
    let mut found = false;
    for path in sockets(None) {
        let Some((name, pid)) = sock_name_and_pid(&path) else {
            continue;
        };
        let Ok(mut s) = UnixStream::connect(&path) else {
            let _ = std::fs::remove_file(&path);
            continue;
        };
        found = true;
        match request(&mut s, line) {
            Ok(reply) if reply == "ok" => writeln!(out, "{done} {name} (pid {pid})")?,
            Ok(reply) => writeln!(out, "couldn't {line} {name} (pid {pid}): {reply}")?,
            Err(e) => writeln!(out, "couldn't {line} {name} (pid {pid}): {e}")?,
        }
    }
    if !found {
        writeln!(out, "no running timer")?;
    }
    Ok(())
}

/// `t- ctl ...`: send a command and report the reply, or for `status` and
/// `list`, print what was asked for. Returns success.
pub fn ctl(name: Option<&str>, command: &crate::cli::CtlCommand) -> Result<(), String> {
    use crate::cli::CtlCommand;
    let printed = match command {
        CtlCommand::Status { follow } => Some(status(name, *follow)),
        CtlCommand::List { json } => Some(list(name, *json)),
        CtlCommand::Pause { all: true } | CtlCommand::Resume { all: true } | CtlCommand::Stop { all: true }
            if name.is_some() =>
        {
            return Err("--all acts on every timer; leave out --name (or --all)".into());
        }
        CtlCommand::Pause { all: true } => Some(on_all("pause", "paused")),
        CtlCommand::Resume { all: true } => Some(on_all("resume", "resumed")),
        CtlCommand::Stop { all: true } => Some(on_all("stop", "stopped")),
        _ => None,
    };
    if let Some(result) = printed {
        return match result {
            // A pipe reader (`| head`, or Waybar) went away; nothing to report.
            Err(e) if e.kind() != io::ErrorKind::BrokenPipe => Err(e.to_string()),
            _ => Ok(()),
        };
    }
    let line = command.line()?;
    let mut s = connect(name).ok_or_else(|| match name {
        Some(n) => format!("no running timer called `{n}`"),
        None => "no running timer".to_string(),
    })?;
    let reply = request(&mut s, &line).map_err(|e| e.to_string())?;
    match reply.strip_prefix("error: ") {
        Some(e) => Err(e.to_string()),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helpers() {
        assert_eq!(sanitize("Deep Work!"), "deep-work-");
        assert_eq!(sanitize(""), "tminus");
        assert_eq!(json_str("a \"b\"\\\n"), r#""a \"b\"\\\n""#);
        assert_eq!(
            sock_name_and_pid(Path::new("/x/focus.123.sock")),
            Some(("focus".into(), 123))
        );
        assert_eq!(sock_name_and_pid(Path::new("/x/other")), None);
    }

    #[test]
    fn names_for_unnamed_timers() {
        // Every generated name is already a valid socket name.
        for words in [ADJECTIVES, NOUNS] {
            for w in words {
                assert_eq!(sanitize(w), *w);
            }
        }
        let rng = &mut rand::rng();
        let none_taken = |_: &str| false;
        let name = pick_name(rng, None, none_taken);
        let (adjective, noun) = name.split_once('-').unwrap();
        assert!(ADJECTIVES.contains(&adjective) && NOUNS.contains(&noun), "{name}");
        // A preset's name (made socket-safe) while it's free...
        assert_eq!(pick_name(rng, Some("Deep Work"), none_taken), "deep-work");
        // ...then with an adjective in front.
        let name = pick_name(rng, Some("pomodoro"), |n| n == "pomodoro");
        let (adjective, rest) = name.split_once('-').unwrap();
        assert!(ADJECTIVES.contains(&adjective) && rest == "pomodoro", "{name}");
    }

    #[test]
    fn info_replies() {
        let fields = parse_info("headless\trunning\t02:41\ttea\t").unwrap();
        assert_eq!(fields[0], "headless");
        let t = Listed {
            name: "tea".into(),
            pid: 1,
            info: Some(fields),
        };
        assert_eq!(t.fields(), ["headless", "running", "02:41", "tea", ""]);
        // A timer from before `info` existed answers with an error.
        assert_eq!(parse_info("error: unknown command `info` (try ...)"), None);
        assert_eq!(parse_info("too\tfew"), None);
        // An empty last field (no sequence) still counts.
        assert!(parse_info("window\trunning\t25:00\tdeep work\t").is_some());
    }

    #[test]
    fn server_round_trip() {
        let dir = std::env::temp_dir().join(format!("tminus-test-{}", std::process::id()));
        // SAFETY: tests touching XDG_RUNTIME_DIR are limited to this one.
        unsafe { std::env::set_var("XDG_RUNTIME_DIR", &dir) };
        let mut server = Server::start("unit test").unwrap();
        let client = std::thread::spawn(|| {
            let mut s = connect(Some("unit test")).expect("socket found");
            request(&mut s, "toggle").unwrap()
        });
        let mut reqs = vec![];
        for _ in 0..200 {
            reqs = server.accept();
            if !reqs.is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let mut req = reqs.pop().expect("request received");
        assert_eq!(req.line, "toggle");
        writeln!(req.stream, "ok").unwrap();
        assert_eq!(client.join().unwrap(), "ok");
        drop(server);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
