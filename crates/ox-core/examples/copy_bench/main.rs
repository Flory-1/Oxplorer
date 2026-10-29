//! Copy benchmark for Oxplorer's transfer engine.
//!
//! `cargo run --release -p ox-core --example copy_bench -- --help`
//!
//! Generates test folders (many small files, one large file, a mix), copies
//! them with the engine (and optionally robocopy, xcopy and `std::fs::copy`),
//! prints the median of several runs and can save the results as a baseline
//! or fail when the engine got slower than a saved baseline.

mod report;
mod scenario;
mod tools;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

use report::{Baseline, Measurement, Row, Verdict};
use scenario::{Kind, Scenario, Sizes};
use tools::{Runner, Tool};

const USAGE: &str = "\
Copy benchmark for Oxplorer's transfer engine

USAGE:
    cargo run --release -p ox-core --example copy_bench -- [OPTIONS]

OPTIONS:
    --src DIR               Where the test files are generated [default: temp folder]
    --dest DIR              Where they are copied to, e.g. another drive, a USB stick
                            or a network share [default: temp folder]
    --scenarios LIST        Any of small,large,mixed [default: small,large,mixed]
    --small-count N         Number of files in 'small' [default: 10000]
    --small-size SIZE       Size of each of them [default: 4K]
    --large-size SIZE       Size of the file in 'large' (mixed uses a quarter) [default: 1G]
    --runs N                Runs per tool and scenario; the median counts [default: 3]
    --compare LIST          Also measure robocopy, xcopy and/or std (sequential
                            std::fs::copy); robocopy and xcopy only on Windows
    --move                  Also measure a move with the engine (undone after each run)
    --workers N             Engine workers per job, 0 = automatic [default: 0]
    --baseline FILE         Compare with a saved baseline; exit code 1 if an Oxplorer
                            result is slower than allowed
    --max-slowdown FACTOR   Allowed time relative to the baseline [default: 1.15]
    --save-baseline FILE    Save the results as a new baseline (TOML)
    -h, --help              Show this help

SIZE takes K, M or G (1024-based), e.g. 4K, 200M, 1.5G.";

#[derive(Debug, Clone, PartialEq)]
struct Args {
    src: Option<PathBuf>,
    dest: Option<PathBuf>,
    scenarios: Vec<Kind>,
    sizes: Sizes,
    runs: usize,
    compare: Vec<Tool>,
    with_move: bool,
    workers: usize,
    baseline: Option<PathBuf>,
    max_slowdown: f64,
    save_baseline: Option<PathBuf>,
    help: bool,
}

impl Default for Args {
    fn default() -> Self {
        Self {
            src: None,
            dest: None,
            scenarios: Kind::ALL.to_vec(),
            sizes: Sizes::default(),
            runs: 3,
            compare: Vec::new(),
            with_move: false,
            workers: 0,
            baseline: None,
            max_slowdown: 1.15,
            save_baseline: None,
            help: false,
        }
    }
}

/// Parse `4096`, `4K`, `200M`, `1.5G` (1024-based).
fn parse_size(s: &str) -> Result<u64, String> {
    let t = s.trim().to_ascii_uppercase();
    let t = t.strip_suffix('B').unwrap_or(&t);
    let (num, mult) = match t.chars().last() {
        Some('K') => (&t[..t.len() - 1], 1u64 << 10),
        Some('M') => (&t[..t.len() - 1], 1 << 20),
        Some('G') => (&t[..t.len() - 1], 1 << 30),
        _ => (t, 1),
    };
    match num.trim().parse::<f64>() {
        Ok(n) if n >= 0.0 && n.is_finite() => Ok((n * mult as f64) as u64),
        _ => Err(format!("invalid size '{s}'")),
    }
}

fn parse_list<T>(s: &str, what: &str, parse: impl Fn(&str) -> Option<T>) -> Result<Vec<T>, String> {
    s.split(',')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(|p| parse(p).ok_or_else(|| format!("unknown {what} '{p}'")))
        .collect()
}

/// Options that take a value.
const VALUE_OPTIONS: [&str; 12] = [
    "--src",
    "--dest",
    "--scenarios",
    "--small-count",
    "--small-size",
    "--large-size",
    "--runs",
    "--compare",
    "--workers",
    "--baseline",
    "--save-baseline",
    "--max-slowdown",
];

