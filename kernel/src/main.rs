//! Kernel binary entry — delegates to architecture-specific `kernel_entry`.
//!
//! Limine loads this ELF and jumps to `_start`, which calls into
//! `arch::x86_64::entry` (or aarch64) to set up HHDM and call `kernel_main`.

#![no_std]
#![no_main]

#[no_mangle]
pub extern "C" fn _start() -> ! {
    #[cfg(target_arch = "x86_64")]
    {
        theory_kernel::arch::x86_64::entry::kernel_entry()
    }
    #[cfg(target_arch = "aarch64")]
    {
        theory_kernel::arch::aarch64::entry::kernel_entry()
    }
}
