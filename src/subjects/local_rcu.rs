use crate::{Kind, Subject, Val};

pub struct LocalRcu;

impl Subject for LocalRcu {
    const NAME: &'static str = "local-rcu";
    const KIND: Kind = Kind::SingleWriter;
    const DESC: &'static str = "Reader::read, Writer::write.";
    type Writer = ::local_rcu::Writer<Val>;
    type Reader = ::local_rcu::Reader<Val>;

    fn new(init: Val, readers: usize) -> (Self::Writer, Vec<Self::Reader>) {
        let w = ::local_rcu::Writer::new(Box::new(init));
        let rs = (0..readers).map(|_| w.reader()).collect();
        (w, rs)
    }

    #[inline]
    fn read(r: &mut Self::Reader) -> u64 {
        r.read().get()
    }

    #[inline]
    fn write(w: &mut Self::Writer, v: Val) {
        drop(w.write(Box::new(v)));
    }
}
