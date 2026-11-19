//! Read and write latency of every subject as the number of simultaneous readers grows.
//!
//! Groups, each measured for a range of reader counts `R`:
//!
//! - `read`:        R readers, no writer
//! - `read-writer`: R readers, while a writer publishes as fast as it can
//! - `read-paced`:  R readers, while a writer publishes at a fixed rate, the same for every
//!   subject (`RCU_BENCH_WRITE_HZ` writes per second, 50000 by default)
//! - `read-paced-fast`: the same at a higher rate (`RCU_BENCH_FAST_WRITE_HZ`, 1000000 by
//!   default)
//! - `write`:       the writer, while R readers read as fast as they can (R starts at 0)
//! - `write-idle`:  the writer, while R readers do nothing: each has read once, on a thread
//!   of its own that then sleeps, and keeps its handle
//!
//! Benchmark ids are `<group>/<subject>/<R>`.
//!
//! `RCU_BENCH_MAX_READERS` caps R. By default it is the number of CPUs (at most 8). The
//! groups with a writer stop one short of the cap, so that the writer has a CPU of its own.
//!
//! Idle readers need no CPU, so `write-idle` has a cap of its own: `RCU_BENCH_MAX_IDLE`, 64
//! by default.
//!
//! In `read-writer` a subject with cheap writes gets more of them, and its readers pay for
//! every one (a cache miss on the new value, at least), so its reads look slower than those of
//! a subject whose writer is slow. The paced groups give every subject the same number of
//! writes per second. The writer spins between writes instead of sleeping, to stay on its CPU
//! and on time. A writer that cannot keep up writes as fast as it can.
//!
//! The read groups with a writer record how many writes per second there were, next to
//! criterion's results (`writes.json`), for `report` to show.
//!
//! Read groups report the time per read as seen by one reader (averaged over the readers).
//! The write group reports the time per write.

use criterion::measurement::WallTime;
use criterion::{criterion_group, criterion_main, BenchmarkGroup, BenchmarkId, Criterion};
use rcu_bench::{for_each_subject, Subject, Val};
use std::hint::black_box;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Barrier};
use std::time::{Duration, Instant};

fn max_readers() -> usize {
    match std::env::var("RCU_BENCH_MAX_READERS") {
        Ok(v) => v.parse().expect("RCU_BENCH_MAX_READERS must be a number"),
        Err(_) => std::thread::available_parallelism().map_or(4, |n| n.get().min(8)),
    }
}

/// The reader counts to measure, up to and including `limit`.
fn reader_counts(limit: usize) -> Vec<usize> {
    [1, 2, 3, 4, 6, 8, 12, 16, 24, 32, 48, 64]
        .into_iter()
        .filter(|&r| r <= limit)
        .collect()
}

/// The numbers of idle readers to measure.
fn idle_counts() -> Vec<usize> {
    let max = match std::env::var("RCU_BENCH_MAX_IDLE") {
        Ok(v) => v.parse().expect("RCU_BENCH_MAX_IDLE must be a number"),
        Err(_) => 64,
    };
    [1, 4, 16, 64].into_iter().filter(|&r| r <= max).collect()
}

/// The time from one write to the next, for a rate read from the environment.
fn write_interval(var: &str, default_hz: f64) -> Duration {
    let hz: f64 = match std::env::var(var) {
        Ok(v) => v
            .parse()
            .unwrap_or_else(|_| panic!("{var} must be a number")),
        Err(_) => default_hz,
    };
    assert!(hz > 0.0, "{var} must be above 0");
    Duration::from_secs_f64(1.0 / hz)
}

/// What the writer does while the readers are measured.
#[derive(Clone, Copy)]
enum WriterLoad {
    /// There is no writer thread.
    None,
    /// Publish as fast as possible.
    Flat,
    /// Publish once per interval: the same rate for every subject.
    Paced(Duration),
}

/// The writes made during one benchmark, over all of its runs.
#[derive(Default)]
struct Writes {
    count: u64,
    took: Duration,
}

