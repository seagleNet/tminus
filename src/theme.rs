//! Colors and themes. Built-in theme names mirror the stock Omarchy themes;
//! more can be defined in the config file (see `custom`).

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    /// Parse `"#rrggbb"`. Usable in consts, so a typo in a theme fails the build.
    pub const fn hex(s: &str) -> Self {
        const fn nib(c: u8) -> u8 {
            match c {
                b'0'..=b'9' => c - b'0',
                b'a'..=b'f' => c - b'a' + 10,
                b'A'..=b'F' => c - b'A' + 10,
                _ => panic!("bad hex digit in color"),
            }
        }
        let b = s.as_bytes();
        assert!(b.len() == 7 && b[0] == b'#', "color must look like #rrggbb");
        Rgb(
            nib(b[1]) << 4 | nib(b[2]),
            nib(b[3]) << 4 | nib(b[4]),
            nib(b[5]) << 4 | nib(b[6]),
        )
    }

    /// Parse a user-supplied color: `#rrggbb`, `rrggbb` or `#rgb`.
    pub fn parse(s: &str) -> Result<Rgb, String> {
        let bad = || format!("`{s}` isn't a color (use \"#rrggbb\")");
        let s = s.trim();
        // `0x` is how Alacritty configs write colors.
        let h = s.strip_prefix("0x").or_else(|| s.strip_prefix('#')).unwrap_or(s);
        let digits: Vec<u8> = h
            .chars()
            .map(|c| c.to_digit(16).map(|d| d as u8))
            .collect::<Option<_>>()
            .ok_or_else(bad)?;
        match digits.as_slice() {
            [r1, r2, g1, g2, b1, b2] => Ok(Rgb(r1 << 4 | r2, g1 << 4 | g2, b1 << 4 | b2)),
            [r, g, b] => Ok(Rgb(r * 17, g * 17, b * 17)),
            _ => Err(bad()),
        }
    }

    pub fn lerp(self, other: Rgb, t: f32) -> Rgb {
        let t = t.clamp(0.0, 1.0);
        let f = |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * t).round() as u8;
        Rgb(f(self.0, other.0), f(self.1, other.1), f(self.2, other.2))
    }

    pub fn scale(self, k: f32) -> Rgb {
        let f = |a: u8| (f32::from(a) * k).clamp(0.0, 255.0) as u8;
        Rgb(f(self.0), f(self.1), f(self.2))
    }
}

#[derive(Clone, Debug)]
pub struct Theme {
    pub name: String,
    /// Cyclic gradient used for the settled text.
    pub gradient: Vec<Rgb>,
    /// Muted color for secondary text.
    pub dim: Rgb,
    /// Bright highlight used by effects (flashes, beam heads).
    pub flash: Rgb,
    /// The theme's background, used as the resting color of alert flashes.
    pub bg: Rgb,
    /// Warning color the background flashes toward near the end.
    pub alert: Rgb,
    /// Where the theme comes from.
    pub origin: Origin,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    Builtin,
    /// Read from an installed Omarchy theme's files.
    Omarchy,
    /// Defined in the config file.
    Config,
}

impl Theme {
    /// Sample the gradient as a loop: `u` wraps, so drifting it is seamless.
    pub fn gradient_at(&self, u: f32) -> Rgb {
        let n = self.gradient.len();
        let x = u.rem_euclid(1.0) * n as f32;
        let i = x.floor() as usize % n;
        self.gradient[i].lerp(self.gradient[(i + 1) % n], x.fract())
    }
}

/// A built-in theme, checked at compile time.
struct Builtin {
    name: &'static str,
    gradient: &'static [Rgb],
    dim: Rgb,
    flash: Rgb,
    bg: Rgb,
    alert: Rgb,
}

