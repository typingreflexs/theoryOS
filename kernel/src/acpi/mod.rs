//! ACPI table parser — RSDP discovery, MADT, FADT, DSDT, SRAT.
//!
//! Used for SMP CPU enumeration, LAPIC/IOAPIC addresses, and NUMA topology.

pub mod tables;
pub mod madt;
pub mod fadt;
pub mod dsdt;
pub mod srat;
pub mod power;

use spin::Once;

use crate::arch::traits::Arch;

pub use tables::{AcpiTables, RsdpLocation};

static ACPI: Once<AcpiContext> = Once::new();

#[derive(Debug)]
pub struct AcpiContext {
    tables: AcpiTables,
}

impl AcpiContext {
    pub fn tables(&self) -> &AcpiTables {
        &self.tables
    }

    pub fn madt(&self) -> &madt::Madt {
        self.tables.madt()
    }

    pub fn fadt(&self) -> Option<&fadt::Fadt> {
        self.tables.fadt()
    }

    pub fn dsdt(&self) -> Option<&dsdt::Dsdt> {
        self.tables.dsdt()
    }

    pub fn srat(&self) -> Option<&srat::Srat> {
        self.tables.srat()
    }

    pub fn local_apic_address(&self) -> crate::arch::memory::PhysAddr {
        self.madt().local_apic_address()
    }
}

pub fn init() {
    let boot = crate::boot_info();
    crate::console::Console::println("[acpi] init");
    let tables = AcpiTables::fallback(boot.bsp_lapic_id, boot.rsdp);
    let ctx = AcpiContext { tables };
    let _ = ACPI.call_once(|| ctx);
    crate::arch::Current::load_acpi_context(ACPI.get().unwrap());
    crate::console::Console::println("[acpi] tables parsed (MADT/FADT/DSDT)");
}

pub fn context() -> Option<&'static AcpiContext> {
    ACPI.get()
}
