use alloc::vec;
use alloc::vec::Vec;
use spin::Mutex;

use crate::arch::memory::{phys_to_virt, VirtAddr};
use crate::mm::layout::PAGE_SIZE;
use crate::mm::numa::NumaNodeId;
use crate::mm::phys::{self, FrameDescriptor};

use super::{BlockDevice, BlockError, BLOCK_SIZE};

pub struct RamDisk {
    blocks: u64,
    frames: Vec<FrameDescriptor>,
    base_virt: VirtAddr,
}

impl RamDisk {
    pub fn new(size_bytes: u64) -> Option<Self> {
        let blocks = size_bytes.div_ceil(BLOCK_SIZE as u64);
        let pages = blocks.div_ceil(PAGE_SIZE / BLOCK_SIZE as u64);
        let order = phys::order_for_count(pages);
        let frame = phys::alloc_frames(order, NumaNodeId::new(0))?;
        let hhdm = crate::boot_info().hhdm_offset;
        let base_virt = phys_to_virt(hhdm, frame.phys());
        let byte_len = frame.byte_len();
        unsafe {
            core::ptr::write_bytes(base_virt.as_mut_ptr::<u8>(), 0, byte_len as usize);
        }
        Some(Self {
            blocks,
            frames: vec![frame],
            base_virt,
        })
    }

    fn block_ptr(&self, block: u64) -> Option<*mut u8> {
        if block >= self.blocks {
            return None;
        }
        let offset = block * BLOCK_SIZE as u64;
        Some(unsafe { self.base_virt.as_ptr::<u8>().add(offset as usize) as *mut u8 })
    }
}

impl BlockDevice for RamDisk {
    fn block_count(&self) -> u64 {
        self.blocks
    }

    fn read_block(&self, block: u64, buf: &mut [u8]) -> Result<(), BlockError> {
        let ptr = self.block_ptr(block).ok_or(BlockError::OutOfRange)?;
        let len = buf.len().min(BLOCK_SIZE);
        unsafe {
            core::ptr::copy_nonoverlapping(ptr, buf.as_mut_ptr(), len);
        }
        Ok(())
    }

    fn write_block(&self, block: u64, buf: &[u8]) -> Result<(), BlockError> {
        let ptr = self.block_ptr(block).ok_or(BlockError::OutOfRange)?;
        let len = buf.len().min(BLOCK_SIZE);
        unsafe {
            core::ptr::copy_nonoverlapping(buf.as_ptr(), ptr, len);
        }
        Ok(())
    }
}

static RAMDISK0: Mutex<Option<RamDisk>> = Mutex::new(None);
static RAMDISK1: Mutex<Option<RamDisk>> = Mutex::new(None);

pub fn init() {
    *RAMDISK0.lock() = RamDisk::new(16 * 1024 * 1024);
    *RAMDISK1.lock() = RamDisk::new(16 * 1024 * 1024);
}

pub fn disk0() -> &'static Mutex<Option<RamDisk>> {
    &RAMDISK0
}

pub fn disk1() -> &'static Mutex<Option<RamDisk>> {
    &RAMDISK1
}