macro_rules! theme {
    ($name:literal, [$($g:literal),+], $dim:literal, $flash:literal, $bg:literal, $alert:literal) => {
        Builtin {
            name: $name,
            gradient: &[$(Rgb::hex($g)),+],
            dim: Rgb::hex($dim),
            flash: Rgb::hex($flash),
            bg: Rgb::hex($bg),
            alert: Rgb::hex($alert),
        }
    };
}

// name, gradient, dim, flash, bg, alert
// tokyo-night first (the default off Omarchy), then the other Omarchy stock
// themes alphabetically, then themes that aren't (or no longer are) stock.
#[rustfmt::skip]
static BUILTIN: &[Builtin] = &[
    theme!("tokyo-night", ["#7aa2f7", "#bb9af7", "#f7768e", "#7dcfff"], "#565f89", "#c0caf5", "#1a1b26", "#f7768e"),
    theme!("catppuccin", ["#89b4fa", "#cba6f7", "#f5c2e7", "#94e2d5"], "#6c7086", "#cdd6f4", "#1e1e2e", "#f38ba8"),
    theme!("catppuccin-latte", ["#1e66f5", "#ea76cb", "#d20f39", "#179299"], "#acb0be", "#4c4f69", "#eff1f5", "#d20f39"),
    theme!("ethereal", ["#7d82d9", "#c89dc1", "#ed5b5a", "#a3bfd1"], "#6d7db6", "#ffcead", "#060b1e", "#ed5b5a"),
    theme!("everforest", ["#a7c080", "#83c092", "#7fbbb3", "#dbbc7f"], "#859289", "#d3c6aa", "#2d353b", "#e67e80"),
    theme!("flexoki-light", ["#205ea6", "#ce5d97", "#d14d41", "#3aa99f"], "#b7b5ac", "#100f0f", "#fffcf0", "#d14d41"),
    theme!("gruvbox", ["#fabd2f", "#fe8019", "#fb4934", "#b8bb26"], "#928374", "#fbf1c7", "#282828", "#fb4934"),
    theme!("hackerman", ["#829dd4", "#86a7df", "#50f872", "#7cf8f7"], "#2d3450", "#ddf7ff", "#0b0c16", "#50f872"),
    theme!("jade-horse", ["#96b7d0", "#b7a1c8", "#d88d91", "#86c7c5"], "#64887b", "#dfede5", "#142620", "#d88d91"),
    theme!("kanagawa", ["#7e9cd8", "#957fb8", "#d27e99", "#7aa89f"], "#727169", "#dcd7ba", "#1f1f28", "#e82424"),
    theme!("last-horizon", ["#b59790", "#c4d8e2", "#c38b7b", "#a5a0b6"], "#584e51", "#fafcfb", "#0c0b0c", "#c38b7b"),
    theme!("lumon", ["#6fb8e3", "#8bc9eb", "#4d86b0", "#b4e4f6"], "#304860", "#d6e2ee", "#16242d", "#4d86b0"),
    theme!("lupine", ["#3264eb", "#8a4ad7", "#c900c4", "#0c67de"], "#9e9e9e", "#212121", "#fafafa", "#c900c4"),
    theme!("matte-black", ["#eaeaea", "#8a8a8d", "#f59e0b", "#b91c1c"], "#5a5a5a", "#ffffff", "#121212", "#d35f5f"),
    theme!("miasma", ["#78824b", "#bb7744", "#685742", "#c9a554"], "#666666", "#c2c2b0", "#222222", "#685742"),
    theme!("nord", ["#88c0d0", "#81a1c1", "#5e81ac", "#8fbcbb"], "#4c566a", "#eceff4", "#2e3440", "#bf616a"),
    theme!("retro-82", ["#3f8f8a", "#3f8f8a", "#f85525", "#8cbfb8"], "#2a6b78", "#f6dcac", "#05182e", "#f85525"),
    theme!("ristretto", ["#f38d70", "#fd6883", "#adda78", "#f9cc6c"], "#72696a", "#e6d9db", "#2c2525", "#fd6883"),
    theme!("rose-pine", ["#ebbcba", "#c4a7e7", "#9ccfd8", "#f6c177"], "#6e6a86", "#e0def4", "#191724", "#eb6f92"),
    theme!("solitude", ["#798186", "#aeaeae", "#565d60", "#707070"], "#4b4e55", "#cacccc", "#101315", "#565d60"),
    theme!("starship", ["#8fb8de", "#ff8fa3", "#ff5f4a", "#a6dcef"], "#8e8e93", "#f5f5f7", "#1c1c1e", "#ff5f4a"),
    theme!("vantablack", ["#8d8d8d", "#9b9b9b", "#a4a4a4", "#b0b0b0"], "#7a7a7a", "#ffffff", "#000000", "#a4a4a4"),
    theme!("white", ["#1a1a1a", "#2e2e2e", "#2a2a2a", "#3e3e3e"], "#808080", "#000000", "#ffffff", "#2a2a2a"),
    theme!("osaka-jade", ["#2dd5b7", "#549e6a", "#509475", "#e5c736"], "#53685b", "#d6dbd9", "#111c18", "#ff5345"),
    theme!("matrix", ["#00ff41", "#008f11", "#00ff9c"], "#0b5d1e", "#d0ffd8", "#000000", "#ff2a2a"),
];

