use crate::{Kind, Subject, Val};
use std::sync::Arc;

type Cell = ::aarc::AtomicArc<Val>;

pub struct Aarc;

impl Subject for Aarc {
    const NAME: &'static str = "aarc";
    const KIND: Kind = Kind::Shared;
    const DESC: &'static str = "AtomicArc::load, store.";
    type Writer = Arc<Cell>;
    type Reader = Arc<Cell>;

    fn new(init: Val, readers: usize) -> (Self::Writer, Vec<Self::Reader>) {
        super::share(Cell::new(init), readers)
    }

    #[inline]
    fn read(r: &mut Self::Reader) -> u64 {
        r.load().unwrap().get()
    }

    #[inline]
    fn write(w: &mut Self::Writer, v: Val) {
        w.store(Some(&::aarc::Arc::new(v)));
    }
}
