//! Benchmarks comparing Rust RCU-like "single value cell" crates.
//!
//! Every crate is wrapped in a [`Subject`], which reduces it to the three operations all of
//! them share: create a cell with some reader handles, read the current value, and publish a
//! new value.

pub mod subjects;

/// The value stored in every cell.
///
/// Small and `Copy` so that crates which only support `Copy` types (`seqlock`) can take part.
/// Big enough that copying it is not free.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Val(pub [u64; 4]);

impl Val {
    #[inline]
    pub fn new(v: u64) -> Self {
        Val([v, v.wrapping_add(1), v.wrapping_add(2), v.wrapping_add(3)])
    }

    /// Look at the value: touches the first and last word, and returns the `v` that was given
    /// to [`Val::new`] (or something else if the read was torn).
    #[inline]
    pub fn get(&self) -> u64 {
        let spread = self.0[3].wrapping_sub(self.0[0]).wrapping_sub(3);
        self.0[0].wrapping_add(spread)
    }
}

/// One crate (or one way of using a crate), reduced to read and write.
pub trait Subject: 'static {
    /// Name used in benchmark ids and reports. Keep it filesystem friendly.
    const NAME: &'static str;

    /// How many readers the subject supports.
    const MAX_READERS: usize = usize::MAX;

    /// Owned by the single writing thread.
    type Writer: Send + 'static;
    /// Owned by one reading thread each.
    type Reader: Send + 'static;

    /// Create a cell holding `init`, and `readers` reader handles for it.
    fn new(init: Val, readers: usize) -> (Self::Writer, Vec<Self::Reader>);

    /// Read the current value and look at it.
    fn read(r: &mut Self::Reader) -> u64;

    /// Publish a new value.
    fn write(w: &mut Self::Writer, v: Val);
}

/// The names of all subjects, in the order of [`for_each_subject`].
pub fn subject_names() -> Vec<&'static str> {
    let mut names = Vec::new();
    macro_rules! name {
        ($t:ty) => {
            names.push(<$t as Subject>::NAME);
        };
    }
    for_each_subject!(name);
    names
}

/// Calls `$m!(Type)` once for every subject.
#[macro_export]
macro_rules! for_each_subject {
    ($m:ident) => {
        $m!($crate::subjects::baseline::Leak);
        $m!($crate::subjects::local_rcu::LocalRcu);
        $m!($crate::subjects::std_locks::RwLockVal);
        $m!($crate::subjects::std_locks::RwLockArc);
        $m!($crate::subjects::std_locks::MutexArc);
        $m!($crate::subjects::tokio_watch::TokioWatch);
        $m!($crate::subjects::triple_buffer::TripleBuffer);
        $m!($crate::subjects::left_right::LeftRight);
        $m!($crate::subjects::swmr_cell::SwmrCell);
        $m!($crate::subjects::swmr_cell::SwmrCellReadPreferred);
        $m!($crate::subjects::read_copy_update::ReadCopyUpdate);
        $m!($crate::subjects::arc_swap::ArcSwap);
        $m!($crate::subjects::arc_swap::ArcSwapLoadFull);
        $m!($crate::subjects::arc_swap::ArcSwapCache);
        $m!($crate::subjects::hazarc::Hazarc);
        $m!($crate::subjects::hazarc::HazarcCache);
        $m!($crate::subjects::aarc::Aarc);
        $m!($crate::subjects::arcshift::ArcShift);
        $m!($crate::subjects::arcu::Arcu);
        $m!($crate::subjects::rcu_cell::RcuCell);
        $m!($crate::subjects::basedrop::Basedrop);
        $m!($crate::subjects::concread::CowCell);
        $m!($crate::subjects::seqlock::SeqLock);
        $m!($crate::subjects::keepcalm::KeepcalmRcu);
        $m!($crate::subjects::crossbeam_epoch::CrossbeamEpoch);
        $m!($crate::subjects::seize::Seize);
        $m!($crate::subjects::sdd::Sdd);
        $m!($crate::subjects::haphazard::Haphazard);
        $m!($crate::subjects::kovan::Kovan);
    };
}
