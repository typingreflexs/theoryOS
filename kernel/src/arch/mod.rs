//! Architecture port layer — dispatches to x86_64 or aarch64 backend.
//!
//! All CPU-specific code lives under `x86_64/` or `aarch64/`. The `Arch` trait
//! in `traits.rs` defines the interface for future multi-arch support.

pub mod traits;
pub mod memory;

#[cfg(target_arch = "x86_64")]
pub mod x86_64;

#[cfg(target_arch = "aarch64")]
pub mod aarch64;

pub use traits::Arch;

#[cfg(target_arch = "x86_64")]
pub use x86_64::X86_64 as Current;

#[cfg(target_arch = "aarch64")]
pub use aarch64::AArch64 as Current;

pub fn early_init() {
    Current::early_init();
}

pub fn interrupt_init() {
    Current::interrupt_init();
}

pub fn apic_init() {
    Current::apic_init();
}

pub fn post_acpi_init() -> ! {
    Current::smp_init()
}

pub fn halt_forever() -> ! {
    Current::halt_forever()
}

pub fn current_cpu_id() -> u32 {
    Current::current_cpu_id()
}

pub fn bsp_lapic_id() -> u32 {
    Current::bsp_lapic_id()
}
