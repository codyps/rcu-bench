//! Turns criterion's output (`target/criterion`) into markdown tables, or into the JSON
//! file the results page reads.
//!
//! Usage: `cargo run --release --bin report [--json] [criterion-dir]`

use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

/// (id, title, what is running, what is reported)
const GROUPS: &[(&str, &str, &str, &str)] = &[
    ("read", "Read", "R readers at once, no writer.", "read"),
    (
        "read-writer",
        "Read beside a writer",
        "R readers, while a writer publishes as fast as it can.",
        "read",
    ),
    (
        "write",
        "Write",
        "The writer, while R readers read as fast as they can.",
        "write",
    ),
];

/// Median time per operation in nanoseconds, with its confidence interval.
struct Estimate {
    median: f64,
    lo: f64,
    hi: f64,
}

fn estimate(path: &Path) -> Option<Estimate> {
    let json: Value = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    let m = &json["median"];
    Some(Estimate {
        median: m["point_estimate"].as_f64()?,
        lo: m["confidence_interval"]["lower_bound"].as_f64()?,
        hi: m["confidence_interval"]["upper_bound"].as_f64()?,
    })
}

/// All results of one subject in one group, by reader count.
fn estimates(dir: &Path, group: &str, subject: &str) -> BTreeMap<u32, Estimate> {
    let mut all = BTreeMap::new();
    let Ok(entries) = std::fs::read_dir(dir.join(group).join(subject)) else {
        return all;
    };
    for entry in entries.flatten() {
        let Some(readers) = entry.file_name().to_str().and_then(|n| n.parse().ok()) else {
            continue;
        };
        if let Some(e) = estimate(&entry.path().join("new/estimates.json")) {
            all.insert(readers, e);
        }
    }
    all
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
    if let Some(line) = info.lines().find(|l| l.starts_with("model name")) {
        return Some(line.split_once(':')?.1.trim().to_string());
    }
    // arm64 has no "model name"; lscpu knows how to decode the part number
    let lscpu = command_output("lscpu", &[])?;
    let line = lscpu.lines().find(|l| l.starts_with("Model name"))?;
    Some(line.split_once(':')?.1.trim().to_string())
}

fn markdown(dir: &Path) -> bool {
    let mut found = false;
    for g in GROUPS {
        let rows: Vec<_> = rcu_bench::subjects()
            .into_iter()
            .map(|s| (s.name, estimates(dir, g.0, s.name)))
            .filter(|(_, e)| !e.is_empty())
            .collect();
        let mut counts: Vec<u32> = rows.iter().flat_map(|(_, e)| e.keys().copied()).collect();
        counts.sort_unstable();
        counts.dedup();
        if counts.is_empty() {
            continue;
        }
        found = true;
        println!("### {}\n\n{} Time per {}.\n", g.1, g.2, g.3);
        let head: Vec<_> = counts.iter().map(|c| format!("R = {c}")).collect();
        println!("| Subject | {} |", head.join(" | "));
        println!("|---|{}", "---:|".repeat(counts.len()));
        for (name, e) in rows {
            let cells: Vec<_> = counts
                .iter()
                .map(|c| e.get(c).map_or("—".into(), |e| fmt_ns(e.median)))
                .collect();
            println!("| `{name}` | {} |", cells.join(" | "));
        }
        println!();
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
                let by_readers: serde_json::Map<_, _> = estimates(dir, g.0, s.name)
                    .into_iter()
                    .map(|(r, e)| {
                        (
                            r.to_string(),
                            json!({ "median": e.median, "lo": e.lo, "hi": e.hi }),
                        )
                    })
                    .collect();
                if !by_readers.is_empty() {
                    found = true;
                    results.insert(g.0.into(), by_readers.into());
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
            "label": std::env::var("BENCH_MACHINE").ok(),
            "arch": std::env::consts::ARCH,
            "date": command_output("date", &["-u", "+%Y-%m-%d %H:%M UTC"]),
            "commit": std::env::var("GITHUB_SHA").ok(),
            "repository": std::env::var("GITHUB_REPOSITORY").ok(),
            "rustc": command_output("rustc", &["-V"]),
            "cpu": cpu_model(),
            "threads": std::thread::available_parallelism().ok().map(|n| n.get()),
            "os": command_output("uname", &["-sr"]),
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