fn parse_args(args: impl IntoIterator<Item = String>) -> Result<Args, String> {
    let mut a = Args::default();
    let mut it = args.into_iter();
    while let Some(arg) = it.next() {
        let (key, inline) = match arg.split_once('=') {
            Some((k, v)) if k.starts_with("--") => (k.to_string(), Some(v.to_string())),
            _ => (arg, None),
        };
        match key.as_str() {
            "-h" | "--help" => a.help = true,
            "--move" if inline.is_some() => return Err("--move takes no value".into()),
            "--move" => a.with_move = true,
            k if !VALUE_OPTIONS.contains(&k) => {
                return Err(format!("unknown option '{k}' (see --help)"))
            }
            _ => {
                let value = match inline {
                    Some(v) => v,
                    None => it.next().ok_or_else(|| format!("{key} needs a value"))?,
                };
                let num = |v: &str| {
                    v.parse::<usize>()
                        .map_err(|_| format!("invalid {key} '{v}'"))
                };
                match key.as_str() {
                    "--src" => a.src = Some(value.into()),
                    "--dest" => a.dest = Some(value.into()),
                    "--scenarios" => a.scenarios = parse_list(&value, "scenario", Kind::parse)?,
                    "--small-count" => a.sizes.small_count = num(&value)? as u64,
                    "--small-size" => a.sizes.small_size = parse_size(&value)?,
                    "--large-size" => a.sizes.large_size = parse_size(&value)?,
                    "--runs" => a.runs = num(&value)?.max(1),
                    "--compare" => a.compare = parse_list(&value, "tool", Tool::parse)?,
                    "--workers" => a.workers = num(&value)?,
                    "--baseline" => a.baseline = Some(value.into()),
                    "--save-baseline" => a.save_baseline = Some(value.into()),
                    "--max-slowdown" => {
                        a.max_slowdown = match value.parse::<f64>() {
                            Ok(f) if f >= 1.0 => f,
                            _ => {
                                return Err(format!("--max-slowdown must be >= 1.0, got '{value}'"))
                            }
                        }
                    }
                    _ => unreachable!("{key} is in VALUE_OPTIONS but not handled"),
                }
            }
        }
    }
    if !cfg!(windows) {
        if let Some(t) = a.compare.iter().find(|t| t.needs_windows()) {
            return Err(format!("{} is only available on Windows", t.name()));
        }
    }
    Ok(a)
}

fn temp_in(dir: Option<&Path>, prefix: &str) -> Result<tempfile::TempDir, String> {
    let dir = dir.map_or_else(std::env::temp_dir, Path::to_path_buf);
    fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    tempfile::Builder::new()
        .prefix(prefix)
        .tempdir_in(&dir)
        .map_err(|e| format!("{}: {e}", dir.display()))
}

struct Outcome {
    tool: Tool,
    m: Measurement,
    fastest: f64,
}

/// Run every tool `runs` times on `s` (tools take turns, so a slow phase of
/// the disk doesn't hit only one of them).
fn bench(
    s: &Scenario,
    tools: &[Tool],
    a: &Args,
    runner: &Runner,
    dest: &Path,
) -> Result<Vec<Outcome>, String> {
    let mut times = vec![Vec::new(); tools.len()];
    let target = dest.join(s.kind.name());
    for run in 1..=a.runs {
        for (i, &tool) in tools.iter().enumerate() {
            let took = runner
                .run(tool, &s.root, dest)
                .map_err(|e| format!("{} on '{}': {e}", tool.label(), s.kind.name()))?;
            // Check the result, so a tool that silently skipped files can't win.
            let check_root = if tool == Tool::OxplorerMove {
                &s.root
            } else {
                &target
            };
            let got = scenario::totals(check_root).map_err(|e| e.to_string())?;
            if got != (s.files, s.bytes) {
                return Err(format!(
                    "{} on '{}' produced {} files / {} bytes, expected {} / {}",
                    tool.label(),
                    s.kind.name(),
                    got.0,
                    got.1,
                    s.files,
                    s.bytes
                ));
            }
            if target.exists() {
                fs::remove_dir_all(&target).map_err(|e| format!("{}: {e}", target.display()))?;
            }
            eprintln!(
                "  {:<6} {:<15} run {run}/{}: {:.3} s",
                s.kind.name(),
                tool.label(),
                a.runs,
                took.as_secs_f64()
            );
            times[i].push(took.as_secs_f64());
        }
    }
    Ok(tools
        .iter()
        .zip(times)
        .map(|(&tool, t)| Outcome {
            tool,
            m: Measurement {
                scenario: s.kind.name().into(),
                tool: tool.name().into(),
                files: s.files,
                bytes: s.bytes,
                seconds: report::median(&t),
            },
            fastest: t.iter().copied().fold(f64::INFINITY, f64::min),
        })
        .collect())
}

/// `--dest` as given, to tell baselines for different drives apart.
fn dest_label(a: &Args) -> String {
    a.dest
        .as_deref()
        .map_or_else(|| "the temp folder".into(), |d| d.display().to_string())
}

