//! Human readable formatting helpers.

use std::time::{Duration, SystemTime};

const UNITS: [&str; 6] = ["B", "KB", "MB", "GB", "TB", "PB"];

/// Format a byte count like Explorer does (`1.4 MB`; `1,4 MB` in German).
pub fn size(bytes: u64) -> String {
    size_in(bytes, crate::i18n::is_german())
}

fn size_in(bytes: u64, decimal_comma: bool) -> String {
    let text = size_dot(bytes);
    if decimal_comma {
        text.replace('.', ",")
    } else {
        text
    }
}

fn size_dot(bytes: u64) -> String {
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if value >= 100.0 {
        format!("{value:.0} {}", UNITS[unit])
    } else if value >= 10.0 {
        format!("{value:.1} {}", UNITS[unit])
    } else {
        format!("{value:.2} {}", UNITS[unit])
    }
}

/// `value` with `places` decimals, with a decimal comma in German (`1,25`).
pub fn decimal(value: f64, places: usize) -> String {
    let text = format!("{value:.places$}");
    if crate::i18n::is_german() {
        text.replace('.', ",")
    } else {
        text
    }
}

/// `1 file`, `3 files`. `noun` is the singular; an "s" is appended for the plural.
pub fn count(n: impl Into<u64>, noun: &str) -> String {
    let n = n.into();
    if n == 1 {
        format!("1 {noun}")
    } else {
        format!("{n} {noun}s")
    }
}

/// Format a transfer rate in bytes per second.
pub fn rate(bytes_per_sec: f64) -> String {
    format!("{}/s", size(bytes_per_sec.max(0.0) as u64))
}

/// Format a duration as `1h 02m`, `3m 05s` or `12s`.
pub fn duration(d: Duration) -> String {
    let secs = d.as_secs();
    if secs >= 3600 {
        format!("{}h {:02}m", secs / 3600, (secs % 3600) / 60)
    } else if secs >= 60 {
        format!("{}m {:02}s", secs / 60, secs % 60)
    } else {
        format!("{secs}s")
    }
}

/// A playing position: "1:05" or "1:02:03".
pub fn clock(secs: f32) -> String {
    let s = secs.max(0.0) as u64;
    let (h, m, s) = (s / 3600, s / 60 % 60, s % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// Format a timestamp in local time using a `chrono` format string.
pub fn time(t: SystemTime, fmt: &str) -> String {
    let dt: chrono::DateTime<chrono::Local> = t.into();
    dt.format(fmt).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clocks() {
        assert_eq!(clock(65.4), "1:05");
        assert_eq!(clock(3723.0), "1:02:03");
        assert_eq!(clock(-1.0), "0:00");
    }

    #[test]
    fn sizes() {
        assert_eq!(size(0), "0 B");
        assert_eq!(size(1023), "1023 B");
        assert_eq!(size(1024), "1.00 KB");
        assert_eq!(size(1536), "1.50 KB");
        assert_eq!(size_in(1536, true), "1,50 KB");
        assert_eq!(size(15 * 1024 * 1024), "15.0 MB");
        assert_eq!(size(500 * 1024 * 1024 * 1024), "500 GB");
    }

    #[test]
    fn counts() {
        assert_eq!(count(1u64, "file"), "1 file");
        assert_eq!(count(0u64, "file"), "0 files");
        assert_eq!(count(12u64, "item"), "12 items");
    }

    #[test]
    fn durations() {
        assert_eq!(duration(Duration::from_secs(5)), "5s");
        assert_eq!(duration(Duration::from_secs(125)), "2m 05s");
        assert_eq!(duration(Duration::from_secs(3720)), "1h 02m");
    }
}
