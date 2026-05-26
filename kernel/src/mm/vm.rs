use crate::arch::memory::VirtAddr;
use crate::mm::layout::{align_up, is_user_address, PAGE_SIZE, USER_MMAP_LIMIT};
use crate::mm::paging::{PageFlags, PageTable};
use crate::mm::phys;
use crate::mm::vma::{Vma, VmaError, VmaKind, VmaTree};

use super::address_space::AddressSpace;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VmError {
    InvalidArgs,
    NoMemory,
    NoSpace,
    NotFound,
    AccessDenied,
    Exists,
}

pub struct VmManager;

static VM: spin::Once<VmManager> = spin::Once::new();

pub fn init() {
    let _ = VM.call_once(|| VmManager);
}

pub fn mmap(
    space: &mut AddressSpace,
    addr: u64,
    length: u64,
    prot: ProtFlags,
    flags: MmapFlags,
    _file_offset: u64,
) -> Result<u64, VmError> {
    if length == 0 {
        return Err(VmError::InvalidArgs);
    }
    let length = align_up(length, PAGE_SIZE);
    let target = if flags.contains(MmapFlags::FIXED) {
        if flags.contains(MmapFlags::FIXED_NOREPLACE) && space.vma.find(addr).is_some() {
            return Err(VmError::Exists);
        }
        addr
    } else {
        let hint = if addr == 0 {
            crate::mm::aslr::user_mmap_base()
        } else {
            addr
        };
        space
            .vma
            .find_gap(hint, length, USER_MMAP_LIMIT)
            .ok_or(VmError::NoSpace)?
    };

    if !is_user_address(target) || target + length > USER_MMAP_LIMIT {
        return Err(VmError::AccessDenied);
    }

    let kind = if flags.contains(MmapFlags::STACK) {
        VmaKind::Stack
    } else if flags.contains(MmapFlags::ANONYMOUS) || !flags.contains(MmapFlags::SHARED) {
        VmaKind::Anonymous
    } else {
        VmaKind::Mmap
    };

    let vma = Vma::new(target, length, prot, flags, kind);
    space
        .vma
        .insert(vma)
        .map_err(|e| match e {
            VmaError::NoSpace | VmaError::Overlap => VmError::NoSpace,
            VmaError::NotFound => VmError::NotFound,
        })?;

    map_vma_demand(space.page_table, &vma)?;
    if flags.contains(MmapFlags::POPULATE) {
        populate_vma(space, &vma)?;
    }
    Ok(target)
}

pub fn munmap(space: &mut AddressSpace, addr: u64, length: u64) -> Result<(), VmError> {
    if length == 0 {
        return Err(VmError::InvalidArgs);
    }
    let length = align_up(length, PAGE_SIZE);
    let vma = space
        .vma
        .remove(addr, length)
        .map_err(|_| VmError::NotFound)?;
    unmap_vma(space.page_table, &vma);
    Ok(())
}

pub fn mprotect(
    space: &mut AddressSpace,
    addr: u64,
    length: u64,
    prot: MprotectFlags,
) -> Result<(), VmError> {
    if length == 0 {
        return Err(VmError::InvalidArgs);
    }
    let length = align_up(length, PAGE_SIZE);
    let mut prot_flags = ProtFlags::empty();
    if prot.contains(MprotectFlags::READ) {
        prot_flags |= ProtFlags::READ;
    }
    if prot.contains(MprotectFlags::WRITE) {
        prot_flags |= ProtFlags::WRITE;
    }
    if prot.contains(MprotectFlags::EXEC) {
        prot_flags |= ProtFlags::EXEC;
    }
    space
        .vma
        .update_prot(addr, length, prot_flags)
        .map_err(|_| VmError::NotFound)?;
    apply_prot(space.page_table, addr, length, prot_flags);
    Ok(())
}

fn map_vma_demand(table: PageTable, vma: &Vma) -> Result<(), VmError> {
    let flags = PageTable::flags_from_prot(vma.prot, true) | PageFlags::SOFTWARE_DEMAND;
    let pages = vma.page_count();
    for i in 0..pages {
        let virt = VirtAddr::new(vma.start + i * PAGE_SIZE);
        table
            .map_page_demand(virt, flags)
            .map_err(|_| VmError::NoMemory)?;
    }
    Ok(())
}

fn populate_vma(space: &mut AddressSpace, vma: &Vma) -> Result<(), VmError> {
    let pages = vma.page_count();
    for i in 0..pages {
        let virt = VirtAddr::new(vma.start + i * PAGE_SIZE);
        let frame = phys::alloc_frame(space.numa_node).ok_or(VmError::NoMemory)?;
        let flags = PageTable::flags_from_prot(vma.prot, true);
        space
            .page_table
            .map_page(virt, frame.phys(), flags)
            .map_err(|_| VmError::NoMemory)?;
    }
    Ok(())
}

fn unmap_vma(table: PageTable, vma: &Vma) {
    let pages = vma.page_count();
    for i in 0..pages {
        let virt = VirtAddr::new(vma.start + i * PAGE_SIZE);
        let _ = table.unmap_page(virt);
    }
}

fn apply_prot(table: PageTable, start: u64, length: u64, prot: ProtFlags) {
    let pages = length / PAGE_SIZE;
    for i in 0..pages {
        let virt = VirtAddr::new(start + i * PAGE_SIZE);
        if let Some((_, mut flags)) = table.resolve(virt) {
            flags.remove(PageFlags::WRITABLE | PageFlags::NO_EXECUTE | PageFlags::USER);
            let new = PageTable::flags_from_prot(prot, true);
            flags |= new;
            let _ = table.set_flags(virt, flags);
        }
    }
}

use crate::mm::permissions::{MmapFlags, MprotectFlags, ProtFlags};
