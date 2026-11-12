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

/// `cached::CachedReader`: stays in its read section between reads.
pub struct LocalRcuCached;

impl Subject for LocalRcuCached {
    const NAME: &'static str = "local-rcu-cached";
    const KIND: Kind = Kind::SingleWriter;
    const DESC: &'static str =
        "cached::CachedReader::read (stays in its read section between reads), Writer::write.";
    type Writer = ::local_rcu::Writer<Val>;
    type Reader = ::local_rcu::cached::CachedReader<Val>;

    fn new(init: Val, readers: usize) -> (Self::Writer, Vec<Self::Reader>) {
        let w = ::local_rcu::Writer::new(Box::new(init));
        let rs = (0..readers).map(|_| w.reader().into()).collect();
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

/// How many writes [`LocalRcuBatched`] lets pass between two looks for old values.
pub const BATCH: u32 = 128;

/// `Writer::write_nosync` for every write, `Writer::try_sync` once every [`BATCH`] writes.
///
/// `Writer::write` reads the epoch of every reader on each write, to learn who might still see
/// the value it replaced. `write_nosync` only stores the new value: no lock, no fence, and no
/// look at the readers. `try_sync` then scans the readers once for everything replaced since
/// the last scan and frees it as a batch, the way an epoch collector such as `crossbeam-epoch`
/// does. Up to two batches of old values are waiting at any time.
pub struct LocalRcuBatched;

/// The writer and the number of writes since it last looked for old values.
pub struct BatchedWriter {
    writer: ::local_rcu::Writer<Val>,
    pending: u32,
}

impl Subject for LocalRcuBatched {
    const NAME: &'static str = "local-rcu-batched";
    const KIND: Kind = Kind::SingleWriter;
    const DESC: &'static str =
        "Reader::read, Writer::write_nosync (a pointer store that leaves the readers alone); old values are scanned for and collected in batches (try_sync) once every 128 writes.";
    type Writer = BatchedWriter;
    type Reader = ::local_rcu::Reader<Val>;

    fn new(init: Val, readers: usize) -> (Self::Writer, Vec<Self::Reader>) {
        let writer = ::local_rcu::Writer::new(Box::new(init));
        let rs = (0..readers).map(|_| writer.reader()).collect();
        (BatchedWriter { writer, pending: 0 }, rs)
    }

    #[inline]
    fn read(r: &mut Self::Reader) -> u64 {
        r.read().get()
    }

    #[inline]
    fn write(w: &mut Self::Writer, v: Val) {
        w.writer.write_nosync(Box::new(v));
        w.pending += 1;
        if w.pending == BATCH {
            w.pending = 0;
            drop(w.writer.try_sync());
        }
    }
}
