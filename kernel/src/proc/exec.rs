//! Program execution — ELF loading, stack setup, execve.

use alloc::vec::Vec;

use crate::arch::memory::{phys_to_virt, VirtAddr};
use crate::arch::x86_64::context::CpuContext;
use crate::fs::vfs::read_path;
use crate::mm::elf::{self, ElfError, LoadedElf};
use crate::mm::layout::{align_down, PAGE_SIZE, USER_HEAP_BASE, USER_STACK_TOP};
use crate::mm::paging::PageTable;
use crate::mm::phys;
use crate::mm::AddressSpace;
use crate::proc::id::{Pid, Tid};
use crate::proc::{self, ProcessState};
use crate::proc::thread::Thread;
use crate::sched::runqueue;

const AT_NULL: u64 = 0;
const AT_PHDR: u64 = 3;
const AT_PHENT: u64 = 4;
const AT_PHNUM: u64 = 5;
const AT_PAGESZ: u64 = 6;
const AT_BASE: u64 = 7;
const AT_ENTRY: u64 = 9;
const AT_UID: u64 = 11;
const AT_EUID: u64 = 12;
const AT_GID: u64 = 13;
const AT_EGID: u64 = 14;
const AT_SECURE: u64 = 23;
const AT_RANDOM: u64 = 25;
const AT_EXECFN: u64 = 31;
const STACK_PAGES: u64 = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExecError {
    NotFound,
    Invalid,
    Elf(ElfError),
    NoMemory,
    TooBig,
}

impl From<ElfError> for ExecError {
    fn from(e: ElfError) -> Self {
        ExecError::Elf(e)
    }
}

pub fn kernel_exec(
    pid: Pid,
    tid: Tid,
    path: &[u8],
    argv: &[&[u8]],
    envp: &[&[u8]],
) -> Result<(), ExecError> {
    let (space, loaded, sp) = prepare_image(path, argv, envp)?;

    proc::table::with_process_mut(pid, |p| {
        p.address_space = Some(space);
        p.address_space_id = p.address_space.as_ref().unwrap().id;
        p.state = ProcessState::Running;
        // brk initialized by init_user_heap inside prepare_image
        p.brk = USER_HEAP_BASE;
        // Reset signal dispositions and close-on-exec FD semantics on exec (POSIX).
        p.signals = crate::signal::types::SignalState::new();
        p.fds.close_on_exec();
    })
    .ok_or(ExecError::Invalid)?;

    proc::table::with_thread_mut(tid, |t| {
        t.context = CpuContext::init_user_thread(sp, loaded.entry);
        t.fs_base = 0;
    })
    .ok_or(ExecError::Invalid)?;
    Ok(())
}

fn prepare_image(
    path: &[u8],
    argv: &[&[u8]],
    envp: &[&[u8]],
) -> Result<(AddressSpace, LoadedElf, u64), ExecError> {
    let mut exec_path = path.to_vec();
    let mut file_data = read_path(&exec_path).map_err(|_| ExecError::NotFound)?;
    let mut extra_argv: alloc::vec::Vec<alloc::vec::Vec<u8>> = alloc::vec::Vec::new();

    if let Some((interp, args)) = elf::parse_shebang(&file_data) {
        exec_path = interp.to_vec();
        for token in args.split(|&b| b == b' ' || b == b'\t').filter(|t| !t.is_empty()) {
            extra_argv.push(token.to_vec());
        }
        file_data = read_path(&exec_path).map_err(|_| ExecError::NotFound)?;
    }

    let mut space = AddressSpace::new_user().ok_or(ExecError::NoMemory)?;
    let loaded = elf::load_executable(&mut space, &file_data)?;
    let brk_end = elf::init_user_heap(&mut space, USER_HEAP_BASE + PAGE_SIZE)?;
    let _ = brk_end;
    map_user_stack(&mut space)?;
    let _ = crate::video::map_into_user(&mut space);

    // If shebang had extra args, prepend them after argv[0] (Linux behavior).
    let mut merged_argv: Vec<&[u8]> = Vec::new();
    if !argv.is_empty() {
        merged_argv.push(argv[0]);
    } else {
        merged_argv.push(path);
    }
    for token in extra_argv.iter() {
        merged_argv.push(token.as_slice());
    }
    for arg in argv.iter().skip(1) {
        merged_argv.push(arg);
    }

    let sp = build_stack_on(
        &space,
        USER_STACK_TOP,
        &merged_argv,
        envp,
        &loaded,
        path,
    )?;
    Ok((space, loaded, sp))
}

