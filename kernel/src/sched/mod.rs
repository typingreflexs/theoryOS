pub mod cfs;
pub mod idle;
pub mod rbtree;
pub mod runqueue;
pub mod timer;

use core::sync::atomic::{AtomicBool, Ordering};

use crate::arch::x86_64::context::{restore_thread_context, save_thread_context, switch_context};
use crate::arch::x86_64::cpu;
use crate::arch::x86_64::interrupts::InterruptFrame;
use crate::arch::x86_64::smp;
use crate::arch::x86_64::tss;
use crate::console::Console;
use crate::mm::address_space::KernelAddressSpace;
use crate::proc::{self, id::{Pid, Tid}};
use crate::proc::table;
use crate::proc::thread::{ThreadFlags, ThreadState};

static NEED_RESCHED: AtomicBool = AtomicBool::new(false);
static SCHED_READY: AtomicBool = AtomicBool::new(false);

pub fn init() {
    proc::init();
    crate::arch::x86_64::xsave::init();

    let cpu_count = smp::cpu_count().max(1);
    for cpu in 0..cpu_count {
        idle::create_idle_thread(cpu as u32);
    }

    timer::init();
    SCHED_READY.store(true, Ordering::Release);
    Console::println("[sched] CFS scheduler initialized");
}

pub fn need_resched() -> bool {
    NEED_RESCHED.load(Ordering::Acquire)
}

pub fn set_need_resched() {
    NEED_RESCHED.store(true, Ordering::Release);
}

pub fn take_need_resched() -> bool {
    NEED_RESCHED.swap(false, Ordering::AcqRel)
}

pub fn start_cpu(cpu: u32) -> ! {
    let idle = runqueue::with_runqueue(cpu, |rq| rq.idle);
    proc::set_current_tid(cpu, idle);
    runqueue::set_current(cpu, idle);
    if let Some(top) = proc::table::with_thread(idle, |t| t.kernel_stack_top) {
        tss::set_rsp0(cpu, top);
        crate::arch::x86_64::syscall::set_kstack(cpu, top);
    }

    Console::println("[boot] Theory OS ready");
    // APIC timer + external IRQs stay off on this path; idle drives the UI clock via TSC.

    let next = runqueue::pick_next_thread(cpu);
    if next.is_valid() && next != idle {
        switch_to(cpu, idle, next, core::ptr::null_mut());
    }

    idle::theory_idle_thread()
}

#[no_mangle]
extern "C" fn init_thread_main() -> ! {
    Console::println("[sched] init thread running on CPU 0");
    crate::net::late_init();
    loop {
        crate::net::rx_poll();
        cpu::pause();
        reschedule();
    }
}

fn idle_loop_forever(_cpu: u32) -> ! {
    loop {
        cpu::pause();
    }
}

pub fn reschedule() {
    if !proc::preempt_enabled() {
        return;
    }
    let cpu = smp::current_cpu_index();
    let next = runqueue::pick_next_thread(cpu);
    if !next.is_valid() {
        return;
    }
    let curr = proc::current_tid();
    if curr == next {
        runqueue::enqueue_thread(next, cpu);
        return;
    }
    switch_to(cpu, curr, next, core::ptr::null_mut());
}

/// Preemptive reschedule from interrupt context — rewrites the frame in place.
pub unsafe fn preempt(frame: *mut InterruptFrame) {
    if !take_need_resched() || !proc::preempt_enabled() {
        return;
    }

    let cpu = smp::current_cpu_index();
    let curr = proc::current_tid();

    // Save current thread state from interrupt frame
    proc::table::with_thread_mut(curr, |t| {
        save_thread_context(&mut t.context, &mut t.extended, &*frame);
        if t.state == ThreadState::Running && !t.is_idle() {
            t.state = ThreadState::Runnable;
            runqueue::enqueue_thread(curr, cpu);
        }
    });

    let next = runqueue::pick_next_thread(cpu);
    if !next.is_valid() || next == curr {
        return;
    }

    switch_to(cpu, curr, next, frame);
}

fn switch_to(cpu: u32, prev: Tid, next: Tid, frame: *mut InterruptFrame) {
    proc::set_current_tid(cpu, next);
    runqueue::set_current(cpu, next);

    let pid = table::with_thread(next, |t| t.pid).unwrap_or(Pid::KERNEL);
    if pid == Pid::KERNEL {
        let mut kernel = KernelAddressSpace::get();
        kernel.activate();
    } else {
        table::with_process_mut(pid, |p| {
            if let Some(ref space) = p.address_space {
                space.activate();
            }
        });
    }

    if let Some(rsp0) = table::with_thread(next, |t| t.kernel_stack_top) {
        tss::set_rsp0(cpu, rsp0);
        crate::arch::x86_64::syscall::set_kstack(cpu, rsp0);
    }

    // Restore per-thread TLS (FS base MSR) for user threads.
    if let Some(fs_base) = table::with_thread(next, |t| t.fs_base) {
        if table::with_thread(next, |t| t.is_user()).unwrap_or(false) {
            cpu::write_msr(0xC000_0100, fs_base);
        }
    }

    proc::table::with_thread_mut(next, |t| {
        t.state = ThreadState::Running;
    });

    if frame.is_null() {
        cpu::disable_interrupts();
        let mut threads = table::lock_threads();
        let prev_idx = prev.as_u32() as usize;
        let next_idx = next.as_u32() as usize;
        unsafe {
            let prev_ctx = &mut threads[prev_idx].as_mut().unwrap().context as *mut _;
            let next_ctx = &threads[next_idx].as_ref().unwrap().context as *const _;
            switch_context(&mut *prev_ctx, &*next_ctx);
        }
    } else {
        unsafe {
            if let Some(t) = proc::table::with_thread_mut(next, |t| {
                restore_thread_context(&t.context, &t.extended, &mut *frame);
            }) {
                let _ = t;
            }
        }
    }
}

pub fn yield_cpu() {
    timer::disable_preemption();
    set_need_resched();
    reschedule();
    timer::enable_preemption();
}

pub fn block_current() {
    let cpu = smp::current_cpu_index();
    let curr = proc::current_tid();
    proc::table::with_thread_mut(curr, |t| {
        t.state = ThreadState::Blocked;
        runqueue::with_runqueue(cpu, |rq| rq.dequeue(curr));
    });
    yield_cpu();
}

pub fn wake_thread(tid: Tid) {
    if let Some(cpu) = proc::table::with_thread(tid, |t| t.cpu) {
        runqueue::enqueue_thread(tid, cpu);
        set_need_resched();
    }
}

/// Apply priority inheritance boost to mutex owner chain.
pub fn pi_boost(owner_tid: Tid, boost: u64) {
    proc::table::with_thread_mut(owner_tid, |t| {
        t.pi_boost = t.pi_boost.saturating_add(boost);
        if t.on_rq {
            let cpu = t.cpu;
            runqueue::with_runqueue(cpu, |rq| rq.dequeue(t.tid));
            runqueue::enqueue_thread(t.tid, cpu);
        }
    });
}

pub fn pi_unboost(owner_tid: Tid, boost: u64) {
    proc::table::with_thread_mut(owner_tid, |t| {
        t.pi_boost = t.pi_boost.saturating_sub(boost);
    });
}
