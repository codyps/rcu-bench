//! Read and write latency of every subject as the number of simultaneous readers grows.
//!
//! Groups, each measured for a range of reader counts `R`:
//!
//! - `read`:        R readers, no writer
//! - `read-writer`: R readers, while a writer publishes as fast as it can
//! - `write`:       the writer, while R readers read as fast as they can (R starts at 0)
//!
//! Benchmark ids are `<group>/<subject>/<R>`.
//!
//! `RCU_BENCH_MAX_READERS` caps R. By default it is the number of CPUs (at most 8). The
//! groups with a writer stop one short of the cap, so that the writer has a CPU of its own.
//!
//! Read groups report the time per read as seen by one reader (averaged over the readers).
//! The write group reports the time per write.

use criterion::measurement::WallTime;
use criterion::{criterion_group, criterion_main, BenchmarkGroup, BenchmarkId, Criterion};
use rcu_bench::{for_each_subject, Subject, Val};
use std::hint::black_box;
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

/// `readers` threads each do `iters` reads. Returns the mean time one reader took.
fn run_read<S: Subject>(iters: u64, readers: usize, with_writer: bool) -> Duration {
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
            let mut i = 0;
            while !stop.load(Ordering::Relaxed) {
                i += 1;
                S::write(&mut w, Val::new(i));
            }
        });
        (None, Some(t))
    } else {
        (Some(w), None)
    };

    let total: Duration = reader_threads.into_iter().map(|t| t.join().unwrap()).sum();
    stop.store(true, Ordering::Relaxed);
    if let Some(t) = writer_thread {
        t.join().unwrap();
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

fn read_one<S: Subject>(g: &mut BenchmarkGroup<'_, WallTime>, counts: &[usize], with_writer: bool) {
    for &readers in counts {
        if readers > S::MAX_READERS {
            continue;
        }
        g.bench_function(BenchmarkId::new(S::NAME, readers), |b| {
            b.iter_custom(|iters| run_read::<S>(iters, readers, with_writer))
        });
    }
}

fn write_one<S: Subject>(g: &mut BenchmarkGroup<'_, WallTime>, counts: &[usize]) {
    for &readers in counts {
        if readers > S::MAX_READERS {
            continue;
        }
        g.bench_function(BenchmarkId::new(S::NAME, readers), |b| {
            b.iter_custom(|iters| run_write::<S>(iters, readers))
        });
    }
}

fn read_group(c: &mut Criterion, name: &str, counts: &[usize], with_writer: bool) {
    let mut g = c.benchmark_group(name);
    macro_rules! one {
        ($t:ty) => {
            read_one::<$t>(&mut g, counts, with_writer);
        };
    }
    for_each_subject!(one);
    g.finish();
}

fn write_group(c: &mut Criterion, name: &str, counts: &[usize]) {
    let mut g = c.benchmark_group(name);
    macro_rules! one {
        ($t:ty) => {
            write_one::<$t>(&mut g, counts);
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

    read_group(c, "read", &alone, false);
    read_group(c, "read-writer", &beside_writer, true);
    write_group(c, "write", &for_write);
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