/// The built-in themes.
pub fn builtin() -> Vec<Theme> {
    BUILTIN
        .iter()
        .map(|b| Theme {
            name: b.name.to_string(),
            gradient: b.gradient.to_vec(),
            dim: b.dim,
            flash: b.flash,
            bg: b.bg,
            alert: b.alert,
            origin: Origin::Builtin,
        })
        .collect()
}

/// Theme names are matched case-insensitively, with `_`/space as `-`.
pub fn normalize(name: &str) -> String {
    name.trim().to_ascii_lowercase().replace([' ', '_'], "-")
}

/// A theme defined in the config file:
///
/// ```toml
/// [themes.sunset]
/// base = "nord"            # optional: start from a built-in theme
/// gradient = ["#ff7e5f", "#feb47b"]
/// dim = "#6b5b6e"          # optional, like flash, bg and alert
/// ```
///
/// Without `base`, `gradient` is required and missing colors are derived from it.
pub fn custom(name: &str, table: &toml::Table) -> Result<Theme, String> {
    const KEYS: &[&str] = &["base", "gradient", "dim", "flash", "bg", "alert"];
    if let Some(k) = table.keys().find(|k| !KEYS.contains(&k.as_str())) {
        return Err(format!("unknown key `{k}` (valid: {})", KEYS.join(", ")));
    }
    let color = |key: &str| -> Result<Option<Rgb>, String> {
        match table.get(key) {
            None => Ok(None),
            Some(toml::Value::String(s)) => Rgb::parse(s).map(Some).map_err(|e| format!("`{key}`: {e}")),
            Some(_) => Err(format!("`{key}` must be a color string like \"#rrggbb\"")),
        }
    };
    let gradient = match table.get("gradient") {
        None => None,
        Some(toml::Value::Array(items)) if !items.is_empty() => Some(
            items
                .iter()
                .map(|v| match v {
                    toml::Value::String(s) => Rgb::parse(s),
                    _ => Err("`gradient` must be a list of color strings".to_string()),
                })
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("`gradient`: {e}"))?,
        ),
        Some(_) => return Err("`gradient` must be a non-empty list of colors".to_string()),
    };
    let base = match table.get("base") {
        None => None,
        Some(toml::Value::String(b)) => Some(
            builtin()
                .into_iter()
                .find(|t| t.name == normalize(b))
                .ok_or_else(|| format!("`base`: no built-in theme called `{b}`"))?,
        ),
        Some(_) => return Err("`base` must be a theme name".to_string()),
    };
    let mut theme = match (base, gradient) {
        (Some(mut base), gradient) => {
            if let Some(g) = gradient {
                base.gradient = g;
            }
            base
        }
        (None, Some(g)) => derived(g),
        (None, None) => return Err("needs a `gradient` (or a `base` theme to start from)".to_string()),
    };
    theme.name = normalize(name);
    theme.origin = Origin::Config;
    if let Some(c) = color("dim")? {
        theme.dim = c;
    }
    if let Some(c) = color("flash")? {
        theme.flash = c;
    }
    if let Some(c) = color("bg")? {
        theme.bg = c;
    }
    if let Some(c) = color("alert")? {
        theme.alert = c;
    }
    Ok(theme)
}

