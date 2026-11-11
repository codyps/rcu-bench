use crate::{Subject, Val};
use ::seize::{reclaim, Collector, Guard};
use std::sync::atomic::{AtomicPtr, Ordering};
use std::sync::Arc;

pub struct Cell {
    collector: Collector,
    ptr: AtomicPtr<Val>,
}

impl Drop for Cell {
    fn drop(&mut self) {
        // SAFETY: `&mut self`, nobody else can reach the pointer any more.
        unsafe { drop(Box::from_raw(*self.ptr.get_mut())) }
    }
}

pub struct Seize;

impl Subject for Seize {
    const NAME: &'static str = "seize";
    type Writer = Arc<Cell>;
    type Reader = Arc<Cell>;

    fn new(init: Val, readers: usize) -> (Self::Writer, Vec<Self::Reader>) {
        super::share(
            Cell {
                collector: Collector::new(),
                ptr: AtomicPtr::new(Box::into_raw(Box::new(init))),
            },
            readers,
        )
    }

    #[inline]
    fn read(r: &mut Self::Reader) -> u64 {
        let guard = r.collector.enter();
        let p = guard.protect(&r.ptr, Ordering::Acquire);
        // SAFETY: never null, and the guard keeps the value alive.
        unsafe { (*p).get() }
    }

    #[inline]
    fn write(w: &mut Self::Writer, v: Val) {
        let old = w.ptr.swap(Box::into_raw(Box::new(v)), Ordering::AcqRel);
        // SAFETY: `old` was just unlinked, came from a `Box`, and is retired once.
        unsafe { w.collector.retire(old, reclaim::boxed) };
    }
}
