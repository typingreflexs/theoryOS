use crate::arch::x86_64::context::CpuContext;
use crate::arch::x86_64::xsave::ExtendedState;
use crate::proc::id::{Pid, Tid};
use crate::sched::rbtree::RbNodeId;

pub const THREAD_KERNEL_STACK_SIZE: usize = 16384;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThreadState {
    Created,
    Runnable,
    Running,
    Sleeping,
    Blocked,
    Zombie,
    Dead,
}

bitflags::bitflags! {
    #[derive(Clone, Copy, Debug)]
    pub struct ThreadFlags: u32 {
        const NONE = 0;
        const KERNEL = 1 << 0;
        const USER = 1 << 1;
        const IDLE = 1 << 2;
        const NEED_RESCHED = 1 << 3;
        const PREEMPT_COUNT = 1 << 4;
    }
}

/// Thread control block — per-thread scheduling and CPU state.
#[derive(Debug)]
pub struct Thread {
    pub tid: Tid,
    pub pid: Pid,
    pub state: ThreadState,
    pub flags: ThreadFlags,
    pub cpu: u32,
    pub nice: i8,
    pub weight: u64,
    pub vruntime: u64,
    pub exec_start: u64,
    pub sum_exec_runtime: u64,
    pub pi_boost: u64,
    pub context: CpuContext,
    pub extended: ExtendedState,
    pub kernel_stack: [u8; THREAD_KERNEL_STACK_SIZE],
    pub kernel_stack_top: u64,
    pub rb_node: RbNodeId,
    pub on_rq: bool,
    pub blocking_mutex: Option<u32>,
    /// x86_64 FS segment base — TLS for musl/pthreads (`arch_prctl(ARCH_SET_FS)`).
    pub fs_base: u64,
    /// User pointer written on thread exit for futex wake (`set_tid_address`).
    pub clear_child_tid: u64,
}

impl Thread {
    pub fn new_user(tid: Tid, pid: Pid, entry: u64, user_stack: u64) -> Self {
        let mut thread = Self {
            tid,
            pid,
            state: ThreadState::Created,
            flags: ThreadFlags::USER,
            cpu: 0,
            nice: 0,
            weight: crate::sched::cfs::weight_for_nice(0),
            vruntime: 0,
            exec_start: 0,
            sum_exec_runtime: 0,
            pi_boost: 0,
            context: CpuContext::empty(),
            extended: ExtendedState::zeroed(),
            kernel_stack: [0; THREAD_KERNEL_STACK_SIZE],
            kernel_stack_top: 0,
            rb_node: RbNodeId::INVALID,
            on_rq: false,
            blocking_mutex: None,
            fs_base: 0,
            clear_child_tid: 0,
        };
        thread.kernel_stack_top = unsafe {
            thread.kernel_stack.as_mut_ptr().add(THREAD_KERNEL_STACK_SIZE) as u64
        };
        thread.context = CpuContext::init_user_thread(user_stack, entry);
        thread
    }

    pub fn new_kernel(tid: Tid, pid: Pid, entry: extern "C" fn() -> !, flags: ThreadFlags) -> Self {
        let mut thread = Self {
            tid,
            pid,
            state: ThreadState::Created,
            flags: flags | ThreadFlags::KERNEL,
            cpu: 0,
            nice: 0,
            weight: crate::sched::cfs::weight_for_nice(0),
            vruntime: 0,
            exec_start: 0,
            sum_exec_runtime: 0,
            pi_boost: 0,
            context: CpuContext::empty(),
            extended: ExtendedState::zeroed(),
            kernel_stack: [0; THREAD_KERNEL_STACK_SIZE],
            kernel_stack_top: 0,
            rb_node: RbNodeId::INVALID,
            on_rq: false,
            blocking_mutex: None,
            fs_base: 0,
            clear_child_tid: 0,
        };
        thread.kernel_stack_top = unsafe {
            thread.kernel_stack.as_mut_ptr().add(THREAD_KERNEL_STACK_SIZE) as u64
        };
        thread.context = CpuContext::init_kernel_thread(thread.kernel_stack_top, entry);
        thread
    }

    pub fn effective_vruntime(&self) -> u64 {
        self.vruntime.saturating_sub(self.pi_boost)
    }

    pub fn is_idle(&self) -> bool {
        self.flags.contains(ThreadFlags::IDLE)
    }

    pub fn is_kernel(&self) -> bool {
        self.flags.contains(ThreadFlags::KERNEL)
    }

    pub fn is_user(&self) -> bool {
        self.flags.contains(ThreadFlags::USER)
    }
}
