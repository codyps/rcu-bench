//! Turns criterion's output (`target/criterion`) into a markdown table, or into the JSON
//! file the results page reads.
//!
//! Usage: `cargo run --release --bin report [--json] [criterion-dir]`

use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Command;

/// (id, title, what is running, what is reported)
const GROUPS: &[(&str, &str, &str, &str)] = &[
    (
        "read-1",
        "Read, one reader",
        "One reader, nothing else running.",
        "read",
    ),
    (
        "read-N",
        "Read, N readers",
        "N readers at once, no writer.",
        "read",
    ),
    (
        "read-1-writer",
        "Read, one reader + writer",
        "One reader, while a writer publishes as fast as it can.",
        "read",
    ),
    (
        "read-N-writer",
        "Read, N readers + writer",
        "N readers, while a writer publishes as fast as it can.",
        "read",
    ),
    ("write-0", "Write, no readers", "The writer alone.", "write"),
    (
        "write-N",
        "Write, N readers",
        "The writer, while N readers read as fast as they can.",
        "write",
    ),
];

/// Median time per operation in nanoseconds, with its confidence interval.
fn estimate(dir: &Path, group: &str, subject: &str) -> Option<(f64, f64, f64)> {
    let path = dir.join(group).join(subject).join("new/estimates.json");
    let json: Value = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    let m = &json["median"];
    Some((
        m["point_estimate"].as_f64()?,
        m["confidence_interval"]["lower_bound"].as_f64()?,
        m["confidence_interval"]["upper_bound"].as_f64()?,
    ))
}

fn fmt_ns(ns: f64) -> String {
    if ns >= 1_000_000.0 {
        format!("{:.1} ms", ns / 1_000_000.0)
    } else if ns >= 10_000.0 {
        format!("{:.1} µs", ns / 1_000.0)
    } else if ns >= 100.0 {
        format!("{ns:.0} ns")
    } else {
        format!("{ns:.1} ns")
    }
}

fn command_output(cmd: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(cmd).args(args).output().ok()?;
    let s = String::from_utf8(out.stdout).ok()?;
    let s = s.trim();
    (!s.is_empty()).then(|| s.to_string())
}

fn cpu_model() -> Option<String> {
    let info = std::fs::read_to_string("/proc/cpuinfo").ok()?;
    let line = info.lines().find(|l| l.starts_with("model name"))?;
    Some(line.split_once(':')?.1.trim().to_string())
}

fn readers() -> u64 {
    std::env::var("RCU_BENCH_READERS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(4)
}

fn markdown(dir: &Path) -> bool {
    let ids: Vec<_> = GROUPS.iter().map(|g| g.0).collect();
    println!("| Subject | {} |", ids.join(" | "));
    println!("|---|{}", "---:|".repeat(ids.len()));
    let mut found = false;
    for subject in rcu_bench::subjects() {
        let cells: Vec<_> = ids.iter().map(|g| estimate(dir, g, subject.name)).collect();
        if cells.iter().all(Option::is_none) {
            continue;
        }
        found = true;
        let cells: Vec<_> = cells
            .into_iter()
            .map(|c| c.map(|c| fmt_ns(c.0)).unwrap_or_else(|| "—".into()))
            .collect();
        println!("| `{}` | {} |", subject.name, cells.join(" | "));
    }
    found
}

fn json(dir: &Path) -> bool {
    let mut found = false;
    let subjects: Vec<Value> = rcu_bench::subjects()
        .into_iter()
        .map(|s| {
            let mut results = serde_json::Map::new();
            for g in GROUPS {
                if let Some((median, lo, hi)) = estimate(dir, g.0, s.name) {
                    found = true;
                    results.insert(g.0.into(), json!({ "median": median, "lo": lo, "hi": hi }));
                }
            }
            json!({ "name": s.name, "kind": s.kind.id(), "desc": s.desc, "results": results })
        })
        .collect();
    let groups: Vec<Value> = GROUPS
        .iter()
        .map(|g| json!({ "id": g.0, "title": g.1, "desc": g.2, "op": g.3 }))
        .collect();
    let doc = json!({
        "meta": {
            "date": command_output("date", &["-u", "+%Y-%m-%d %H:%M UTC"]),
            "commit": std::env::var("GITHUB_SHA").ok(),
            "repository": std::env::var("GITHUB_REPOSITORY").ok(),
            "rustc": command_output("rustc", &["-V"]),
            "cpu": cpu_model(),
            "threads": std::thread::available_parallelism().ok().map(|n| n.get()),
            "os": command_output("uname", &["-sr"]),
            "readers": readers(),
        },
        "groups": groups,
        "subjects": subjects,
    });
    println!("{}", serde_json::to_string_pretty(&doc).unwrap());
    found
}

fn main() {
    let mut as_json = false;
    let mut dir = PathBuf::from("target/criterion");
    for arg in std::env::args_os().skip(1) {
        if arg == "--json" {
            as_json = true;
        } else {
            dir = PathBuf::from(arg);
        }
    }

    let found = if as_json { json(&dir) } else { markdown(&dir) };
    if !found {
        eprintln!("no results in {}; run `cargo bench` first", dir.display());
        std::process::exit(1);
    }
}
