#![cfg_attr(not(test), no_std)]
#![cfg_attr(not(test), no_main)]
#![feature(abi_x86_interrupt)]
#![cfg_attr(target_arch = "x86_64", feature(naked_functions))]

extern crate alloc;

pub mod fs;
pub mod signal;
pub mod syscall;
pub mod proc;
pub mod sched;
pub mod acpi;
pub mod arch;
pub mod boot;
pub mod console;
pub mod input;
pub mod video;
pub mod mm;
pub mod net;
pub mod panic;
pub mod ipc;
pub mod security;
pub mod sync;

#[cfg(not(test))]
use core::panic::PanicInfo;

use boot::BootInfo;
use console::Console;

static mut BOOT_INFO: Option<BootInfo> = None;

pub fn boot_info() -> &'static BootInfo {
    unsafe { BOOT_INFO.as_ref().expect("boot info not initialized") }
}

pub fn kernel_main(boot: BootInfo) -> ! {
    unsafe {
        BOOT_INFO = Some(boot);
    }

    Console::init();
    Console::println("Theory OS kernel starting");

    arch::early_init();
    mm::init();
    acpi::init();
    video::init();
    security::init();
    fs::init();
    ipc::init();
    net::init();
    syscall::init();
    arch::apic_init();
    arch::post_acpi_init()
}

#[cfg(not(test))]
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    panic::handle(info)
}

#[cfg(not(test))]
#[global_allocator]
static GLOBAL_ALLOCATOR: mm::heap::KernelAllocator = mm::heap::KernelAllocator;
