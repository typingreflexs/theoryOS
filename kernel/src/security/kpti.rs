//! Kernel Page Table Isolation (KPTI) — Meltdown mitigation.

use core::sync::atomic::{AtomicU64, Ordering};

use spin::Mutex;

use crate::arch::memory::PhysAddr;
use crate::arch::x86_64::cpu;
use crate::mm::paging::PageTable;
use crate::proc;

static KERNEL_CR3: AtomicU64 = AtomicU64::new(0);
static KPTI_ENABLED: AtomicU64 = AtomicU64::new(0);

static PERCPU_USER_CR3: Mutex<[u64; 256]> = Mutex::new([0; 256]);

pub fn init() {
    let cr3 = cpu::read_cr3() & !0xFFF;
    KERNEL_CR3.store(cr3, Ordering::Release);
    KPTI_ENABLED.store(1, Ordering::Release);
}

pub fn kernel_cr3() -> PhysAddr {
    PhysAddr::new(KERNEL_CR3.load(Ordering::Acquire))
}

pub fn enabled() -> bool {
    KPTI_ENABLED.load(Ordering::Acquire) != 0
}

fn current_user_cr3() -> u64 {
    let tid = proc::current_tid();
    let pid = proc::table::with_thread(tid, |t| t.pid).unwrap_or(proc::Pid::KERNEL);
    if pid == proc::Pid::KERNEL {
        return KERNEL_CR3.load(Ordering::Acquire);
    }
    proc::table::with_process(pid, |p| {
        p.address_space
            .as_ref()
            .map(|s| s.page_table.cr3.as_u64())
            .unwrap_or_else(|| KERNEL_CR3.load(Ordering::Acquire))
    })
    .unwrap_or_else(|| KERNEL_CR3.load(Ordering::Acquire))
}

/// Switch from user CR3 to kernel CR3 on syscall/exception entry.
#[no_mangle]
pub extern "C" fn theory_kpti_enter() {
    if !enabled() {
        return;
    }
    let cpu = crate::arch::x86_64::smp::current_cpu_index() as usize;
    let user_cr3 = cpu::read_cr3();
    let kernel = KERNEL_CR3.load(Ordering::Acquire);
    if user_cr3 & !0xFFF == kernel & !0xFFF {
        return;
    }
    PERCPU_USER_CR3.lock()[cpu] = user_cr3;
    cpu::write_cr3(kernel);
}

/// Restore user CR3 on return to ring 3.
#[no_mangle]
pub extern "C" fn theory_kpti_exit() {
    if !enabled() {
        return;
    }
    let cpu = crate::arch::x86_64::smp::current_cpu_index() as usize;
    let user_cr3 = PERCPU_USER_CR3.lock()[cpu];
    if user_cr3 != 0 {
        cpu::write_cr3(user_cr3);
    }
}

/// Temporarily switch to user page tables for uaccess under KPTI.
pub fn with_user_as<F, R>(f: F) -> R
where
    F: FnOnce() -> R,
{
    if !enabled() {
        return f();
    }
    let saved = cpu::read_cr3();
    let user = current_user_cr3();
    if saved & !0xFFF != user & !0xFFF {
        cpu::write_cr3(user);
    }
    let result = f();
    if saved & !0xFFF != user & !0xFFF {
        cpu::write_cr3(saved);
    }
    result
}

/// Build a user page table without kernel upper-half mappings (KPTI user view).
pub fn init_user_page_table(table: &PageTable) {
    let _ = table;
    // User tables start empty — no kernel mappings in upper canonical half.
}

/// Mirror user lower-half entries into kernel CR3 for the active process (optional fast path).
pub fn sync_user_to_kernel(_user: &PageTable, _kernel: &PageTable) {
    // Full sync deferred — uaccess uses with_user_as instead.
}
