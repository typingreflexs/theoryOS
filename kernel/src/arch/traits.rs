use crate::acpi::AcpiContext;

/// Architecture-specific operations shared across x86-64 and AArch64 ports.
pub trait Arch {
    fn early_init();
    fn interrupt_init();
    fn apic_init();
    fn smp_init() -> !;
    fn halt_forever() -> !;
    fn current_cpu_id() -> u32;
    fn bsp_lapic_id() -> u32;
    fn init_per_cpu(cpu_index: u32, lapic_id: u32);
    fn load_acpi_context(ctx: &'static AcpiContext);
}
