use crate::{Subject, Val};

/// Single consumer only.
pub struct TripleBuffer;

impl Subject for TripleBuffer {
    const NAME: &'static str = "triple_buffer";
    const MAX_READERS: usize = 1;
    type Writer = ::triple_buffer::Input<Val>;
    type Reader = ::triple_buffer::Output<Val>;

    fn new(init: Val, readers: usize) -> (Self::Writer, Vec<Self::Reader>) {
        assert!(readers <= 1);
        let (i, o) = ::triple_buffer::triple_buffer(&init);
        (i, Some(o).into_iter().take(readers).collect())
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
