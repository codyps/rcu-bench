use crate::{Subject, Val};
use std::sync::Arc;

type Cell = ::hazarc::AtomicArc<Val>;

pub struct Hazarc;

impl Subject for Hazarc {
    const NAME: &'static str = "hazarc";
    type Writer = Arc<Cell>;
    type Reader = Arc<Cell>;

    fn new(init: Val, readers: usize) -> (Self::Writer, Vec<Self::Reader>) {
        super::share(Cell::new(Arc::new(init)), readers)
    }

    #[inline]
    fn read(r: &mut Self::Reader) -> u64 {
        r.load().get()
    }

    #[inline]
    fn write(w: &mut Self::Writer, v: Val) {
        w.store(Arc::new(v));
    }
}

/// A per-reader `Cache`.
pub struct HazarcCache;

impl Subject for HazarcCache {
    const NAME: &'static str = "hazarc-cache";
    type Writer = Arc<Cell>;
    type Reader = ::hazarc::Cache<Arc<Cell>>;

    fn new(init: Val, readers: usize) -> (Self::Writer, Vec<Self::Reader>) {
        let (w, rs) = super::share(Cell::new(Arc::new(init)), readers);
        (w, rs.into_iter().map(::hazarc::Cache::new).collect())
    }

    #[inline]
    fn read(r: &mut Self::Reader) -> u64 {
        r.load().get()
    }

    #[inline]
    fn write(w: &mut Self::Writer, v: Val) {
        w.store(Arc::new(v));
    }
}