/// A theme from just a gradient, with the other colors derived from it.
fn derived(gradient: Vec<Rgb>) -> Theme {
    let n = gradient.len() as u32;
    let sum = gradient.iter().fold((0, 0, 0), |a, c| {
        (a.0 + u32::from(c.0), a.1 + u32::from(c.1), a.2 + u32::from(c.2))
    });
    let avg = Rgb((sum.0 / n) as u8, (sum.1 / n) as u8, (sum.2 / n) as u8);
    Theme {
        name: String::new(),
        dim: avg.lerp(Rgb(128, 128, 128), 0.5).scale(0.65),
        flash: avg.lerp(Rgb(255, 255, 255), 0.75),
        bg: avg.scale(0.1),
        alert: Rgb::hex("#ff4d4d"),
        origin: Origin::Config,
        gradient,
    }
}

/// Add custom themes to `themes`; one with a built-in's name replaces it.
pub fn merge(themes: &mut Vec<Theme>, custom: Vec<Theme>) {
    for t in custom {
        match themes.iter_mut().find(|x| x.name == t.name) {
            Some(slot) => *slot = t,
            None => themes.push(t),
        }
    }
}

pub fn find(themes: &[Theme], name: &str) -> Option<usize> {
    let n = normalize(name);
    themes.iter().position(|t| t.name == n)
}

/// Directory holding the active Omarchy theme (`theme/` and `theme.name`):
/// `$XDG_STATE_HOME/omarchy/current` on current Omarchy, `~/.config/omarchy/current`
/// on older versions.
fn omarchy_current() -> Option<std::path::PathBuf> {
    let home = std::path::PathBuf::from(std::env::var_os("HOME")?);
    let state = std::env::var_os("XDG_STATE_HOME")
        .filter(|v| !v.is_empty())
        .map_or_else(|| home.join(".local/state"), std::path::PathBuf::from);
    [state.join("omarchy/current"), home.join(".config/omarchy/current")]
        .into_iter()
        .find(|d| d.join("theme").is_dir())
}

/// The active Omarchy theme's name.
fn omarchy_name() -> Option<String> {
    let base = omarchy_current()?;
    if let Ok(s) = std::fs::read_to_string(base.join("theme.name")) {
        return Some(s.trim().to_string());
    }
    let link = std::fs::read_link(base.join("theme")).ok()?;
    Some(link.file_name()?.to_str()?.to_string())
}

/// A theme from an Omarchy theme directory's color files (`colors.toml`,
/// `alacritty.toml` or `kitty.conf`).
fn from_omarchy_dir(dir: &std::path::Path, name: &str) -> Option<Theme> {
    let read = |f: &str| std::fs::read_to_string(dir.join(f)).ok();
    let palette = read("colors.toml")
        .and_then(|t| palette_from_keys(&flat_toml(&t)?))
        .or_else(|| read("alacritty.toml").and_then(|t| palette_from_alacritty(&t)))
        .or_else(|| read("kitty.conf").and_then(|t| palette_from_keys(&kitty_keys(&t))))?;
    Some(palette.into_theme(name))
}

