use crate::{Kind, Subject, Val};
use ::sdd::{AtomicOwned, Guard, Owned, Tag};
use std::sync::atomic::Ordering;
use std::sync::Arc;

type Cell = AtomicOwned<Val>;

pub struct Sdd;

impl Subject for Sdd {
    const NAME: &'static str = "sdd";
    const KIND: Kind = Kind::Framework;
    const DESC: &'static str = "AtomicOwned; a Guard per read, swap per write.";
    type Writer = Arc<Cell>;
    type Reader = Arc<Cell>;

    fn new(init: Val, readers: usize) -> (Self::Writer, Vec<Self::Reader>) {
        super::share(Cell::new(init), readers)
    }

    #[inline]
    fn read(r: &mut Self::Reader) -> u64 {
        let guard = Guard::new();
        r.load(Ordering::Acquire, &guard).as_ref().unwrap().get()
    }

    #[inline]
    fn write(w: &mut Self::Writer, v: Val) {
        // Dropping the returned `Owned` retires the old value.
        drop(w.swap((Some(Owned::new(v)), Tag::None), Ordering::AcqRel));
    }
}