pub fn spawn_init() -> Option<Tid> {
    let argv: [&[u8]; 1] = [b"init"];
    let envp: [&[u8]; 2] = [b"HOME=/", b"PATH=/bin"];
    let (space, loaded, sp) = prepare_image(b"/bin/init", &argv, &envp).ok()?;

    let pid = Pid::new(1);
    proc::id::seed_next_pid(2);
    let tid = proc::alloc_tid();

    let mut process = crate::proc::pcb::Process::new_user(pid, Pid::KERNEL, space);
    process.state = ProcessState::Running;
    process.cred = crate::security::Credentials::root();
    proc::table::insert_process(process)?;
    proc::table::emplace_thread(tid, |tid| Thread::new_user(tid, pid, loaded.entry, sp))?;
    runqueue::enqueue_thread(tid, 0);
    crate::console::Console::println("[proc] PID 1 init spawned");
    Some(tid)
}

fn map_user_stack(space: &mut AddressSpace) -> Result<(), ExecError> {
    use crate::mm::permissions::{MmapFlags, ProtFlags};
    use crate::mm::vma::{Vma, VmaKind};

    let stack_bottom = USER_STACK_TOP - STACK_PAGES * PAGE_SIZE;
    let vma = Vma::new(
        stack_bottom,
        STACK_PAGES * PAGE_SIZE,
        ProtFlags::USER | ProtFlags::READ | ProtFlags::WRITE,
        MmapFlags::PRIVATE | MmapFlags::ANONYMOUS | MmapFlags::STACK,
        VmaKind::Stack,
    );
    space.vma.insert(vma).map_err(|_| ExecError::NoMemory)?;

    for i in 0..STACK_PAGES {
        let vaddr = stack_bottom + i * PAGE_SIZE;
        let frame = phys::alloc_frame(crate::mm::numa::local_node()).ok_or(ExecError::NoMemory)?;
        let flags = crate::mm::paging::PageFlags::PRESENT
            | crate::mm::paging::PageFlags::USER
            | crate::mm::paging::PageFlags::WRITABLE;
        space
            .page_table
            .map_page(VirtAddr::new(vaddr), frame.phys(), flags)
            .map_err(|_| ExecError::NoMemory)?;
    }
    Ok(())
}

fn build_stack_on(
    space: &AddressSpace,
    stack_top: u64,
    argv: &[&[u8]],
    envp: &[&[u8]],
    loaded: &LoadedElf,
    execfn: &[u8],
) -> Result<u64, ExecError> {
    let region_size = (STACK_PAGES * PAGE_SIZE) as usize;
    let region_base = stack_top - region_size as u64;
    let mut image = alloc::vec![0u8; region_size];
    let mut sp = stack_top;

    let mut arg_ptrs = Vec::new();
    for s in argv.iter().rev() {
        sp = push_str(&mut image, region_base, sp, s)?;
        arg_ptrs.push(sp);
    }
    arg_ptrs.reverse();

    let mut env_ptrs = Vec::new();
    for s in envp.iter().rev() {
        sp = push_str(&mut image, region_base, sp, s)?;
        env_ptrs.push(sp);
    }
    env_ptrs.reverse();

    sp = align16(sp);
    sp = sp.checked_sub(16).ok_or(ExecError::TooBig)?;
    let random_addr = sp;
    fill_random(&mut image, region_base, random_addr);

    let execfn_addr = push_str(&mut image, region_base, sp, execfn)?;

    let cred = proc::current_process_mut(|p| p.cred)
        .unwrap_or(crate::security::Credentials::root());
    let secure = cred.uid != cred.euid || cred.gid != cred.egid;

    sp = execfn_addr;
    sp = align16(sp);
    push_auxv(&mut image, region_base, &mut sp, AT_NULL, 0)?;
    push_auxv(&mut image, region_base, &mut sp, AT_EXECFN, execfn_addr)?;
    push_auxv(&mut image, region_base, &mut sp, AT_RANDOM, random_addr)?;
    push_auxv(&mut image, region_base, &mut sp, AT_SECURE, if secure { 1 } else { 0 })?;
    push_auxv(&mut image, region_base, &mut sp, AT_EGID, cred.egid as u64)?;
    push_auxv(&mut image, region_base, &mut sp, AT_EUID, cred.euid as u64)?;
    push_auxv(&mut image, region_base, &mut sp, AT_GID, cred.gid as u64)?;
    push_auxv(&mut image, region_base, &mut sp, AT_UID, cred.uid as u64)?;
    if loaded.is_dynamic {
        push_auxv(&mut image, region_base, &mut sp, AT_BASE, loaded.load_bias)?;
    }
    push_auxv(&mut image, region_base, &mut sp, AT_ENTRY, loaded.entry)?;
    push_auxv(&mut image, region_base, &mut sp, AT_PAGESZ, PAGE_SIZE)?;
    push_auxv(&mut image, region_base, &mut sp, AT_PHNUM, loaded.phdr_count as u64)?;
    push_auxv(&mut image, region_base, &mut sp, AT_PHENT, loaded.phdr_entry_size as u64)?;
    push_auxv(&mut image, region_base, &mut sp, AT_PHDR, loaded.phdr_vaddr)?;

    sp = align16(sp);
    sp = push_u64(&mut image, region_base, sp, 0)?;
    for p in env_ptrs.iter().rev() {
        sp = push_u64(&mut image, region_base, sp, *p)?;
    }

    sp = align16(sp);
    sp = push_u64(&mut image, region_base, sp, 0)?;
    for p in arg_ptrs.iter().rev() {
        sp = push_u64(&mut image, region_base, sp, *p)?;
    }

    sp = align16(sp);
    sp = push_u64(&mut image, region_base, sp, argv.len() as u64)?;

    let start_off = (sp - region_base) as usize;
    write_user_region(&space.page_table, sp, &image[start_off..region_size])?;
    Ok(sp)
}

