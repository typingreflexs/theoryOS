//! Process and thread management — PCB, TCB, fork/exec/exit, capability checks.
//!
//! `table.rs` holds static process/thread tables; `exec.rs` loads ELF binaries;
//! `current_tid()` tracks the running thread per CPU.

pub mod exec;
pub mod id;
pub mod pcb;
pub mod table;
pub mod thread;

use core::sync::atomic::{AtomicU32, Ordering};

use crate::arch::x86_64::smp;

pub use id::{alloc_pid, alloc_tid, Pid, Tid};
pub use pcb::{Process, ProcessFlags, ProcessState};
pub use thread::{Thread, ThreadFlags, ThreadState};

static CURRENT_THREADS: [AtomicU32; 256] = [const { AtomicU32::new(u32::MAX) }; 256];

pub fn init() {
    table::init();
}

pub fn current_tid() -> Tid {
    let cpu = smp::current_cpu_index() as usize;
    Tid::new(CURRENT_THREADS[cpu].load(Ordering::Acquire))
}

pub fn set_current_tid(cpu: u32, tid: Tid) {
    CURRENT_THREADS[cpu as usize].store(tid.as_u32(), Ordering::Release);
}

pub fn current_thread<F, R>(f: F) -> Option<R>
where
    F: FnOnce(&Thread) -> R,
{
    table::with_thread(current_tid(), f)
}

pub fn current_thread_mut<F, R>(f: F) -> Option<R>
where
    F: FnOnce(&mut Thread) -> R,
{
    table::with_thread_mut(current_tid(), f)
}

pub fn current_process_mut<F, R>(f: F) -> Option<R>
where
    F: FnOnce(&mut Process) -> R,
{
    let tid = current_tid();
    let pid = table::with_thread(tid, |t| t.pid)?;
    table::with_process_mut(pid, f)
}

pub fn capable(cap: crate::security::CapSet) -> bool {
    current_process_mut(|p| p.cred.capable(cap)).unwrap_or(false)
}

pub fn spawn_init_process() -> Option<Tid> {
    exec::spawn_init()
}

pub fn with_current_user_address_space<F, R>(f: F) -> Option<R>
where
    F: FnOnce(&mut crate::mm::address_space::AddressSpace) -> R,
{
    let tid = current_tid();
    let pid = table::with_thread(tid, |t| t.pid)?;
    if pid == Pid::KERNEL {
        return None;
    }
    table::with_process_mut(pid, |p| p.address_space.as_mut().map(|space| f(space)))?
}

pub fn spawn_kernel_thread(entry: extern "C" fn() -> !, flags: ThreadFlags) -> Option<Tid> {
    let tid = alloc_tid();
    let pid = Pid::KERNEL;
    table::emplace_thread(tid, |tid| Thread::new_kernel(tid, pid, entry, flags))?;
    table::with_process_mut(pid, |p| {
        p.add_thread(tid);
    })?;
    Some(tid)
}

static PREEMPT_COUNT: AtomicU32 = AtomicU32::new(0);

pub fn preempt_disable() {
    PREEMPT_COUNT.fetch_add(1, Ordering::Relaxed);
}

pub fn preempt_enable() {
    PREEMPT_COUNT.fetch_sub(1, Ordering::Relaxed);
}

pub fn preempt_enabled() -> bool {
    PREEMPT_COUNT.load(Ordering::Relaxed) == 0
}
