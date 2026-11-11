//! Timekeeping that stays correct across suspend, NTP corrections and DST.
//!
//! * Duration timers count on a monotonic clock that keeps running while the
//!   machine sleeps (`CLOCK_BOOTTIME` on Linux, `CLOCK_MONOTONIC` on macOS), so
//!   "25m" means 25 real minutes.
//! * `--until` timers measure against the wall clock every frame, so they end
//!   at the requested time of day even if the clock is corrected mid-run.

use std::time::{Duration, SystemTime};

use chrono::{DateTime, Datelike, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Weekday};

/// A point on a monotonic clock that includes time spent suspended.
#[derive(Clone, Copy, Debug)]
pub struct BootInstant(Duration);

impl BootInstant {
    /// Linux: `CLOCK_BOOTTIME`. macOS: `CLOCK_MONOTONIC`, which there (unlike
    /// the clock behind `Instant`) keeps counting while the machine sleeps.
    #[cfg(any(target_os = "linux", target_os = "android", target_os = "macos", target_os = "ios"))]
    pub fn now() -> Self {
        #[cfg(any(target_os = "linux", target_os = "android"))]
        const CLOCK: libc::clockid_t = libc::CLOCK_BOOTTIME;
        #[cfg(any(target_os = "macos", target_os = "ios"))]
        const CLOCK: libc::clockid_t = libc::CLOCK_MONOTONIC;
        let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
        // SAFETY: `ts` is a valid, writable timespec and CLOCK a clock this OS has.
        let rc = unsafe { libc::clock_gettime(CLOCK, &raw mut ts) };
        assert_eq!(rc, 0, "clock_gettime failed");
        BootInstant(Duration::new(ts.tv_sec as u64, ts.tv_nsec as u32))
    }

    /// Elsewhere fall back to `Instant` (which may pause during sleep).
    #[cfg(not(any(target_os = "linux", target_os = "android", target_os = "macos", target_os = "ios")))]
    pub fn now() -> Self {
        static EPOCH: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
        BootInstant(EPOCH.get_or_init(std::time::Instant::now).elapsed())
    }

    pub fn elapsed(self) -> Duration {
        Self::now().0.saturating_sub(self.0)
    }
}

/// A countdown of `total`, measured one of two ways.
pub struct Timer {
    pub total: Duration,
    mode: Mode,
}

enum Mode {
    /// Fixed-length countdown; can be paused.
    Span {
        accumulated: Duration,
        running_since: Option<BootInstant>,
    },
    /// Counting to `started + total` on the wall clock (`--until`).
    Wall { started: SystemTime },
}

impl Timer {
    pub fn new(total: Duration) -> Self {
        Timer {
            total,
            mode: Mode::Span {
                accumulated: Duration::ZERO,
                running_since: Some(BootInstant::now()),
            },
        }
    }

    /// Count down to an absolute wall-clock time.
    pub fn until(target: SystemTime) -> Self {
        let started = SystemTime::now();
        let total = target.duration_since(started).unwrap_or_default();
        Timer {
            total,
            mode: Mode::Wall { started },
        }
    }

    pub fn elapsed(&self) -> Duration {
        match &self.mode {
            Mode::Span {
                accumulated,
                running_since,
            } => *accumulated + running_since.map_or(Duration::ZERO, BootInstant::elapsed),
            // If the clock is stepped back before `started`, treat it as just started.
            Mode::Wall { started } => SystemTime::now().duration_since(*started).unwrap_or_default(),
        }
    }

    /// Pause or resume. Pausing a wall-clock timer turns it into a plain
    /// countdown of whatever was left, since a time of day can't be paused.
    pub fn toggle(&mut self) {
        let elapsed = self.elapsed();
        self.mode = match self.mode {
            Mode::Span {
                running_since: None,
                accumulated,
            } => Mode::Span {
                accumulated,
                running_since: Some(BootInstant::now()),
            },
            Mode::Span {
                running_since: Some(_), ..
            }
            | Mode::Wall { .. } => Mode::Span {
                accumulated: elapsed,
                running_since: None,
            },
        };
    }

    pub fn paused(&self) -> bool {
        matches!(
            self.mode,
            Mode::Span {
                running_since: None,
                ..
            }
        )
    }

