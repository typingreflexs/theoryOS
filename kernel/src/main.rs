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
