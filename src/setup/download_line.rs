//! The download progress line (bugfix 2026-11-07, user report 3): while a
//! release downloads, one line on stderr shows bytes, percent, speed and
//! remaining time, redrawn in place — instead of a screen that freezes for
//! the whole download. It draws only when stderr is a terminal, so piped
//! and captured output stays clean, and every draw is throttled.
//!
//! The [`Reporter`] hands the release download a [`ProgressHook`]; the
//! menu suspends its full-screen view around the download and the
//! numbered fallback prints the line directly.

use std::io::{IsTerminal, Write};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The hook the release download calls: bytes so far, total bytes when the
/// server named a size.
pub type ProgressHook = Arc<dyn Fn(u64, Option<u64>) + Send + Sync>;

/// How often the line is redrawn at most.
const REDRAW: Duration = Duration::from_millis(100);

/// A smoothed bytes-per-second estimate over the samples fed in.
#[derive(Debug, Default)]
pub struct Speedometer {
    last: Option<(u64, Instant)>,
    speed: f64,
}

impl Speedometer {
    /// A new estimate at zero.
    pub fn new() -> Self {
        Self::default()
    }

    /// One sample; returns the current estimate. Samples less than 100 ms
    /// apart only move the byte baseline (the window stays open), so a
    /// burst of samples cannot spike the estimate.
    pub fn feed(&mut self, bytes: u64, now: Instant) -> f64 {
        if let Some((at, then)) = self.last {
            let dt = now.saturating_duration_since(then).as_secs_f64();
            if dt >= 0.1 {
                let burst = bytes.saturating_sub(at) as f64 / dt;
                self.speed = if self.speed == 0.0 {
                    burst
                } else {
                    0.7 * self.speed + 0.3 * burst
                };
                self.last = Some((bytes, now));
            } else {
                self.last = Some((bytes, then));
            }
        } else {
            self.last = Some((bytes, now));
        }
        self.speed
    }
}

/// `512 B`, `4.1 MB`, `1.2 GB` (1024-based, one decimal above 1024).
pub fn human_bytes(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = 1024.0 * KB;
    const GB: f64 = 1024.0 * MB;
    let n = bytes as f64;
    if n >= GB {
        format!("{:.1} GB", n / GB)
    } else if n >= MB {
        format!("{:.1} MB", n / MB)
    } else if n >= KB {
        format!("{:.1} KB", n / KB)
    } else {
        format!("{bytes} B")
    }
}

/// The seconds left at `speed`, when that is computable and meaningful.
fn seconds_left(bytes: u64, total: u64, speed: f64) -> Option<u64> {
    let rest = total.saturating_sub(bytes);
    (speed > 0.0 && rest > 0).then(|| (rest as f64 / speed).round() as u64)
}

/// The line drawn for one sample of `label`'s download.
pub fn progress_line(label: &str, bytes: u64, total: Option<u64>, speed: f64) -> String {
    let speed_part = if speed > 0.0 {
        format!(" · {}/s", human_bytes(speed as u64))
    } else {
        String::new()
    };
    let left = total
        .filter(|&t| t > 0)
        .and_then(|t| seconds_left(bytes, t, speed))
        .map(|s| format!(" · {s}s left"))
        .unwrap_or_default();
    let size = match total.filter(|&t| t > 0) {
        Some(total) => format!("{} ({}%)", pair(bytes, total), percent(bytes, total)),
        None => human_bytes(bytes),
    };
    format!("Downloading {label}: {size}{speed_part}{left}")
}

/// `4.0/8.0 MB`: both sizes in the unit the total needs, so the fraction
/// reads as one measurement.
fn pair(bytes: u64, total: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = 1024.0 * KB;
    const GB: f64 = 1024.0 * MB;
    let (div, unit) = if total as f64 >= GB {
        (GB, "GB")
    } else if total as f64 >= MB {
        (MB, "MB")
    } else if total as f64 >= KB {
        (KB, "KB")
    } else {
        (1.0, "B")
    };
    format!(
        "{:.1}/{:.1} {}",
        bytes as f64 / div,
        total as f64 / div,
        unit
    )
}

