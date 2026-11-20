//! Read and write latency of every subject as the number of simultaneous readers grows.
//!
//! Groups, each measured for a range of reader counts `R`:
//!
//! - `read`:        R readers, no writer
//! - `read-writer`: R readers, while a writer publishes as fast as it can
//! - `read-paced`:  R readers, while a writer publishes at a fixed rate, the same for every
//!   subject (`RCU_BENCH_WRITE_HZ` writes per second, 1000000 by default)
//! - `write`:       the writer, while R readers read as fast as they can (R starts at 0)
//! - `write-held`:  the writer, while R readers read and keep their access to the value
//!   (guard, lock, borrow, `Arc`, ...) for a fixed time before they give it up and read again
//!   (`RCU_BENCH_HOLD_US` microseconds, 10 by default)
//! - `write-idle`:  the writer, while R readers do nothing: each has read once, on a thread
//!   of its own that then sleeps, and keeps its handle
//!
//! Benchmark ids are `<group>/<subject>/<R>`.
//!
//! `RCU_BENCH_MAX_READERS` caps R. By default it is the number of CPUs (at most 8). The
//! groups with a writer stop one short of the cap, so that the writer has a CPU of its own.
//!
//! `read-writer` and `write` are measured once more with more readers than there are CPUs, so
//! that readers (and the writer) are taken off their CPU in the middle of what they do:
//! `RCU_BENCH_OVERSUBSCRIBED` readers, twice the number of CPUs by default, 0 for none.
//!
//! With more threads than CPUs a thread can get nowhere at all: a writer that has to find a
//! moment with no reader in its way may never find one. Before anything oversubscribed is
//! measured, the threads that would be timed are given 2 seconds for 1000 operations. If
//! they do not get there, nothing is measured and `stalled.json` says how far they got.
//!
//! Idle readers need no CPU, so `write-idle` has a cap of its own: `RCU_BENCH_MAX_IDLE`, 64
//! by default.
//!
//! In `read-writer` a subject with cheap writes gets more of them, and its readers pay for
//! every one (a cache miss on the new value, at least), so its reads look slower than those of
//! a subject whose writer is slow. The paced group gives every subject the same number of
//! writes per second. The writer spins between writes instead of sleeping, to stay on its CPU
//! and on time. A writer that cannot keep up writes as fast as it can.
//!
//! The read groups with a writer record how many writes per second there were, next to
//! criterion's results (`writes.json`), for `report` to show.
//!
//! Criterion times many operations at once and reports the time per operation, which says
//! nothing about the slow ones. So `read-writer` and `write` are run once more after
//! criterion is done, with every read (or write) timed on its own, for
//! `RCU_BENCH_TAIL_MS` milliseconds (500 by default, 0 for not at all). The 99th and 99.9th
//! percentile and the slowest one go to `tails.json`, next to criterion's results.
//!
//! Read groups report the time per read as seen by one reader (averaged over the readers).
//! The write group reports the time per write.

use criterion::measurement::WallTime;
use criterion::{criterion_group, BenchmarkGroup, BenchmarkId, Criterion};
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

/// A number of readers that is more than the CPUs can run at once, to measure beside the
/// others: `RCU_BENCH_OVERSUBSCRIBED`, twice the number of CPUs by default. `None` if it is
/// set to 0.
fn oversubscribed() -> Option<usize> {
    let n = match std::env::var("RCU_BENCH_OVERSUBSCRIBED") {
        Ok(v) => v
            .parse()
            .expect("RCU_BENCH_OVERSUBSCRIBED must be a number"),
        Err(_) => 2 * std::thread::available_parallelism().map_or(4, |n| n.get()),
    };
    (n > 0).then_some(n)
}

/// `counts` and the oversubscribed number of readers.
fn with_oversubscribed(counts: &[usize]) -> Vec<usize> {
    let mut all = counts.to_vec();
    all.extend(oversubscribed().filter(|n| !counts.contains(n)));
    all
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
        let doc = serde_json::json!({ "hz": hz, "target_hz": target });
        write_beside_criterion(group, subject, readers, "writes.json", &doc);
    }
}

