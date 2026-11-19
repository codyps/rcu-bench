use crate::{Kind, Subject, Val};
use std::sync::Arc;

type Cell = ::rcu_cell::RcuCell<Val>;

pub struct RcuCell;

impl Subject for RcuCell {
    const NAME: &'static str = "rcu_cell";
    const KIND: Kind = Kind::Shared;
    const DESC: &'static str = "RcuCell::read, write.";
    type Writer = Arc<Cell>;
    type Reader = Arc<Cell>;

    fn new(init: Val, readers: usize) -> (Self::Writer, Vec<Self::Reader>) {
        super::share(Cell::new(init), readers)
    }

    #[inline]
    fn read_with<T>(r: &mut Self::Reader, f: impl FnOnce(&Val) -> T) -> T {
        f(&r.read().unwrap())
    }

    #[inline]
    fn write(w: &mut Self::Writer, v: Val) {
        drop(w.write(v));
    }
}