/// All installed Omarchy themes with their real colors: the stock ones
/// (`$OMARCHY_PATH/themes`), the user's (`~/.config/omarchy/themes`, which win
/// over stock ones of the same name) and the active one, read from its
/// generated files so it matches exactly even if it's installed elsewhere.
pub fn from_omarchy() -> Vec<Theme> {
    let mut themes: Vec<Theme> = Vec::new();
    let mut add = |t: Theme| match themes.iter_mut().find(|x| x.name == t.name) {
        Some(slot) => *slot = t,
        None => themes.push(t),
    };
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    let mut roots: Vec<std::path::PathBuf> = Vec::new();
    if let Some(p) = std::env::var_os("OMARCHY_PATH").filter(|v| !v.is_empty()) {
        roots.push(std::path::PathBuf::from(p).join("themes"));
    } else {
        roots.push("/usr/share/omarchy/themes".into());
        roots.extend(home.iter().map(|h| h.join(".local/share/omarchy/themes")));
    }
    roots.extend(home.iter().map(|h| h.join(".config/omarchy/themes")));
    for root in roots {
        let Ok(entries) = std::fs::read_dir(&root) else {
            continue;
        };
        let mut dirs: Vec<_> = entries.filter_map(Result::ok).map(|e| e.path()).collect();
        dirs.sort();
        for dir in dirs {
            if let Some(t) = dir
                .file_name()
                .and_then(|n| n.to_str())
                .and_then(|n| from_omarchy_dir(&dir, n))
            {
                add(t);
            }
        }
    }
    let active = omarchy_current().zip(omarchy_name());
    if let Some(t) = active.and_then(|(dir, name)| from_omarchy_dir(&dir.join("theme"), &name)) {
        add(t);
    }
    themes
}

/// The terminal colors a theme is made from.
#[derive(Debug, PartialEq)]
struct Palette {
    bg: Rgb,
    fg: Rgb,
    /// "Bright black", the usual muted gray.
    gray: Rgb,
    red: Rgb,
    green: Rgb,
    blue: Rgb,
    magenta: Rgb,
    cyan: Rgb,
}

impl Palette {
    fn into_theme(self, name: &str) -> Theme {
        Theme {
            name: normalize(name),
            gradient: vec![self.blue, self.magenta, self.red, self.cyan],
            dim: self.gray,
            flash: self.fg,
            bg: self.bg,
            alert: self.red,
            origin: Origin::Omarchy,
        }
    }
}

/// Palette from `background` / `foreground` plus either named colors
/// (`red`, `blue`, `muted`, ..., Omarchy's `colors.toml`) or `color0`..`color15`
/// (kitty).
fn palette_from_keys(keys: &std::collections::HashMap<String, String>) -> Option<Palette> {
    let c = |k: &str| keys.get(k).and_then(|v| Rgb::parse(v).ok());
    let either = |name: &str, n: &str| c(name).or_else(|| c(n));
    Some(Palette {
        bg: either("background", "color0")?,
        fg: either("foreground", "color7")?,
        gray: c("muted").or_else(|| c("color8")).or_else(|| c("color0"))?,
        red: either("red", "color1")?,
        green: either("green", "color2")?,
        blue: either("blue", "color4")?,
        magenta: either("magenta", "color5")?,
        cyan: either("cyan", "color6")?,
    })
}

/// `key value` lines of a kitty config.
fn kitty_keys(text: &str) -> std::collections::HashMap<String, String> {
    text.lines()
        .filter_map(|l| {
            let mut parts = l.split_whitespace();
            Some((parts.next()?.to_string(), parts.next()?.to_string()))
        })
        .collect()
}

/// Top-level string values of a TOML file.
fn flat_toml(text: &str) -> Option<std::collections::HashMap<String, String>> {
    let table: toml::Table = text.parse().ok()?;
    Some(
        table
            .into_iter()
            .filter_map(|(k, v)| Some((k, v.as_str()?.to_string())))
            .collect(),
    )
}

