//! Checks that every adapter really is a cell: a write is visible to later reads, and
//! concurrent readers only ever see whole values that never go backwards.

use rcu_bench::{for_each_subject, Subject, Val};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

fn sequential<S: Subject>() {
    let (mut w, mut rs) = S::new(Val::new(7), 2.min(S::MAX_READERS));
    for r in &mut rs {
        assert_eq!(S::read(r), 7, "{}: initial value", S::NAME);
    }
    for i in 100..200 {
        S::write(&mut w, Val::new(i));
        for r in &mut rs {
            assert_eq!(S::read(r), i, "{}: value after write", S::NAME);
        }
    }
}

fn concurrent<S: Subject>() {
    const LAST: u64 = 20_000;
    let (mut w, rs) = S::new(Val::new(0), 3.min(S::MAX_READERS));
    let done = Arc::new(AtomicBool::new(false));
    let readers: Vec<_> = rs
        .into_iter()
        .map(|mut r| {
            let done = done.clone();
            std::thread::spawn(move || {
                let mut prev = 0;
                loop {
                    let finished = done.load(Ordering::Acquire);
                    let v = S::read(&mut r);
                    assert!(v <= LAST, "{}: torn or stale read {v}", S::NAME);
                    assert!(v >= prev, "{}: went backwards {prev} -> {v}", S::NAME);
                    prev = v;
                    if finished {
                        assert_eq!(v, LAST, "{}: final value", S::NAME);
                        break;
                    }
                }
            })
        })
        .collect();
    for i in 1..=LAST {
        S::write(&mut w, Val::new(i));
    }
    done.store(true, Ordering::Release);
    for r in readers {
        r.join().unwrap();
    }
}

#[test]
fn every_subject_sequential() {
    macro_rules! one {
        ($t:ty) => {
            sequential::<$t>();
        };
    }
    for_each_subject!(one);
}

#[test]
fn every_subject_concurrent() {
    macro_rules! one {
        ($t:ty) => {
            concurrent::<$t>();
        };
    }
    for_each_subject!(one);
}
