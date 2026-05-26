use core::mem::MaybeUninit;

use super::bitmap::{self, FrameBitmap};

pub const MAX_ORDER: u32 = 10;
pub const MAX_NODES: usize = 8;

#[derive(Clone, Copy, Debug)]
pub struct FreeBlock {
    pub frame: u64,
    pub order: u32,
    pub next: Option<usize>,
}

pub struct BuddyAllocator {
    free_lists: [[Option<usize>; (MAX_ORDER + 1) as usize]; MAX_NODES],
    blocks: &'static mut [MaybeUninit<FreeBlock>],
    block_count: usize,
    next_block: usize,
}

impl BuddyAllocator {
    pub fn new(block_storage: &'static mut [MaybeUninit<FreeBlock>]) -> Self {
        let block_count = block_storage.len();
        Self {
            free_lists: [[None; (MAX_ORDER + 1) as usize]; MAX_NODES],
            blocks: block_storage,
            block_count,
            next_block: 0,
        }
    }

    pub fn add_region(&mut self, bitmap: &mut FrameBitmap, start_frame: u64, frame_count: u64, node: usize) {
        if frame_count == 0 || node >= MAX_NODES {
            return;
        }
        let mut frame = start_frame;
        let end = start_frame + frame_count;
        while frame < end {
            let mut order = MAX_ORDER;
            while order > 0 {
                let block_frames = 1u64 << order;
                if frame % block_frames != 0 || frame + block_frames > end {
                    order -= 1;
                    continue;
                }
                let mut ok = true;
                for f in frame..frame + block_frames {
                    if !bitmap.is_free(f) {
                        ok = false;
                        break;
                    }
                }
                if ok {
                    break;
                }
                order -= 1;
            }
            let block_frames = 1u64 << order;
            for f in frame..frame + block_frames {
                bitmap.set_used(f);
            }
            self.push_block(node, frame, order);
            frame += block_frames;
        }
    }

    pub fn alloc(&mut self, bitmap: &mut FrameBitmap, order: u32, node: usize) -> Option<u64> {
        let order = order.min(MAX_ORDER);
        for search in (order..=MAX_ORDER).rev() {
            if let Some(idx) = self.free_lists[node][search as usize] {
                let block = self.block(idx);
                self.free_lists[node][search as usize] = block.next;
                let mut current_order = search;
                let mut frame = block.frame;
                while current_order > order {
                    current_order -= 1;
                    let buddy = frame + (1u64 << current_order);
                    bitmap.set_free(buddy);
                    self.push_block(node, buddy, current_order);
                }
                bitmap.set_used(frame);
                return Some(frame);
            }
        }
        // Split larger blocks from other nodes if needed
        for other in 0..MAX_NODES {
            if other == node {
                continue;
            }
            for search in (order..=MAX_ORDER).rev() {
                if let Some(idx) = self.free_lists[other][search as usize] {
                    let block = self.block(idx);
                    self.free_lists[other][search as usize] = block.next;
                    let mut current_order = search;
                    let mut frame = block.frame;
                    while current_order > order {
                        current_order -= 1;
                        let buddy = frame + (1u64 << current_order);
                        bitmap.set_free(buddy);
                        self.push_block(other, buddy, current_order);
                    }
                    bitmap.set_used(frame);
                    return Some(frame);
                }
            }
        }
        None
    }

    pub fn free(&mut self, bitmap: &mut FrameBitmap, frame: u64, order: u32, node: usize) {
        let mut frame = frame;
        let mut order = order.min(MAX_ORDER);
        bitmap.set_free(frame);
        while order <= MAX_ORDER {
            let buddy = frame ^ (1u64 << order);
            if buddy >= bitmap.frame_count() || !self.take_buddy(bitmap, node, buddy, order) {
                break;
            }
            frame = frame.min(buddy);
            order += 1;
            bitmap.set_free(frame);
        }
        self.push_block(node, frame, order);
    }

    fn take_buddy(&mut self, bitmap: &FrameBitmap, node: usize, buddy: u64, order: u32) -> bool {
        if !bitmap.is_free(buddy) {
            return false;
        }
        let order_idx = order as usize;
        let mut prev = None;
        let mut current = self.free_lists[node][order_idx];
        while let Some(idx) = current {
            let block = self.block(idx);
            if block.frame == buddy && block.order == order {
                let next = block.next;
                match prev {
                    None => self.free_lists[node][order_idx] = next,
                    Some(p) => self.block_mut(p).next = next,
                }
                return true;
            }
            prev = Some(idx);
            current = block.next;
        }
        false
    }

    fn push_block(&mut self, node: usize, frame: u64, order: u32) {
        assert!(self.next_block < self.block_count, "buddy block pool exhausted");
        let idx = self.next_block;
        self.next_block += 1;
        self.blocks[idx].write(FreeBlock {
            frame,
            order,
            next: self.free_lists[node][order as usize],
        });
        self.free_lists[node][order as usize] = Some(idx);
    }

    fn block(&self, idx: usize) -> FreeBlock {
        unsafe { *self.blocks[idx].assume_init_ref() }
    }

    fn block_mut(&mut self, idx: usize) -> &mut FreeBlock {
        unsafe { self.blocks[idx].assume_init_mut() }
    }
}

pub fn order_for_count(pages: u64) -> u32 {
    let mut order = 0u32;
    let mut need = pages.next_power_of_two().max(1);
    while (1u64 << order) < need && order < MAX_ORDER {
        order += 1;
    }
    order
}
