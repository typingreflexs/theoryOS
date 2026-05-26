pub mod entry;

use crate::acpi::AcpiContext;
use crate::arch::traits::Arch;
use crate::console::Console;

pub struct AArch64;

impl Arch for AArch64 {
    fn early_init() {
        Console::println("[aarch64] early init (stub — port in progress)");
    }

    fn interrupt_init() {
        Console::println("[aarch64] interrupt init (stub)");
    }

    fn apic_init() {
        Console::println("[aarch64] GIC init (stub)");
    }

    fn smp_init() -> ! {
        Console::println("[aarch64] SMP init (stub)");
        Self::halt_forever()
    }

    fn halt_forever() -> ! {
        loop {
            core::arch::asm!("wfi", options(nomem, nostack));
        }
    }

    fn current_cpu_id() -> u32 {
        0
    }

    fn bsp_lapic_id() -> u32 {
        0
    }

    fn init_per_cpu(_cpu_index: u32, _lapic_id: u32) {}

    fn load_acpi_context(_ctx: &'static AcpiContext) {}
}
