//! Optional TOML config file with defaults for the command-line options.
//!
//! Keys are the long option names (`theme`, `status-pos`, `bar`, ...; `_` works
//! in place of `-`). Each entry is turned into the equivalent command-line
//! argument and put *before* the real arguments, so clap validates config
//! values exactly like flags, and anything given on the command line wins.
//!
//! The exceptions are `[themes.<name>]` tables, which define custom themes
//! (see `theme::custom`), and the `[presets]` table of named sequences:
//! `focus = ["50m:focus", "10m:break", "x3"]`, used as `t- @focus`.

use std::{
    collections::HashMap,
    ffi::OsString,
    path::{Path, PathBuf},
};

use clap::Command;

use crate::theme::{self, Theme};

/// Options that only make sense for a single run, so not in the config.
const NOT_CONFIGURABLE: &[&str] = &[
    "until",
    "config",
    "no-config",
    "list-effects",
    "list-themes",
    "help",
    "version",
];

/// `$XDG_CONFIG_HOME/tminus/config.toml`, falling back to `~/.config/...`.
pub fn default_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("tminus").join("config.toml"))
}

/// What a config file provides.
#[derive(Default, Debug)]
pub struct Config {
    /// Option defaults, as command-line arguments for `cmd`.
    pub args: Vec<OsString>,
    /// Custom themes from `[themes.<name>]` tables.
    pub themes: Vec<Theme>,
    /// Named sequences from the `[presets]` table, as command-line tokens.
    pub presets: HashMap<String, Vec<String>>,
}

/// Read `path`. A missing file is an empty config unless `required` is set.
pub fn load(path: &Path, cmd: &Command, required: bool) -> Result<Config, String> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && !required => return Ok(Config::default()),
        Err(e) => return Err(e.to_string()),
    };
    parse(&text, cmd)
}

fn parse(text: &str, cmd: &Command) -> Result<Config, String> {
    let mut table: toml::Table = text.parse().map_err(|e: toml::de::Error| e.to_string())?;
    let themes = match table.remove("themes") {
        None => vec![],
        Some(toml::Value::Table(themes)) => themes
            .iter()
            .map(|(name, def)| match def {
                toml::Value::Table(t) => theme::custom(name, t).map_err(|e| format!("theme `{name}`: {e}")),
                _ => Err(format!("`themes.{name}` must be a table: [themes.{name}]")),
            })
            .collect::<Result<_, _>>()?,
        Some(_) => return Err("`themes` must be tables, like [themes.mine]".to_string()),
    };
    let presets = match table.remove("presets") {
        None => HashMap::new(),
        Some(toml::Value::Table(presets)) => presets
            .iter()
            .map(|(name, v)| {
                let tokens = match v {
                    toml::Value::Array(items) => items
                        .iter()
                        .map(|i| i.as_str().map(str::to_string))
                        .collect::<Option<Vec<_>>>(),
                    _ => None,
                };
                tokens
                    .filter(|t| !t.is_empty())
                    .map(|t| (theme::normalize(name), t))
                    .ok_or_else(|| format!("preset `{name}` must be a list like [\"25m:work\", \"5m:break\", \"x4\"]"))
            })
            .collect::<Result<_, _>>()?,
        Some(_) => return Err("`presets` must be a table: [presets]".to_string()),
    };
    Ok(Config {
        args: to_args(&table, cmd)?,
        themes,
        presets,
    })
}

