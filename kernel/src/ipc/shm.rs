//! SysV-style shared memory (shmget, shmat, shmdt, shmctl).

use spin::Mutex;

use crate::arch::memory::VirtAddr;
use crate::mm::layout::{align_up, PAGE_SIZE, USER_MMAP_LIMIT};
use crate::mm::paging::{PageFlags, PageTable};
use crate::mm::permissions::ProtFlags;
use crate::mm::phys;
use crate::mm::vma::{Vma, VmaKind};
use crate::proc;

const MAX_SEGMENTS: usize = 32;

struct ShmSegment {
    id: i32,
    key: i32,
    size: u64,
    frames: [Option<u64>; 256],
    frame_count: usize,
    attach_count: u32,
}

impl ShmSegment {
    fn alloc_frames(&mut self) -> Result<(), ShmError> {
        let pages = align_up(self.size, PAGE_SIZE) / PAGE_SIZE;
        if pages as usize > self.frames.len() {
            return Err(ShmError::NoMem);
        }
        for i in 0..pages {
            let frame = phys::alloc_frame(crate::mm::numa::local_node()).ok_or(ShmError::NoMem)?;
            self.frames[i as usize] = Some(frame.frame);
        }
        self.frame_count = pages as usize;
        self.size = pages * PAGE_SIZE;
        Ok(())
    }
}

static SEGMENTS: Mutex<[Option<ShmSegment>; MAX_SEGMENTS]> = Mutex::new([const { None }; MAX_SEGMENTS]);
static NEXT_SHMID: Mutex<i32> = Mutex::new(1);

pub const IPC_CREAT: i32 = 0o1000;
pub const IPC_EXCL: i32 = 0o2000;
pub const IPC_RMID: i32 = 0;
pub const SHM_RDONLY: i32 = 0o10000;

pub fn shmget(key: i32, size: u64, flags: i32) -> Result<i32, ShmError> {
    let mut segs = SEGMENTS.lock();
    for slot in segs.iter() {
        if let Some(s) = slot {
            if s.key == key {
                return Ok(s.id);
            }
        }
    }
    if flags & IPC_CREAT == 0 {
        return Err(ShmError::NoEnt);
    }
    if size == 0 {
        return Err(ShmError::Inval);
    }
    let id = {
        let mut next = NEXT_SHMID.lock();
        let v = *next;
        *next += 1;
        v
    };
    for slot in segs.iter_mut() {
        if slot.is_none() {
            let mut seg = ShmSegment {
                id,
                key,
                size,
                frames: [None; 256],
                frame_count: 0,
                attach_count: 0,
            };
            seg.alloc_frames()?;
            *slot = Some(seg);
            return Ok(id);
        }
    }
    Err(ShmError::NoSpace)
}

pub fn shmat(shmid: i32, addr: u64, flags: i32) -> Result<u64, ShmError> {
    let mut segs = SEGMENTS.lock();
    let seg = find_seg_mut(&mut segs, shmid)?;
    let size = seg.size;
    let frame_count = seg.frame_count;
    let mut frames = [0u64; 256];
    for i in 0..frame_count {
        frames[i] = seg.frames[i].ok_or(ShmError::Inval)?;
    }
    seg.attach_count += 1;
    drop(segs);

    let readonly = flags & SHM_RDONLY != 0;
    proc::with_current_user_address_space(|space| {
        let target = if addr == 0 {
            space
                .vma
                .find_gap(0x5000_0000, size, USER_MMAP_LIMIT)
                .ok_or(ShmError::NoSpace)?
        } else {
            addr
        };
        let prot = if readonly {
            ProtFlags::READ
        } else {
            ProtFlags::READ | ProtFlags::WRITE
        };
        let vma = Vma::new(
            target,
            size,
            prot,
            crate::mm::permissions::MmapFlags::SHARED,
            VmaKind::Mmap,
        );
        space.vma.insert(vma).map_err(|_| ShmError::NoSpace)?;
        let page_flags = PageFlags::PRESENT | PageFlags::USER
            | if readonly { PageFlags::empty() } else { PageFlags::WRITABLE };
        for i in 0..frame_count {
            let virt = VirtAddr::new(target + i as u64 * PAGE_SIZE);
            let phys = crate::arch::memory::PhysAddr::new(frames[i] * PAGE_SIZE);
            space
                .page_table
                .map_page(virt, phys, page_flags)
                .map_err(|_| ShmError::NoMem)?;
        }
        Ok(target)
    })
    .ok_or(ShmError::Inval)?
}

pub fn shmdt(addr: u64) -> Result<(), ShmError> {
    proc::with_current_user_address_space(|space| {
        let vma = space.vma.find(addr).ok_or(ShmError::Inval)?;
        let len = vma.length();
        space.vma.remove(addr, len).map_err(|_| ShmError::Inval)?;
        let pages = len / PAGE_SIZE;
        for i in 0..pages {
            let virt = VirtAddr::new(addr + i * PAGE_SIZE);
            let _ = space.page_table.unmap_page(virt);
        }
        Ok(())
    })
    .ok_or(ShmError::Inval)?
}

pub fn shmctl(shmid: i32, cmd: i32) -> Result<(), ShmError> {
    if cmd != IPC_RMID {
        return Err(ShmError::Inval);
    }
    let mut segs = SEGMENTS.lock();
    for slot in segs.iter_mut() {
        if let Some(s) = slot {
            if s.id == shmid {
                if s.attach_count > 0 {
                    return Err(ShmError::Busy);
                }
                *slot = None;
                return Ok(());
            }
        }
    }
    Err(ShmError::Inval)
}

fn find_seg(segs: &[Option<ShmSegment>; MAX_SEGMENTS], shmid: i32) -> Result<&ShmSegment, ShmError> {
    for slot in segs.iter() {
        if let Some(s) = slot {
            if s.id == shmid {
                return Ok(s);
            }
        }
    }
    Err(ShmError::Inval)
}

fn find_seg_mut(
    segs: &mut [Option<ShmSegment>; MAX_SEGMENTS],
    shmid: i32,
) -> Result<&mut ShmSegment, ShmError> {
    for slot in segs.iter_mut() {
        if let Some(s) = slot {
            if s.id == shmid {
                return Ok(s);
            }
        }
    }
    Err(ShmError::Inval)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShmError {
    Inval,
    NoEnt,
    NoMem,
    NoSpace,
    Busy,
}

pub fn shm_err_to_errno(err: ShmError) -> crate::syscall::errno::Errno {
    use crate::syscall::errno::Errno;
    match err {
        ShmError::Inval => Errno::EINVAL,
        ShmError::NoEnt => Errno::ENOENT,
        ShmError::NoMem => Errno::ENOMEM,
        ShmError::NoSpace => Errno::ENOSPC,
        ShmError::Busy => Errno::EBUSY,
    }
}
