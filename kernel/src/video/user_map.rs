use crate::arch::memory::VirtAddr;
use crate::mm::layout::{PAGE_SIZE, USER_FB_BASE, USER_FB_META_BASE};
use crate::mm::permissions::{MmapFlags, ProtFlags};
use crate::mm::paging::PageFlags;
use crate::mm::phys;
use crate::mm::vma::{Vma, VmaKind};
use crate::mm::AddressSpace;
use crate::proc::exec::write_user_region;
use crate::video::{framebuffer, Framebuffer, UserFbMeta};

/// Map Limine framebuffer and metadata page into a new user address space.
pub fn map_into_user(space: &mut AddressSpace) -> Result<(), ()> {
    let fb = framebuffer().ok_or(())?;
    map_meta_page(space, fb)?;
    map_pixel_pages(space, fb)?;
    Ok(())
}

fn map_meta_page(space: &mut AddressSpace, fb: &Framebuffer) -> Result<(), ()> {
    let frame = phys::alloc_frame(crate::mm::numa::local_node()).ok_or(())?;
    let flags = PageFlags::PRESENT | PageFlags::WRITABLE | PageFlags::USER | PageFlags::NO_EXECUTE;
    space
        .page_table
        .map_page(VirtAddr::new(USER_FB_META_BASE), frame.phys(), flags)
        .map_err(|_| ())?;

    let vma = Vma::new(
        USER_FB_META_BASE,
        PAGE_SIZE,
        ProtFlags::USER | ProtFlags::READ | ProtFlags::WRITE,
        MmapFlags::PRIVATE | MmapFlags::ANONYMOUS,
        VmaKind::Framebuffer,
    );
    space.vma.insert(vma).map_err(|_| ())?;

    let meta = fb.user_meta();
    write_user_region(&space.page_table, USER_FB_META_BASE, bytes_of(&meta)).map_err(|_| ())?;
    Ok(())
}

fn map_pixel_pages(space: &mut AddressSpace, fb: &Framebuffer) -> Result<(), ()> {
    let phys_base = fb.phys_base().ok_or(())?;
    let pages = fb.page_count();
    let byte_size = pages * PAGE_SIZE;

    let vma = Vma::new(
        USER_FB_BASE,
        byte_size,
        ProtFlags::USER | ProtFlags::READ | ProtFlags::WRITE,
        MmapFlags::PRIVATE | MmapFlags::SHARED,
        VmaKind::Framebuffer,
    );
    space.vma.insert(vma).map_err(|_| ())?;

    let flags = PageFlags::PRESENT | PageFlags::WRITABLE | PageFlags::USER | PageFlags::NO_EXECUTE;
    for i in 0..pages {
        let phys = phys_base.as_u64() + i * PAGE_SIZE;
        let virt = USER_FB_BASE + i * PAGE_SIZE;
        space
            .page_table
            .map_page(VirtAddr::new(virt), crate::arch::memory::PhysAddr::new(phys), flags)
            .map_err(|_| ())?;
    }
    Ok(())
}

fn bytes_of(meta: &UserFbMeta) -> &[u8] {
    // SAFETY: UserFbMeta is plain data with no uninit padding.
    unsafe {
        core::slice::from_raw_parts(
            meta as *const UserFbMeta as *const u8,
            core::mem::size_of::<UserFbMeta>(),
        )
    }
}

pub fn proc_fb_line() -> alloc::string::String {
    let Some(fb) = framebuffer() else {
        return alloc::string::String::from("0 0 0 0\n");
    };
    alloc::format!(
        "{} {} {} {}\n",
        fb.width,
        fb.height,
        fb.pitch,
        fb.bpp
    )
}
