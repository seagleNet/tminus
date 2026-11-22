//! The debug panel's numbers (`d` in the timer): frame rate, time spent
//! drawing and writing, output, CPU, memory and key latency.
//!
//! The TUI loop records one `Sample` per frame; the panel shows a summary of
//! the last second, worked out a few times a second so it can be read (and so
//! the panel itself doesn't rewrite cells every frame).

use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

/// How much history the summary covers.
const WINDOW: Duration = Duration::from_secs(1);
/// How often the summary (and so the panel's text) is worked out again.
const REFRESH: Duration = Duration::from_millis(250);

/// One frame, as the TUI loop measured it.
#[derive(Clone, Copy)]
pub struct Sample {
    /// When the frame was presented.
    pub at: Instant,
    /// Building the frame (`App::draw`).
    pub draw: Duration,
    /// Diffing it and writing it out (`Term::present`).
    pub present: Duration,
    /// Bytes written to the terminal.
    pub bytes: u64,
    /// Cells rewritten.
    pub cells: usize,
    /// The whole screen was repainted (first frame, resize, background change).
    pub full: bool,
}

/// The last second, summed up.
#[derive(Clone, Default)]
pub struct Summary {
    pub fps: f64,
    pub draw_avg: Duration,
    pub draw_max: Duration,
    pub present_avg: Duration,
    pub present_max: Duration,
    /// Longest gap between two frames.
    pub gap_max: Duration,
    pub bytes_per_s: f64,
    pub cells_avg: f64,
    pub full_repaints: usize,
    /// CPU time used per wall time since the last summary (1.0 = one core).
    pub cpu: Option<f64>,
    /// Resident memory in bytes, and whether it's the peak rather than now.
    pub memory: Option<(u64, bool)>,
    /// Nixie tube cache since the last summary: the share of tubes taken
    /// from it, and tubes drawn per second.
    pub tubes: Option<(f64, f64)>,
}

#[derive(Default)]
pub struct Stats {
    /// The panel is open (`d`).
    shown: bool,
    /// The widest the value column has been since it opened: the box only
    /// grows, or it would jump as rows change length.
    pub value_w: i32,
    samples: VecDeque<Sample>,
    /// From reading a key to the frame showing it, for the last key.
    pub key_latency: Option<Duration>,
    /// The terminal clears with the background color (terminfo `bce`).
    pub bce: bool,
    summary: Summary,
    /// When `summary` was worked out, and the CPU time and tube cache
    /// counters then (for rates).
    summed_at: Option<Instant>,
    cpu_then: Option<Duration>,
    tubes_then: (u64, u64),
}

impl Stats {
    /// Open or close the panel. It opens at its smallest, and without rates
    /// averaged over the time it was closed.
    pub fn toggle(&mut self) {
        self.shown = !self.shown;
        self.value_w = 0;
        self.summed_at = None;
    }

    pub fn shown(&self) -> bool {
        self.shown
    }

    pub fn record(&mut self, s: Sample) {
        while self.samples.front().is_some_and(|f| s.at.duration_since(f.at) > WINDOW) {
            self.samples.pop_front();
        }
        self.samples.push_back(s);
    }

    /// The summary, worked out again when it's older than `REFRESH`.
    pub fn summary(&mut self) -> &Summary {
        let now = Instant::now();
        if self.summed_at.is_some_and(|t| now.duration_since(t) < REFRESH) {
            return &self.summary;
        }
        let since = self.summed_at.map(|t| now.duration_since(t));
        self.summed_at = Some(now);

        let n = self.samples.len();
        let span = match (self.samples.front(), self.samples.back()) {
            (Some(a), Some(b)) if n > 1 => b.at.duration_since(a.at).as_secs_f64(),
            _ => 0.0,
        };
        let avg = |d: Duration| if n == 0 { Duration::ZERO } else { d / n as u32 };
        let draw: Duration = self.samples.iter().map(|s| s.draw).sum();
        let present: Duration = self.samples.iter().map(|s| s.present).sum();
        // Rates over the frames after the first (their intervals span `span`).
        let bytes: u64 = self.samples.iter().skip(1).map(|s| s.bytes).sum();
        let gap_max = self
            .samples
            .iter()
            .zip(self.samples.iter().skip(1))
            .map(|(a, b)| b.at.duration_since(a.at))
            .max()
            .unwrap_or_default();

        let cpu_now = cpu_time();
        let cpu = match (cpu_now, self.cpu_then, since) {
            (Some(now), Some(then), Some(wall)) if !wall.is_zero() => {
                Some(now.saturating_sub(then).as_secs_f64() / wall.as_secs_f64())
            }
            _ => None,
        };
        self.cpu_then = cpu_now;

        let tubes_now = crate::nixie::cache_counts();
        let (hits, drawn) = (tubes_now.0 - self.tubes_then.0, tubes_now.1 - self.tubes_then.1);
        self.tubes_then = tubes_now;

        self.summary = Summary {
            fps: if span > 0.0 { (n - 1) as f64 / span } else { 0.0 },
            draw_avg: avg(draw),
            draw_max: self.samples.iter().map(|s| s.draw).max().unwrap_or_default(),
            present_avg: avg(present),
            present_max: self.samples.iter().map(|s| s.present).max().unwrap_or_default(),
            gap_max,
            bytes_per_s: if span > 0.0 { bytes as f64 / span } else { 0.0 },
            cells_avg: if n == 0 {
                0.0
            } else {
                self.samples.iter().map(|s| s.cells).sum::<usize>() as f64 / n as f64
            },
            full_repaints: self.samples.iter().filter(|s| s.full).count(),
            cpu,
            memory: memory(),
            tubes: since.filter(|_| hits + drawn > 0).map(|wall| {
                (
                    hits as f64 / (hits + drawn) as f64,
                    drawn as f64 / wall.as_secs_f64().max(1e-3),
                )
            }),
        };
        &self.summary
    }
}

