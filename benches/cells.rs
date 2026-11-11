//! Read and write latency of every subject, alone and under contention.
//!
//! Groups (`N` is `RCU_BENCH_READERS`, default 4):
//!
//! - `read-1`:        one reader, nothing else running
//! - `read-N`:        N readers at once, no writer
//! - `read-1-writer`: one reader, while a writer publishes as fast as it can
//! - `read-N-writer`: N readers, while a writer publishes as fast as it can
//! - `write-0`:       the writer alone
//! - `write-N`:       the writer, while N readers read as fast as they can
//!
//! Read groups report the time per read as seen by one reader (averaged over the readers).
//! Write groups report the time per write.

use criterion::measurement::WallTime;
use criterion::{criterion_group, criterion_main, BenchmarkGroup, Criterion};
use rcu_bench::{for_each_subject, Subject, Val};
use std::hint::black_box;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Barrier};
use std::time::{Duration, Instant};

fn n_readers() -> usize {
    std::env::var("RCU_BENCH_READERS")
        .ok()
        .map(|v| v.parse().expect("RCU_BENCH_READERS must be a number"))
        .unwrap_or(4)
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

fn read_one<S: Subject>(g: &mut BenchmarkGroup<'_, WallTime>, readers: usize, with_writer: bool) {
    if readers > S::MAX_READERS {
        return;
    }
    g.bench_function(S::NAME, |b| {
        b.iter_custom(|iters| run_read::<S>(iters, readers, with_writer))
    });
}

fn write_one<S: Subject>(g: &mut BenchmarkGroup<'_, WallTime>, readers: usize) {
    if readers > S::MAX_READERS {
        return;
    }
    g.bench_function(S::NAME, |b| {
        b.iter_custom(|iters| run_write::<S>(iters, readers))
    });
}

fn read_group(c: &mut Criterion, name: &str, readers: usize, with_writer: bool) {
    let mut g = c.benchmark_group(name);
    macro_rules! one {
        ($t:ty) => {
            read_one::<$t>(&mut g, readers, with_writer);
        };
    }
    for_each_subject!(one);
    g.finish();
}

fn write_group(c: &mut Criterion, name: &str, readers: usize) {
    let mut g = c.benchmark_group(name);
    macro_rules! one {
        ($t:ty) => {
            write_one::<$t>(&mut g, readers);
        };
    }
    for_each_subject!(one);
    g.finish();
}

fn benches(c: &mut Criterion) {
    let n = n_readers();
    read_group(c, "read-1", 1, false);
    read_group(c, "read-N", n, false);
    read_group(c, "read-1-writer", 1, true);
    read_group(c, "read-N-writer", n, true);
    write_group(c, "write-0", 0);
    write_group(c, "write-N", n);
}

criterion_group! {
    name = cells;
    config = Criterion::default()
        .sample_size(20)
        .warm_up_time(Duration::from_millis(500))
        .measurement_time(Duration::from_secs(2));
    targets = benches
}
criterion_main!(cells);
