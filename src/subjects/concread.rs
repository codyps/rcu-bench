use crate::{Kind, Subject, Val};
use std::sync::Arc;

type Cell = ::concread::CowCell<Val>;

pub struct CowCell;

impl Subject for CowCell {
    const NAME: &'static str = "concread-cowcell";
    const KIND: Kind = Kind::Reference;
    const DESC: &'static str =
        "CowCell::read, write transaction + commit (two mutexes and an Arc).";
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
        let mut txn = w.write();
        *txn = v;
        txn.commit();
    }
}