fn run(a: &Args) -> Result<bool, String> {
    let baseline = a.baseline.as_deref().map(Baseline::load).transpose()?;
    let src = temp_in(a.src.as_deref(), "oxbench-src-")?;
    let dest = temp_in(a.dest.as_deref(), "oxbench-dst-")?;
    println!("Source:      {}", src.path().display());
    println!("Destination: {}", dest.path().display());

    let mut tools = vec![Tool::Oxplorer];
    if a.with_move {
        tools.push(Tool::OxplorerMove);
    }
    for &t in &a.compare {
        if !tools.contains(&t) {
            tools.push(t);
        }
    }
    let runner = Runner::new(a.workers);

    let mut results = Vec::new();
    for &kind in &a.scenarios {
        let start = Instant::now();
        let s = scenario::generate(kind, src.path(), &a.sizes)
            .map_err(|e| format!("generating '{}': {e}", kind.name()))?;
        println!(
            "Scenario {}: {} files, {} (generated in {:.1} s)",
            kind.name(),
            s.files,
            report::format_bytes(s.bytes),
            start.elapsed().as_secs_f64()
        );
        results.extend(bench(&s, &tools, a, &runner, dest.path())?);
        fs::remove_dir_all(&s.root).map_err(|e| format!("{}: {e}", s.root.display()))?;
    }

    let rows: Vec<Row> = results
        .iter()
        .map(|r| Row {
            label: r.tool.label(),
            m: &r.m,
            fastest: r.fastest,
            verdict: baseline.as_ref().map_or(Verdict::None, |b| {
                report::compare(&r.m, b, a.max_slowdown, r.tool.is_checked())
            }),
        })
        .collect();
    println!();
    print!("{}", report::table(&rows));
    println!();
    println!(
        "Median of {} run(s); 1 MB = 1024 x 1024 bytes. Repeated runs read the source from the \
         Windows file cache, so they are faster than a cold copy; use files larger than the \
         RAM (--large-size) for uncached numbers.",
        a.runs
    );

    let mut ok = true;
    if let (Some(b), Some(path)) = (&baseline, &a.baseline) {
        let dest_now = dest_label(a);
        if b.destination != dest_now {
            println!(
                "Note: the baseline was copied to {}, this run to {dest_now}.",
                b.destination
            );
        }
        let failed: Vec<&Row> = rows.iter().filter(|r| r.verdict.is_failure()).collect();
        if failed.is_empty() {
            println!(
                "Baseline {} ({}): no Oxplorer result is more than {:.0} % slower.",
                path.display(),
                b.created,
                (a.max_slowdown - 1.0) * 100.0
            );
        } else {
            ok = false;
            println!(
                "Baseline {} ({}): {} result(s) more than {:.0} % slower:",
                path.display(),
                b.created,
                failed.len(),
                (a.max_slowdown - 1.0) * 100.0
            );
            for r in failed {
                println!("  {} / {}", r.m.scenario, r.label);
            }
        }
    }

    if let Some(path) = &a.save_baseline {
        let b = Baseline {
            created: chrono::Local::now().format("%Y-%m-%d %H:%M").to_string(),
            destination: dest_label(a),
            runs: a.runs,
            results: results.into_iter().map(|r| r.m).collect(),
        };
        b.save(path)?;
        println!("Saved baseline to {}", path.display());
    }
    Ok(ok)
}

fn main() -> ExitCode {
    let a = match parse_args(std::env::args().skip(1)) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("error: {e}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    if a.help {
        println!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    match run(&a) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(s: &str) -> Result<Args, String> {
        parse_args(s.split_whitespace().map(String::from))
    }

    #[test]
    fn sizes() {
        assert_eq!(parse_size("4096"), Ok(4096));
        assert_eq!(parse_size("4K"), Ok(4096));
        assert_eq!(parse_size("200m"), Ok(200 << 20));
        assert_eq!(parse_size("1.5G"), Ok(3 << 29));
        assert_eq!(parse_size("2GB"), Ok(2 << 30));
        assert!(parse_size("lots").is_err());
        assert!(parse_size("-1K").is_err());
    }

    #[test]
    fn options() {
        assert_eq!(args("").unwrap(), Args::default());
        let a = args(
            "--dest D:\\bench --scenarios small,large --runs=5 --compare std --move \
             --small-count 2000 --large-size 200M --baseline b.toml --max-slowdown 1.2",
        )
        .unwrap();
        assert_eq!(a.dest, Some(PathBuf::from("D:\\bench")));
        assert_eq!(a.scenarios, vec![Kind::Small, Kind::Large]);
        assert_eq!(a.runs, 5);
        assert_eq!(a.compare, vec![Tool::Std]);
        assert!(a.with_move);
        assert_eq!(a.sizes.small_count, 2000);
        assert_eq!(a.sizes.large_size, 200 << 20);
        assert_eq!(a.baseline, Some(PathBuf::from("b.toml")));
        assert_eq!(a.max_slowdown, 1.2);
    }

    #[test]
    fn bad_options() {
        assert!(args("--runs").is_err());
        assert!(args("--scenarios small,huge").is_err());
        assert!(args("--compare teracopy").is_err());
        assert!(args("--max-slowdown 0.9").is_err());
        assert!(args("--move=yes").is_err());
        assert_eq!(
            args("--frobnicate").unwrap_err(),
            "unknown option '--frobnicate' (see --help)"
        );
    }
}
