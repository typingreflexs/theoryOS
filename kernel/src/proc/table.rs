use spin::Mutex;

use crate::proc::id::{Pid, Tid};
use crate::proc::pcb::Process;
use crate::proc::thread::Thread;

pub const MAX_PROCESSES: usize = 256;
pub const MAX_THREADS: usize = 1024;

static PROCESSES: Mutex<[Option<Process>; MAX_PROCESSES]> = Mutex::new([const { None }; MAX_PROCESSES]);
static THREADS: Mutex<[Option<Thread>; MAX_THREADS]> = Mutex::new([const { None }; MAX_THREADS]);

pub fn init() {
    let mut processes = PROCESSES.lock();
    processes[0] = Some(Process::kernel());
}

pub fn insert_process(process: Process) -> Option<Pid> {
    let pid = process.pid.as_u32() as usize;
    if pid >= MAX_PROCESSES {
        return None;
    }
    let mut processes = PROCESSES.lock();
    if processes[pid].is_some() {
        return None;
    }
    let id = process.pid;
    processes[pid] = Some(process);
    Some(id)
}

pub fn insert_thread(thread: Thread) -> Option<Tid> {
    let tid = thread.tid.as_u32() as usize;
    if tid >= MAX_THREADS {
        return None;
    }
    let mut threads = THREADS.lock();
    if threads[tid].is_some() {
        return None;
    }
    let id = thread.tid;
    threads[tid] = Some(thread);
    Some(id)
}

/// Construct a thread directly in the global table to avoid large on-stack moves.
pub fn emplace_thread(tid: Tid, build: impl FnOnce(Tid) -> Thread) -> Option<Tid> {
    let idx = tid.as_u32() as usize;
    if idx >= MAX_THREADS {
        return None;
    }
    let mut threads = THREADS.lock();
    if threads[idx].is_some() {
        return None;
    }
    threads[idx] = Some(build(tid));
    Some(tid)
}

pub fn with_process<F, R>(pid: Pid, f: F) -> Option<R>
where
    F: FnOnce(&Process) -> R,
{
    let idx = pid.as_u32() as usize;
    if idx >= MAX_PROCESSES {
        return None;
    }
    PROCESSES.lock()[idx].as_ref().map(f)
}

pub fn with_process_mut<F, R>(pid: Pid, f: F) -> Option<R>
where
    F: FnOnce(&mut Process) -> R,
{
    let idx = pid.as_u32() as usize;
    if idx >= MAX_PROCESSES {
        return None;
    }
    PROCESSES.lock()[idx].as_mut().map(f)
}

pub fn with_thread<F, R>(tid: Tid, f: F) -> Option<R>
where
    F: FnOnce(&Thread) -> R,
{
    let idx = tid.as_u32() as usize;
    if idx >= MAX_THREADS {
        return None;
    }
    THREADS.lock()[idx].as_ref().map(f)
}

pub fn with_thread_mut<F, R>(tid: Tid, f: F) -> Option<R>
where
    F: FnOnce(&mut Thread) -> R,
{
    let idx = tid.as_u32() as usize;
    if idx >= MAX_THREADS {
        return None;
    }
    THREADS.lock()[idx].as_mut().map(f)
}

pub fn lock_threads() -> spin::MutexGuard<'static, [Option<Thread>; MAX_THREADS]> {
    THREADS.lock()
}

pub fn thread_count() -> usize {
    THREADS.lock().iter().filter(|t| t.is_some()).count()
}

pub fn process_count() -> usize {
    PROCESSES.lock().iter().filter(|p| p.is_some()).count()
}
