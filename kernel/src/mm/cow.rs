use crate::arch::memory::{phys_to_virt, VirtAddr};
use crate::mm::layout::PAGE_SIZE;
use crate::mm::paging::{PageFlags, PageTable};
use crate::mm::phys;
use crate::mm::vma::Vma;

/// Duplicate a VMA for fork using copy-on-write semantics.
pub fn duplicate_vma(parent: PageTable, child: PageTable, vma: &Vma) -> Option<()> {
    let pages = vma.page_count();
    for i in 0..pages {
        let virt = VirtAddr::new(vma.start + i * PAGE_SIZE);
        let flags = parent.get_flags(virt)?;
        if flags.contains(PageFlags::SOFTWARE_DEMAND) {
            child
                .map_page_demand(virt, cow_demand_flags(vma))
                .ok()?;
            continue;
        }
        let (phys, _) = parent.resolve(virt)?;
        let cow_flags = (flags - PageFlags::WRITABLE) | PageFlags::COW;
        parent.set_flags(virt, cow_flags).ok()?;
        child.map_page(virt, phys, cow_flags).ok()?;
    }
    Some(())
}

/// Break a COW mapping on write fault.
pub fn break_cow(table: PageTable, virt: VirtAddr, flags: PageFlags) -> Option<()> {
    let (phys, _) = table.resolve(virt)?;
    let frame = phys::alloc_frame(crate::mm::numa::local_node())?;
    let hhdm = crate::boot_info().hhdm_offset;
    let src = phys_to_virt(hhdm, phys);
    let dst = phys_to_virt(hhdm, frame.phys());
    unsafe {
        core::ptr::copy_nonoverlapping(
            src.as_ptr::<u8>(),
            dst.as_mut_ptr::<u8>(),
            PAGE_SIZE as usize,
        );
    }
    let mut new_flags = flags;
    new_flags.remove(PageFlags::COW);
    new_flags.insert(PageFlags::WRITABLE | PageFlags::DIRTY);
    table.map_page(virt, frame.phys(), new_flags).ok()?;
    Some(())
}

pub fn is_cow(flags: PageFlags) -> bool {
    flags.contains(PageFlags::COW)
}

fn cow_demand_flags(vma: &Vma) -> PageFlags {
    let mut flags = PageTable::flags_from_prot(vma.prot, true) | PageFlags::SOFTWARE_DEMAND;
    if vma.prot.contains(crate::mm::permissions::ProtFlags::WRITE) {
        flags.remove(PageFlags::WRITABLE);
        flags |= PageFlags::COW;
    }
    flags
}
