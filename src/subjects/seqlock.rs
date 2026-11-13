use crate::{Kind, Subject, Val};
use std::sync::Arc;

type Cell = ::seqlock::SeqLock<Val>;

/// `Copy` values only: a read copies the value out.
pub struct SeqLock;

impl Subject for SeqLock {
    const NAME: &'static str = "seqlock";
    const KIND: Kind = Kind::Reference;
    const DESC: &'static str = "SeqLock::read copies the value out; lock_write. Copy values only.";
    const COPY_ONLY: bool = true;
    type Writer = Arc<Cell>;
    type Reader = Arc<Cell>;

    fn new(init: Val, readers: usize) -> (Self::Writer, Vec<Self::Reader>) {
        super::share(Cell::new(init), readers)
    }

    #[inline]
    fn read(r: &mut Self::Reader) -> u64 {
        r.read().get()
    }

    #[inline]
    fn write(w: &mut Self::Writer, v: Val) {
        *w.lock_write() = v;
    }
}
