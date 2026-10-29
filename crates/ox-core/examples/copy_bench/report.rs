//! Results table and the baseline file that a later run is checked against.

use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// Results whose baseline time is shorter than this are too noisy to check.
pub const MIN_CHECKED_SECONDS: f64 = 0.05;

const MB: f64 = 1024.0 * 1024.0;

/// The median result of one tool on one scenario.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Measurement {
    pub scenario: String,
    pub tool: String,
    pub files: u64,
    pub bytes: u64,
    /// Median wall-clock time of all runs.
    pub seconds: f64,
}

impl Measurement {
    pub fn mb_per_s(&self) -> f64 {
        self.bytes as f64 / MB / self.seconds.max(1e-9)
    }

    pub fn files_per_s(&self) -> f64 {
        self.files as f64 / self.seconds.max(1e-9)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Baseline {
    pub created: String,
    /// Where the copies went (informational; another drive gives other numbers).
    pub destination: String,
    pub runs: usize,
    #[serde(default)]
    pub results: Vec<Measurement>,
}

impl Baseline {
    pub fn load(path: &Path) -> Result<Self, String> {
        let text = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        let text = toml::to_string(self).map_err(|e| e.to_string())?;
        fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))
    }

    fn find(&self, m: &Measurement) -> Option<&Measurement> {
        self.results
            .iter()
            .find(|b| b.scenario == m.scenario && b.tool == m.tool)
    }
}

/// How a result compares to the baseline.
#[derive(Debug, Clone, PartialEq)]
pub enum Verdict {
    /// No baseline given, or the baseline has no such result.
    None,
    /// The scenario had other files, or it was too fast to measure reliably.
    NotCompared(&'static str),
    /// Time relative to the baseline (1.10 = 10 % slower).
    Within(f64),
    Slower(f64),
}

impl Verdict {
    pub fn is_failure(&self) -> bool {
        matches!(self, Verdict::Slower(_))
    }

    fn cell(&self) -> String {
        match self {
            Verdict::None => String::new(),
            Verdict::NotCompared(why) => format!("({why})"),
            Verdict::Within(r) => format!("{:+.1} %", (r - 1.0) * 100.0),
            Verdict::Slower(r) => format!("{:+.1} %  SLOWER", (r - 1.0) * 100.0),
        }
    }
}

/// Compare `m` with its baseline result. `checked` says whether a slowdown
/// beyond `max_slowdown` counts as a failure (or is only shown).
pub fn compare(m: &Measurement, baseline: &Baseline, max_slowdown: f64, checked: bool) -> Verdict {
    let Some(b) = baseline.find(m) else {
        return Verdict::None;
    };
    if b.files != m.files || b.bytes != m.bytes {
        return Verdict::NotCompared("other sizes");
    }
    if b.seconds < MIN_CHECKED_SECONDS {
        return Verdict::NotCompared("too short");
    }
    let ratio = m.seconds / b.seconds;
    if checked && ratio > max_slowdown {
        Verdict::Slower(ratio)
    } else {
        Verdict::Within(ratio)
    }
}

/// Median of the run times (mean of the middle two for an even count).
pub fn median(times: &[f64]) -> f64 {
    let mut v = times.to_vec();
    v.sort_by(f64::total_cmp);
    match v.len() {
        0 => 0.0,
        n if n % 2 == 1 => v[n / 2],
        n => (v[n / 2 - 1] + v[n / 2]) / 2.0,
    }
}

pub fn format_bytes(bytes: u64) -> String {
    let b = bytes as f64;
    if b >= 1024.0 * MB {
        format!("{:.2} GB", b / (1024.0 * MB))
    } else if b >= MB {
        format!("{:.1} MB", b / MB)
    } else {
        format!("{:.1} KB", b / 1024.0)
    }
}

/// One table row: the label of the tool, its median and fastest run, and the verdict.
pub struct Row<'a> {
    pub label: &'a str,
    pub m: &'a Measurement,
    pub fastest: f64,
    pub verdict: Verdict,
}

