use crate::fs::fd::FdTable;
use crate::ipc::wait::WaitQueue;
use crate::mm::address_space::{AddressSpace, AddressSpaceId};
use crate::mm::layout::USER_HEAP_BASE;
use crate::proc::id::{Pid, Tid};
use crate::security::{Credentials, SeccompFilter};
use crate::signal::types::SignalState;

pub const MAX_THREADS_PER_PROCESS: usize = 64;
pub const MAX_CHILDREN: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProcessState {
    Created,
    Running,
    Sleeping,
    Zombie,
    Dead,
}

bitflags::bitflags! {
    #[derive(Clone, Copy, Debug)]
    pub struct ProcessFlags: u32 {
        const NONE = 0;
        const KERNEL = 1 << 0;
        const TRACED = 1 << 1;
        const NO_ASLR = 1 << 2;
    }
}

/// Process control block — full process state.
#[derive(Debug)]
pub struct Process {
    pub pid: Pid,
    pub parent: Pid,
    pub state: ProcessState,
    pub exit_code: i32,
    pub nice: i8,
    pub flags: ProcessFlags,
    pub address_space: Option<AddressSpace>,
    pub address_space_id: AddressSpaceId,
    pub main_thread: Tid,
    pub threads: [Tid; MAX_THREADS_PER_PROCESS],
    pub thread_count: usize,
    pub cpu_time_ns: u64,
    pub brk: u64,
    pub brk_limit: u64,
    pub cwd: [u8; 256],
    pub fds: FdTable,
    pub cred: Credentials,
    pub seccomp: SeccompFilter,
    pub signals: SignalState,
    pub children: [Pid; MAX_CHILDREN],
    pub child_count: usize,
    pub child_wait: WaitQueue,
}

impl Process {
    pub fn kernel() -> Self {
        Self {
            pid: Pid::KERNEL,
            parent: Pid::KERNEL,
            state: ProcessState::Running,
            exit_code: 0,
            nice: 0,
            flags: ProcessFlags::KERNEL,
            address_space: None,
            address_space_id: AddressSpaceId::new(0),
            main_thread: Tid::INVALID,
            threads: [Tid::INVALID; MAX_THREADS_PER_PROCESS],
            thread_count: 0,
            cpu_time_ns: 0,
            brk: USER_HEAP_BASE,
            brk_limit: USER_HEAP_BASE + 128 * 1024 * 1024,
            cwd: {
                let mut c = [0u8; 256];
                c[0] = b'/';
                c
            },
            fds: FdTable::new(),
            cred: Credentials::root(),
            seccomp: SeccompFilter::disabled(),
            signals: SignalState::new(),
            children: [Pid::INVALID; MAX_CHILDREN],
            child_count: 0,
            child_wait: WaitQueue::new(),
        }
    }

    pub fn new_user(pid: Pid, parent: Pid, address_space: AddressSpace) -> Self {
        let as_id = address_space.id;
        let mut fds = FdTable::new();
        fds.init_stdio();
        Self {
            pid,
            parent,
            state: ProcessState::Created,
            exit_code: 0,
            nice: 0,
            flags: ProcessFlags::NONE,
            address_space: Some(address_space),
            address_space_id: as_id,
            main_thread: Tid::INVALID,
            threads: [Tid::INVALID; MAX_THREADS_PER_PROCESS],
            thread_count: 0,
            cpu_time_ns: 0,
            brk: USER_HEAP_BASE,
            brk_limit: USER_HEAP_BASE + 128 * 1024 * 1024,
            cwd: {
                let mut c = [0u8; 256];
                c[0] = b'/';
                c
            },
            fds,
            cred: Credentials::user(),
            seccomp: SeccompFilter::disabled(),
            signals: SignalState::new(),
            children: [Pid::INVALID; MAX_CHILDREN],
            child_count: 0,
            child_wait: WaitQueue::new(),
        }
    }

    pub fn add_thread(&mut self, tid: Tid) -> bool {
        if self.thread_count >= MAX_THREADS_PER_PROCESS {
            return false;
        }
        self.threads[self.thread_count] = tid;
        self.thread_count += 1;
        if !self.main_thread.is_valid() {
            self.main_thread = tid;
        }
        true
    }

    pub fn add_child(&mut self, child: Pid) -> bool {
        if self.child_count >= MAX_CHILDREN {
            return false;
        }
        self.children[self.child_count] = child;
        self.child_count += 1;
        true
    }

    pub fn address_space_mut(&mut self) -> Option<&mut AddressSpace> {
        self.address_space.as_mut()
    }
}
