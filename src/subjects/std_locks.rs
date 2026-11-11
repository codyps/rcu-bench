//! What people reach for first.

use crate::{Subject, Val};
use std::sync::{Arc, Mutex, RwLock};

/// `Arc<RwLock<T>>`: read in place under the lock.
pub struct RwLockVal;

impl Subject for RwLockVal {
    const NAME: &'static str = "std-rwlock";
    type Writer = Arc<RwLock<Val>>;
    type Reader = Arc<RwLock<Val>>;

    fn new(init: Val, readers: usize) -> (Self::Writer, Vec<Self::Reader>) {
        super::share(RwLock::new(init), readers)
    }

    #[inline]
    fn read(r: &mut Self::Reader) -> u64 {
        r.read().unwrap().get()
    }

    #[inline]
    fn write(w: &mut Self::Writer, v: Val) {
        *w.write().unwrap() = v;
    }
}

/// `RwLock<Arc<T>>`: clone the `Arc` under the lock, read outside of it.
pub struct RwLockArc;

impl Subject for RwLockArc {
    const NAME: &'static str = "std-rwlock-arc";
    type Writer = Arc<RwLock<Arc<Val>>>;
    type Reader = Arc<RwLock<Arc<Val>>>;

    fn new(init: Val, readers: usize) -> (Self::Writer, Vec<Self::Reader>) {
        super::share(RwLock::new(Arc::new(init)), readers)
    }

    #[inline]
    fn read(r: &mut Self::Reader) -> u64 {
        let v = r.read().unwrap().clone();
        v.get()
    }

    #[inline]
    fn write(w: &mut Self::Writer, v: Val) {
        let old = std::mem::replace(&mut *w.write().unwrap(), Arc::new(v));
        drop(old);
    }
}

/// `Mutex<Arc<T>>`: clone the `Arc` under the lock, read outside of it.
pub struct MutexArc;

impl Subject for MutexArc {
    const NAME: &'static str = "std-mutex-arc";
    type Writer = Arc<Mutex<Arc<Val>>>;
    type Reader = Arc<Mutex<Arc<Val>>>;

    fn new(init: Val, readers: usize) -> (Self::Writer, Vec<Self::Reader>) {
        super::share(Mutex::new(Arc::new(init)), readers)
    }

    #[inline]
    fn read(r: &mut Self::Reader) -> u64 {
        let v = r.lock().unwrap().clone();
        v.get()
    }

    #[inline]
    fn write(w: &mut Self::Writer, v: Val) {
        let old = std::mem::replace(&mut *w.lock().unwrap(), Arc::new(v));
        drop(old);
    }
}