/// The directory criterion keeps the results of one benchmark in.
fn criterion_dir(group: &str, subject: &str, readers: usize) -> PathBuf {
    let dir = match std::env::var_os("CRITERION_HOME") {
        Some(home) => PathBuf::from(home),
        None => std::env::var_os("CARGO_TARGET_DIR")
            .map_or("target".into(), PathBuf::from)
            .join("criterion"),
    };
    dir.join(group).join(subject).join(readers.to_string())
}

/// Writes `doc` to `file` in the directory criterion keeps the results of one benchmark in.
fn write_beside_criterion(
    group: &str,
    subject: &str,
    readers: usize,
    file: &str,
    doc: &serde_json::Value,
) {
    let dir = criterion_dir(group, subject, readers);
    let written = std::fs::create_dir_all(&dir)
        .and_then(|()| std::fs::write(dir.join(file), doc.to_string()));
    if let Err(e) = written {
        eprintln!("cannot write {}: {e}", dir.join(file).display());
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

/// Which threads a benchmark times.
#[derive(Clone, Copy)]
enum Timed {
    Readers,
    Writer,
}

/// The operations every timed thread has to get done in [`PROGRESS_TIME`] for a benchmark
/// with more threads than CPUs to be worth starting.
const PROGRESS_OPS: u64 = 1000;
const PROGRESS_TIME: Duration = Duration::from_secs(2);

/// Whether `readers` readers and a writer, all going as fast as they can, are more threads
/// than there are CPUs.
fn is_oversubscribed(readers: usize) -> bool {
    readers + 1 > std::thread::available_parallelism().map_or(usize::MAX, |n| n.get())
}

/// Lets `readers` readers and a writer run as fast as they can, and waits for each of the
/// `timed` threads to get [`PROGRESS_OPS`] operations done. Returns how far the slowest of them
/// got if that takes longer than [`PROGRESS_TIME`].
///
/// All threads are told to stop before any of them is waited for: one that the others kept
/// from getting anywhere can only finish what it is in the middle of once they have stopped.
fn progress<S: Subject>(readers: usize, timed: Timed) -> Result<(), u64> {
    use std::sync::atomic::AtomicU64;

    let (mut w, rs) = S::new(Val::new(0), readers);
    let stop_readers = Arc::new(AtomicBool::new(false));
    let stop_writer = Arc::new(AtomicBool::new(false));
    let mut done = Vec::new();
    let mut counter = |counted: bool| {
        let n = Arc::new(AtomicU64::new(0));
        if counted {
            done.push(n.clone());
        }
        n
    };

    let reader_threads: Vec<_> = rs
        .into_iter()
        .map(|mut r| {
            let stop = stop_readers.clone();
            let n = counter(matches!(timed, Timed::Readers));
            std::thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    black_box(S::read(&mut r));
                    n.fetch_add(1, Ordering::Relaxed);
                }
            })
        })
        .collect();
    let writer_thread = {
        let stop = stop_writer.clone();
        let n = counter(matches!(timed, Timed::Writer));
        std::thread::spawn(move || {
            let mut i = 0;
            while !stop.load(Ordering::Relaxed) {
                i += 1;
                S::write(&mut w, Val::new(i));
                n.fetch_add(1, Ordering::Relaxed);
            }
        })
    };

    let slowest = || {
        done.iter()
            .map(|n| n.load(Ordering::Relaxed))
            .min()
            .unwrap_or(PROGRESS_OPS)
    };
    let start = Instant::now();
    while slowest() < PROGRESS_OPS && start.elapsed() < PROGRESS_TIME {
        std::thread::sleep(Duration::from_millis(1));
    }
    let got = slowest();

    stop_readers.store(true, Ordering::Relaxed);
    stop_writer.store(true, Ordering::Relaxed);
    for t in reader_threads {
        t.join().unwrap();
    }
    writer_thread.join().unwrap();
    if got < PROGRESS_OPS {
        Err(got)
    } else {
        Ok(())
    }
}

