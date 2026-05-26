use core::mem;

use crate::arch::memory::{phys_to_virt, PhysAddr, VirtAddr};

use super::dsdt::Dsdt;
use super::fadt::Fadt;
use super::madt::Madt;
use super::srat::Srat;

#[derive(Clone, Copy, Debug)]
pub struct RsdpLocation {
    raw: u64,
}

impl RsdpLocation {
    pub const fn from_limine(raw: u64) -> Self {
        Self { raw }
    }

    pub fn resolve_virt(&self, hhdm: u64) -> crate::arch::memory::VirtAddr {
        if self.raw >= hhdm {
            crate::arch::memory::VirtAddr::new(self.raw)
        } else {
            phys_to_virt(hhdm, PhysAddr::new(self.raw))
        }
    }

    pub fn virt(&self) -> crate::arch::memory::VirtAddr {
        self.resolve_virt(crate::boot_info().hhdm_offset)
    }

    pub fn address(&self) -> PhysAddr {
        let hhdm = crate::boot_info().hhdm_offset;
        if self.raw >= hhdm {
            PhysAddr::new(self.raw - hhdm)
        } else {
            PhysAddr::new(self.raw)
        }
    }
}

fn acpi_ptr(hhdm: u64, addr: u64) -> VirtAddr {
    if addr >= hhdm {
        VirtAddr::new(addr)
    } else {
        phys_to_virt(hhdm, PhysAddr::new(addr))
    }
}

#[derive(Debug)]
pub struct AcpiTables {
    hhdm: u64,
    rsdp: RsdpLocation,
    madt: Madt,
    fadt: Option<Fadt>,
    dsdt: Option<Dsdt>,
    srat: Option<Srat>,
}

impl AcpiTables {
    pub fn from_rsdp(rsdp: RsdpLocation) -> Option<Self> {
        let hhdm = crate::boot_info().hhdm_offset;
        let rsdp_virt = rsdp.resolve_virt(hhdm);

        let xsdt = locate_xsdt(hhdm, rsdp_virt);
        let madt = find_table(hhdm, xsdt, b"APIC").map(Madt::parse)?;
        let fadt = find_table(hhdm, xsdt, b"FACP").map(|addr| Fadt::parse(hhdm, addr));
        let dsdt = fadt
            .as_ref()
            .and_then(|f| f.dsdt_address())
            .map(|addr| Dsdt::parse(hhdm, addr));
        let srat = find_table(hhdm, xsdt, b"SRAT").map(Srat::parse);

        Some(Self {
            hhdm,
            rsdp,
            madt,
            fadt,
            dsdt,
            srat,
        })
    }

    pub fn fallback(bsp_lapic_id: u32, rsdp: Option<RsdpLocation>) -> Self {
        let hhdm = crate::boot_info().hhdm_offset;
        Self {
            hhdm,
            rsdp: rsdp.unwrap_or(RsdpLocation::from_limine(0)),
            madt: Madt::qemu_default(bsp_lapic_id),
            fadt: None,
            dsdt: None,
            srat: None,
        }
    }

    pub fn madt(&self) -> &Madt {
        &self.madt
    }

    pub fn fadt(&self) -> Option<&Fadt> {
        self.fadt.as_ref()
    }

    pub fn dsdt(&self) -> Option<&Dsdt> {
        self.dsdt.as_ref()
    }

    pub fn srat(&self) -> Option<&Srat> {
        self.srat.as_ref()
    }
}

#[repr(C, packed)]
struct RsdpV2 {
    signature: [u8; 8],
    checksum: u8,
    oem_id: [u8; 6],
    revision: u8,
    rsdt_address: u32,
    length: u32,
    xsdt_address: u64,
    extended_checksum: u8,
    reserved: [u8; 3],
}

#[repr(C, packed)]
struct SdtHeader {
    signature: [u8; 4],
    length: u32,
    revision: u8,
    checksum: u8,
    oem_id: [u8; 6],
    oem_table_id: [u8; 8],
    oem_revision: u32,
    creator_id: u32,
    creator_revision: u32,
}

#[repr(C, packed)]
struct Rsdt {
    header: SdtHeader,
    entries: [u32; 0],
}

#[repr(C, packed)]
struct Xsdt {
    header: SdtHeader,
    entries: [u64; 0],
}

fn locate_xsdt(hhdm: u64, rsdp_virt: VirtAddr) -> VirtAddr {
    unsafe {
        let rsdp = &*(rsdp_virt.as_ptr::<RsdpV2>());
        if rsdp.revision >= 2 && rsdp.xsdt_address != 0 {
            return acpi_ptr(hhdm, rsdp.xsdt_address);
        }
        acpi_ptr(hhdm, rsdp.rsdt_address as u64)
    }
}

const MAX_ACPI_TABLE_LEN: u32 = 1 << 20; // 1 MiB — guard corrupt firmware lengths

fn find_table(hhdm: u64, sdt: VirtAddr, signature: &[u8; 4]) -> Option<VirtAddr> {
    unsafe {
        let header = &*(sdt.as_ptr::<SdtHeader>());
        if header.length < mem::size_of::<SdtHeader>() as u32
            || header.length > MAX_ACPI_TABLE_LEN
        {
            return None;
        }

        let root_sig =
            core::ptr::read_unaligned(core::ptr::addr_of!((*sdt.as_ptr::<SdtHeader>()).signature));
        let is_xsdt = root_sig == *b"XSDT";
        let entry_count = ((header.length as usize - mem::size_of::<SdtHeader>())
            / if is_xsdt { 8 } else { 4 })
            .min(64);

        let entries_ptr = sdt.as_ptr::<u8>().add(mem::size_of::<SdtHeader>());

        for i in 0..entry_count {
            let phys = if is_xsdt {
                let entry = entries_ptr.add(i * 8) as *const u64;
                PhysAddr::new(*entry)
            } else {
                let entry = entries_ptr.add(i * 4) as *const u32;
                PhysAddr::new(*entry as u64)
            };

            let addr = phys.as_u64();
            if addr == 0 {
                continue;
            }

            let virt = acpi_ptr(hhdm, addr);
            let hdr = &*(virt.as_ptr::<SdtHeader>());
            if hdr.length < mem::size_of::<SdtHeader>() as u32
                || hdr.length > MAX_ACPI_TABLE_LEN
            {
                continue;
            }
            let sig =
                core::ptr::read_unaligned(core::ptr::addr_of!((*virt.as_ptr::<SdtHeader>()).signature));
            if sig == *signature {
                return Some(virt);
            }
        }
    }
    None
}

fn validate_table(hhdm: u64, phys: PhysAddr) -> bool {
    let virt = phys_to_virt(hhdm, phys);
    unsafe {
        let header = &*(virt.as_ptr::<SdtHeader>());
        if header.length < mem::size_of::<SdtHeader>() as u32
            || header.length > MAX_ACPI_TABLE_LEN
        {
            return false;
        }
        let slice = core::slice::from_raw_parts(virt.as_ptr::<u8>(), header.length as usize);
        acpi_checksum(slice)
    }
}

fn acpi_checksum(data: &[u8]) -> bool {
    data.iter().fold(0u8, |acc, b| acc.wrapping_add(*b)) == 0
}
