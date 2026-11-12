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
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicUsize, Ordering};
use std::sync::mpsc;

/// The system allocator, counting the allocations that are currently alive, and able to
/// tell whether a few particular ones still are.
struct Counting;

static LIVE: AtomicIsize = AtomicIsize::new(0);

/// While set, every new allocation is added to [`WATCHED`].
static RECORD: AtomicBool = AtomicBool::new(false);
/// Whether [`WATCHED`] may hold anything, so that frees can skip it otherwise.
static WATCHING: AtomicBool = AtomicBool::new(false);
/// Addresses of the watched allocations that have not been freed yet (0: empty slot).
static WATCHED: [AtomicUsize; 64] = [const { AtomicUsize::new(0) }; 64];

impl Counting {
    fn born(ptr: *mut u8) {
        LIVE.fetch_add(1, Ordering::Relaxed);
        if RECORD.load(Ordering::Relaxed) && !ptr.is_null() {
            Self::replace(0, ptr as usize);
        }
    }

    /// Puts `new` into the first slot holding `old`.
    fn replace(old: usize, new: usize) {
        for slot in &WATCHED {
            if slot
                .compare_exchange(old, new, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
            {
                return;
            }
        }
    }

    /// Watch the allocations made from now on, until [`Counting::stop_recording`].
    fn start_recording() {
        for slot in &WATCHED {
            slot.store(0, Ordering::Relaxed);
        }
        WATCHING.store(true, Ordering::Relaxed);
        RECORD.store(true, Ordering::Relaxed);
    }

    fn stop_recording() {
        RECORD.store(false, Ordering::Relaxed);
    }

    /// How many of the watched allocations are still alive. Ends the watch.
    fn watched_alive() -> u64 {
        let alive = WATCHED
            .iter()
            .filter(|slot| slot.load(Ordering::Relaxed) != 0)
            .count();
        WATCHING.store(false, Ordering::Relaxed);
        alive as u64
    }
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc(layout) };
        Self::born(ptr);
        ptr
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        LIVE.fetch_sub(1, Ordering::Relaxed);
        if WATCHING.load(Ordering::Relaxed) {
            Self::replace(ptr as usize, 0);
        }
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc_zeroed(layout) };
        Self::born(ptr);
        ptr
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let new = unsafe { System.realloc(ptr, layout, new_size) };
        if WATCHING.load(Ordering::Relaxed) && !new.is_null() && new != ptr {
            Self::replace(ptr as usize, new as usize);
        }
        new
    }
}

#[global_allocator]
static ALLOC: Counting = Counting;

/// How many values the writer publishes past an idle reader.
const IDLE_WRITES: u64 = 100_000;

/// What is left after the writer has published [`IDLE_WRITES`] values past one reader.
struct Left {
    /// Allocations still alive that were not there before those writes.
    retained: u64,
    /// Allocations still alive out of those made to publish the value the reader read.
    read_value: u64,
}

/// What one idle reader did to a subject.
struct Idle {
    /// With a reader that read once and then kept its handle without using it.
    idle: Left,
    /// When that reader dropped its handle and its thread ended before the writes.
    alone: Left,
}

/// What an idle reader keeps from being collected.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Keeps {
    /// Nothing.
    Nothing,
    /// The value it read, but nothing published after it.
    ReadValue,
    /// The value it read and every value published since.
    Everything,
}

impl Keeps {
    fn id(self) -> &'static str {
        match self {
            Keeps::Nothing => "none",
            Keeps::ReadValue => "one",
            Keeps::Everything => "all",
        }
    }
}

impl Idle {
    fn keeps(&self) -> Keeps {
        // Most of what was written is still allocated, and would not be without the reader.
        // Subjects that free in batches keep a few hundred at most, far below this.
        if self.idle.retained >= IDLE_WRITES / 2 && self.alone.retained < IDLE_WRITES / 2 {
            Keeps::Everything
        } else if self.idle.read_value > self.alone.read_value {
            Keeps::ReadValue
        } else {
            Keeps::Nothing
        }
    }
}

/// The writer publishes a value, and one reader reads it on its own thread. With `stays` the
/// reader then does nothing but keep its handle; without, it drops the handle and its thread
/// ends. The writer then publishes [`IDLE_WRITES`] more values.
fn left_after_writes<S: Subject>(stays: bool) -> Left {
    let (mut w, mut rs) = S::new(Val::new(0), 1);
    let mut r = rs.pop().expect("one reader");
    drop(rs);
    // whatever the first write sets up is not part of the value
    S::write(&mut w, Val::new(1));
    // the value the reader is going to read: watch what is allocated to publish it
    Counting::start_recording();
    S::write(&mut w, Val::new(2));
    Counting::stop_recording();

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
        S::write(&mut w, Val::new(2 + i));
    }
    let after = LIVE.load(Ordering::Relaxed);
    let read_value = Counting::watched_alive();

    drop(finish);
    if let Some(reader) = reader {
        reader.join().unwrap();
    }
    Left {
        retained: (after - before).max(0) as u64,
        read_value,
    }
}

fn idle<S: Subject>() -> Idle {
    Idle {
        alone: left_after_writes::<S>(false),
        idle: left_after_writes::<S>(true),
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
            "The writer publishes a value, one reader reads it and then does nothing, and the \
             writer publishes {IDLE_WRITES} more. \"Left\" counts the allocations made by those \
             writes that are still alive, \"read value\" whether the allocations of the value \
             the reader read are; both also for a reader that is gone before the writes.\n"
        );
        println!(
            "| Subject | Left | Read value | Left, no reader | Read value, no reader | Idle reader keeps |"
        );
        println!("|---|---:|---:|---:|---:|---|");
        for (s, i) in rcu_bench::subjects().iter().zip(idle_all()) {
            let keeps = match i.keeps() {
                Keeps::Nothing => "nothing",
                Keeps::ReadValue => "**the value it read**",
                Keeps::Everything => "**everything since its read**",
            };
            println!(
                "| `{}` | {} | {} | {} | {} | {keeps} |",
                s.name, i.idle.retained, i.idle.read_value, i.alone.retained, i.alone.read_value
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
                    "retained": idle.idle.retained,
                    "retained_alone": idle.alone.retained,
                    "read_value": idle.idle.read_value,
                    "read_value_alone": idle.alone.read_value,
                    "keeps": idle.keeps().id(),
                    "blocks": idle.keeps() == Keeps::Everything,
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
