use crate::{Subject, Val};

pub struct SwmrCell;

impl Subject for SwmrCell {
    const NAME: &'static str = "swmr-cell";
    type Writer = ::swmr_cell::SwmrCell<Val>;
    type Reader = ::swmr_cell::LocalReader<Val>;

    fn new(init: Val, readers: usize) -> (Self::Writer, Vec<Self::Reader>) {
        let w = ::swmr_cell::SwmrCell::new(init);
        let rs = (0..readers).map(|_| w.local_reader()).collect();
        (w, rs)
    }

    #[inline]
    fn read(r: &mut Self::Reader) -> u64 {
        r.pin().get()
    }

    #[inline]
    fn write(w: &mut Self::Writer, v: Val) {
        w.store(v);
    }
}

/// `read_preferred()`: moves the barrier cost from the readers to the writer.
pub struct SwmrCellReadPreferred;

impl Subject for SwmrCellReadPreferred {
    const NAME: &'static str = "swmr-cell-read-preferred";
    type Writer = ::swmr_cell::SwmrCell<Val, true>;
    type Reader = ::swmr_cell::LocalReader<Val, true>;

    fn new(init: Val, readers: usize) -> (Self::Writer, Vec<Self::Reader>) {
        let w = ::swmr_cell::SwmrCell::builder()
            .read_preferred()
            .build(init);
        let rs = (0..readers).map(|_| w.local_reader()).collect();
        (w, rs)
    }

    #[inline]
    fn read(r: &mut Self::Reader) -> u64 {
        r.pin().get()
    }

    #[inline]
    fn write(w: &mut Self::Writer, v: Val) {
        w.store(v);
    }
}