/// Palette from Alacritty's `[colors.primary]`, `[colors.normal]` and `[colors.bright]`.
fn palette_from_alacritty(text: &str) -> Option<Palette> {
    let table: toml::Table = text.parse().ok()?;
    let colors = table.get("colors")?.as_table()?;
    let get = |section: &str, key: &str| {
        colors
            .get(section)?
            .as_table()?
            .get(key)?
            .as_str()
            .and_then(|v| Rgb::parse(v).ok())
    };
    Some(Palette {
        bg: get("primary", "background")?,
        fg: get("primary", "foreground")?,
        gray: get("bright", "black").or_else(|| get("normal", "black"))?,
        red: get("normal", "red")?,
        green: get("normal", "green")?,
        blue: get("normal", "blue")?,
        magenta: get("normal", "magenta")?,
        cyan: get("normal", "cyan")?,
    })
}

/// Best-effort: pick up the active Omarchy theme if we're running on Omarchy.
/// A custom theme named like the Omarchy theme counts, so custom Omarchy
/// themes can get matching colors.
pub fn detect_omarchy(themes: &[Theme]) -> Option<usize> {
    find(themes, &omarchy_name()?)
}

/// Whether list output gets colors: stdout is a terminal and `NO_COLOR` isn't set.
pub fn color_output() -> bool {
    use std::io::IsTerminal;
    std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none_or(|v| v.is_empty())
}

