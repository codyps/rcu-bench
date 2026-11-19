use crate::{Kind, Subject, Val};
use std::sync::atomic::Ordering;
use std::sync::Arc;

type Cell = ::kovan::Atom<Val>;

pub struct Kovan;

impl Subject for Kovan {
    const NAME: &'static str = "kovan";
    const KIND: Kind = Kind::Framework;
    const DESC: &'static str = "Atom::load, store.";
    type Writer = Arc<Cell>;
    type Reader = Arc<Cell>;

    fn new(init: Val, readers: usize) -> (Self::Writer, Vec<Self::Reader>) {
        super::share(Cell::new(init), readers)
    }

    #[inline]
    fn read(r: &mut Self::Reader) -> u64 {
        r.load().get()
    }

    #[inline]
    fn write(w: &mut Self::Writer, v: Val) {
        w.store(v);
    }
}

/// What `kovan`'s own benchmarks store: the value behind the `RetiredNode` that `retire`
/// needs at offset 0.
#[repr(C)]
struct Node {
    retired: ::kovan::RetiredNode,
    val: Val,
}

impl Node {
    fn boxed(val: Val) -> *mut Node {
        Box::into_raw(Box::new(Node {
            retired: ::kovan::RetiredNode::new(),
            val,
        }))
    }
}

pub struct RawCell(::kovan::Atomic<Node>);

// SAFETY: the pointer is only reached through the atomic, and a `Node` is plain data that is
// never changed once it is published.
unsafe impl Send for RawCell {}
unsafe impl Sync for RawCell {}

impl Drop for RawCell {
    fn drop(&mut self) {
        let guard = ::kovan::pin();
        let last = self.0.load(Ordering::Acquire, &guard).as_raw();
        // SAFETY: `&mut self`, nobody else can reach the pointer any more, and it was never
        // retired.
        unsafe { drop(Box::from_raw(last)) }
    }
}

/// The lower level API, used the way `kovan`'s own benchmarks use it.
pub struct KovanRaw;

impl Subject for KovanRaw {
    const NAME: &'static str = "kovan-raw";
    const KIND: Kind = Kind::Framework;
    const DESC: &'static str =
        "Atomic of a node embedding a RetiredNode; pin + load per read, swap + retire per write.";
    type Writer = Arc<RawCell>;
    type Reader = Arc<RawCell>;

    fn new(init: Val, readers: usize) -> (Self::Writer, Vec<Self::Reader>) {
        super::share(RawCell(::kovan::Atomic::new(Node::boxed(init))), readers)
    }

    #[inline]
    fn read(r: &mut Self::Reader) -> u64 {
        let guard = ::kovan::pin();
        let p = r.0.load(Ordering::Acquire, &guard);
        // SAFETY: never null, and the guard keeps the node alive.
        unsafe { p.deref().val.get() }
    }

    #[inline]
    fn write(w: &mut Self::Writer, v: Val) {
        let guard = ::kovan::pin();
        // SAFETY: a fresh allocation.
        let new = unsafe { ::kovan::Shared::from_raw(Node::boxed(v)) };
        let old = w.0.swap(new, Ordering::AcqRel, &guard);
        // SAFETY: `old` was just unlinked, came from a `Box`, starts with a `RetiredNode`, and
        // is retired once.
        unsafe { ::kovan::retire(old.as_raw()) };
    }
}
