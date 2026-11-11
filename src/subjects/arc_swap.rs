use crate::{Subject, Val};
use std::sync::Arc;

type Cell = ::arc_swap::ArcSwap<Val>;

/// `load()`: a `Guard`, usually without touching the reference count.
pub struct ArcSwap;

impl Subject for ArcSwap {
    const NAME: &'static str = "arc-swap";
    type Writer = Arc<Cell>;
    type Reader = Arc<Cell>;

    fn new(init: Val, readers: usize) -> (Self::Writer, Vec<Self::Reader>) {
        super::share(Cell::from_pointee(init), readers)
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

/// `load_full()`: an owned `Arc`.
pub struct ArcSwapLoadFull;

impl Subject for ArcSwapLoadFull {
    const NAME: &'static str = "arc-swap-load_full";
    type Writer = Arc<Cell>;
    type Reader = Arc<Cell>;

    fn new(init: Val, readers: usize) -> (Self::Writer, Vec<Self::Reader>) {
        super::share(Cell::from_pointee(init), readers)
    }

    #[inline]
    fn read(r: &mut Self::Reader) -> u64 {
        r.load_full().get()
    }

    #[inline]
    fn write(w: &mut Self::Writer, v: Val) {
        w.store(Arc::new(v));
    }
}

/// A per-reader `Cache`.
pub struct ArcSwapCache;

impl Subject for ArcSwapCache {
    const NAME: &'static str = "arc-swap-cache";
    type Writer = Arc<Cell>;
    type Reader = ::arc_swap::cache::Cache<Arc<Cell>, Arc<Val>>;

    fn new(init: Val, readers: usize) -> (Self::Writer, Vec<Self::Reader>) {
        let (w, rs) = super::share(Cell::from_pointee(init), readers);
        (
            w,
            rs.into_iter().map(::arc_swap::cache::Cache::new).collect(),
        )
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
