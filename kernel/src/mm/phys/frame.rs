use core::mem::MaybeUninit;

use crate::arch::memory::PhysAddr;
use crate::boot::info::{MemoryKind, MemoryRegion};
use crate::mm::layout::{self, PAGE_SIZE};
use crate::mm::numa::NumaNodeId;

use super::bitmap::{self, FrameBitmap};
use super::buddy::{self, BuddyAllocator, MAX_NODES, MAX_ORDER};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameDescriptor {
    pub frame: u64,
    pub order: u32,
    pub node: NumaNodeId,
}

impl FrameDescriptor {
    pub fn phys(&self) -> PhysAddr {
        PhysAddr::new(bitmap::phys_from_frame(self.frame))
    }

    pub fn page_count(&self) -> u64 {
        1u64 << self.order
    }

    pub fn byte_len(&self) -> u64 {
        self.page_count() * PAGE_SIZE
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct PhysicalMemoryStats {
    pub total_frames: u64,
    pub free_frames: u64,
    pub free_bytes: u64,
}

const MAX_BITMAP_WORDS: usize = 65536; // tracks up to 4M frames (16 GiB)
const MAX_BUDDY_BLOCKS: usize = 8192;

static mut BITMAP_STORAGE: [u64; MAX_BITMAP_WORDS] = [0; MAX_BITMAP_WORDS];
static mut BUDDY_STORAGE: [MaybeUninit<buddy::FreeBlock>; MAX_BUDDY_BLOCKS] =
    [MaybeUninit::uninit(); MAX_BUDDY_BLOCKS];

pub struct FrameAllocator {
    bitmap: FrameBitmap,
    buddy: BuddyAllocator,
    total_frames: u64,
}

// SAFETY: protected by global mutex in phys/mod.rs
unsafe impl Send for FrameAllocator {}

impl FrameAllocator {
    pub fn new() -> Self {
        let boot = crate::boot_info();
        let max_phys = boot
            .memory_map
            .iter()
            .map(|r| r.start.as_u64() + r.length)
            .max()
            .unwrap_or(256 * 1024 * 1024);
        let frame_count = layout::align_up(max_phys, PAGE_SIZE) / PAGE_SIZE;
        let words = bitmap::words_for_frames(frame_count).min(MAX_BITMAP_WORDS);

        let mut bitmap = unsafe {
            FrameBitmap::new(
                &mut BITMAP_STORAGE[..words],
                (words as u64) * 64,
            )
        };

        // Mark every tracked frame allocated, then release usable ranges.
        bitmap.mark_all_used();

        reserve_kernel(&mut bitmap, boot.kernel_physical_base);
        reserve_bitmap_metadata(&mut bitmap);

        let frame_cap = bitmap.frame_count();
        for region in boot.memory_map {
            if !is_allocatable(region.kind) {
                continue;
            }
            let start_frame = bitmap::frame_from_phys(region.start.as_u64());
            if start_frame >= frame_cap {
                continue;
            }
            let mut end_frame = start_frame.saturating_add(region.length / PAGE_SIZE);
            if end_frame > frame_cap {
                end_frame = frame_cap;
            }
            for frame in start_frame..end_frame {
                bitmap.set_free(frame);
            }
        }

        let total_frames = bitmap.frame_count();
        let buddy = unsafe { BuddyAllocator::new(&mut BUDDY_STORAGE) };
        Self {
            bitmap,
            buddy,
            total_frames,
        }
    }

    pub fn stats(&self) -> PhysicalMemoryStats {
        let free_frames = self.bitmap.count_free();
        PhysicalMemoryStats {
            total_frames: self.total_frames,
            free_frames,
            free_bytes: free_frames * PAGE_SIZE,
        }
    }

    pub fn alloc_one(&mut self, node: NumaNodeId) -> Option<FrameDescriptor> {
        if let Some(frame) = self.bitmap.find_free() {
            self.bitmap.set_used(frame);
            return Some(FrameDescriptor {
                frame,
                order: 0,
                node,
            });
        }
        self.alloc_order(0, node)
    }

    pub fn alloc_order(&mut self, order: u32, node: NumaNodeId) -> Option<FrameDescriptor> {
        let order = order.min(MAX_ORDER);
        if order == 0 {
            if let Some(frame) = self.bitmap.find_free() {
                self.bitmap.set_used(frame);
                return Some(FrameDescriptor {
                    frame,
                    order: 0,
                    node,
                });
            }
        }
        let frame = self
            .buddy
            .alloc(&mut self.bitmap, order, node.as_usize().min(MAX_NODES - 1))?;
        Some(FrameDescriptor {
            frame,
            order,
            node,
        })
    }

    pub fn free(&mut self, desc: FrameDescriptor) {
        if desc.order == 0 {
            self.bitmap.set_free(desc.frame);
            return;
        }
        self.buddy.free(
            &mut self.bitmap,
            desc.frame,
            desc.order,
            desc.node.as_usize().min(MAX_NODES - 1),
        );
    }

    pub fn reserve(&mut self, start: PhysAddr, frames: u64) {
        self.bitmap.reserve(bitmap::frame_from_phys(start.as_u64()), frames);
    }

    pub fn mark_used(&mut self, frame: u64) {
        self.bitmap.set_used(frame);
    }

    pub fn mark_free(&mut self, frame: u64) {
        self.bitmap.set_free(frame);
    }

    pub fn is_free(&self, frame: u64) -> bool {
        self.bitmap.is_free(frame)
    }
}

fn is_allocatable(kind: MemoryKind) -> bool {
    matches!(
        kind,
        MemoryKind::Usable | MemoryKind::BootloaderReclaimable | MemoryKind::AcpiReclaimable
    )
}

fn reserve_kernel(bitmap: &mut FrameBitmap, kernel_base: u64) {
    let start = bitmap::frame_from_phys(kernel_base & !(PAGE_SIZE - 1));
    // Reserve 16 MiB for kernel image + early structures.
    bitmap.reserve(start, 4096);
}

fn reserve_bitmap_metadata(bitmap: &mut FrameBitmap) {
    let _ = bitmap;
}