fn to_args(table: &toml::Table, cmd: &Command) -> Result<Vec<OsString>, String> {
    let mut args = Vec::new();
    for (key, value) in table {
        let name = key.replace('_', "-");
        let arg = cmd
            .get_arguments()
            .find(|a| {
                a.get_long() == Some(name.as_str()) && !a.is_hide_set() && !NOT_CONFIGURABLE.contains(&name.as_str())
            })
            .ok_or_else(|| format!("unknown key `{key}` (valid keys: {})", valid_keys(cmd).join(", ")))?;
        let flag = format!("--{name}");
        if arg.get_action().takes_values() {
            let val = match value {
                toml::Value::String(s) => s.clone(),
                toml::Value::Integer(n) => n.to_string(),
                toml::Value::Float(n) => n.to_string(),
                toml::Value::Array(items) => items.iter().map(scalar).collect::<Result<Vec<_>, _>>()?.join(","),
                _ => return Err(format!("`{key}` needs a value, like {key} = \"...\"")),
            };
            args.extend([flag.into(), val.into()]);
        } else {
            match value {
                toml::Value::Boolean(true) => args.push(flag.into()),
                toml::Value::Boolean(false) => {}
                _ => return Err(format!("`{key}` is an on/off switch: use {key} = true or false")),
            }
        }
    }
    Ok(args)
}

fn scalar(v: &toml::Value) -> Result<String, String> {
    match v {
        toml::Value::String(s) => Ok(s.clone()),
        toml::Value::Integer(n) => Ok(n.to_string()),
        toml::Value::Float(n) => Ok(n.to_string()),
        _ => Err(format!("lists can only hold strings and numbers, not {}", v.type_str())),
    }
}

fn valid_keys(cmd: &Command) -> Vec<&str> {
    cmd.get_arguments()
        .filter(|a| !a.is_hide_set())
        .filter_map(clap::Arg::get_long)
        .filter(|l| !NOT_CONFIGURABLE.contains(l))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    fn args(toml: &str) -> Result<Vec<String>, String> {
        parse(toml, &crate::cli::Cli::command()).map(|c| c.args.into_iter().map(|a| a.into_string().unwrap()).collect())
    }

    #[test]
    fn maps_keys_to_flags() {
        let a = args(
            "theme = \"nord\"\nbar = true\nno_bell = false\neffect = [\"matrix\", \"burn\"]\ndecimals = 2\nevery = 7.5",
        )
        .unwrap()
        .join(" ");
        // Table order isn't file order, so check each mapping separately.
        for part in [
            "--theme nord",
            "--bar",
            "--effect matrix,burn",
            "--decimals 2",
            "--every 7.5",
        ] {
            assert!(a.contains(part), "{part:?} missing from {a:?}");
        }
        assert!(!a.contains("no-bell"), "false switches add nothing: {a:?}");
    }

    #[test]
    fn reads_custom_themes() {
        let c = parse(
            "theme = \"sunset\"\n[themes.sunset]\ngradient = [\"#ff7e5f\", \"#feb47b\"]\n[themes.Calm]\nbase = \"nord\"\n",
            &crate::cli::Cli::command(),
        )
        .unwrap();
        let mut names: Vec<_> = c.themes.iter().map(|t| t.name.as_str()).collect();
        names.sort_unstable();
        assert_eq!(names, ["calm", "sunset"]);
        assert_eq!(c.args, ["--theme", "sunset"]);
        let err = parse("[themes.bad]\ngradient = [\"nope\"]", &crate::cli::Cli::command()).unwrap_err();
        assert!(err.contains("theme `bad`"), "{err}");
    }

    #[test]
    fn reads_presets() {
        let c = parse(
            "[presets]\nPomodoro = [\"25m:work\", \"5m:break\", \"x4\"]",
            &crate::cli::Cli::command(),
        )
        .unwrap();
        assert_eq!(c.presets["pomodoro"], ["25m:work", "5m:break", "x4"]);
        assert!(
            parse("[presets]\nbad = \"25m\"", &crate::cli::Cli::command())
                .unwrap_err()
                .contains("must be a list")
        );
    }

    #[test]
    fn rejects_bad_config() {
        assert!(args("colour = \"red\"").unwrap_err().contains("unknown key `colour`"));
        assert!(args("until = \"17:00\"").unwrap_err().contains("unknown key"));
        assert!(args("bar = \"yes\"").unwrap_err().contains("on/off"));
        assert!(args("theme = true").unwrap_err().contains("needs a value"));
        assert!(args("theme = ").is_err());
    }
}