pub fn table(rows: &[Row]) -> String {
    let header = [
        "Scenario", "Tool", "Files", "Size", "Median", "Fastest", "MB/s", "Files/s", "vs base",
    ];
    let cells: Vec<[String; 9]> = rows
        .iter()
        .map(|r| {
            [
                r.m.scenario.clone(),
                r.label.to_string(),
                r.m.files.to_string(),
                format_bytes(r.m.bytes),
                format!("{:.3} s", r.m.seconds),
                format!("{:.3} s", r.fastest),
                format!("{:.1}", r.m.mb_per_s()),
                format!("{:.0}", r.m.files_per_s()),
                r.verdict.cell(),
            ]
        })
        .collect();
    let mut widths = header.map(str::len);
    for row in &cells {
        for (w, c) in widths.iter_mut().zip(row) {
            *w = (*w).max(c.len());
        }
    }
    let mut out = String::new();
    let mut line = |cols: &[&str]| {
        let mut l = String::new();
        for (i, (c, w)) in cols.iter().zip(widths).enumerate() {
            // Text columns left-aligned, numbers right-aligned.
            if i < 2 {
                let _ = write!(l, "{c:<w$}  ");
            } else {
                let _ = write!(l, "{c:>w$}  ");
            }
        }
        out.push_str(l.trim_end());
        out.push('\n');
    };
    line(&header);
    let rule: Vec<String> = widths.iter().map(|w| "-".repeat(*w)).collect();
    line(&rule.iter().map(String::as_str).collect::<Vec<_>>());
    for row in &cells {
        line(&row.iter().map(String::as_str).collect::<Vec<_>>());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(scenario: &str, tool: &str, seconds: f64) -> Measurement {
        Measurement {
            scenario: scenario.into(),
            tool: tool.into(),
            files: 10,
            bytes: 1000,
            seconds,
        }
    }

    fn baseline() -> Baseline {
        Baseline {
            created: "2026-01-01 12:00".into(),
            destination: "D:\\bench".into(),
            runs: 3,
            results: vec![m("small", "oxplorer", 1.0), m("large", "oxplorer", 0.01)],
        }
    }

    #[test]
    fn baseline_round_trips_through_toml() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("base.toml");
        baseline().save(&file).unwrap();
        assert_eq!(Baseline::load(&file).unwrap(), baseline());
    }

    #[test]
    fn compare_against_baseline() {
        let b = baseline();
        assert_eq!(
            compare(&m("small", "oxplorer", 1.1), &b, 1.15, true),
            Verdict::Within(1.1)
        );
        assert!(compare(&m("small", "oxplorer", 1.2), &b, 1.15, true).is_failure());
        // Comparison tools are only shown, never fail.
        assert!(!compare(&m("small", "oxplorer", 1.2), &b, 1.15, false).is_failure());
        assert_eq!(
            compare(&m("mixed", "oxplorer", 9.0), &b, 1.15, true),
            Verdict::None
        );
        assert_eq!(
            compare(&m("large", "oxplorer", 9.0), &b, 1.15, true),
            Verdict::NotCompared("too short")
        );
        let mut other = m("small", "oxplorer", 9.0);
        other.files = 11;
        assert_eq!(
            compare(&other, &b, 1.15, true),
            Verdict::NotCompared("other sizes")
        );
    }

    #[test]
    fn median_of_runs() {
        assert_eq!(median(&[3.0, 1.0, 2.0]), 2.0);
        assert_eq!(median(&[4.0, 1.0, 2.0, 3.0]), 2.5);
        assert_eq!(median(&[]), 0.0);
    }

    #[test]
    fn table_lines_up() {
        let a = m("small", "oxplorer", 1.0);
        let t = table(&[Row {
            label: "Oxplorer copy",
            m: &a,
            fastest: 0.9,
            verdict: Verdict::Slower(1.2),
        }]);
        let lines: Vec<&str> = t.lines().collect();
        assert_eq!(lines.len(), 3);
        assert!(lines[2].contains("+20.0 %  SLOWER"));
        assert!(lines[2].starts_with("small     Oxplorer copy"));
    }
}