/// 16-byte seed for AT_RANDOM — not cryptographic; uses RDTSC + scheduler tick.
/// Linux uses a CSPRNG; we document this deviation for embedded/QEMU bring-up.
fn fill_random(image: &mut [u8], base: u64, addr: u64) {
    let off = (addr - base) as usize;
    if off + 16 > image.len() {
        return;
    }
    let tsc: u64;
    // SAFETY: rdtsc is a valid x86_64 instruction with no memory operands.
    unsafe {
        core::arch::asm!("rdtsc", lateout("rax") tsc, options(nomem, nostack));
    }
    let tick = crate::sched::timer::ticks();
    for i in 0..16 {
        image[off + i] = ((tsc >> (i * 4)) ^ (tick >> i)) as u8;
    }
}

fn push_auxv(
    image: &mut [u8],
    base: u64,
    sp: &mut u64,
    tag: u64,
    val: u64,
) -> Result<(), ExecError> {
    *sp = push_u64(image, base, *sp, val)?;
    *sp = push_u64(image, base, *sp, tag)?;
    Ok(())
}

fn align16(v: u64) -> u64 {
    v & !0xf
}

fn push_str(image: &mut [u8], base: u64, sp: u64, s: &[u8]) -> Result<u64, ExecError> {
    let sp = sp
        .checked_sub(s.len() as u64 + 1)
        .ok_or(ExecError::TooBig)?;
    let sp = align16(sp);
    let off = (sp - base) as usize;
    if off + s.len() + 1 > image.len() {
        return Err(ExecError::TooBig);
    }
    image[off..off + s.len()].copy_from_slice(s);
    image[off + s.len()] = 0;
    Ok(sp)
}

fn push_u64(image: &mut [u8], base: u64, sp: u64, val: u64) -> Result<u64, ExecError> {
    let sp = sp.checked_sub(8).ok_or(ExecError::TooBig)?;
    let off = (sp - base) as usize;
    if off + 8 > image.len() {
        return Err(ExecError::TooBig);
    }
    image[off..off + 8].copy_from_slice(&val.to_le_bytes());
    Ok(sp)
}

pub(crate) fn write_user_region(table: &PageTable, addr: u64, data: &[u8]) -> Result<(), ExecError> {
    let hhdm = crate::boot_info().hhdm_offset;
    let mut written = 0usize;
    while written < data.len() {
        let va = addr + written as u64;
        let page = align_down(va, PAGE_SIZE);
        let page_off = (va - page) as usize;
        let (phys, _) = table.resolve(VirtAddr::new(page)).ok_or(ExecError::NoMemory)?;
        let kva = phys_to_virt(hhdm, phys);
        let n = (PAGE_SIZE as usize - page_off).min(data.len() - written);
        // SAFETY: destination page is mapped in the target address space via HHDM.
        unsafe {
            core::ptr::copy_nonoverlapping(
                data.as_ptr().add(written),
                kva.as_mut_ptr::<u8>().add(page_off),
                n,
            );
        }
        written += n;
    }
    Ok(())
}

pub fn exec_err_to_errno(e: ExecError) -> crate::syscall::errno::Errno {
    use crate::syscall::errno::Errno;
    match e {
        ExecError::NotFound => Errno::ENOENT,
        ExecError::Invalid => Errno::EINVAL,
        ExecError::NoMemory | ExecError::TooBig => Errno::ENOMEM,
        ExecError::Elf(ElfError::NotElf | ElfError::NotExec | ElfError::Invalid) => Errno::ENOEXEC,
        ExecError::Elf(ElfError::MapFailed) => Errno::ENOMEM,
        ExecError::Elf(ElfError::InterpMissing) => Errno::ENOENT,
        ExecError::Elf(ElfError::RelocFailed) => Errno::ENOEXEC,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stack_align16() {
        assert_eq!(align16(0x1001), 0x1000);
        assert_eq!(align16(0x1000), 0x1000);
    }
}
