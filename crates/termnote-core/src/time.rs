//! Timestamp helpers.
//!
//! Every event is persisted with a nanosecond-resolution Unix timestamp
//! (`i64`). Nanosecond precision is captured internally per PRD §24; UIs and
//! exporters are responsible for rendering coarser, human-friendly units.

use chrono::{DateTime, Local, TimeZone, Utc};

/// Current wall-clock time as nanoseconds since the Unix epoch.
pub fn now_unix_ns() -> i64 {
    let dur = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    dur.as_nanos() as i64
}

/// Render a nanosecond Unix timestamp as a local `YYYY-MM-DD HH:MM:SS` string.
pub fn format_local(ts_ns: i64) -> String {
    format_local_with(ts_ns, "%Y-%m-%d %H:%M:%S")
}

/// Render just the local time-of-day, used for compact timeline views.
pub fn format_local_time(ts_ns: i64) -> String {
    format_local_with(ts_ns, "%H:%M:%S")
}

fn format_local_with(ts_ns: i64, fmt: &str) -> String {
    let secs = ts_ns.div_euclid(1_000_000_000);
    let nanos = ts_ns.rem_euclid(1_000_000_000) as u32;
    match Utc.timestamp_opt(secs, nanos) {
        chrono::LocalResult::Single(dt) => local_dt(dt).format(fmt).to_string(),
        _ => "unknown-time".to_string(),
    }
}

fn local_dt(dt: DateTime<Utc>) -> DateTime<Local> {
    dt.with_timezone(&Local)
}

/// Render a nanosecond duration as a short human string (`823 ms`, `4m 31s`).
pub fn format_duration_ns(ns: i64) -> String {
    if ns < 0 {
        return "?".to_string();
    }
    let ms = ns / 1_000_000;
    if ms < 1000 {
        format!("{ms} ms")
    } else if ms < 60_000 {
        format!("{:.2} s", ms as f64 / 1000.0)
    } else {
        let total_secs = ms / 1000;
        let h = total_secs / 3600;
        let m = (total_secs % 3600) / 60;
        let s = total_secs % 60;
        if h > 0 {
            format!("{h}h {m}m {s}s")
        } else {
            format!("{m}m {s}s")
        }
    }
}
