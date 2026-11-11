//! Remote control over a Unix socket, for Waybar modules and keybindings.
//!
//! Every running timer listens on `$XDG_RUNTIME_DIR/tminus/<name>.<pid>.sock`
//! (a private, per-user directory). The protocol is one line of text per
//! connection: `status`, `follow` (stream status lines), or a command such as
//! `toggle` or `add 5m`; the reply is a JSON status line, `ok` or `error: ...`.

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
pub fn connect(name: Option<&str>) -> Option<UnixStream> {
    let want = name.map(sanitize);
    let mut socks: Vec<(std::time::SystemTime, PathBuf)> = std::fs::read_dir(socket_dir())
        .ok()?
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
    for (_, path) in socks {
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
    Ok(reply.trim_end().to_string())
}

/// `t- status [--follow]`: print status JSON. Following never ends on its
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

/// `t- ctl ...`: send a command and report the reply. Returns success.
pub fn ctl(name: Option<&str>, command: &[String]) -> Result<(), String> {
    let mut s = connect(name).ok_or_else(|| match name {
        Some(n) => format!("no running timer called `{n}`"),
        None => "no running timer".to_string(),
    })?;
    let reply = request(&mut s, &command.join(" ")).map_err(|e| e.to_string())?;
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
