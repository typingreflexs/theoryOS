pub mod bitmap;
pub mod buddy;
pub mod frame;

pub use frame::{FrameAllocator, FrameDescriptor, PhysicalMemoryStats};

use spin::Mutex;

static FRAME_ALLOC: Mutex<Option<FrameAllocator>> = Mutex::new(None);

pub fn init() {
    let mut guard = FRAME_ALLOC.lock();
    *guard = Some(FrameAllocator::new());
}

pub fn with<F, R>(f: F) -> R
where
    F: FnOnce(&mut FrameAllocator) -> R,
{
    f(FRAME_ALLOC.lock().as_mut().expect("frame allocator not initialized"))
}

pub fn stats() -> PhysicalMemoryStats {
    with(|alloc| alloc.stats())
}

pub fn alloc_frame(preferred_node: crate::mm::numa::NumaNodeId) -> Option<FrameDescriptor> {
    with(|alloc| alloc.alloc_one(preferred_node))
}

pub fn alloc_frames(order: u32, preferred_node: crate::mm::numa::NumaNodeId) -> Option<FrameDescriptor> {
    with(|alloc| alloc.alloc_order(order, preferred_node))
}

pub fn order_for_count(pages: u64) -> u32 {
    buddy::order_for_count(pages)
}

pub fn free_frame(frame: FrameDescriptor) {
    with(|alloc| alloc.free(frame));
}

pub fn reserve_range(start: crate::arch::memory::PhysAddr, frames: u64) {
    with(|alloc| alloc.reserve(start, frames));
}

pub fn mark_used(frame: u64) {
    with(|alloc| alloc.mark_used(frame));
}

pub fn mark_free(frame: u64) {
    with(|alloc| alloc.mark_free(frame));
}

pub fn frame_is_free(frame: u64) -> bool {
    with(|alloc| alloc.is_free(frame))
}
