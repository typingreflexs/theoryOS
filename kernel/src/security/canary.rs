//! Kernel stack canaries — detect stack buffer overflows.

use core::sync::atomic::{AtomicU64, Ordering};

use crate::console::Console;

static STACK_CHK_GUARD: AtomicU64 = AtomicU64::new(0);

pub fn init() {
    let tsc = unsafe {
        let lo: u32;
        let hi: u32;
        core::arch::asm!("rdtsc", out("eax") lo, out("edx") hi, options(nomem, nostack));
        ((hi as u64) << 32) | lo as u64
    };
    let canary = tsc ^ 0xDEAD_BEEF_CAFE_BABE ^ crate::arch::x86_64::cpu::read_cr3();
    STACK_CHK_GUARD.store(canary, Ordering::Release);
    crate::arch::x86_64::tss::stamp_stack_canary(canary);
}

pub fn guard() -> u64 {
    STACK_CHK_GUARD.load(Ordering::Acquire)
}

pub fn verify_cpu_stack(cpu: usize) {
    crate::arch::x86_64::tss::verify_stack_canary(guard(), cpu as u32);
}

#[no_mangle]
pub extern "C" fn __stack_chk_fail() -> ! {
    Console::print("\n*** STACK SMASH DETECTED ***\n");
    crate::arch::halt_forever()
}

#[no_mangle]
pub static mut __stack_chk_guard: u64 = 0;

pub fn export_guard_to_compiler() {
    unsafe {
        __stack_chk_guard = guard();
    }
}
