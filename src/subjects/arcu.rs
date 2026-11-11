use crate::{Subject, Val};
use ::arcu::epoch_counters::GlobalEpochCounterPool;
use ::arcu::Rcu;
use std::sync::Arc;

type Cell = ::arcu::atomic::Arcu<Val, GlobalEpochCounterPool>;

pub struct Arcu;

impl Subject for Arcu {
    const NAME: &'static str = "arcu";
    type Writer = Arc<Cell>;
    type Reader = Arc<Cell>;

    fn new(init: Val, readers: usize) -> (Self::Writer, Vec<Self::Reader>) {
        super::share(Cell::new(init, GlobalEpochCounterPool), readers)
    }

    #[inline]
    fn read(r: &mut Self::Reader) -> u64 {
        r.read().get()
    }

    #[inline]
    fn write(w: &mut Self::Writer, v: Val) {
        drop(w.replace(v));
    }
}
