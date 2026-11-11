//! Turns criterion's output (`target/criterion`) into a markdown table.
//!
//! Usage: `cargo run --release --bin report [criterion-dir]`

use std::path::{Path, PathBuf};

const GROUPS: &[&str] = &[
    "read-1",
    "read-N",
    "read-1-writer",
    "read-N-writer",
    "write-0",
    "write-N",
];

/// Median time per operation in nanoseconds.
fn median_ns(dir: &Path, group: &str, subject: &str) -> Option<f64> {
    let path = dir.join(group).join(subject).join("new/estimates.json");
    let json: serde_json::Value = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    json["median"]["point_estimate"].as_f64()
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

fn main() {
    let dir = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/criterion"));

    println!("| Subject | {} |", GROUPS.join(" | "));
    println!("|---|{}", "---:|".repeat(GROUPS.len()));
    let mut found = false;
    for subject in rcu_bench::subject_names() {
        let cells: Vec<_> = GROUPS.iter().map(|g| median_ns(&dir, g, subject)).collect();
        if cells.iter().all(Option::is_none) {
            continue;
        }
        found = true;
        let cells: Vec<_> = cells
            .into_iter()
            .map(|c| c.map(fmt_ns).unwrap_or_else(|| "—".into()))
            .collect();
        println!("| `{subject}` | {} |", cells.join(" | "));
    }
    if !found {
        eprintln!("no results in {}; run `cargo bench` first", dir.display());
        std::process::exit(1);
    }
}