/// Whether the benchmark `group/S/readers` can be measured. With more threads than CPUs that
/// is only so if the timed threads get somewhere: see [`progress`]. If they do not, this says
/// so, and leaves `stalled.json` where the results would be.
fn measurable<S: Subject>(group: &str, readers: usize, timed: Timed) -> bool {
    let stalled = criterion_dir(group, S::NAME, readers).join("stalled.json");
    // whatever an earlier run found
    let _ = std::fs::remove_file(&stalled);
    if !is_oversubscribed(readers) {
        return true;
    }
    let Err(got) = progress::<S>(readers, timed) else {
        return true;
    };
    let secs = PROGRESS_TIME.as_secs_f64();
    eprintln!(
        "{group}/{}/{readers}: stalled, not measured: {got} of {PROGRESS_OPS} operations in \
         {secs} s",
        S::NAME
    );
    let doc = serde_json::json!({ "done": got, "wanted": PROGRESS_OPS, "secs": secs });
    write_beside_criterion(group, S::NAME, readers, "stalled.json", &doc);
    false
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

/// The writer does `iters` writes while `readers` threads keep reading, each staying in its
/// read for `hold`: it spins with its access to the value kept, gives the access up, and reads
/// again at once. Returns the time the writes took.
fn run_write_held<S: Subject>(iters: u64, readers: usize, hold: Duration) -> Duration {
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
                    black_box(S::read_with(&mut r, |v| {
                        let v = v.get();
                        let until = Instant::now() + hold;
                        while Instant::now() < until {
                            std::hint::spin_loop();
                        }
                        v
                    }));
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
        if !matches!(load, WriterLoad::None)
            && wanted(group, S::NAME, readers)
            && !measurable::<S>(group, readers, Timed::Readers)
        {
            continue;
        }
        let mut writes = Writes::default();
        g.bench_function(BenchmarkId::new(S::NAME, readers), |b| {
            b.iter_custom(|iters| run_read::<S>(iters, readers, load, &mut writes))
        });
        writes.record(group, S::NAME, readers, load);
    }
}

/// What the readers do while the writer is measured.
#[derive(Clone, Copy)]
enum ReaderLoad {
    /// Read as fast as possible.
    Flat,
    /// Read once and sleep.
    Idle,
    /// Read, and stay in every read for this long.
    Held(Duration),
}

