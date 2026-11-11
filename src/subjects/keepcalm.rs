use crate::{Kind, Subject, Val};

pub struct KeepcalmRcu;

impl Subject for KeepcalmRcu {
    const NAME: &'static str = "keepcalm-rcu";
    const KIND: Kind = Kind::Reference;
    const DESC: &'static str = "SharedMut::new_rcu, read, set (a parking_lot RwLock<Arc<T>>).";
    type Writer = ::keepcalm::SharedMut<Val>;
    type Reader = ::keepcalm::SharedMut<Val>;

    fn new(init: Val, readers: usize) -> (Self::Writer, Vec<Self::Reader>) {
        let w = ::keepcalm::SharedMut::new_rcu(init);
        let rs = (0..readers).map(|_| w.clone()).collect();
        (w, rs)
    }

    #[inline]
    fn read(r: &mut Self::Reader) -> u64 {
        r.read().get()
    }

    #[inline]
    fn write(w: &mut Self::Writer, v: Val) {
        w.set(v);
    }
}