impl Writes {
    /// Writes `writes.json` into the directory criterion keeps this benchmark's results in:
    /// the writes per second there were, and the rate the writer was paced at, if it was.
    fn record(&self, group: &str, subject: &str, readers: usize, load: WriterLoad) {
        if self.took.is_zero() {
            // no writer, or the benchmark was filtered out
            return;
        }
        let hz = self.count as f64 / self.took.as_secs_f64();
        let target = match load {
            // the interval is in whole nanoseconds: undo what that did to a round rate
            WriterLoad::Paced(interval) => Some((1.0 / interval.as_secs_f64()).round()),
            _ => None,
        };
        if let Some(target) = target.filter(|t| hz < 0.9 * t) {
            eprintln!(
                "{group}/{subject}/{readers}: the writer cannot keep up: {hz:.0} writes per \
                 second instead of {target:.0}"
            );
        }
        let dir = match std::env::var_os("CRITERION_HOME") {
            Some(home) => PathBuf::from(home),
            None => std::env::var_os("CARGO_TARGET_DIR")
                .map_or("target".into(), PathBuf::from)
                .join("criterion"),
        };
        let path = dir
            .join(group)
            .join(subject)
            .join(readers.to_string())
            .join("writes.json");
        let doc = serde_json::json!({ "hz": hz, "target_hz": target });
        if let Err(e) = std::fs::write(&path, doc.to_string()) {
            eprintln!("cannot write {}: {e}", path.display());
        }
    }
}

/// Publishes once per `interval` until `stop`, spinning in between. The times to write at are
/// fixed from the start, so a writer that falls behind (it was descheduled, or one write took
/// long) writes without a pause until it has caught up: what is the same for every subject is
/// the number of writes the readers see. A writer that is always behind never pauses.
/// Returns the number of writes.
fn paced_writes<S: Subject>(w: &mut S::Writer, interval: Duration, stop: &AtomicBool) -> u64 {
    let mut writes = 0;
    let mut next = Instant::now();
    loop {
        next += interval;
        while Instant::now() < next && !stop.load(Ordering::Relaxed) {
            std::hint::spin_loop();
        }
        if stop.load(Ordering::Relaxed) {
            return writes;
        }
        writes += 1;
        S::write(w, Val::new(writes));
    }
}

/// `readers` threads each do `iters` reads. Returns the mean time one reader took, and adds
/// what the writer did meanwhile to `writes`.
fn run_read<S: Subject>(
    iters: u64,
    readers: usize,
    load: WriterLoad,
    writes: &mut Writes,
) -> Duration {
    let with_writer = !matches!(load, WriterLoad::None);
    let (mut w, rs) = S::new(Val::new(0), readers);
    let start = Arc::new(Barrier::new(readers + usize::from(with_writer)));
    let stop = Arc::new(AtomicBool::new(false));

    let reader_threads: Vec<_> = rs
        .into_iter()
        .map(|mut r| {
            let start = start.clone();
            std::thread::spawn(move || {
                start.wait();
                let t = Instant::now();
                for _ in 0..iters {
                    black_box(S::read(&mut r));
                }
                t.elapsed()
            })
        })
        .collect();

    // Without a writer thread the writer handle still has to outlive the readers: some
    // subjects stop handing out values once it is gone.
    let (idle_writer, writer_thread) = if with_writer {
        let start = start.clone();
        let stop = stop.clone();
        let t = std::thread::spawn(move || {
            start.wait();
            let t = Instant::now();
            let count = if let WriterLoad::Paced(interval) = load {
                paced_writes::<S>(&mut w, interval, &stop)
            } else {
                let mut i = 0;
                while !stop.load(Ordering::Relaxed) {
                    i += 1;
                    S::write(&mut w, Val::new(i));
                }
                i
            };
            (count, t.elapsed())
        });
        (None, Some(t))
    } else {
        (Some(w), None)
    };

    let total: Duration = reader_threads.into_iter().map(|t| t.join().unwrap()).sum();
    stop.store(true, Ordering::Relaxed);
    if let Some(t) = writer_thread {
        let (count, took) = t.join().unwrap();
        writes.count += count;
        writes.took += took;
    }
    drop(idle_writer);
    total / readers as u32
}

/// The writer does `iters` writes while `readers` threads keep reading. Returns the time the
/// writes took.
fn run_write<S: Subject>(iters: u64, readers: usize) -> Duration {
    let (mut w, rs) = S::new(Val::new(0), readers);
    let start = Arc::new(Barrier::new(readers + 1));
    let stop = Arc::new(AtomicBool::new(false));

    let reader_threads: Vec<_> = rs
        .into_iter()
        .map(|mut r| {
            let start = start.clone();
            let stop = stop.clone();
            std::thread::spawn(move || {
                start.wait();
                while !stop.load(Ordering::Relaxed) {
                    black_box(S::read(&mut r));
                }
            })
        })
        .collect();

    start.wait();
    let t = Instant::now();
    for i in 0..iters {
        S::write(&mut w, Val::new(i));
    }
    let elapsed = t.elapsed();

    stop.store(true, Ordering::Relaxed);
    for t in reader_threads {
        t.join().unwrap();
    }
    elapsed
}

