//! Theory OS kernel — UNIX-like OS for x86-64, booted via Limine.
//!
//! # Boot order (`kernel_main`)
//! 1. `arch::early_init` — GDT, TSS, disable legacy PIC
//! 2. `mm::init` — physical frames, paging, slab heap, page faults
//! 3. `acpi::init` — parse ACPI tables (MADT, FADT, DSDT)
//! 4. `video::init` — framebuffer desktop, PS/2 input
//! 5. `security::init` — KPTI, stack canaries, capabilities
//! 6. `fs::init` — mount VFS, populate embedded rootfs
//! 7. `ipc::init` — pipes, mq, shm, futex
//! 8. `net::init` — protocol stack (NIC probed lazily)
//! 9. `syscall::init` — SYSCALL/SYSRET MSRs
//! 10. `arch::apic_init` → `post_acpi_init` — LAPIC, SMP, scheduler, never returns
//!
//! **Author:** typingreflexs — only error fixes assisted by Cursor.

#![cfg_attr(not(test), no_std)]
#![cfg_attr(not(test), no_main)]
#![feature(abi_x86_interrupt)]
#![cfg_attr(target_arch = "x86_64", feature(naked_functions))]

extern crate alloc;

pub mod acpi;
pub mod arch;
pub mod audio;
pub mod boot;
pub mod console;
pub mod fs;
pub mod input;
pub mod ipc;
pub mod mm;
pub mod net;
pub mod panic;
pub mod proc;
pub mod sched;
pub mod security;
pub mod signal;
pub mod sync;
pub mod syscall;
pub mod video;

#[cfg(not(test))]
use core::panic::PanicInfo;

use boot::BootInfo;
use console::Console;

/// Limine boot info — set once during entry, read-only after.
static mut BOOT_INFO: Option<BootInfo> = None;

/// Immutable boot-time information (memory map, HHDM, framebuffer, etc.).
pub fn boot_info() -> &'static BootInfo {
    unsafe { BOOT_INFO.as_ref().expect("boot info not initialized") }
}

/// Central kernel entry after architecture-specific setup completes.
pub fn kernel_main(boot: BootInfo) -> ! {
    unsafe {
        BOOT_INFO = Some(boot);
    }

    Console::init();
    Console::println("Theory OS kernel starting");

    arch::early_init();   // CPU segments, TSS, serial-ready arch state
    mm::init();           // Physical memory, page tables, kernel heap
    acpi::init();         // ACPI table discovery and parsing
    #[cfg(target_arch = "x86_64")]
    arch::x86_64::rtc::init(); // Real-time clock baseline (depends on FADT)
    video::init();        // Linear framebuffer + desktop UI
    security::init();     // Hardening: KPTI, canaries, seccomp framework
    fs::init();           // VFS mounts and embedded userspace files
    ipc::init();          // Inter-process communication primitives
    net::init();          // TCP/UDP/ARP/DHCP stack (no NIC probe yet)
    audio::init();        // PC speaker + Intel HDA enumeration
    fs::block::ahci::probe(); // SATA disks (no-op if none)
    fs::block::nvme::probe(); // NVMe SSDs (no-op if none)
    syscall::init();      // Fast userspace entry via SYSCALL instruction
    arch::apic_init();    // Local APIC + IOAPIC
    arch::post_acpi_init() // SMP, scheduler, idle/UI loop — does not return
}

#[cfg(not(test))]
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    panic::handle(info)
}

#[cfg(not(test))]
#[global_allocator]
static GLOBAL_ALLOCATOR: mm::heap::KernelAllocator = mm::heap::KernelAllocator;
