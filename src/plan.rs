//! Sequences: turning `25m:work 5m:break x4` into a list of segments.
//!
//! Each token is one of:
//! * `DURATION[:LABEL][@THEME]`, e.g. `25m`, `25m:work`, `10:00:deep work@nord`
//! * `xN` — repeat the segments listed before it (in this list) N times in all
//! * `@PRESET` — the segments of a preset: built in (`@pomodoro`) or from the
//!   config's `[presets]` table, which can also replace a built-in one

use std::{collections::HashMap, time::Duration};

use crate::cli::parse_duration;

#[derive(Clone, Debug, PartialEq)]
pub struct Segment {
    pub dur: Duration,
    pub label: Option<String>,
    /// Theme name for this segment (resolved by the caller).
    pub theme: Option<String>,
}

const MAX_SEGMENTS: usize = 10_000;

/// Presets that need no config. A config preset with the same name replaces one.
const BUILTIN_PRESETS: &[(&str, &[&str])] = &[
    // The Pomodoro Technique: four 25-minute work blocks with short breaks,
    // and a long break after the fourth.
    (
        "pomodoro",
        &["25m:work", "5m:break", "x3", "25m:work", "15m:long break"],
    ),
];

/// Parse sequence tokens; `presets` are the config's, on top of the built-in ones.
pub fn parse(tokens: &[String], presets: &HashMap<String, Vec<String>>) -> Result<Vec<Segment>, String> {
    parse_level(tokens, &with_builtins(presets), true)
}

/// The built-in presets with the config's on top (a config preset with a
/// built-in's name replaces it).
pub fn with_builtins(presets: &HashMap<String, Vec<String>>) -> HashMap<String, Vec<String>> {
    let mut all: HashMap<String, Vec<String>> = BUILTIN_PRESETS
        .iter()
        .map(|(name, items)| ((*name).to_string(), items.iter().map(|t| (*t).to_string()).collect()))
        .collect();
    all.extend(presets.iter().map(|(k, v)| (k.clone(), v.clone())));
    all
}

fn parse_level(tokens: &[String], presets: &HashMap<String, Vec<String>>, top: bool) -> Result<Vec<Segment>, String> {
    let mut out: Vec<Segment> = Vec::new();
    for token in tokens {
        let token = token.trim();
        if let Some(name) = token.strip_prefix('@') {
            if !top {
                return Err(format!("presets can't include other presets (`{token}`)"));
            }
            let key = crate::theme::normalize(name);
            let items = presets.get(&key).ok_or_else(|| {
                let mut known: Vec<&str> = presets.keys().map(String::as_str).collect();
                known.sort_unstable();
                format!("no preset `{name}` (available: {})", known.join(", "))
            })?;
            out.extend(parse_level(items, presets, false).map_err(|e| format!("preset `{name}`: {e}"))?);
        } else if let Some(n) = repeat_count(token) {
            if out.is_empty() {
                return Err(format!("`{token}` needs segments before it to repeat"));
            }
            if n == 0 || out.len().saturating_mul(n) > MAX_SEGMENTS {
                return Err(format!(
                    "`{token}`: repeat count must be between 1 and {MAX_SEGMENTS} segments in all"
                ));
            }
            let once = out.clone();
            for _ in 1..n {
                out.extend(once.iter().cloned());
            }
        } else {
            out.push(segment(token).map_err(|e| {
                // `:pomodoro` or `pomodoro` for `@pomodoro`.
                let word = token.trim_start_matches(':');
                if presets.contains_key(&crate::theme::normalize(word)) {
                    format!("can't understand `{token}`; did you mean `@{word}`?")
                } else {
                    e
                }
            })?);
        }
    }
    Ok(out)
}

/// `x4` / `X4` → 4.
fn repeat_count(token: &str) -> Option<usize> {
    let digits = token.strip_prefix(['x', 'X'])?;
    (!digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())).then(|| digits.parse().unwrap_or(usize::MAX))
}

