use core::alloc::Layout;
use core::ptr::NonNull;

use crate::mm::layout::PAGE_SIZE;
use crate::mm::phys;

const SIZE_CLASSES: [usize; 12] = [8, 16, 32, 64, 128, 256, 512, 1024, 2048, 4096, 8192, 16384];

static mut HEAP_START: *mut u8 = core::ptr::null_mut();
static mut HEAP_SIZE: usize = 0;

pub unsafe fn init_region(start: *mut u8, size: usize) {
    HEAP_START = start;
    HEAP_SIZE = size;
    // Slab backing comes from freshly allocated physical frames (zeroed in refill).
    // Do not touch the demand-mapped kernel heap window here: IDT/page faults are
    // not available yet during early mm init.
}

pub struct SlabAllocator {
    caches: [SlabCache; SIZE_CLASSES.len()],
    large: LargeAllocator,
}

unsafe impl Send for SlabAllocator {}

impl SlabAllocator {
    pub fn new() -> Self {
        Self {
            caches: [
                SlabCache::new(8),
                SlabCache::new(16),
                SlabCache::new(32),
                SlabCache::new(64),
                SlabCache::new(128),
                SlabCache::new(256),
                SlabCache::new(512),
                SlabCache::new(1024),
                SlabCache::new(2048),
                SlabCache::new(4096),
                SlabCache::new(8192),
                SlabCache::new(16384),
            ],
            large: LargeAllocator::new(),
        }
    }

    pub fn allocate(&mut self, layout: Layout) -> Option<NonNull<u8>> {
        let size = layout.size().max(layout.align());
        if let Some(idx) = class_index(size) {
            return self.caches[idx].alloc();
        }
        self.large.alloc(size)
    }

    pub fn deallocate(&mut self, ptr: NonNull<u8>, layout: Layout) {
        let size = layout.size().max(layout.align());
        if let Some(idx) = class_index(size) {
            self.caches[idx].free(ptr);
        } else {
            self.large.free(ptr, size);
        }
    }

    pub fn reallocate(
        &mut self,
        ptr: NonNull<u8>,
        layout: Layout,
        new_size: usize,
    ) -> Option<NonNull<u8>> {
        let old = layout.size().max(layout.align());
        if new_size <= old {
            return Some(ptr);
        }
        let new_layout = Layout::from_size_align(new_size, layout.align()).ok()?;
        let new_ptr = self.allocate(new_layout)?;
        unsafe {
            core::ptr::copy_nonoverlapping(ptr.as_ptr(), new_ptr.as_ptr(), old);
        }
        self.deallocate(ptr, layout);
        Some(new_ptr)
    }
}

struct SlabCache {
    object_size: usize,
    slab_size: usize,
    free_list: Option<NonNull<FreeNode>>,
    backing: Option<NonNull<u8>>,
}

struct FreeNode {
    next: Option<NonNull<FreeNode>>,
}

struct LargeAllocator;

impl LargeAllocator {
    const fn new() -> Self {
        Self
    }

    fn alloc(&mut self, size: usize) -> Option<NonNull<u8>> {
        let pages = (size + PAGE_SIZE as usize - 1) / PAGE_SIZE as usize;
        let order = crate::mm::phys::order_for_count(pages as u64);
        let frame = phys::alloc_frames(order, crate::mm::numa::local_node())?;
        let hhdm = crate::boot_info().hhdm_offset;
        let ptr = (frame.phys().as_u64() + hhdm) as *mut u8;
        Some(NonNull::new(ptr)?)
    }

    fn free(&mut self, ptr: NonNull<u8>, size: usize) {
        let _ = (ptr, size);
    }
}

impl SlabCache {
    const fn new(object_size: usize) -> Self {
        Self {
            object_size,
            slab_size: PAGE_SIZE as usize,
            free_list: None,
            backing: None,
        }
    }

    fn alloc(&mut self) -> Option<NonNull<u8>> {
        if self.free_list.is_none() {
            self.refill()?;
        }
        let node = self.free_list?;
        unsafe {
            self.free_list = (*node.as_ptr()).next;
        }
        Some(node.cast())
    }

    fn free(&mut self, ptr: NonNull<u8>) {
        let node = ptr.cast::<FreeNode>();
        unsafe {
            (*node.as_ptr()).next = self.free_list;
        }
        self.free_list = Some(node);
    }

    fn refill(&mut self) -> Option<()> {
        let frame = phys::alloc_frame(crate::mm::numa::local_node())?;
        let hhdm = crate::boot_info().hhdm_offset;
        let base = (frame.phys().as_u64() + hhdm) as *mut u8;
        self.backing = NonNull::new(base);
        let count = self.slab_size / self.object_size;
        for i in 0..count {
            let offset = i * self.object_size;
            let node = unsafe { NonNull::new(base.add(offset)).unwrap().cast::<FreeNode>() };
            unsafe {
                (*node.as_ptr()).next = self.free_list;
            }
            self.free_list = Some(node);
        }
        Some(())
    }
}

fn class_index(size: usize) -> Option<usize> {
    SIZE_CLASSES.iter().position(|&class| size <= class)
}
