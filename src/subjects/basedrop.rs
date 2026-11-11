use crate::{Subject, Val};
use ::basedrop::{Collector, Handle, Shared, SharedCell};
use std::sync::Arc;

pub struct Basedrop;

/// The writer also plays the collector thread: it collects after every write.
pub struct Writer {
    cell: Arc<SharedCell<Val>>,
    collector: Collector,
    handle: Handle,
}

impl Subject for Basedrop {
    const NAME: &'static str = "basedrop";
    type Writer = Writer;
    type Reader = Arc<SharedCell<Val>>;

    fn new(init: Val, readers: usize) -> (Self::Writer, Vec<Self::Reader>) {
        let collector = Collector::new();
        let handle = collector.handle();
        let (cell, rs) = super::share(SharedCell::new(Shared::new(&handle, init)), readers);
        (
            Writer {
                cell,
                collector,
                handle,
            },
            rs,
        )
    }

    #[inline]
    fn read(r: &mut Self::Reader) -> u64 {
        r.get().get()
    }

    #[inline]
    fn write(w: &mut Self::Writer, v: Val) {
        w.cell.set(Shared::new(&w.handle, v));
        w.collector.collect();
    }
}
