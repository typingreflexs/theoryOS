pub mod ramdisk;

use spin::Mutex;

pub const BLOCK_SIZE: usize = 4096;

pub trait BlockDevice: Send + Sync {
    fn block_size(&self) -> usize {
        BLOCK_SIZE
    }

    fn block_count(&self) -> u64;

    fn read_block(&self, block: u64, buf: &mut [u8]) -> Result<(), BlockError>;

    fn write_block(&self, block: u64, buf: &[u8]) -> Result<(), BlockError>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockError {
    OutOfRange,
    Io,
}

const CACHE_SLOTS: usize = 64;

struct CacheEntry {
    device_id: u32,
    block: u64,
    data: [u8; BLOCK_SIZE],
    dirty: bool,
    valid: bool,
}

impl CacheEntry {
    const fn empty() -> Self {
        Self {
            device_id: u32::MAX,
            block: 0,
            data: [0; BLOCK_SIZE],
            dirty: false,
            valid: false,
        }
    }
}

struct BufferCache {
    entries: [CacheEntry; CACHE_SLOTS],
    next: usize,
}

impl BufferCache {
    const fn new() -> Self {
        Self {
            entries: [const { CacheEntry::empty() }; CACHE_SLOTS],
            next: 0,
        }
    }

    fn find(&self, device_id: u32, block: u64) -> Option<usize> {
        self.entries.iter().position(|e| {
            e.valid && e.device_id == device_id && e.block == block
        })
    }

    fn evict_slot(&mut self) -> usize {
        let slot = self.next;
        self.next = (self.next + 1) % CACHE_SLOTS;
        if self.entries[slot].dirty {
            // Caller must flush before reuse — simplified: drop dirty flag
            self.entries[slot].dirty = false;
        }
        slot
    }
}

static CACHE: Mutex<BufferCache> = Mutex::new(BufferCache::new());

pub fn read_block(dev: &dyn BlockDevice, device_id: u32, block: u64, buf: &mut [u8]) -> Result<(), BlockError> {
    let mut cache = CACHE.lock();
    if let Some(idx) = cache.find(device_id, block) {
        let len = buf.len().min(BLOCK_SIZE);
        buf[..len].copy_from_slice(&cache.entries[idx].data[..len]);
        return Ok(());
    }
    let slot = cache.evict_slot();
    dev.read_block(block, &mut cache.entries[slot].data)?;
    cache.entries[slot].device_id = device_id;
    cache.entries[slot].block = block;
    cache.entries[slot].valid = true;
    cache.entries[slot].dirty = false;
    let len = buf.len().min(BLOCK_SIZE);
    buf[..len].copy_from_slice(&cache.entries[slot].data[..len]);
    Ok(())
}

pub fn write_block(dev: &dyn BlockDevice, device_id: u32, block: u64, buf: &[u8]) -> Result<(), BlockError> {
    let mut cache = CACHE.lock();
    if let Some(idx) = cache.find(device_id, block) {
        let len = buf.len().min(BLOCK_SIZE);
        cache.entries[idx].data[..len].copy_from_slice(&buf[..len]);
        cache.entries[idx].dirty = true;
    }
    dev.write_block(block, buf)?;
    Ok(())
}

pub fn flush_device(dev: &dyn BlockDevice, device_id: u32) {
    let mut cache = CACHE.lock();
    for entry in cache.entries.iter_mut() {
        if entry.valid && entry.device_id == device_id && entry.dirty {
            let _ = dev.write_block(entry.block, &entry.data);
            entry.dirty = false;
        }
    }
}
