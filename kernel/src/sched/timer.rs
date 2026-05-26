use core::sync::atomic::{AtomicU64, Ordering};

use crate::arch::x86_64::cpu;
use crate::arch::x86_64::interrupts::InterruptFrame;
use crate::console::Console;
use crate::proc;

static TICKS: AtomicU64 = AtomicU64::new(0);
static TSC_HZ: AtomicU64 = AtomicU64::new(1_000_000_000);

pub fn init() {
    crate::arch::x86_64::interrupts::register_irq_handler(0, timer_irq);
    calibrate_tsc();
    Console::println("[sched] APIC timer handler registered (vector 32, ~1ms tick)");
}

fn calibrate_tsc() {
    let t0 = crate::arch::x86_64::cpu::rdtsc();
    spin_delay_ms(50);
    let t1 = crate::arch::x86_64::cpu::rdtsc();
    let delta = t1.saturating_sub(t0);
    if delta > 0 {
        TSC_HZ.store(delta * 20, Ordering::Relaxed);
    }
}

fn spin_delay_ms(ms: u32) {
    for _ in 0..(ms as u64 * 100_000) {
        crate::arch::x86_64::cpu::pause();
    }
}

fn timer_irq(_frame: &InterruptFrame) {
    TICKS.fetch_add(1, Ordering::Relaxed);
}

pub fn ticks() -> u64 {
    TICKS.load(Ordering::Relaxed)
}

pub fn monotonic_ns() -> u64 {
    let ticks = TICKS.load(Ordering::Relaxed);
    if ticks > 0 {
        return ticks * 1_000_000;
    }
    let hz = TSC_HZ.load(Ordering::Relaxed).max(1);
    crate::arch::x86_64::cpu::rdtsc()
        .saturating_mul(1_000_000_000)
        / hz
}

pub fn enable_preemption() {
    cpu::enable_interrupts();
    proc::preempt_enable();
}

pub fn disable_preemption() {
    proc::preempt_disable();
    cpu::disable_interrupts();
}
