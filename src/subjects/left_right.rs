use crate::{Kind, Subject, Val};
use ::left_right::{Absorb, ReadHandle, WriteHandle};

/// The whole value is the "data structure", and the only operation is "replace it".
#[derive(Clone)]
pub struct Slot(Val);

impl Absorb<Val> for Slot {
    fn absorb_first(&mut self, operation: &mut Val, _other: &Self) {
        self.0 = *operation;
    }

    fn sync_with(&mut self, first: &Self) {
        self.0 = first.0;
    }
}

pub struct LeftRight;

impl Subject for LeftRight {
    const NAME: &'static str = "left-right";
    const KIND: Kind = Kind::SingleWriter;
    const DESC: &'static str = "The value is the data structure and the only operation replaces it; append + publish per write.";
    type Writer = WriteHandle<Slot, Val>;
    type Reader = ReadHandle<Slot>;

    fn new(init: Val, readers: usize) -> (Self::Writer, Vec<Self::Reader>) {
        let (w, r) = ::left_right::new_from_empty::<Slot, Val>(Slot(init));
        let rs = (0..readers).map(|_| r.clone()).collect();
        (w, rs)
    }

    #[inline]
    fn read(r: &mut Self::Reader) -> u64 {
        r.enter().unwrap().0.get()
    }

    #[inline]
    fn write(w: &mut Self::Writer, v: Val) {
        w.append(v);
        w.publish();
    }
}
