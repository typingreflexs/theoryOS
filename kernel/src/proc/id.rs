use core::sync::atomic::{AtomicU32, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Pid(u32);

impl Pid {
    pub const KERNEL: Pid = Pid(0);
    pub const INVALID: Pid = Pid(u32::MAX);

    pub fn new(id: u32) -> Self {
        Self(id)
    }

    pub fn as_u32(self) -> u32 {
        self.0
    }

    pub fn is_valid(self) -> bool {
        self.0 != u32::MAX
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Tid(u32);

impl Tid {
    pub const INVALID: Tid = Tid(u32::MAX);

    pub fn new(id: u32) -> Self {
        Self(id)
    }

    pub fn as_u32(self) -> u32 {
        self.0
    }

    pub fn is_valid(self) -> bool {
        self.0 != u32::MAX
    }
}

static NEXT_PID: AtomicU32 = AtomicU32::new(1);
static NEXT_TID: AtomicU32 = AtomicU32::new(1);

pub fn alloc_pid() -> Pid {
    Pid(NEXT_PID.fetch_add(1, Ordering::Relaxed))
}

pub fn alloc_tid() -> Tid {
    Tid(NEXT_TID.fetch_add(1, Ordering::Relaxed))
}

pub fn seed_next_pid(next: u32) {
    NEXT_PID.store(next, Ordering::Relaxed);
}
