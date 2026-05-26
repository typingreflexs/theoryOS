//! Memory-related syscalls: mmap, munmap, mprotect, brk.

use crate::arch::memory::VirtAddr;
use crate::mm::layout::{align_up, PAGE_SIZE, USER_HEAP_BASE};
use crate::mm::paging::PageFlags;
use crate::mm::permissions::{MmapFlags, MprotectFlags, ProtFlags};
use crate::mm::phys;
use crate::mm::vm::{self, VmError};
use crate::mm::vma::VmaKind;
use crate::proc;
use crate::syscall::errno::{err, ok, Errno, SysResult};

pub fn sys_mmap(addr: u64, len: u64, prot: u64, flags: u64, fd: u64, offset: u64) -> SysResult {
    let mut prot_flags = ProtFlags::USER;
    if prot & 0x1 != 0 {
        prot_flags |= ProtFlags::READ;
    }
    if prot & 0x2 != 0 {
        prot_flags |= ProtFlags::WRITE;
    }
    if prot & 0x4 != 0 {
        prot_flags |= ProtFlags::EXEC;
    }
    let mut mmap_flags = MmapFlags::PRIVATE | MmapFlags::ANONYMOUS;
    if flags & 0x01 != 0 {
        mmap_flags |= MmapFlags::SHARED;
    }
    if flags & 0x02 != 0 {
        mmap_flags.remove(MmapFlags::PRIVATE);
    }
    if flags & 0x10 != 0 {
        mmap_flags |= MmapFlags::FIXED;
    }
    if flags & 0x20 != 0 {
        mmap_flags |= MmapFlags::ANONYMOUS;
    }

    // File-backed mmap: fd + offset passed through when not MAP_ANONYMOUS.
    let file_off = if flags & 0x20 != 0 { 0 } else { offset };

    proc::with_current_user_address_space(|space| {
        match vm::mmap(space, addr, len, prot_flags, mmap_flags, file_off) {
            Ok(v) => ok(v as isize),
            Err(e) => Err(vm_err(e)),
        }
    })
    .unwrap_or(err(Errno::EFAULT))
}

pub fn sys_munmap(addr: u64, len: u64, _: u64, _: u64, _: u64, _: u64) -> SysResult {
    proc::with_current_user_address_space(|space| match vm::munmap(space, addr, len) {
        Ok(()) => ok(0),
        Err(e) => Err(vm_err(e)),
    })
    .unwrap_or(err(Errno::EFAULT))
}

pub fn sys_mprotect(addr: u64, len: u64, prot: u64, _: u64, _: u64, _: u64) -> SysResult {
    let mut mprot = MprotectFlags::empty();
    if prot & 0x1 != 0 {
        mprot |= MprotectFlags::READ;
    }
    if prot & 0x2 != 0 {
        mprot |= MprotectFlags::WRITE;
    }
    if prot & 0x4 != 0 {
        mprot |= MprotectFlags::EXEC;
    }
    proc::with_current_user_address_space(|space| match vm::mprotect(space, addr, len, mprot) {
        Ok(()) => ok(0),
        Err(e) => Err(vm_err(e)),
    })
    .unwrap_or(err(Errno::EFAULT))
}

pub fn sys_brk(addr: u64, _: u64, _: u64, _: u64, _: u64, _: u64) -> SysResult {
    proc::current_process_mut(|p| {
        if addr == 0 {
            return ok(p.brk as isize);
        }
        if addr < USER_HEAP_BASE || addr > p.brk_limit {
            return ok(p.brk as isize);
        }
        let old_brk = p.brk;
        if addr > old_brk {
            let space = p.address_space.as_mut().ok_or(Errno::ENOMEM)?;
            grow_heap(space, old_brk, addr).map_err(|_| Errno::ENOMEM)?;
        }
        p.brk = addr;
        ok(p.brk as isize)
    })
    .unwrap_or(err(Errno::ENOMEM))
}

/// Map zero-filled pages for each page boundary crossed by a brk increase.
fn grow_heap(
    space: &mut crate::mm::AddressSpace,
    old_brk: u64,
    new_brk: u64,
) -> Result<(), ()> {
    let mut page = align_up(old_brk, PAGE_SIZE);
    if page == old_brk {
        page += PAGE_SIZE;
    }
    let end = align_up(new_brk, PAGE_SIZE);
    let flags = PageFlags::PRESENT | PageFlags::USER | PageFlags::WRITABLE | PageFlags::NO_EXECUTE;
    while page <= end {
        // Skip if already mapped (initial heap page from exec).
        if space.page_table.resolve(VirtAddr::new(page)).is_none() {
            let frame = phys::alloc_frame(space.numa_node).ok_or(())?;
            let hhdm = crate::boot_info().hhdm_offset;
            let kva = crate::arch::memory::phys_to_virt(hhdm, frame.phys());
            // SAFETY: zeroing a freshly allocated frame before user mapping.
            unsafe {
                core::ptr::write_bytes(kva.as_mut_ptr::<u8>(), 0, PAGE_SIZE as usize);
            }
            space
                .page_table
                .map_page(VirtAddr::new(page), frame.phys(), flags)
                .map_err(|_| ())?;
        }
        page += PAGE_SIZE;
    }
    // Extend heap VMA metadata if present.
    if let Some(vma) = space.vma.find(USER_HEAP_BASE) {
        if vma.kind == VmaKind::Heap && new_brk > vma.end {
            let _ = space.vma.remove(USER_HEAP_BASE, vma.length());
            let _ = space.vma.insert(crate::mm::vma::Vma::new(
                USER_HEAP_BASE,
                align_up(new_brk - USER_HEAP_BASE, PAGE_SIZE),
                ProtFlags::USER | ProtFlags::READ | ProtFlags::WRITE,
                MmapFlags::PRIVATE | MmapFlags::ANONYMOUS,
                VmaKind::Heap,
            ));
        }
    }
    Ok(())
}

fn vm_err(e: VmError) -> Errno {
    match e {
        VmError::InvalidArgs => Errno::EINVAL,
        VmError::NoMemory => Errno::ENOMEM,
        VmError::NoSpace => Errno::ENOMEM,
        VmError::NotFound => Errno::EINVAL,
        VmError::AccessDenied => Errno::EFAULT,
        VmError::Exists => Errno::EEXIST,
    }
}