fn write_one<S: Subject>(
    g: &mut BenchmarkGroup<'_, WallTime>,
    group: &str,
    counts: &[usize],
    load: ReaderLoad,
) {
    for &readers in counts {
        if readers > S::MAX_READERS {
            continue;
        }
        if matches!(load, ReaderLoad::Flat)
            && wanted(group, S::NAME, readers)
            && !measurable::<S>(group, readers, Timed::Writer)
        {
            continue;
        }
        g.bench_function(BenchmarkId::new(S::NAME, readers), |b| {
            b.iter_custom(|iters| match load {
                ReaderLoad::Flat => run_write::<S>(iters, readers),
                ReaderLoad::Idle => run_write_idle::<S>(iters, readers),
                ReaderLoad::Held(hold) => run_write_held::<S>(iters, readers, hold),
            })
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

fn write_group(c: &mut Criterion, name: &str, counts: &[usize], load: ReaderLoad) {
    let mut g = c.benchmark_group(name);
    macro_rules! one {
        ($t:ty) => {
            write_one::<$t>(&mut g, name, counts, load);
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
    let flat = with_oversubscribed(&beside_writer);
    read_group(c, "read-writer", &flat, WriterLoad::Flat);
    let paced = WriterLoad::Paced(write_interval("RCU_BENCH_WRITE_HZ", 1_000_000.0));
    read_group(c, "read-paced", &beside_writer, paced);
    write_group(
        c,
        "write",
        &with_oversubscribed(&for_write),
        ReaderLoad::Flat,
    );
    write_group(c, "write-idle", &idle_counts(), ReaderLoad::Idle);
    let held = ReaderLoad::Held(rcu_bench::hold_time());
    write_group(c, "write-held", &beside_writer, held);
}

criterion_group! {
    name = cells;
    config = Criterion::default()
        .sample_size(20)
        .warm_up_time(Duration::from_millis(400))
        .measurement_time(Duration::from_millis(1500));
    targets = benches
}

/// How many single operations took how long. The times are counted in buckets, 16 for every
/// doubling, so a percentile is at most 1/16 above the real one.
struct Histogram {
    buckets: Vec<u64>,
    count: u64,
    max: u64,
}

impl Histogram {
    fn new() -> Self {
        Histogram {
            buckets: vec![0; 61 * 16],
            count: 0,
            max: 0,
        }
    }

    /// The bucket of a time in nanoseconds: one for each time below 16, then 16 for each
    /// power of two.
    fn bucket(ns: u64) -> usize {
        if ns < 16 {
            return ns as usize;
        }
        let exp = 63 - ns.leading_zeros();
        let sub = (ns >> (exp - 4)) & 15;
        ((exp - 3) * 16) as usize + sub as usize
    }

    /// The longest time that goes into `bucket`.
    fn upper(bucket: usize) -> u64 {
        if bucket < 16 {
            return bucket as u64;
        }
        let shift = (bucket / 16 + 3 - 4) as u32;
        let low = (16 + (bucket % 16) as u64) << shift;
        low + ((1 << shift) - 1)
    }

    #[inline]
    fn record(&mut self, ns: u64) {
        self.buckets[Self::bucket(ns)] += 1;
        self.count += 1;
        self.max = self.max.max(ns);
    }

    fn merge(&mut self, other: &Histogram) {
        for (a, b) in self.buckets.iter_mut().zip(&other.buckets) {
            *a += b;
        }
        self.count += other.count;
        self.max = self.max.max(other.max);
    }

    /// The shortest time that goes into `bucket`.
    fn lower(bucket: usize) -> u64 {
        if bucket < 16 {
            return bucket as u64;
        }
        (16 + (bucket % 16) as u64) << (bucket / 16 - 1)
    }

    /// The counts for the results page, as `[key, count, key, count, ...]` without the empty
    /// ones. These buckets are coarser, 8 for every doubling: `key / 8` is the power of two
    /// and `key % 8` the eighth above it, so a bucket starts at `2^(key / 8) * (1 + (key % 8) / 8)`
    /// nanoseconds and ends where the next one starts. Key 0 also has the times below 1 ns.
    fn coarse(&self) -> Vec<u64> {
        let mut out: Vec<u64> = Vec::new();
        for (i, &n) in self.buckets.iter().enumerate() {
            if n == 0 {
                continue;
            }
            let ns = Self::lower(i).max(1);
            let exp = 63 - ns.leading_zeros() as u64;
            // the eighth of the doubling that `ns` is in: (ns / 2^exp - 1) * 8, rounded down
            let key = exp * 8 + (((ns - (1 << exp)) * 8) >> exp);
            match out.len() {
                l if l >= 2 && out[l - 2] == key => out[l - 1] += n,
                _ => out.extend([key, n]),
            }
        }
        out
    }

    /// The time that the share `q` of the operations stayed at or below.
    fn quantile(&self, q: f64) -> u64 {
        let wanted = ((q * self.count as f64).ceil() as u64).clamp(1, self.count.max(1));
        let mut seen = 0;
        for (i, n) in self.buckets.iter().enumerate() {
            seen += n;
            if seen >= wanted {
                return Self::upper(i).min(self.max);
            }
        }
        self.max
    }
}

/// Does `op` again and again for `time`, and times every single one. The clock is read once
/// per operation: the end of one is the start of the next, so each time includes one reading
/// of the clock. The first tenth of the time is a warm-up: an operation that ends in it is not
/// counted. One that ends after it is, however early it began, so that an operation which
/// takes the whole time is still there as the slowest.
fn time_each(time: Duration, mut op: impl FnMut()) -> Histogram {
    let mut h = Histogram::new();
    let mut last = Instant::now();
    let counted_from = last + time / 10;
    let end = last + time;
    loop {
        op();
        let now = Instant::now();
        if now >= counted_from {
            h.record((now - last).as_nanos() as u64);
        }
        last = now;
        if now >= end {
            return h;
        }
    }
}

/// `readers` threads read for `time`, each read timed on its own, beside the writer that
/// `load` asks for. Returns the times of all the reads.
fn tail_read<S: Subject>(readers: usize, time: Duration, load: WriterLoad) -> Histogram {
    let (mut w, rs) = S::new(Val::new(0), readers);
    let start = Arc::new(Barrier::new(readers + 1));
    let stop = Arc::new(AtomicBool::new(false));

    let reader_threads: Vec<_> = rs
        .into_iter()
        .map(|mut r| {
            let start = start.clone();
            std::thread::spawn(move || {
                start.wait();
                time_each(time, || {
                    black_box(S::read(&mut r));
                })
            })
        })
        .collect();
    let writer_thread = {
        let start = start.clone();
        let stop = stop.clone();
        std::thread::spawn(move || {
            start.wait();
            match load {
                // keep the writer's half of the cell until the readers are done
                WriterLoad::None => {
                    while !stop.load(Ordering::Relaxed) {
                        std::thread::park();
                    }
                }
                WriterLoad::Flat => {
                    let mut i = 0;
                    while !stop.load(Ordering::Relaxed) {
                        i += 1;
                        S::write(&mut w, Val::new(i));
                    }
                }
                WriterLoad::Paced(interval) => {
                    paced_writes::<S>(&mut w, interval, &stop);
                }
            }
            drop(w);
        })
    };

    let mut all = Histogram::new();
    for t in reader_threads {
        all.merge(&t.join().unwrap());
    }
    stop.store(true, Ordering::Relaxed);
    writer_thread.thread().unpark();
    writer_thread.join().unwrap();
    all
}

/// The writer writes for `time`, each write timed on its own, beside `readers` threads that
/// do what `load` asks for. Returns the times of the writes.
fn tail_write<S: Subject>(readers: usize, time: Duration, load: ReaderLoad) -> Histogram {
    let (mut w, rs) = S::new(Val::new(0), readers);
    let start = Arc::new(Barrier::new(readers + 1));
    let stop = Arc::new(AtomicBool::new(false));

    let reader_threads: Vec<_> = rs
        .into_iter()
        .map(|mut r| {
            let start = start.clone();
            let stop = stop.clone();
            std::thread::spawn(move || match load {
                ReaderLoad::Flat => {
                    start.wait();
                    while !stop.load(Ordering::Relaxed) {
                        black_box(S::read(&mut r));
                    }
                }
                // read once, then sleep without giving up the handle
                ReaderLoad::Idle => {
                    black_box(S::read(&mut r));
                    start.wait();
                    while !stop.load(Ordering::Relaxed) {
                        std::thread::park();
                    }
                    drop(r);
                }
                ReaderLoad::Held(hold) => {
                    start.wait();
                    while !stop.load(Ordering::Relaxed) {
                        black_box(S::read_with(&mut r, |v| {
                            let v = v.get();
                            let until = Instant::now() + hold;
                            while Instant::now() < until {
                                std::hint::spin_loop();
                            }
                            v
                        }));
                    }
                }
            })
        })
        .collect();

    start.wait();
    let mut i = 0;
    let h = time_each(time, || {
        i += 1;
        S::write(&mut w, Val::new(i));
    });

    stop.store(true, Ordering::Relaxed);
    for t in reader_threads {
        t.thread().unpark();
        t.join().unwrap();
    }
    h
}

/// Whether the filter on the command line lets the benchmark `group/subject/readers` run.
fn wanted(group: &str, subject: &str, readers: usize) -> bool {
    static FILTER: std::sync::OnceLock<Option<regex::Regex>> = std::sync::OnceLock::new();
    let id = format!("{group}/{subject}/{readers}");
    FILTER
        .get_or_init(filter)
        .as_ref()
        .is_none_or(|f| f.is_match(&id))
}

/// The benchmark filter from the command line, as criterion understands it: the first argument
/// that is neither an option nor the value of one.
fn filter() -> Option<regex::Regex> {
    const WITH_VALUE: &[&str] = &[
        "--baseline",
        "--baseline-lenient",
        "--color",
        "--confidence-level",
        "--format",
        "--load-baseline",
        "--logfile",
        "--measurement-time",
        "--noise-threshold",
        "--nresamples",
        "--output-format",
        "--plotting-backend",
        "--profile-time",
        "--sample-size",
        "--save-baseline",
        "--significance-level",
        "--warm-up-time",
    ];
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if WITH_VALUE.contains(&arg.as_str()) {
            args.next();
        } else if !arg.starts_with('-') {
            return Some(regex::Regex::new(&arg).expect("the filter must be a regular expression"));
        }
    }
    None
}

fn tail_one<S: Subject>(
    group: &str,
    counts: &[usize],
    time: Duration,
    clock: u64,
    run: impl Fn(usize, Duration) -> Histogram,
) {
    for &readers in counts {
        let id = format!("{group}/{}/{readers}", S::NAME);
        if readers > S::MAX_READERS || !wanted(group, S::NAME, readers) {
            continue;
        }
        // what criterion's run of this benchmark found
        if criterion_dir(group, S::NAME, readers)
            .join("stalled.json")
            .exists()
        {
            continue;
        }
        let h = run(readers, time);
        if h.count == 0 {
            continue;
        }
        let (p50, p99, p999) = (h.quantile(0.5), h.quantile(0.99), h.quantile(0.999));
        println!(
            "{id}: one at a time: half within {p50} ns, 99% within {p99} ns, 99.9% within \
             {p999} ns, slowest {} ns ({} timed)",
            h.max, h.count
        );
        let doc = serde_json::json!({
            "count": h.count, "p50": p50, "p99": p99, "p999": p999, "max": h.max,
            "clock": clock, "hist": h.coarse(),
        });
        write_beside_criterion(group, S::NAME, readers, "tails.json", &doc);
    }
}

/// Every group once more, with every operation timed on its own.
fn tails() {
    let ms: u64 = match std::env::var("RCU_BENCH_TAIL_MS") {
        Ok(v) => v.parse().expect("RCU_BENCH_TAIL_MS must be a number"),
        Err(_) => 500,
    };
    // without `--bench` criterion only tries every benchmark once, and so with these options
    let args: Vec<String> = std::env::args().collect();
    let has = |opt: &str| args.iter().any(|a| a == opt);
    if ms == 0 || !has("--bench") || has("--list") || has("--test") || has("--profile-time") {
        return;
    }
    let time = Duration::from_millis(ms);

    // what reading the clock costs: every time measured includes it once
    let clock = time_each(Duration::from_millis(50), || {}).quantile(0.5);

    let max = max_readers().max(2);
    let alone = reader_counts(max);
    let beside_writer = reader_counts(max - 1);
    let mut for_write = vec![0];
    for_write.extend(&beside_writer);
    let flat = with_oversubscribed(&beside_writer);
    let writes = with_oversubscribed(&for_write);
    let paced = WriterLoad::Paced(write_interval("RCU_BENCH_WRITE_HZ", 1_000_000.0));
    let idle = idle_counts();
    let held = ReaderLoad::Held(rcu_bench::hold_time());

    // the groups in the order criterion ran them, each for all subjects
    macro_rules! read_tails {
        ($group:literal, $counts:expr, $load:expr) => {{
            macro_rules! one {
                ($t:ty) => {
                    tail_one::<$t>($group, $counts, time, clock, |r, t| {
                        tail_read::<$t>(r, t, $load)
                    });
                };
            }
            for_each_subject!(one);
        }};
    }
    macro_rules! write_tails {
        ($group:literal, $counts:expr, $load:expr) => {{
            macro_rules! one {
                ($t:ty) => {
                    tail_one::<$t>($group, $counts, time, clock, |r, t| {
                        tail_write::<$t>(r, t, $load)
                    });
                };
            }
            for_each_subject!(one);
        }};
    }
    read_tails!("read", &alone, WriterLoad::None);
    read_tails!("read-writer", &flat, WriterLoad::Flat);
    read_tails!("read-paced", &beside_writer, paced);
    write_tails!("write", &writes, ReaderLoad::Flat);
    write_tails!("write-idle", &idle, ReaderLoad::Idle);
    write_tails!("write-held", &beside_writer, held);
}

fn main() {
    cells();
    Criterion::default().configure_from_args().final_summary();
    tails();
}
