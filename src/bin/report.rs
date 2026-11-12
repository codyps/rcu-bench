//! Turns criterion's output (`target/criterion`) into markdown tables, or into the JSON
//! file the results page reads.
//!
//! Usage: `cargo run --release --bin report [--json] [criterion-dir]`
//!
//! It also measures, right here, what an idle reader does to each subject: see [`idle`].

use rcu_bench::{for_each_subject, Subject, Val};
use serde_json::{json, Value};
use std::alloc::{GlobalAlloc, Layout, System};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::mpsc;

/// The system allocator, counting the allocations that are currently alive.
struct Counting;

static LIVE: AtomicIsize = AtomicIsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        LIVE.fetch_add(1, Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        LIVE.fetch_sub(1, Ordering::Relaxed);
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        LIVE.fetch_add(1, Ordering::Relaxed);
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOC: Counting = Counting;

/// How many values the writer publishes past an idle reader.
const IDLE_WRITES: u64 = 100_000;

/// What one idle reader did to a subject.
struct Idle {
    /// Allocations still alive after [`IDLE_WRITES`] writes that were not there before them,
    /// with a reader that read once and then kept its handle without using it.
    retained: u64,
    /// The same, when that reader dropped its handle and its thread ended before the writes.
    retained_alone: u64,
}

impl Idle {
    /// Whether the reader kept old values from being collected: most of what was written is
    /// still allocated, and would not be without the reader. Subjects that free in batches
    /// keep a few hundred at most, far below this.
    fn blocks(&self) -> bool {
        self.retained >= IDLE_WRITES / 2 && self.retained_alone < IDLE_WRITES / 2
    }
}

/// One reader reads once on its own thread. With `stays` it then does nothing but keep its
/// handle; without, it drops the handle and its thread ends. The writer then publishes
/// [`IDLE_WRITES`] values. Returns how many allocations that left behind.
fn retained_after_writes<S: Subject>(stays: bool) -> u64 {
    let (mut w, mut rs) = S::new(Val::new(0), 1);
    let mut r = rs.pop().expect("one reader");
    drop(rs);
    let (has_read, wait_read) = mpsc::channel();
    let (finish, wait_finish) = mpsc::channel::<()>();
    let reader = std::thread::spawn(move || {
        std::hint::black_box(S::read(&mut r));
        has_read.send(()).unwrap();
        if stays {
            // idle: holds the handle, never reads again
            let _ = wait_finish.recv();
        }
        drop(r);
    });
    wait_read.recv().unwrap();
    let reader = if stays {
        Some(reader)
    } else {
        reader.join().unwrap();
        None
    };

    let before = LIVE.load(Ordering::Relaxed);
    for i in 1..=IDLE_WRITES {
        S::write(&mut w, Val::new(i));
    }
    let after = LIVE.load(Ordering::Relaxed);

    drop(finish);
    if let Some(reader) = reader {
        reader.join().unwrap();
    }
    (after - before).max(0) as u64
}

fn idle<S: Subject>() -> Idle {
    Idle {
        retained_alone: retained_after_writes::<S>(false),
        retained: retained_after_writes::<S>(true),
    }
}

/// [`idle`] for every subject, in the order of [`rcu_bench::subjects`].
fn idle_all() -> Vec<Idle> {
    let mut all = Vec::new();
    macro_rules! one {
        ($t:ty) => {
            all.push(idle::<$t>());
        };
    }
    for_each_subject!(one);
    all
}

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
    if found {
        println!("### Idle readers\n");
        println!(
            "One reader reads once and then does nothing, while the writer publishes \
             {IDLE_WRITES} values. Allocations still alive afterwards, and for comparison \
             when the reader is gone before the writes.\n"
        );
        println!(
            "| Subject | Still allocated | Without the reader | Idle reader blocks collection |"
        );
        println!("|---|---:|---:|---|");
        for (s, i) in rcu_bench::subjects().iter().zip(idle_all()) {
            let blocks = if i.blocks() { "**yes**" } else { "no" };
            println!(
                "| `{}` | {} | {} | {blocks} |",
                s.name, i.retained, i.retained_alone
            );
        }
        println!();
    }
    found
}

fn json(dir: &Path) -> bool {
    let mut found = false;
    let subjects: Vec<Value> = rcu_bench::subjects()
        .into_iter()
        .zip(idle_all())
        .map(|(s, idle)| {
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
            json!({
                "name": s.name,
                "kind": s.kind.id(),
                "desc": s.desc,
                "idle": {
                    "writes": IDLE_WRITES,
                    "retained": idle.retained,
                    "retained_alone": idle.retained_alone,
                    "blocks": idle.blocks(),
                },
                "results": results,
            })
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
