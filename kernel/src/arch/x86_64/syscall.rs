//! SYSCALL/SYSRET fast path — MSR setup and register frame.

use core::sync::atomic::{AtomicU64, Ordering};

use crate::arch::x86_64::cpu;
use crate::arch::x86_64::gdt;
use crate::console::Console;

const IA32_EFER: u32 = 0xC000_0080;
const IA32_STAR: u32 = 0xC000_0081;
const IA32_LSTAR: u32 = 0xC000_0082;
const IA32_FMASK: u32 = 0xC000_0084;
const EFER_SCE: u64 = 1 << 0;

extern "C" {
    fn theory_syscall_entry();
}

/// Register frame saved on syscall entry (Linux x86_64 order + user state).
#[repr(C)]
pub struct SyscallFrame {
    pub r15: u64,
    pub r14: u64,
    pub r13: u64,
    pub r12: u64,
    pub r11: u64,
    pub r10: u64,
    pub r9: u64,
    pub r8: u64,
    pub rbp: u64,
    pub rbx: u64,
    pub rdx: u64,
    pub rsi: u64,
    pub rdi: u64,
    pub rax: u64,
    pub user_rsp: u64,
    pub user_rflags: u64,
    pub user_rip: u64,
}

static PERCPU_KSTACK: [AtomicU64; 256] = [const { AtomicU64::new(0) }; 256];

pub fn init_msrs() {
    let efer = cpu::read_msr(IA32_EFER);
    cpu::write_msr(IA32_EFER, efer | EFER_SCE);

    // STAR: bits 48..63 = sysret CS/SS, bits 32..47 = syscall CS/SS
    let kcs = gdt::kernel_code_selector() as u64;
    let kss = gdt::kernel_data_selector() as u64;
    let ucs = gdt::user_code_selector() as u64;
    let uss = gdt::user_data_selector() as u64;
    let star = ((ucs & 0xFFFC) << 48) | ((kcs & 0xFFFC) << 32) | ((kss & 0xFFFC) << 16);
    cpu::write_msr(IA32_STAR, star);

    cpu::write_msr(IA32_LSTAR, theory_syscall_entry as u64);
    // Mask IF during syscall entry
    cpu::write_msr(IA32_FMASK, 1 << 9);

    Console::println("[syscall] MSRs programmed (LSTAR/SSTAR/FMASK/EFER.SCE)");
}

pub fn set_kstack(cpu: u32, top: u64) {
    PERCPU_KSTACK[cpu as usize].store(top, Ordering::Release);
}

pub fn percpu_kstack(cpu: usize) -> u64 {
    PERCPU_KSTACK[cpu].load(Ordering::Acquire)
}