    /// True while still tracking the wall clock (an un-paused `--until`).
    pub fn synced(&self) -> bool {
        matches!(self.mode, Mode::Wall { .. })
    }
}

/// Parse a `--until` target and return it as an absolute time. Accepts
/// `17:30`, `17:30:15`, `tomorrow 9:00`, `today 18:00`, `fri 17:00`,
/// `friday`, `2026-12-24`, `2026-12-24 18:00` and `2026-12-24T18:00`.
pub fn parse_until(s: &str) -> Result<SystemTime, String> {
    resolve_until(&chrono::Local::now(), s).map(SystemTime::from)
}

pub fn resolve_until<Tz: TimeZone>(now: &DateTime<Tz>, s: &str) -> Result<DateTime<Tz>, String> {
    let bad = || format!("can't understand time {s:?} (try 17:30, tomorrow 9:00, fri 17:00 or 2026-12-24 18:00)");
    let text = s.trim().to_ascii_lowercase();
    // ISO style `2026-12-24T18:00`.
    let text = match text.split_once('t') {
        Some((d, t)) if NaiveDate::parse_from_str(d, "%Y-%m-%d").is_ok() => format!("{d} {t}"),
        _ => text,
    };
    let words: Vec<&str> = text.split_whitespace().collect();
    let (day, time) = match words.as_slice() {
        [one] => match parse_time(one) {
            Some(t) => return next_occurrence(now, t).ok_or_else(bad),
            None => (*one, NaiveTime::MIN),
        },
        [day, time] => (*day, parse_time(time).ok_or_else(bad)?),
        _ => return Err(bad()),
    };
    let today = now.date_naive();
    let at = if let Ok(date) = NaiveDate::parse_from_str(day, "%Y-%m-%d") {
        local_at(&now.timezone(), date.and_time(time)).ok_or_else(bad)?
    } else if day == "today" || day == "tomorrow" {
        let date = if day == "today" {
            today
        } else {
            today.succ_opt().ok_or_else(bad)?
        };
        local_at(&now.timezone(), date.and_time(time)).ok_or_else(bad)?
    } else {
        // A weekday: its next occurrence (today, if that time is still ahead).
        let wd: Weekday = day.parse().map_err(|_| bad())?;
        (0..=7)
            .filter_map(|n| today.checked_add_days(chrono::Days::new(n)))
            .filter(|d| d.weekday() == wd)
            .filter_map(|d| local_at(&now.timezone(), d.and_time(time)))
            .find(|at| at > now)
            .ok_or_else(bad)?
    };
    if at <= *now {
        return Err(format!("{s:?} is in the past"));
    }
    Ok(at)
}

fn parse_time(s: &str) -> Option<NaiveTime> {
    NaiveTime::parse_from_str(s, "%H:%M:%S")
        .or_else(|_| NaiveTime::parse_from_str(s, "%H:%M"))
        .ok()
}

/// The instant a clock in `tz` shows `local`. DST edge cases: a time that
/// occurs twice (clocks go back) resolves to the first one; a time that's
/// skipped (clocks go forward) to the first valid minute after the gap.
fn local_at<Tz: TimeZone>(tz: &Tz, local: NaiveDateTime) -> Option<DateTime<Tz>> {
    let mut local = local;
    // Gaps are at most a few hours.
    for _ in 0..24 * 60 {
        if let Some(at) = tz.from_local_datetime(&local).earliest() {
            return Some(at);
        }
        local += chrono::Duration::minutes(1);
    }
    None
}