/// The writer does `iters` writes while `idle` readers do nothing. Each of them has read once,
/// on a thread of its own, and that thread then sleeps without giving up its handle (or
/// whatever the subject keeps per thread). Returns the time the writes took.
fn run_write_idle<S: Subject>(iters: u64, idle: usize) -> Duration {
    let (mut w, rs) = S::new(Val::new(0), idle);
    let have_read = Arc::new(Barrier::new(idle + 1));
    let stop = Arc::new(AtomicBool::new(false));

    let reader_threads: Vec<_> = rs
        .into_iter()
        .map(|mut r| {
            let have_read = have_read.clone();
            let stop = stop.clone();
            std::thread::spawn(move || {
                black_box(S::read(&mut r));
                have_read.wait();
                while !stop.load(Ordering::Acquire) {
                    std::thread::park();
                }
                drop(r);
            })
        })
        .collect();

    have_read.wait();
    let t = Instant::now();
    for i in 0..iters {
        S::write(&mut w, Val::new(i));
    }
    let elapsed = t.elapsed();

    stop.store(true, Ordering::Release);
    for t in reader_threads {
        t.thread().unpark();
        t.join().unwrap();
    }
    elapsed
}

fn read_one<S: Subject>(
    g: &mut BenchmarkGroup<'_, WallTime>,
    group: &str,
    counts: &[usize],
    load: WriterLoad,
) {
    for &readers in counts {
        if readers > S::MAX_READERS {
            continue;
        }
        let mut writes = Writes::default();
        g.bench_function(BenchmarkId::new(S::NAME, readers), |b| {
            b.iter_custom(|iters| run_read::<S>(iters, readers, load, &mut writes))
        });
        writes.record(group, S::NAME, readers, load);
    }
}

fn write_one<S: Subject>(g: &mut BenchmarkGroup<'_, WallTime>, counts: &[usize], idle: bool) {
    for &readers in counts {
        if readers > S::MAX_READERS {
            continue;
        }
        g.bench_function(BenchmarkId::new(S::NAME, readers), |b| {
            if idle {
                b.iter_custom(|iters| run_write_idle::<S>(iters, readers))
            } else {
                b.iter_custom(|iters| run_write::<S>(iters, readers))
            }
        });
    }
}

fn read_group(c: &mut Criterion, name: &str, counts: &[usize], load: WriterLoad) {
    let mut g = c.benchmark_group(name);
    macro_rules! one {
        ($t:ty) => {
            read_one::<$t>(&mut g, name, counts, load);
        };
    }
    for_each_subject!(one);
    g.finish();
}

fn write_group(c: &mut Criterion, name: &str, counts: &[usize], idle: bool) {
    let mut g = c.benchmark_group(name);
    macro_rules! one {
        ($t:ty) => {
            write_one::<$t>(&mut g, counts, idle);
        };
    }
    for_each_subject!(one);
    g.finish();
}

fn benches(c: &mut Criterion) {
    let max = max_readers().max(2);
    let alone = reader_counts(max);
    let beside_writer = reader_counts(max - 1);
    let mut for_write = vec![0];
    for_write.extend(&beside_writer);

    read_group(c, "read", &alone, WriterLoad::None);
    read_group(c, "read-writer", &beside_writer, WriterLoad::Flat);
    let paced = WriterLoad::Paced(write_interval("RCU_BENCH_WRITE_HZ", 50_000.0));
    read_group(c, "read-paced", &beside_writer, paced);
    let fast = WriterLoad::Paced(write_interval("RCU_BENCH_FAST_WRITE_HZ", 1_000_000.0));
    read_group(c, "read-paced-fast", &beside_writer, fast);
    write_group(c, "write", &for_write, false);
    write_group(c, "write-idle", &idle_counts(), true);
}

criterion_group! {
    name = cells;
    config = Criterion::default()
        .sample_size(20)
        .warm_up_time(Duration::from_millis(400))
        .measurement_time(Duration::from_millis(1500));
    targets = benches
}
criterion_main!(cells);