/// CPU time this process has used (user + system).
#[cfg(unix)]
fn cpu_time() -> Option<Duration> {
    // SAFETY: `rusage` is plain data; all zeroes is a valid value.
    let mut ru: libc::rusage = unsafe { std::mem::zeroed() };
    // SAFETY: `ru` is a valid, writable rusage.
    if unsafe { libc::getrusage(libc::RUSAGE_SELF, &raw mut ru) } != 0 {
        return None;
    }
    let tv = |t: libc::timeval| Duration::from_secs(t.tv_sec as u64) + Duration::from_micros(t.tv_usec as u64);
    Some(tv(ru.ru_utime) + tv(ru.ru_stime))
}

#[cfg(not(unix))]
fn cpu_time() -> Option<Duration> {
    None
}

/// Resident memory in bytes: current on Linux, the peak elsewhere (`true`).
#[cfg(target_os = "linux")]
fn memory() -> Option<(u64, bool)> {
    let statm = std::fs::read_to_string("/proc/self/statm").ok()?;
    let pages: u64 = statm.split_whitespace().nth(1)?.parse().ok()?;
    // SAFETY: sysconf has no preconditions.
    let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    Some((pages * u64::try_from(page).ok()?, false))
}

#[cfg(all(unix, not(target_os = "linux")))]
fn memory() -> Option<(u64, bool)> {
    // SAFETY: as in `cpu_time`.
    let mut ru: libc::rusage = unsafe { std::mem::zeroed() };
    // SAFETY: `ru` is a valid, writable rusage.
    if unsafe { libc::getrusage(libc::RUSAGE_SELF, &raw mut ru) } != 0 {
        return None;
    }
    // macOS reports bytes (Linux would be KiB, but it reads statm above).
    Some((ru.ru_maxrss as u64, true))
}

#[cfg(not(unix))]
fn memory() -> Option<(u64, bool)> {
    None
}

/// A duration in milliseconds, e.g. `1.24ms`.
pub fn ms(d: Duration) -> String {
    format!("{:.2}ms", d.as_secs_f64() * 1000.0)
}

/// A byte count, e.g. `512 B`, `12.3 KB`, `4.1 MB`.
pub fn bytes(n: f64) -> String {
    if n < 1000.0 {
        format!("{n:.0} B")
    } else if n < 1e6 {
        format!("{:.1} KB", n / 1e3)
    } else {
        format!("{:.1} MB", n / 1e6)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sums_up_the_last_second() {
        let mut st = Stats::default();
        let t0 = Instant::now();
        for i in 0..=20u32 {
            st.record(Sample {
                at: t0 + Duration::from_millis(50) * i,
                draw: Duration::from_millis(u64::from(i % 2 + 1)),
                present: Duration::from_millis(1),
                bytes: 100,
                cells: 10,
                full: i == 0,
            });
        }
        // 1s of history: frames 0..=20 are 50ms apart, all kept.
        let s = st.summary().clone();
        assert!((s.fps - 20.0).abs() < 1e-6, "{}", s.fps);
        assert!((s.bytes_per_s - 2000.0).abs() < 1e-6);
        assert_eq!(s.draw_max, Duration::from_millis(2));
        assert_eq!(s.gap_max, Duration::from_millis(50));
        assert_eq!(s.full_repaints, 1);
        // Older frames drop out.
        st.record(Sample {
            at: t0 + Duration::from_millis(1100),
            ..st.samples[0]
        });
        assert!(st.samples.front().unwrap().at > t0);
        assert_eq!(bytes(512.0), "512 B");
        assert_eq!(bytes(12_345.0), "12.3 KB");
        assert_eq!(ms(Duration::from_micros(1240)), "1.24ms");
    }

    #[cfg(unix)]
    #[test]
    fn reads_cpu_and_memory() {
        assert!(cpu_time().is_some());
        assert!(memory().is_some_and(|(b, _)| b > 0));
    }
}
