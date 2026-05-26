//! Thread wait queues for blocking IPC primitives.

use crate::proc::id::Tid;
use crate::sched;

#[derive(Debug)]
pub struct WaitQueue {
    threads: [Tid; 32],
    len: usize,
}

impl WaitQueue {
    pub const fn new() -> Self {
        Self {
            threads: [Tid::INVALID; 32],
            len: 0,
        }
    }

    pub fn push(&mut self, tid: Tid) -> bool {
        if self.len >= 32 {
            return false;
        }
        self.threads[self.len] = tid;
        self.len += 1;
        true
    }

    pub fn pop(&mut self) -> Option<Tid> {
        if self.len == 0 {
            return None;
        }
        self.len -= 1;
        Some(self.threads[self.len])
    }

    pub fn wake_one(&mut self) {
        if let Some(tid) = self.pop() {
            sched::wake_thread(tid);
        }
    }

    pub fn wake_all(&mut self) {
        while let Some(tid) = self.pop() {
            sched::wake_thread(tid);
        }
    }

    pub fn block_on(&mut self) {
        let tid = crate::proc::current_tid();
        self.push(tid);
        sched::block_current();
    }
}
