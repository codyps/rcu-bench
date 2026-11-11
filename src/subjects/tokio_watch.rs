use crate::{Subject, Val};
use tokio::sync::watch;

pub struct TokioWatch;

impl Subject for TokioWatch {
    const NAME: &'static str = "tokio-watch";
    type Writer = watch::Sender<Val>;
    type Reader = watch::Receiver<Val>;

    fn new(init: Val, readers: usize) -> (Self::Writer, Vec<Self::Reader>) {
        let (tx, rx) = watch::channel(init);
        let rs = (0..readers).map(|_| rx.clone()).collect();
        (tx, rs)
    }

    #[inline]
    fn read(r: &mut Self::Reader) -> u64 {
        r.borrow().get()
    }

    #[inline]
    fn write(w: &mut Self::Writer, v: Val) {
        w.send_replace(v);
    }
}
