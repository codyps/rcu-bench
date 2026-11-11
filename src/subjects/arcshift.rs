use crate::{Kind, Subject, Val};

pub struct ArcShift;

impl Subject for ArcShift {
    const NAME: &'static str = "arcshift";
    const KIND: Kind = Kind::Shared;
    const DESC: &'static str = "ArcShift::get (&mut), update.";
    type Writer = ::arcshift::ArcShift<Val>;
    type Reader = ::arcshift::ArcShift<Val>;

    fn new(init: Val, readers: usize) -> (Self::Writer, Vec<Self::Reader>) {
        let w = ::arcshift::ArcShift::new(init);
        let rs = (0..readers).map(|_| w.clone()).collect();
        (w, rs)
    }

    #[inline]
    fn read(r: &mut Self::Reader) -> u64 {
        r.get().get()
    }

    #[inline]
    fn write(w: &mut Self::Writer, v: Val) {
        w.update(v);
    }
}