/// The whole percent, capped at 100.
fn percent(bytes: u64, total: u64) -> u64 {
    ((bytes as f64 / total as f64) * 100.0).round() as u64
}

/// The shared state one download updates.
struct Shared {
    label: String,
    state: Mutex<State>,
    out: Mutex<Box<dyn Write + Send>>,
}

#[derive(Default)]
struct State {
    speed: Speedometer,
    drawn: Option<Instant>,
    last: Option<(u64, Option<u64>)>,
}

/// One redrawn progress line (see the module docs).
pub struct Reporter {
    shared: Option<Arc<Shared>>,
}

impl Reporter {
    /// A reporter drawing on stderr when that is a terminal (`None`
    /// otherwise: piped output stays clean).
    pub fn stderr(label: &str) -> Self {
        Self::enabled(label, std::io::stderr().is_terminal())
    }

    /// A reporter over an in-memory sink (tests).
    #[cfg(test)]
    pub fn memory(label: &str) -> (Self, Arc<Mutex<Vec<u8>>>) {
        let sink: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
        let writer = SinkWriter(sink.clone());
        let shared = Shared {
            label: label.to_owned(),
            state: Mutex::new(State::default()),
            out: Mutex::new(Box::new(writer)),
        };
        (
            Self {
                shared: Some(Arc::new(shared)),
            },
            sink,
        )
    }

    /// A reporter that draws (`enabled`) to stderr; split out so the tests
    /// of the drawing itself run through [`Reporter::memory`].
    fn enabled(label: &str, enabled: bool) -> Self {
        Self {
            shared: enabled.then(|| {
                Arc::new(Shared {
                    label: label.to_owned(),
                    state: Mutex::new(State::default()),
                    out: Mutex::new(Box::new(std::io::stderr())),
                })
            }),
        }
    }

    /// The hook for the release download; `None` when nothing draws.
    pub fn hook(&self) -> Option<ProgressHook> {
        let shared = self.shared.clone()?;
        Some(Arc::new(move |bytes, total| draw(&shared, bytes, total)))
    }

    /// The final draw of the last sample, without the throttle, so the
    /// line ends on the truth; the caller's note (the verification tick)
    /// appends to it.
    pub fn finish(&self) {
        let Some(shared) = &self.shared else {
            return;
        };
        let Ok(mut state) = shared.state.lock() else {
            return;
        };
        let Some((bytes, total)) = state.last else {
            return;
        };
        let speed = state.speed.feed(bytes, Instant::now());
        drop(state);
        write_line(shared, progress_line(&shared.label, bytes, total, speed));
    }
}

/// One sample: update the state and (throttled) redraw the line.
fn draw(shared: &Shared, bytes: u64, total: Option<u64>) {
    let Ok(mut state) = shared.state.lock() else {
        return;
    };
    state.last = Some((bytes, total));
    let speed = state.speed.feed(bytes, Instant::now());
    let now = Instant::now();
    if state
        .drawn
        .is_some_and(|then| now.saturating_duration_since(then) < REDRAW)
    {
        return;
    }
    state.drawn = Some(now);
    drop(state);
    write_line(shared, progress_line(&shared.label, bytes, total, speed));
}

/// Redraw: return to the line's start, clear it, write and flush.
fn write_line(shared: &Shared, line: String) {
    let Ok(mut out) = shared.out.lock() else {
        return;
    };
    let _ = write!(out, "\r\x1b[2K{line}");
    let _ = out.flush();
}

/// Writes through the shared in-memory sink (tests).
#[cfg(test)]
struct SinkWriter(Arc<Mutex<Vec<u8>>>);

#[cfg(test)]
impl Write for SinkWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .map(|mut sink| {
                sink.extend_from_slice(buf);
                buf.len()
            })
            .map_err(|_| std::io::Error::other("sink poisoned"))
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
#[path = "download_line_tests.rs"]
mod tests;
