//! Per-CPU idle thread — polls UI input, network RX, DHCP, and browser load state.
//!
//! Runs forever on each CPU; BSP idle also updates the desktop clock via TSC.

use crate::arch::x86_64::cpu;
use crate::proc::{self, Tid};
use crate::proc::id::Pid;
use crate::proc::thread::{ThreadFlags, ThreadState};
use crate::sched::runqueue;

#[no_mangle]
pub extern "C" fn theory_idle_thread() -> ! {
    let cpu = crate::arch::x86_64::smp::current_cpu_index();
    let mut last_clock_ms = 0u64;
    loop {
        cpu::pause();
        if crate::video::is_available() {
            crate::video::poll_input();
        }
        if crate::net::device::has_device() {
            crate::net::rx_poll();
            crate::net::dhcp::tick();
        }
        crate::video::browser::tick();
        let now_ms = crate::sched::timer::monotonic_ns() / 1_000_000;
        if crate::video::is_available() && now_ms >= last_clock_ms + 500 {
            last_clock_ms = now_ms;
            crate::video::update_clock();
        }
        if runqueue::runnable_count(cpu) > 0 || crate::sched::take_need_resched() {
            crate::sched::reschedule();
        }
    }
}

pub fn create_idle_thread(cpu: u32) -> Option<Tid> {
    let tid = proc::alloc_tid();
    proc::table::emplace_thread(tid, |tid| {
        let mut thread = crate::proc::thread::Thread::new_kernel(
            tid,
            Pid::KERNEL,
            theory_idle_thread,
            ThreadFlags::KERNEL | ThreadFlags::IDLE,
        );
        thread.cpu = cpu;
        thread.state = ThreadState::Runnable;
        thread
    })?;
    runqueue::init_cpu(cpu, tid);
    Some(tid)
}
