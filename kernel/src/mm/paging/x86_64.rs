use bitflags::bitflags;

use crate::arch::memory::{phys_to_virt, PhysAddr, VirtAddr};
use crate::arch::x86_64::cpu;
use crate::mm::layout::{self, PAGE_SIZE, RECURSIVE_INDEX};
use crate::mm::permissions::ProtFlags;
use crate::mm::phys;

bitflags! {
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct PageFlags: u64 {
        const PRESENT = 1 << 0;
        const WRITABLE = 1 << 1;
        const USER = 1 << 2;
        const WRITE_THROUGH = 1 << 3;
        const NO_CACHE = 1 << 4;
        const ACCESSED = 1 << 5;
        const DIRTY = 1 << 6;
        const HUGE = 1 << 7;
        const GLOBAL = 1 << 8;
        const NO_EXECUTE = 1 << 63;
        const COW = 1 << 9;
        const SOFTWARE_DEMAND = 1 << 10;
        const SOFTWARE_FILE = 1 << 11;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PhysFrame {
    pub number: u64,
}

impl PhysFrame {
    pub fn from_phys(addr: PhysAddr) -> Self {
        Self {
            number: addr.as_u64() / PAGE_SIZE,
        }
    }

    pub fn start_address(self) -> PhysAddr {
        PhysAddr::new(self.number * PAGE_SIZE)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PageTable {
    pub cr3: PhysAddr,
}

impl PageTable {
    pub fn new_empty() -> Option<Self> {
        let frame = phys::alloc_frame(crate::mm::numa::local_node())?;
        let hhdm = crate::boot_info().hhdm_offset;
        let virt = phys_to_virt(hhdm, frame.phys());
        unsafe {
            core::ptr::write_bytes(virt.as_mut_ptr::<u8>(), 0, PAGE_SIZE as usize);
        }
        Some(Self { cr3: frame.phys() })
    }

    pub fn kernel() -> Self {
        Self {
            cr3: PhysAddr::new(cpu::read_cr3() & !0xFFF),
        }
    }

    pub fn map_page(&self, virt: VirtAddr, phys: PhysAddr, flags: PageFlags) -> Result<(), MapError> {
        let entry = self.walk_mut(virt, true)?;
        if unsafe { entry.read() } & PageFlags::PRESENT.bits() != 0 {
            return Err(MapError::AlreadyMapped);
        }
        unsafe { entry.write(phys.as_u64() | flags.bits()) };
        flush_tlb_addr(virt.as_u64());
        Ok(())
    }

    pub fn map_page_demand(&self, virt: VirtAddr, flags: PageFlags) -> Result<(), MapError> {
        let entry = self.walk_mut(virt, true)?;
        let demand = (flags | PageFlags::SOFTWARE_DEMAND) & !PageFlags::PRESENT;
        unsafe { entry.write(demand.bits()) };
        Ok(())
    }

    pub fn unmap_page(&self, virt: VirtAddr) -> Result<PhysAddr, MapError> {
        let entry = self.walk_mut(virt, false)?;
        let value = unsafe { entry.read() };
        if value & PageFlags::PRESENT.bits() == 0 {
            return Err(MapError::NotMapped);
        }
        unsafe { entry.write(0) };
        flush_tlb_addr(virt.as_u64());
        Ok(PhysAddr::new(value & 0x000F_FFFF_FFFF_F000))
    }

    pub fn get_flags(&self, virt: VirtAddr) -> Option<PageFlags> {
        let entry = self.walk_mut(virt, false).ok()?;
        let value = unsafe { entry.read() };
        if value == 0 {
            return None;
        }
        Some(PageFlags::from_bits_truncate(value))
    }

    pub fn set_flags(&self, virt: VirtAddr, flags: PageFlags) -> Result<(), MapError> {
        let entry = self.walk_mut(virt, false)?;
        let value = unsafe { entry.read() };
        if value == 0 {
            return Err(MapError::NotMapped);
        }
        let phys = value & 0x000F_FFFF_FFFF_F000;
        unsafe { entry.write(phys | flags.bits()) };
        flush_tlb_addr(virt.as_u64());
        Ok(())
    }

    pub fn resolve(&self, virt: VirtAddr) -> Option<(PhysAddr, PageFlags)> {
        let entry = self.walk_mut(virt, false).ok()?;
        let value = unsafe { entry.read() };
        if value & PageFlags::PRESENT.bits() == 0 {
            return None;
        }
        Some((
            PhysAddr::new(value & 0x000F_FFFF_FFFF_F000),
            PageFlags::from_bits_truncate(value),
        ))
    }

    pub fn flags_from_prot(prot: ProtFlags, user: bool) -> PageFlags {
        let mut flags = PageFlags::PRESENT | PageFlags::ACCESSED;
        if prot.contains(ProtFlags::WRITE) {
            flags |= PageFlags::WRITABLE | PageFlags::DIRTY;
        }
        if user || prot.contains(ProtFlags::USER) {
            flags |= PageFlags::USER;
        }
        // W^X: non-executable mappings always get NX when EFER.NXE is enabled.
        if !prot.contains(ProtFlags::EXEC) {
            flags |= PageFlags::NO_EXECUTE;
        } else {
            flags &= !PageFlags::NO_EXECUTE;
        }
        flags
    }

    /// Kernel mappings default to NX unless explicitly executable.
    pub fn kernel_data_flags() -> PageFlags {
        PageFlags::PRESENT | PageFlags::WRITABLE | PageFlags::NO_EXECUTE | PageFlags::ACCESSED
    }

    fn walk_mut(&self, virt: VirtAddr, create: bool) -> Result<*mut u64, MapError> {
        let hhdm = crate::boot_info().hhdm_offset;
        let indices = page_indices(virt.as_u64());
        let mut table_phys = self.cr3;

        for level in 0..3 {
            let table_virt = phys_to_virt(hhdm, table_phys);
            let entry = unsafe { table_virt.as_mut_ptr::<u64>().add(indices.at(level)) };
            let value = unsafe { entry.read() };
            if value & PageFlags::PRESENT.bits() == 0 {
                if !create {
                    return Err(MapError::NotMapped);
                }
                let frame = phys::alloc_frame(crate::mm::numa::local_node()).ok_or(MapError::NoMemory)?;
                let new_table = phys_to_virt(hhdm, frame.phys());
                unsafe {
                    core::ptr::write_bytes(new_table.as_mut_ptr::<u8>(), 0, PAGE_SIZE as usize);
                    entry.write(
                        frame.phys().as_u64()
                            | PageFlags::PRESENT.bits()
                            | PageFlags::WRITABLE.bits()
                            | PageFlags::USER.bits(),
                    );
                }
                table_phys = frame.phys();
            } else {
                table_phys = PhysAddr::new(value & 0x000F_FFFF_FFFF_F000);
            }
        }

        let pt_virt = phys_to_virt(hhdm, table_phys);
        Ok(unsafe { pt_virt.as_mut_ptr::<u64>().add(indices.pt as usize) })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MapError {
    AlreadyMapped,
    NotMapped,
    NoMemory,
}

struct PageIndices {
    pml4: u64,
    pdpt: u64,
    pd: u64,
    pt: u64,
}

impl PageIndices {
    fn at(&self, level: usize) -> usize {
        match level {
            0 => self.pml4 as usize,
            1 => self.pdpt as usize,
            2 => self.pd as usize,
            _ => self.pt as usize,
        }
    }
}

fn page_indices(addr: u64) -> PageIndices {
    PageIndices {
        pml4: (addr >> 39) & 0x1FF,
        pdpt: (addr >> 30) & 0x1FF,
        pd: (addr >> 21) & 0x1FF,
        pt: (addr >> 12) & 0x1FF,
    }
}

pub fn install_recursive_mapping() {
    let cr3 = PhysAddr::new(cpu::read_cr3() & !0xFFF);
    let hhdm = crate::boot_info().hhdm_offset;
    let pml4 = phys_to_virt(hhdm, cr3);
    let entry = unsafe { pml4.as_mut_ptr::<u64>().add(RECURSIVE_INDEX) };
    let value = unsafe { entry.read() };
    if value & PageFlags::PRESENT.bits() == 0 {
        unsafe {
            entry.write(
                cr3.as_u64() | PageFlags::PRESENT.bits() | PageFlags::WRITABLE.bits(),
            );
        }
        flush_tlb();
    }
}

pub fn map_kernel_heap_window() {
    // Kernel heap pages are demand-mapped on first fault via the heap VMA.
}

pub fn map_mmio_windows() {
    let table = PageTable::kernel();
    let hhdm = crate::boot_info().hhdm_offset;
    let flags = PageFlags::PRESENT
        | PageFlags::WRITABLE
        | PageFlags::NO_EXECUTE
        | PageFlags::NO_CACHE;
    for base in [0xFEC0_0000u64, 0xFEE0_0000u64] {
        let virt = VirtAddr::new(hhdm + base);
        let phys = PhysAddr::new(base);
        let _ = table.map_page(virt, phys, flags);
    }
}

pub fn current() -> PageTable {
    PageTable::kernel()
}

pub fn switch(table: PageTable) {
    cpu::write_cr3(table.cr3.as_u64());
}

pub fn flush_tlb() {
    let cr3 = cpu::read_cr3();
    cpu::write_cr3(cr3);
}

pub fn flush_tlb_addr(addr: u64) {
    unsafe {
        core::arch::asm!("invlpg [{}]", in(reg) addr, options(nomem, nostack));
    }
}
