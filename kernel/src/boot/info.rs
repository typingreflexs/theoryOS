use crate::acpi::{AcpiTables, RsdpLocation};
use crate::arch::memory::PhysAddr;
use crate::boot::limine::FramebufferInfo;

/// Normalized boot information independent of the bootloader protocol.
#[derive(Clone, Debug)]
pub struct BootInfo {
    pub hhdm_offset: u64,
    pub kernel_physical_base: u64,
    pub rsdp: Option<RsdpLocation>,
    pub memory_map: &'static [MemoryRegion],
    pub cmdline: Option<&'static str>,
    pub bsp_lapic_id: u32,
    pub framebuffer: Option<FramebufferInfo>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemoryKind {
    Usable,
    Reserved,
    AcpiReclaimable,
    AcpiNvs,
    BadMemory,
    BootloaderReclaimable,
    Unknown,
}

#[derive(Clone, Copy, Debug)]
pub struct MemoryRegion {
    pub start: PhysAddr,
    pub length: u64,
    pub kind: MemoryKind,
}

impl BootInfo {
    pub fn from_limine(raw: super::limine::LimineBootInfo) -> Self {
        Self {
            hhdm_offset: raw.hhdm_offset,
            kernel_physical_base: raw.kernel_physical_base,
            rsdp: raw.rsdp,
            memory_map: raw.memory_map,
            cmdline: raw.cmdline,
            bsp_lapic_id: raw.bsp_lapic_id,
            framebuffer: raw.framebuffer,
        }
    }

    pub fn phys_to_virt(&self, phys: PhysAddr) -> u64 {
        phys.as_u64().wrapping_add(self.hhdm_offset)
    }

    pub fn virt_to_phys(&self, virt: u64) -> Option<PhysAddr> {
        if virt >= self.hhdm_offset {
            Some(PhysAddr::new(virt - self.hhdm_offset))
        } else {
            None
        }
    }

    pub fn acpi_tables(&self) -> Option<AcpiTables> {
        self.rsdp.and_then(AcpiTables::from_rsdp)
    }
}