/// The first instant after `now` when a clock in `now`'s zone reads `t`.
pub fn next_occurrence<Tz: TimeZone>(now: &DateTime<Tz>, t: NaiveTime) -> Option<DateTime<Tz>> {
    let today = now.date_naive();
    [today, today.succ_opt()?, today.succ_opt()?.succ_opt()?]
        .into_iter()
        .filter_map(|day| local_at(&now.timezone(), day.and_time(t)))
        .find(|at| at > now)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono_tz::Europe::Berlin;

    fn berlin(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> DateTime<chrono_tz::Tz> {
        Berlin.with_ymd_and_hms(y, mo, d, h, mi, 0).earliest().unwrap()
    }
    fn hm(h: u32, m: u32) -> NaiveTime {
        NaiveTime::from_hms_opt(h, m, 0).unwrap()
    }

    #[test]
    fn later_today_or_tomorrow() {
        let now = berlin(2026, 6, 1, 12, 0);
        assert_eq!(next_occurrence(&now, hm(17, 30)), Some(berlin(2026, 6, 1, 17, 30)));
        assert_eq!(next_occurrence(&now, hm(9, 0)), Some(berlin(2026, 6, 2, 9, 0)));
        // Exactly now means tomorrow, not zero.
        assert_eq!(next_occurrence(&now, hm(12, 0)), Some(berlin(2026, 6, 2, 12, 0)));
    }

    #[test]
    fn spring_forward_gap() {
        // 2026-03-29: Berlin jumps 02:00 -> 03:00, so 02:30 doesn't exist.
        let now = berlin(2026, 3, 28, 22, 0);
        let at = next_occurrence(&now, hm(2, 30)).unwrap();
        assert_eq!(at, berlin(2026, 3, 29, 3, 0));
        // 22:00 -> 03:00 reads as 5h on the wall, but only 4 real hours pass.
        assert_eq!((at - now).num_hours(), 4);
    }

    #[test]
    fn fall_back_ambiguous() {
        // 2026-10-25: Berlin goes 03:00 -> 02:00, so 02:30 happens twice. Take the first.
        let now = berlin(2026, 10, 24, 23, 0);
        let at = next_occurrence(&now, hm(2, 30)).unwrap();
        assert_eq!(at.offset().to_string(), "CEST");
        assert_eq!((at - now).num_minutes(), 3 * 60 + 30);
        // A target after the change accounts for the extra hour.
        let at = next_occurrence(&now, hm(8, 0)).unwrap();
        assert_eq!((at - now).num_hours(), 10);
    }

    #[test]
    fn dates_weekdays_and_relative_days() {
        // Friday 2026-06-05, 12:00.
        let now = berlin(2026, 6, 5, 12, 0);
        let at = |s: &str| resolve_until(&now, s);
        assert_eq!(at("17:30"), Ok(berlin(2026, 6, 5, 17, 30)));
        assert_eq!(at("tomorrow 9:00"), Ok(berlin(2026, 6, 6, 9, 0)));
        assert_eq!(at("today 18:00"), Ok(berlin(2026, 6, 5, 18, 0)));
        assert_eq!(at("fri 17:00"), Ok(berlin(2026, 6, 5, 17, 0))); // later today
        assert_eq!(at("Friday 9:00"), Ok(berlin(2026, 6, 12, 9, 0))); // already passed: next week
        assert_eq!(at("mon"), Ok(berlin(2026, 6, 8, 0, 0)));
        assert_eq!(at("2026-12-24 18:00"), Ok(berlin(2026, 12, 24, 18, 0)));
        assert_eq!(at("2026-12-24T18:00"), Ok(berlin(2026, 12, 24, 18, 0)));
        assert_eq!(at("2026-12-24"), Ok(berlin(2026, 12, 24, 0, 0)));
        assert!(at("today 9:00").unwrap_err().contains("in the past"));
        assert!(at("2020-01-01").unwrap_err().contains("in the past"));
        assert!(at("someday").unwrap_err().contains("can't understand"));
        assert!(at("25:00").is_err());
    }

    #[test]
    fn dates_across_dst() {
        // 2026-03-29 02:30 doesn't exist in Berlin: the first valid minute after.
        let now = berlin(2026, 3, 20, 12, 0);
        assert_eq!(resolve_until(&now, "2026-03-29 02:30"), Ok(berlin(2026, 3, 29, 3, 0)));
    }

    #[test]
    fn pausing_a_wall_timer_keeps_remaining() {
        let mut t = Timer::until(SystemTime::now() + Duration::from_secs(600));
        assert!(t.synced());
        t.toggle();
        assert!(t.paused() && !t.synced());
        let left = t.total.saturating_sub(t.elapsed()).as_secs_f32();
        assert!((599.0..=600.0).contains(&left), "{left}");
    }
}
