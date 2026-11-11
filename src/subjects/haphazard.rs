use crate::{Kind, Subject, Val};
use ::haphazard::{AtomicPtr, HazardPointer};
use std::sync::Arc;

type Cell = AtomicPtr<Val>;

pub struct Haphazard;

/// Each reader keeps one hazard pointer for its whole life.
pub struct Reader {
    cell: Arc<Cell>,
    hp: HazardPointer<'static>,
}

impl Subject for Haphazard {
    const NAME: &'static str = "haphazard";
    const KIND: Kind = Kind::Framework;
    const DESC: &'static str = "AtomicPtr, one HazardPointer kept per reader; retire per write.";
    type Writer = Arc<Cell>;
    type Reader = Reader;

    fn new(init: Val, readers: usize) -> (Self::Writer, Vec<Self::Reader>) {
        let (w, rs) = super::share(Cell::from(Box::new(init)), readers);
        let rs = rs
            .into_iter()
            .map(|cell| Reader {
                cell,
                hp: HazardPointer::new(),
            })
            .collect();
        (w, rs)
    }

    #[inline]
    fn read(r: &mut Self::Reader) -> u64 {
        let v = r.cell.safe_load(&mut r.hp).unwrap().get();
        r.hp.reset_protection();
        v
    }

    #[inline]
    fn write(w: &mut Self::Writer, v: Val) {
        let old = w.swap(Box::new(v)).unwrap();
        // SAFETY: `old` was just unlinked and can't be loaded again.
        unsafe { old.retire() };
    }
}