/// `DURATION[:LABEL][@THEME]`. Durations may contain colons themselves
/// (`10:00`), so the label split is the rightmost colon whose left side is a
/// valid duration: `10:00:work` → 10:00 + "work".
fn segment(token: &str) -> Result<Segment, String> {
    let (rest, theme) = match token.rsplit_once('@') {
        Some((rest, theme)) if !theme.is_empty() => (rest, Some(theme.to_string())),
        _ => (token, None),
    };
    if let Ok(dur) = parse_duration(rest) {
        return Ok(Segment {
            dur,
            label: None,
            theme,
        });
    }
    for (i, _) in rest.match_indices(':').rev() {
        if let Ok(dur) = parse_duration(&rest[..i]) {
            let label = rest[i + 1..].trim();
            return Ok(Segment {
                dur,
                label: (!label.is_empty()).then(|| label.to_string()),
                theme,
            });
        }
    }
    Err(format!(
        "can't understand `{token}` (expected a duration like 25m with an optional :label and @theme, \
         xN to repeat, or @preset)"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(tokens: &[&str]) -> Result<Vec<Segment>, String> {
        let presets = HashMap::from([
            (
                "pomodoro".to_string(),
                vec!["25m:work".to_string(), "5m:break".to_string(), "x2".to_string()],
            ),
            ("loop".to_string(), vec!["@pomodoro".to_string()]),
        ]);
        parse(&tokens.iter().map(|s| (*s).to_string()).collect::<Vec<_>>(), &presets)
    }
    fn mins(m: u64) -> Duration {
        Duration::from_secs(m * 60)
    }

    #[test]
    fn labels_themes_and_colon_durations() {
        let s = p(&[
            "25m:work",
            "10:00",
            "10:00:deep work@nord",
            "1:30:00",
            "5m@everforest",
            "90:2",
        ])
        .unwrap();
        assert_eq!(
            s[0],
            Segment {
                dur: mins(25),
                label: Some("work".into()),
                theme: None
            }
        );
        assert_eq!(
            s[1],
            Segment {
                dur: mins(10),
                label: None,
                theme: None
            }
        );
        assert_eq!(
            s[2],
            Segment {
                dur: mins(10),
                label: Some("deep work".into()),
                theme: Some("nord".into())
            }
        );
        assert_eq!(s[3].dur, mins(90));
        assert_eq!(
            (s[4].label.as_deref(), s[4].theme.as_deref()),
            (None, Some("everforest"))
        );
        // `90:2` is a valid mm:ss duration, not 90s labelled "2".
        assert_eq!(s[5].dur, Duration::from_secs(90 * 60 + 2));
    }

    #[test]
    fn repeats_and_presets() {
        let s = p(&["25m:work", "5m:break", "x4"]).unwrap();
        assert_eq!(s.len(), 8);
        assert_eq!(s[6].label.as_deref(), Some("work"));
        let s = p(&["@pomodoro", "15m:long break"]).unwrap();
        assert_eq!(s.len(), 5);
        assert_eq!(s[4].dur, mins(15));
    }

    #[test]
    fn builtin_pomodoro() {
        let s = parse(&["@pomodoro".to_string()], &HashMap::new()).unwrap();
        let plan: Vec<(u64, &str)> = s
            .iter()
            .map(|x| (x.dur.as_secs() / 60, x.label.as_deref().unwrap_or("")))
            .collect();
        assert_eq!(
            plan,
            [
                (25, "work"),
                (5, "break"),
                (25, "work"),
                (5, "break"),
                (25, "work"),
                (5, "break"),
                (25, "work"),
                (15, "long break")
            ]
        );
        // A config preset of the same name replaces it (see `p`).
        assert_eq!(p(&["@pomodoro"]).unwrap().len(), 4);
    }

    #[test]
    fn errors() {
        assert!(p(&["x3"]).unwrap_err().contains("needs segments"));
        assert!(p(&["5m", "x0"]).unwrap_err().contains("repeat count"));
        assert!(p(&["5m", "x999999"]).unwrap_err().contains("repeat count"));
        assert!(p(&["@nope"]).unwrap_err().contains("available: loop, pomodoro"));
        assert!(p(&["@loop"]).unwrap_err().contains("can't include other presets"));
        assert!(p(&["soon:work"]).unwrap_err().contains("or @preset"));
        // A preset name without its @ gets a hint.
        assert!(p(&[":pomodoro"]).unwrap_err().contains("did you mean `@pomodoro`?"));
        assert!(p(&["Loop"]).unwrap_err().contains("did you mean `@Loop`?"));
    }
}
