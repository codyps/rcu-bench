use crate::{Kind, Subject, Val};
use ::crossbeam_epoch::{self as epoch, Atomic, Owned};
use std::sync::atomic::Ordering;
use std::sync::Arc;

/// An `Atomic<T>` in the default (global) collector.
pub struct Cell(Atomic<Val>);

impl Drop for Cell {
    fn drop(&mut self) {
        // SAFETY: `&mut self`, nobody else can reach the pointer any more.
        unsafe { drop(std::mem::replace(&mut self.0, Atomic::null()).into_owned()) }
    }
}

pub struct CrossbeamEpoch;

impl Subject for CrossbeamEpoch {
    const NAME: &'static str = "crossbeam-epoch";
    const KIND: Kind = Kind::Framework;
    const DESC: &'static str =
        "Atomic<T> in the global collector; pin per read, defer_destroy per write.";
    type Writer = Arc<Cell>;
    type Reader = Arc<Cell>;

    fn new(init: Val, readers: usize) -> (Self::Writer, Vec<Self::Reader>) {
        super::share(Cell(Atomic::new(init)), readers)
    }

    #[inline]
    fn read(r: &mut Self::Reader) -> u64 {
        let guard = epoch::pin();
        // SAFETY: never null, and the guard keeps the value alive.
        unsafe { r.0.load(Ordering::Acquire, &guard).deref().get() }
    }

    #[inline]
    fn write(w: &mut Self::Writer, v: Val) {
        let guard = epoch::pin();
        let old = w.0.swap(Owned::new(v), Ordering::AcqRel, &guard);
        // SAFETY: `old` was just unlinked and can't be loaded again.
        unsafe { guard.defer_destroy(old) };
    }
}
