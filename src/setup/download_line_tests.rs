//! Tests of the download progress line (bugfix 2026-11-07).

use std::time::{Duration, Instant};

use super::*;

#[test]
fn human_bytes_covers_the_scales() {
    assert_eq!(human_bytes(0), "0 B");
    assert_eq!(human_bytes(512), "512 B");
    assert_eq!(human_bytes(4096), "4.0 KB");
    assert_eq!(human_bytes(4 * 1024 * 1024), "4.0 MB");
    assert_eq!(human_bytes(3 * 1024 * 1024 * 1024), "3.0 GB");
}

#[test]
fn the_line_shows_percent_speed_and_time_left() {
    let speed = 1024.0 * 1024.0; // 1.0 MB/s
    assert_eq!(
        progress_line("v0.5.2", 4 * 1024 * 1024, Some(8 * 1024 * 1024), speed),
        "Downloading v0.5.2: 4.0/8.0 MB (50%) · 1.0 MB/s · 4s left"
    );
}

#[test]
fn the_line_shows_bytes_only_without_a_total_or_speed() {
    assert_eq!(
        progress_line("v0.5.2", 4096, None, 0.0),
        "Downloading v0.5.2: 4.0 KB"
    );
    assert_eq!(
        progress_line("v0.5.2", 4096, Some(0), 0.0),
        "Downloading v0.5.2: 4.0 KB"
    );
}

#[test]
fn the_speed_estimate_smooths_and_ignores_bursts() {
    let mut speed = Speedometer::new();
    let t0 = Instant::now();
    assert_eq!(speed.feed(0, t0), 0.0);
    // Too close to the first sample: no estimate yet.
    assert_eq!(speed.feed(1024, t0 + Duration::from_millis(10)), 0.0);
    // 200 ms later: the burst was 5 KB/s.
    assert!((speed.feed(1024 + 1024, t0 + Duration::from_millis(200)) - 5120.0).abs() < 1.0);
    // A single 10 MB/s burst cannot pull the estimate all the way up:
    // smoothed (0.3 weight) it lands far under the raw burst rate.
    let next = speed.feed(1024 * 1024, t0 + Duration::from_millis(300));
    let raw_burst = 1024.0 * 1024.0 * 10.0; // > the instantaneous rate here
    assert!(next < raw_burst, "smoothed, not spiked: {next}");
}

#[test]
fn the_hook_redraws_in_place_and_finish_ends_on_the_truth() {
    let (reporter, sink) = Reporter::memory("v0.5.2");
    let hook = reporter.hook().expect("memory reporter draws");
    hook(1024, Some(4096));
    let text = String::from_utf8(sink.lock().unwrap().clone()).unwrap();
    assert!(
        text.contains("\r\x1b[2KDownloading v0.5.2: 1.0/4.0 KB (25%)"),
        "{text:?}"
    );
    // A sample right after lands inside the redraw throttle, so the truth
    // comes from finish(), which draws without it.
    hook(2048, Some(4096));
    reporter.finish();
    let text = String::from_utf8(sink.lock().unwrap().clone()).unwrap();
    assert!(text.ends_with("(50%)"), "{text:?}");
    // The download's last sample, then the final draw.
    hook(4096, Some(4096));
    reporter.finish();
    let text = String::from_utf8(sink.lock().unwrap().clone()).unwrap();
    assert!(text.ends_with("(100%)"), "{text:?}");
    // finish without any sample draws nothing new.
    let (idle, sink) = Reporter::memory("v0.5.2");
    idle.finish();
    assert!(sink.lock().unwrap().is_empty());
}

#[test]
fn a_disabled_reporter_hands_out_no_hook() {
    // Pins the piped-output contract: nothing draws, nothing panics.
    let reporter = Reporter::enabled("v0.5.2", false);
    let hook = reporter.hook();
    if let Some(hook) = hook {
        hook(1, Some(2)); // must not panic
    }
    reporter.finish();
}
