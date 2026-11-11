//! Not a real cell: an `AtomicPtr` whose old values are never freed.
//!
//! This is the floor for the read path (one atomic load) and for the write path (one
//! allocation and one atomic swap).

use crate::{Subject, Val};
use std::sync::atomic::{AtomicPtr, Ordering};
use std::sync::Arc;

pub struct Leak;

impl Subject for Leak {
    const NAME: &'static str = "baseline-leak";
    type Writer = Arc<AtomicPtr<Val>>;
    type Reader = Arc<AtomicPtr<Val>>;

    fn new(init: Val, readers: usize) -> (Self::Writer, Vec<Self::Reader>) {
        super::share(AtomicPtr::new(Box::into_raw(Box::new(init))), readers)
    }

    #[inline]
    fn read(r: &mut Self::Reader) -> u64 {
        // SAFETY: values are never freed.
        unsafe { (*r.load(Ordering::Acquire)).get() }
    }

    #[inline]
    fn write(w: &mut Self::Writer, v: Val) {
        w.swap(Box::into_raw(Box::new(v)), Ordering::AcqRel);
    }
}
