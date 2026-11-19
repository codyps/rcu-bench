use crate::{Kind, Subject, Val};
use std::sync::Arc;

type Cell = ::kovan::Atom<Val>;

pub struct Kovan;

impl Subject for Kovan {
    const NAME: &'static str = "kovan";
    const KIND: Kind = Kind::Framework;
    const DESC: &'static str = "Atom::load, store.";
    type Writer = Arc<Cell>;
    type Reader = Arc<Cell>;

    fn new(init: Val, readers: usize) -> (Self::Writer, Vec<Self::Reader>) {
        super::share(Cell::new(init), readers)
    }

    #[inline]
    fn read_with<T>(r: &mut Self::Reader, f: impl FnOnce(&Val) -> T) -> T {
        f(&r.load())
    }

    #[inline]
    fn write(w: &mut Self::Writer, v: Val) {
        w.store(v);
    }
}