/// The theme names, each in its own colors with swatches, when stdout is a
/// color terminal. Otherwise plain names, one per line, for scripts.
pub fn list(themes: &[Theme], default: usize) -> String {
    use std::fmt::Write;
    let color = color_output();
    let mut out = String::new();
    if !color {
        for t in themes {
            out += &t.name;
            out.push('\n');
        }
        return out;
    }
    let fg = |c: Rgb| format!("\x1b[38;2;{};{};{}m", c.0, c.1, c.2);
    let width = themes.iter().map(|t| t.name.chars().count()).max().unwrap_or(0);
    let swatches = themes.iter().map(|t| t.gradient.len()).max().unwrap_or(0);
    for (i, t) in themes.iter().enumerate() {
        let mut line = String::from(if i == default { "* " } else { "  " });
        let n = t.name.chars().count();
        for (j, ch) in t.name.chars().enumerate() {
            // Spread the first part of the cyclic gradient across the name.
            line += &fg(t.gradient_at(j as f32 / n as f32 * 0.75));
            line.push(ch);
        }
        line += &" ".repeat(width - n + 2);
        for &c in &t.gradient {
            line += &fg(c);
            line += "██";
        }
        line += &" ".repeat(2 * (swatches - t.gradient.len()) + 1);
        line += &fg(t.alert);
        line += "██";
        match t.origin {
            Origin::Builtin => {}
            Origin::Omarchy => {
                line += &fg(t.dim);
                line += "  (Omarchy)";
            }
            Origin::Config => {
                line += &fg(t.dim);
                line += "  (custom)";
            }
        }
        let _ = writeln!(out, "{line}\x1b[0m");
    }
    out += "\n* default (the active Omarchy theme if detected)\n";
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(s: &str) -> toml::Table {
        s.parse().unwrap()
    }

    #[test]
    fn parses_colors() {
        assert_eq!(Rgb::parse("#ff8000"), Ok(Rgb(255, 128, 0)));
        assert_eq!(Rgb::parse("FF8000"), Ok(Rgb(255, 128, 0)));
        assert_eq!(Rgb::parse("#f80"), Ok(Rgb(255, 136, 0)));
        assert!(Rgb::parse("#ff80").is_err());
        assert!(Rgb::parse("orange").is_err());
    }

    #[test]
    fn custom_from_gradient_derives_the_rest() {
        let t = custom("Sunset", &table(r##"gradient = ["#ff7e5f", "#feb47b"]"##)).unwrap();
        assert_eq!(t.name, "sunset");
        assert_eq!(t.gradient, [Rgb(0xff, 0x7e, 0x5f), Rgb(0xfe, 0xb4, 0x7b)]);
        assert_ne!(t.flash, t.dim);
    }

    #[test]
    fn custom_from_base_overrides_some_colors() {
        let nord = &builtin()[find(&builtin(), "nord").unwrap()];
        let t = custom(
            "my-nord",
            &table(
                r##"base = "nord"
alert = "#00ff00""##,
            ),
        )
        .unwrap();
        assert_eq!(t.gradient, nord.gradient);
        assert_eq!(t.dim, nord.dim);
        assert_eq!(t.alert, Rgb(0, 255, 0));
    }

    #[test]
    fn custom_errors() {
        let err = |s: &str| custom("x", &table(s)).unwrap_err();
        assert!(err(r##"dim = "#000000""##).contains("needs a `gradient`"));
        assert!(err("gradient = []").contains("non-empty"));
        assert!(err(r#"gradient = ["red"]"#).contains("isn't a color"));
        assert!(err(r#"base = "nope""#).contains("no built-in theme"));
        assert!(err(r##"colour = "#000000""##).contains("unknown key"));
    }

    #[test]
    fn palette_from_omarchy_colors_toml() {
        // Format of Omarchy's colors.toml (named keys, extra ones ignored).
        let colors = r##"mode = "dark"
accent = "#ffc55b"
muted = "#5e5263"
background = "#1d1626"
foreground = "#e8def4"
red = "#f67389"
green = "#b4d888"
cyan = "#76cbee"
blue = "#9aa2f3"
magenta = "#c9a1f7"
bright_red = "#fd95a7"
"##;
        let p = palette_from_keys(&flat_toml(colors).unwrap()).unwrap();
        assert_eq!(p.gray, Rgb::hex("#5e5263"));
        assert_eq!(p.red, Rgb::hex("#f67389"));
        assert_eq!(p.blue, Rgb::hex("#9aa2f3"));
        assert_eq!(p.bg, Rgb::hex("#1d1626"));
    }

    #[test]
    fn palettes_from_terminal_configs() {
        let alacritty = r##"
[colors.primary]
background = "0x1a1b26"
foreground = "#c0caf5"
[colors.normal]
black = "#15161e"
red = "#f7768e"
green = "#9ece6a"
yellow = "#e0af68"
blue = "#7aa2f7"
magenta = "#bb9af7"
cyan = "#7dcfff"
white = "#a9b1d6"
[colors.bright]
black = "#414868"
"##;
        let p = palette_from_alacritty(alacritty).unwrap();
        assert_eq!(
            (p.bg, p.gray, p.blue),
            (Rgb(0x1a, 0x1b, 0x26), Rgb(0x41, 0x48, 0x68), Rgb(0x7a, 0xa2, 0xf7))
        );

        let kitty = "foreground #c0caf5\nbackground #1a1b26\n# comment\ncolor1 #f7768e\ncolor2 #9ece6a\n\
                     color4 #7aa2f7\ncolor5 #bb9af7\ncolor6 #7dcfff\ncolor8 #414868\n";
        assert_eq!(palette_from_keys(&kitty_keys(kitty)), Some(p));

        let t = palette_from_alacritty(alacritty).unwrap().into_theme("My Theme");
        assert_eq!(
            (t.name.as_str(), t.origin, t.alert),
            ("my-theme", Origin::Omarchy, Rgb(0xf7, 0x76, 0x8e))
        );
        assert!(palette_from_alacritty("[colors.primary]\nbackground = \"#000000\"").is_none());
    }

    #[test]
    fn custom_replaces_builtin_with_same_name() {
        let mut themes = builtin();
        let n = themes.len();
        merge(
            &mut themes,
            vec![custom("Nord", &table(r##"gradient = ["#123456"]"##)).unwrap()],
        );
        assert_eq!(themes.len(), n);
        assert_eq!(themes[find(&themes, "nord").unwrap()].gradient, [Rgb(0x12, 0x34, 0x56)]);
    }
}
