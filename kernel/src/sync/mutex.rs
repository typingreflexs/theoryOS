//! Priority-inheritance mutex — blocks thread and boosts owner priority on contention.

use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use spin::Mutex as SpinMutex;

use crate::proc::{self, id::Tid};
use crate::sched;

/// Mutex with priority inheritance (PI) for kernel synchronization.
pub struct PiMutex<T> {
    locked: AtomicBool,
    owner: AtomicU32,
    waiters: SpinMutex<WaitQueue>,
    data: UnsafeCell<T>,
}

struct WaitQueue {
    threads: [Tid; 32],
    len: usize,
}

impl WaitQueue {
    const fn new() -> Self {
        Self {
            threads: [Tid::INVALID; 32],
            len: 0,
        }
    }

    fn push(&mut self, tid: Tid) -> bool {
        if self.len >= 32 {
            return false;
        }
        self.threads[self.len] = tid;
        self.len += 1;
        true
    }

    fn pop(&mut self) -> Option<Tid> {
        if self.len == 0 {
            return None;
        }
        self.len -= 1;
        Some(self.threads[self.len])
    }
}

const PI_BOOST_NS: u64 = 500_000;

unsafe impl<T: Send> Send for PiMutex<T> {}
unsafe impl<T: Send> Sync for PiMutex<T> {}

impl<T> PiMutex<T> {
    pub const fn new(value: T) -> Self {
        Self {
            locked: AtomicBool::new(false),
            owner: AtomicU32::new(u32::MAX),
            waiters: SpinMutex::new(WaitQueue::new()),
            data: UnsafeCell::new(value),
        }
    }

    pub fn lock(&self) -> PiMutexGuard<'_, T> {
        loop {
            if self
                .locked
                .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
                .is_ok()
            {
                let tid = proc::current_tid();
                self.owner.store(tid.as_u32(), Ordering::Release);
                break;
            }

            let owner = Tid::new(self.owner.load(Ordering::Acquire));
            if owner.is_valid() {
                sched::pi_boost(owner, PI_BOOST_NS);
            }

            let tid = proc::current_tid();
            self.waiters.lock().push(tid);
            sched::block_current();
        }
        PiMutexGuard { mutex: self }
    }

    fn unlock(&self) {
        self.owner.store(u32::MAX, Ordering::Release);
        self.locked.store(false, Ordering::Release);

        if let Some(next) = self.waiters.lock().pop() {
            sched::wake_thread(next);
        }
    }
}

pub struct PiMutexGuard<'a, T> {
    mutex: &'a PiMutex<T>,
}

impl<T> core::ops::Deref for PiMutexGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        unsafe { &*self.mutex.data.get() }
    }
}

impl<T> core::ops::DerefMut for PiMutexGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { &mut *self.mutex.data.get() }
    }
}

impl<T> Drop for PiMutexGuard<'_, T> {
    fn drop(&mut self) {
        self.mutex.unlock();
    }
}

pub type Mutex<T> = PiMutex<T>;
