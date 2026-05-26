//! ELF64 image loading into a user address space — PT_LOAD, PT_INTERP, RELA.

use crate::arch::memory::{phys_to_virt, VirtAddr};
use crate::mm::layout::{align_down, align_up, PAGE_SIZE, USER_HEAP_BASE};
use crate::mm::paging::PageFlags;
use crate::mm::permissions::{MmapFlags, ProtFlags};
use crate::mm::phys;
use crate::mm::vma::{Vma, VmaKind};
use crate::mm::AddressSpace;

use super::parse::{
    self, parse_dynamic, phdr_at, phdr_vaddr_in_image, min_load_vaddr, validate_hdr, ParseError,
    Elf64Ehdr, Elf64Phdr, Elf64Rela, ET_DYN, PF_W, PF_X, PT_LOAD,
};
use super::relocate::{self, RelocError};

/// Default load address for PIE ET_DYN executables when linked at vaddr 0.
const PIE_LOAD_BASE: u64 = 0x400000;

#[derive(Clone, Copy, Debug)]
pub struct LoadedElf {
    pub entry: u64,
    pub phdr_vaddr: u64,
    pub phdr_count: u16,
    pub phdr_entry_size: u16,
    pub load_bias: u64,
    pub is_dynamic: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ElfError {
    Invalid,
    NotElf,
    NotExec,
    MapFailed,
    InterpMissing,
    RelocFailed,
}

impl From<ParseError> for ElfError {
    fn from(e: ParseError) -> ElfError {
        match e {
            ParseError::NotElf => ElfError::NotElf,
            ParseError::BadMachine | ParseError::BadType => ElfError::NotExec,
            ParseError::Invalid => ElfError::Invalid,
        }
    }
}

impl From<RelocError> for ElfError {
    fn from(_: RelocError) -> ElfError {
        ElfError::RelocFailed
    }
}

pub fn is_elf(data: &[u8]) -> bool {
    parse::is_elf(data)
}

/// Load an ET_EXEC or ET_DYN binary, optional PT_INTERP dynamic linker, apply RELA fixups.
pub fn load_executable(space: &mut AddressSpace, data: &[u8]) -> Result<LoadedElf, ElfError> {
    let hdr = parse::parse_ehdr(data).map_err(ElfError::from)?;
    validate_hdr(&hdr).map_err(ElfError::from)?;

    let load_bias = compute_load_bias(data, &hdr)?;
    let mut loaded = map_image(space, data, &hdr, load_bias, false)?;

    let interp_path = parse::extract_interp_path(data, &hdr).map_err(ElfError::from)?;
    if let Some(interp_path) = interp_path {
        let interp_data =
            crate::fs::vfs::read_path(interp_path).map_err(|_| ElfError::InterpMissing)?;
        let ihdr = parse::parse_ehdr(&interp_data).map_err(ElfError::from)?;
        validate_hdr(&ihdr).map_err(ElfError::from)?;
        let iload_bias = compute_load_bias(&interp_data, &ihdr)?;
        let iloaded = map_image(space, &interp_data, &ihdr, iload_bias, true)?;
        // Dynamic linker is the program entry when PT_INTERP is present (Linux execve semantics).
        loaded.entry = iloaded.entry;
        loaded.phdr_vaddr = iloaded.phdr_vaddr;
        loaded.phdr_count = iloaded.phdr_count;
        loaded.phdr_entry_size = iloaded.phdr_entry_size;
        loaded.is_dynamic = true;
    }

    Ok(loaded)
}

fn compute_load_bias(data: &[u8], hdr: &Elf64Ehdr) -> Result<u64, ElfError> {
    if hdr.e_type == parse::ET_EXEC {
        return Ok(0);
    }
    // ET_DYN: PIE executables link at vaddr 0; shared objects use their own link addresses.
    let min_v = min_load_vaddr(data, hdr)?;
    if min_v == 0 {
        Ok(PIE_LOAD_BASE)
    } else {
        Ok(0)
    }
}

fn map_image(
    space: &mut AddressSpace,
    data: &[u8],
    hdr: &Elf64Ehdr,
    load_bias: u64,
    is_interp: bool,
) -> Result<LoadedElf, ElfError> {
    for i in 0..hdr.e_phnum {
        let ph = phdr_at(data, hdr, i).map_err(ElfError::from)?;
        if ph.p_type == PT_LOAD {
            map_segment(space, data, &ph, load_bias)?;
        }
    }

    // Ensure PHDR table is reachable for AT_PHDR even when not covered by PT_LOAD.
    let phdr_va = match phdr_vaddr_in_image(data, hdr, load_bias) {
        Ok(va) => va,
        Err(_) => map_phdr_copy(space, data, hdr, load_bias)?,
    };

    apply_relocations(space, data, hdr, load_bias, is_interp)?;

    Ok(LoadedElf {
        entry: hdr.e_entry + load_bias,
        phdr_vaddr: phdr_va,
        phdr_count: hdr.e_phnum,
        phdr_entry_size: hdr.e_phentsize,
        load_bias,
        is_dynamic: is_interp,
    })
}

fn map_phdr_copy(
    space: &mut AddressSpace,
    data: &[u8],
    hdr: &Elf64Ehdr,
    load_bias: u64,
) -> Result<u64, ElfError> {
    let size = align_up(hdr.e_phnum as u64 * hdr.e_phentsize as u64, PAGE_SIZE);
    // Place PHDR copy page just below the first PT_LOAD mapping.
    let base = min_load_vaddr(data, hdr).map_err(ElfError::from)? + load_bias;
    let va = align_down(base.wrapping_sub(size), PAGE_SIZE);
    let frame = phys::alloc_frame(space.numa_node).ok_or(ElfError::MapFailed)?;
    let hhdm = crate::boot_info().hhdm_offset;
    let kva = phys_to_virt(hhdm, frame.phys());
    // SAFETY: freshly allocated frame mapped exclusively for PHDR copy.
    unsafe {
        core::ptr::write_bytes(kva.as_mut_ptr::<u8>(), 0, PAGE_SIZE as usize);
    }
    let phsize = hdr.e_phnum as usize * hdr.e_phentsize as usize;
    let src = hdr.e_phoff as usize;
    if src + phsize <= data.len() {
        // SAFETY: validated slice bounds.
        unsafe {
            core::ptr::copy_nonoverlapping(
                data.as_ptr().add(src),
                kva.as_mut_ptr::<u8>(),
                phsize,
            );
        }
    }
    space
        .page_table
        .map_page(
            VirtAddr::new(va),
            frame.phys(),
            PageFlags::PRESENT | PageFlags::USER | PageFlags::NO_EXECUTE,
        )
        .map_err(|_| ElfError::MapFailed)?;
    Ok(va)
}

fn map_segment(
    space: &mut AddressSpace,
    data: &[u8],
    ph: &Elf64Phdr,
    load_bias: u64,
) -> Result<(), ElfError> {
    let vaddr_base = ph.p_vaddr + load_bias;
    let seg_start = align_down(vaddr_base, PAGE_SIZE);
    let seg_end = align_up(vaddr_base + ph.p_memsz, PAGE_SIZE);
    let mut vaddr = seg_start;
    while vaddr < seg_end {
        let frame = phys::alloc_frame(space.numa_node).ok_or(ElfError::MapFailed)?;
        let hhdm = crate::boot_info().hhdm_offset;
        let kva = phys_to_virt(hhdm, frame.phys());
        // SAFETY: zeroing a newly allocated physical page before mapping into userspace.
        unsafe {
            core::ptr::write_bytes(kva.as_mut_ptr::<u8>(), 0, PAGE_SIZE as usize);
        }
        let page_lo = vaddr;
        let page_hi = vaddr + PAGE_SIZE;
        let copy_lo = vaddr_base.max(page_lo);
        let copy_hi = (vaddr_base + ph.p_filesz).min(page_hi);
        if copy_lo < copy_hi {
            let dst_off = (copy_lo - page_lo) as usize;
            let src_off = (ph.p_offset + (copy_lo - vaddr_base)) as usize;
            let len = (copy_hi - copy_lo) as usize;
            if src_off + len <= data.len() {
                // SAFETY: file slice bounds checked; destination is our exclusive frame.
                unsafe {
                    core::ptr::copy_nonoverlapping(
                        data.as_ptr().add(src_off),
                        kva.as_mut_ptr::<u8>().add(dst_off),
                        len,
                    );
                }
            }
        }
        let mut flags = PageFlags::PRESENT | PageFlags::USER;
        if ph.p_flags & PF_W != 0 {
            flags |= PageFlags::WRITABLE;
        }
        if ph.p_flags & PF_X == 0 {
            flags |= PageFlags::NO_EXECUTE;
        }
        space
            .page_table
            .map_page(VirtAddr::new(vaddr), frame.phys(), flags)
            .map_err(|_| ElfError::MapFailed)?;
        vaddr += PAGE_SIZE;
    }

    // Record executable/file mappings in the VMA tree for /proc/pid/maps and fault handling.
    let mut prot = ProtFlags::USER | ProtFlags::READ;
    if ph.p_flags & PF_W != 0 {
        prot |= ProtFlags::WRITE;
    }
    if ph.p_flags & PF_X != 0 {
        prot |= ProtFlags::EXEC;
    }
    let vma = Vma::new(
        seg_start,
        seg_end - seg_start,
        prot,
        MmapFlags::PRIVATE,
        VmaKind::File,
    );
    let _ = space.vma.insert(vma);
    Ok(())
}

fn apply_relocations(
    space: &mut AddressSpace,
    data: &[u8],
    hdr: &Elf64Ehdr,
    load_bias: u64,
    allow_irelative: bool,
) -> Result<(), ElfError> {
    let Some(dyn_info) = parse_dynamic(data, hdr, load_bias).map_err(ElfError::from)? else {
        return Ok(());
    };
    if dyn_info.relaent == 0 {
        return Ok(());
    }
    let count = relocate::rela_count(dyn_info.relasz, dyn_info.relaent);
    let mut entries = alloc::vec::Vec::with_capacity(count);
    for i in 0..count {
        // RELA entries live at runtime virtual addresses (post load_bias), not file offsets.
        let rela = read_user_rela(space, dyn_info.rela + i as u64 * dyn_info.relaent)?;
        entries.push(rela);
    }
    if dyn_info.pltrelsz > 0 && dyn_info.jmprel != 0 {
        let plt_count = relocate::rela_count(dyn_info.pltrelsz, dyn_info.relaent);
        for i in 0..plt_count {
            let rela = read_user_rela(space, dyn_info.jmprel + i as u64 * dyn_info.relaent)?;
            entries.push(rela);
        }
    }
    relocate::apply_rela_table(
        &entries,
        load_bias,
        |_| None,
        |addr, val| write_user_u64(space, addr, val),
        allow_irelative,
    )?;
    Ok(())
}

fn read_user_rela(space: &AddressSpace, addr: u64) -> Result<Elf64Rela, ElfError> {
    let mut buf = [0u8; 24];
    read_user_bytes(space, addr, &mut buf)?;
    // SAFETY: buf holds 24 bytes for Elf64Rela.
    Ok(unsafe { (buf.as_ptr() as *const Elf64Rela).read_unaligned() })
}

fn read_user_bytes(space: &AddressSpace, addr: u64, buf: &mut [u8]) -> Result<(), ElfError> {
    let hhdm = crate::boot_info().hhdm_offset;
    let mut done = 0usize;
    while done < buf.len() {
        let page = align_down(addr + done as u64, PAGE_SIZE);
        let off = (addr + done as u64 - page) as usize;
        let (phys, _) = space
            .page_table
            .resolve(VirtAddr::new(page))
            .ok_or(ElfError::Invalid)?;
        let kva = phys_to_virt(hhdm, phys);
        let n = (PAGE_SIZE as usize - off).min(buf.len() - done);
        // SAFETY: reading from mapped user page via HHDM alias.
        unsafe {
            core::ptr::copy_nonoverlapping(
                kva.as_ptr::<u8>().add(off),
                buf.as_mut_ptr().add(done),
                n,
            );
        }
        done += n;
    }
    Ok(())
}

fn write_user_u64(space: &AddressSpace, addr: u64, value: u64) -> Result<(), RelocError> {
    let hhdm = crate::boot_info().hhdm_offset;
    let page = align_down(addr, PAGE_SIZE);
    let off = (addr - page) as usize;
    let (phys, _) = space
        .page_table
        .resolve(VirtAddr::new(page))
        .ok_or(RelocError::BadOffset)?;
    let kva = phys_to_virt(hhdm, phys);
    // SAFETY: writing 8 bytes within a mapped user page via HHDM alias.
    unsafe {
        core::ptr::write_unaligned(kva.as_mut_ptr::<u8>().add(off) as *mut u64, value);
    }
    Ok(())
}

/// Map initial heap VMA and brk page so the first brk() does not fault.
pub fn init_user_heap(space: &mut AddressSpace, brk_limit: u64) -> Result<u64, ElfError> {
    let heap_len = align_up(brk_limit - USER_HEAP_BASE, PAGE_SIZE);
    let vma = Vma::new(
        USER_HEAP_BASE,
        heap_len,
        ProtFlags::USER | ProtFlags::READ | ProtFlags::WRITE,
        MmapFlags::PRIVATE | MmapFlags::ANONYMOUS,
        VmaKind::Heap,
    );
    space.vma.insert(vma).map_err(|_| ElfError::MapFailed)?;
    // Demand-map first brk page so brk == USER_HEAP_BASE is valid immediately.
    let frame = phys::alloc_frame(space.numa_node).ok_or(ElfError::MapFailed)?;
    let flags = PageFlags::PRESENT | PageFlags::USER | PageFlags::WRITABLE | PageFlags::NO_EXECUTE;
    space
        .page_table
        .map_page(VirtAddr::new(USER_HEAP_BASE), frame.phys(), flags)
        .map_err(|_| ElfError::MapFailed)?;
    Ok(USER_HEAP_BASE + PAGE_SIZE)
}
