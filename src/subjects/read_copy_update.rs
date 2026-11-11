use crate::{Subject, Val};

pub struct ReadCopyUpdate;

impl Subject for ReadCopyUpdate {
    const NAME: &'static str = "read-copy-update";
    type Writer = ::read_copy_update::Rcu<Val>;
    type Reader = ::read_copy_update::Reader<Val>;

    fn new(init: Val, readers: usize) -> (Self::Writer, Vec<Self::Reader>) {
        let mut w = ::read_copy_update::Rcu::new(init);
        let rs = (0..readers).map(|_| w.reader()).collect();
        (w, rs)
    }

    #[inline]
    fn read(r: &mut Self::Reader) -> u64 {
        r.read().get()
    }

    #[inline]
    fn write(w: &mut Self::Writer, v: Val) {
        w.write(v);
    }
}
