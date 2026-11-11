//! One module per crate.

pub mod aarc;
pub mod arc_swap;
pub mod arcshift;
pub mod arcu;
pub mod basedrop;
pub mod baseline;
pub mod concread;
pub mod crossbeam_epoch;
pub mod haphazard;
pub mod hazarc;
pub mod keepcalm;
pub mod kovan;
pub mod left_right;
pub mod local_rcu;
pub mod rcu_cell;
pub mod read_copy_update;
pub mod sdd;
pub mod seize;
pub mod seqlock;
pub mod std_locks;
pub mod swmr_cell;
pub mod tokio_watch;
pub mod triple_buffer;

/// For cells that are shared by reference: the writer and every reader get an `Arc` of it.
pub(crate) fn share<C>(cell: C, readers: usize) -> (std::sync::Arc<C>, Vec<std::sync::Arc<C>>) {
    let cell = std::sync::Arc::new(cell);
    let rs = (0..readers).map(|_| cell.clone()).collect();
    (cell, rs)
}
