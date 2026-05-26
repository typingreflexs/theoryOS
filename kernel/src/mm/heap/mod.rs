//! Kernel heap — slab allocator implementing `GlobalAlloc`.

pub mod slab;

use core::alloc::{GlobalAlloc, Layout};
use core::ptr::NonNull;

use spin::Mutex;

use crate::console::Console;
use crate::mm::aslr;
use crate::mm::layout::PAGE_SIZE;

static HEAP: Mutex<Option<slab::SlabAllocator>> = Mutex::new(None);

pub fn init() {
    let base = aslr::kernel_heap_base();
    let size = crate::mm::layout::KERNEL_HEAP_SIZE as usize;
    unsafe {
        slab::init_region(base as *mut u8, size);
    }
    *HEAP.lock() = Some(slab::SlabAllocator::new());
    Console::println("[mm] slab heap initialized");
}

pub fn alloc(layout: Layout) -> Option<NonNull<u8>> {
    HEAP.lock()
        .as_mut()
        .and_then(|heap| heap.allocate(layout))
}

pub fn dealloc(ptr: NonNull<u8>, layout: Layout) {
    if let Some(heap) = HEAP.lock().as_mut() {
        heap.deallocate(ptr, layout);
    }
}

pub fn realloc(ptr: NonNull<u8>, layout: Layout, new_size: usize) -> Option<NonNull<u8>> {
    HEAP.lock()
        .as_mut()
        .and_then(|heap| heap.reallocate(ptr, layout, new_size))
}

pub struct KernelAllocator;

unsafe impl GlobalAlloc for KernelAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        alloc(layout)
            .map(|p| p.as_ptr())
            .unwrap_or(core::ptr::null_mut())
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if let Some(non_null) = NonNull::new(ptr) {
            dealloc(non_null, layout);
        }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let Some(non_null) = NonNull::new(ptr) else {
            return self.alloc(Layout::from_size_align(new_size, layout.align()).unwrap());
        };
        realloc(non_null, layout, new_size)
            .map(|p| p.as_ptr())
            .unwrap_or(core::ptr::null_mut())
    }
}
