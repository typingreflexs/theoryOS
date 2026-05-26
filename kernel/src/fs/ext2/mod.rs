//! ext2 read/write driver on a block device.

use alloc::collections::BTreeMap;
use alloc::string::String;
use spin::Mutex;

use crate::fs::block::{self, BlockDevice, BlockError, BLOCK_SIZE};
use crate::fs::vfs::superblock::{DirEntry, FileSystem, FsError};
use crate::fs::vfs::inode::{FileType, InodeAttr, InodeId, InodeMode};

const EXT2_MAGIC: u16 = 0xEF53;
const ROOT_INO: u32 = 2;
const INODES_PER_GROUP: u32 = 128;
const BLOCKS_PER_GROUP: u32 = 4096;

fn blk(result: Result<(), BlockError>) -> Result<(), FsError> {
    result.map_err(|_| FsError::Io)
}

#[repr(C, packed)]
struct Ext2Super {
    inodes_count: u32,
    blocks_count: u32,
    free_blocks: u32,
    free_inodes: u32,
    first_data_block: u32,
    log_block_size: u32,
    blocks_per_group: u32,
    inodes_per_group: u32,
    magic: u16,
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
struct Ext2Inode {
    mode: u16,
    size: u32,
    blocks: u32,
    block: [u32; 12],
}

#[repr(C, packed)]
struct Ext2DirEntry {
    inode: u32,
    rec_len: u16,
    name_len: u8,
    file_type: u8,
}

struct Ext2Inner {
    mount_id: u32,
    device_id: u32,
    dev: &'static dyn BlockDevice,
    superblock: Ext2Super,
    path_cache: BTreeMap<u64, BTreeMap<String, u32>>,
}

pub struct Ext2Fs {
    inner: Mutex<Ext2Inner>,
}

impl Ext2Fs {
    pub fn format_and_mount(
        mount_id: u32,
        device_id: u32,
        dev: &'static dyn BlockDevice,
    ) -> Result<Self, FsError> {
        let blocks = dev.block_count() as u32;
        let inodes = INODES_PER_GROUP;
        let mut sb = Ext2Super {
            inodes_count: inodes,
            blocks_count: blocks,
            free_blocks: blocks - 10,
            free_inodes: inodes - 2,
            first_data_block: 1,
            log_block_size: 2,
            blocks_per_group: BLOCKS_PER_GROUP.min(blocks),
            inodes_per_group: INODES_PER_GROUP,
            magic: EXT2_MAGIC,
        };

        write_super(dev, device_id, &sb)?;
        init_root_dir(dev, device_id)?;

        Ok(Self {
            inner: Mutex::new(Ext2Inner {
                mount_id,
                device_id,
                dev,
                superblock: sb,
                path_cache: BTreeMap::new(),
            }),
        })
    }

    fn block_size(&self) -> u32 {
        1024 << self.inner.lock().superblock.log_block_size
    }

    fn read_inode_raw(&self, ino: u32) -> Result<Ext2Inode, FsError> {
        let inner = self.inner.lock();
        let bs = 1024 << inner.superblock.log_block_size;
        let inode_table_block = 5u32;
        let inode_size = 128u32;
        let index = ino - 1;
        let block = inode_table_block + (index * inode_size) / bs;
        let offset = ((index * inode_size) % bs) as usize;
        let mut buf = [0u8; BLOCK_SIZE];
        blk(block::read_block(inner.dev, inner.device_id, block as u64, &mut buf))?;
        let raw = unsafe { &*(buf.as_ptr().add(offset) as *const Ext2Inode) };
        Ok(*raw)
    }

    fn write_inode_raw(&self, ino: u32, inode: &Ext2Inode) -> Result<(), FsError> {
        let mut inner = self.inner.lock();
        let bs = 1024 << inner.superblock.log_block_size;
        let inode_table_block = 5u32;
        let inode_size = 128u32;
        let index = ino - 1;
        let block = inode_table_block + (index * inode_size) / bs;
        let offset = ((index * inode_size) % bs) as usize;
        let mut buf = [0u8; BLOCK_SIZE];
        blk(block::read_block(inner.dev, inner.device_id, block as u64, &mut buf))?;
        unsafe {
            core::ptr::copy_nonoverlapping(
                inode as *const Ext2Inode as *const u8,
                buf.as_mut_ptr().add(offset),
                core::mem::size_of::<Ext2Inode>(),
            );
        }
        blk(block::write_block(inner.dev, inner.device_id, block as u64, &buf))?;
        Ok(())
    }

    fn read_file_blocks(&self, inode: &Ext2Inode, offset: u64, buf: &mut [u8]) -> Result<usize, FsError> {
        let inner = self.inner.lock();
        let bs = self.block_size() as u64;
        if offset >= inode.size as u64 {
            return Ok(0);
        }
        let mut done = 0usize;
        let mut pos = offset;
        while done < buf.len() && pos < inode.size as u64 {
            let block_idx = (pos / bs) as usize;
            if block_idx >= 12 {
                break;
            }
            let block_num = inode.block[block_idx];
            if block_num == 0 {
                break;
            }
            let mut block_buf = [0u8; BLOCK_SIZE];
            blk(block::read_block(inner.dev, inner.device_id, block_num as u64, &mut block_buf))?;
            let start = (pos % bs) as usize;
            let take = (bs as usize - start).min(buf.len() - done).min(inode.size as usize - pos as usize);
            buf[done..done + take].copy_from_slice(&block_buf[start..start + take]);
            done += take;
            pos += take as u64;
        }
        Ok(done)
    }

    fn write_file_blocks(&self, ino: u32, inode: &mut Ext2Inode, offset: u64, data: &[u8]) -> Result<usize, FsError> {
        let inner = self.inner.lock();
        let bs = self.block_size() as u64;
        let mut done = 0usize;
        let mut pos = offset;
        while done < data.len() {
            let block_idx = (pos / bs) as usize;
            if block_idx >= 12 {
                return Err(FsError::NoSpace);
            }
            if inode.block[block_idx] == 0 {
                inode.block[block_idx] = inner.superblock.free_blocks;
                // simplified allocator
            }
            let block_num = inode.block[block_idx];
            let mut block_buf = [0u8; BLOCK_SIZE];
            blk(block::read_block(inner.dev, inner.device_id, block_num as u64, &mut block_buf))?;
            let start = (pos % bs) as usize;
            let take = (bs as usize - start).min(data.len() - done);
            block_buf[start..start + take].copy_from_slice(&data[done..done + take]);
            blk(block::write_block(inner.dev, inner.device_id, block_num as u64, &block_buf))?;
            done += take;
            pos += take as u64;
        }
        let new_size = (offset + done as u64).max(inode.size as u64) as u32;
        inode.size = new_size;
        drop(inner);
        self.write_inode_raw(ino, inode)?;
        Ok(done)
    }

    fn mode_to_type(mode: u16) -> FileType {
        match mode & 0xF000 {
            0x4000 => FileType::Directory,
            0xA000 => FileType::Symlink,
            0x2000 => FileType::CharDev,
            0x6000 => FileType::BlockDev,
            _ => FileType::Regular,
        }
    }
}

fn write_super(dev: &dyn BlockDevice, device_id: u32, sb: &Ext2Super) -> Result<(), FsError> {
    let mut buf = [0u8; BLOCK_SIZE];
    unsafe {
        core::ptr::copy_nonoverlapping(
            sb as *const Ext2Super as *const u8,
            buf.as_mut_ptr().add(1024),
            core::mem::size_of::<Ext2Super>(),
        );
    }
    block::write_block(dev, device_id, 1, &buf).map_err(|_| FsError::Io)
}

fn init_root_dir(dev: &dyn BlockDevice, device_id: u32) -> Result<(), FsError> {
    let mut inode = Ext2Inode {
        mode: 0x41ED,
        size: BLOCK_SIZE as u32,
        blocks: 1,
        block: [0; 12],
    };
    inode.block[0] = 10;
    let mut dir_block = [0u8; BLOCK_SIZE];
    let mut off = 0usize;
    write_dir_entry(&mut dir_block, &mut off, 2, ".", 1);
    write_dir_entry(&mut dir_block, &mut off, 2, "..", 1);
    block::write_block(dev, device_id, 10, &dir_block).map_err(|_| FsError::Io)?;

    let inode_table_block = 5u32;
    let mut buf = [0u8; BLOCK_SIZE];
    block::read_block(dev, device_id, inode_table_block as u64, &mut buf).map_err(|_| FsError::Io)?;
    unsafe {
        core::ptr::copy_nonoverlapping(
            &inode as *const Ext2Inode as *const u8,
            buf.as_mut_ptr().add(128),
            core::mem::size_of::<Ext2Inode>(),
        );
    }
    block::write_block(dev, device_id, inode_table_block as u64, &buf).map_err(|_| FsError::Io)
}

fn write_dir_entry(block: &mut [u8], off: &mut usize, ino: u32, name: &str, file_type: u8) {
    let name_bytes = name.as_bytes();
    let rec_len = (8 + name_bytes.len() + 3) & !3;
    let entry = Ext2DirEntry {
        inode: ino,
        rec_len: rec_len as u16,
        name_len: name_bytes.len() as u8,
        file_type,
    };
    unsafe {
        core::ptr::copy_nonoverlapping(
            &entry as *const Ext2DirEntry as *const u8,
            block.as_mut_ptr().add(*off),
            8,
        );
        core::ptr::copy_nonoverlapping(
            name_bytes.as_ptr(),
            block.as_mut_ptr().add(*off + 8),
            name_bytes.len(),
        );
    }
    *off += rec_len;
}

impl FileSystem for Ext2Fs {
    fn fs_type(&self) -> &'static str {
        "ext2"
    }

    fn root(&self) -> InodeId {
        InodeId::new(self.inner.lock().mount_id, ROOT_INO as u64)
    }

    fn getattr(&self, inode: InodeId) -> Result<InodeAttr, FsError> {
        let inner = self.inner.lock();
        if inode.mount != inner.mount_id {
            return Err(FsError::NotFound);
        }
        let raw = self.read_inode_raw(inode.ino as u32)?;
        Ok(InodeAttr {
            ino: inode.ino,
            mode: InodeMode::from_bits_truncate(raw.mode & 0o7777),
            file_type: Self::mode_to_type(raw.mode),
            size: raw.size as u64,
            nlink: 1,
            generation: 1,
            rdev: 0,
        })
    }

    fn lookup(&self, dir: InodeId, name: &[u8]) -> Result<InodeId, FsError> {
        let raw = self.read_inode_raw(dir.ino as u32)?;
        if raw.mode & 0xF000 != 0x4000 {
            return Err(FsError::NotDir);
        }
        let inner = self.inner.lock();
        let mut block_buf = [0u8; BLOCK_SIZE];
        blk(block::read_block(inner.dev, inner.device_id, raw.block[0] as u64, &mut block_buf))?;
        let mut off = 0usize;
        while off < raw.size as usize {
            let entry = unsafe { &*(block_buf.as_ptr().add(off) as *const Ext2DirEntry) };
            if entry.inode == 0 {
                break;
            }
            let nlen = entry.name_len as usize;
            let ename = &block_buf[off + 8..off + 8 + nlen];
            if ename == name {
                return Ok(InodeId::new(inner.mount_id, entry.inode as u64));
            }
            off += entry.rec_len as usize;
        }
        Err(FsError::NotFound)
    }

    fn create(
        &self,
        dir: InodeId,
        name: &[u8],
        file_type: FileType,
        mode: InodeMode,
    ) -> Result<InodeId, FsError> {
        let mut inner = self.inner.lock();
        let new_ino = inner.superblock.free_inodes;
        inner.superblock.free_inodes -= 1;
        let mode_bits = match file_type {
            FileType::Directory => 0x4000,
            FileType::Regular => 0x8000,
            _ => 0x8000,
        } | mode.bits();
        let inode = Ext2Inode {
            mode: mode_bits,
            size: 0,
            blocks: 0,
            block: [0; 12],
        };
        drop(inner);
        self.write_inode_raw(new_ino, &inode)?;

        let dir_raw = self.read_inode_raw(dir.ino as u32)?;
        let mut block_buf = [0u8; BLOCK_SIZE];
        let inner = self.inner.lock();
        blk(block::read_block(inner.dev, inner.device_id, dir_raw.block[0] as u64, &mut block_buf))?;
        let mut off = dir_raw.size as usize;
        let name_str = core::str::from_utf8(name).map_err(|_| FsError::Invalid)?;
        write_dir_entry(&mut block_buf, &mut off, new_ino, name_str, 1);
        blk(block::write_block(inner.dev, inner.device_id, dir_raw.block[0] as u64, &block_buf))?;
        Ok(InodeId::new(inner.mount_id, new_ino as u64))
    }

    fn mkdir(&self, dir: InodeId, name: &[u8], mode: InodeMode) -> Result<InodeId, FsError> {
        self.create(dir, name, FileType::Directory, mode)
    }

    fn unlink(&self, dir: InodeId, name: &[u8]) -> Result<(), FsError> {
        let _ = (dir, name);
        Err(FsError::NotSupported)
    }

    fn read(&self, inode: InodeId, offset: u64, buf: &mut [u8]) -> Result<usize, FsError> {
        let raw = self.read_inode_raw(inode.ino as u32)?;
        self.read_file_blocks(&raw, offset, buf)
    }

    fn write(&self, inode: InodeId, offset: u64, buf: &[u8]) -> Result<usize, FsError> {
        let mut raw = self.read_inode_raw(inode.ino as u32)?;
        self.write_file_blocks(inode.ino as u32, &mut raw, offset, buf)
    }

    fn readdir(&self, dir: InodeId, index: u64, entry: &mut DirEntry) -> Result<bool, FsError> {
        let raw = self.read_inode_raw(dir.ino as u32)?;
        let inner = self.inner.lock();
        let mut block_buf = [0u8; BLOCK_SIZE];
        blk(block::read_block(inner.dev, inner.device_id, raw.block[0] as u64, &mut block_buf))?;
        let mut off = 0usize;
        let mut i = 0u64;
        while off < raw.size as usize {
            let de = unsafe { &*(block_buf.as_ptr().add(off) as *const Ext2DirEntry) };
            if de.inode == 0 {
                break;
            }
            if i == index {
                let nlen = de.name_len as usize;
                entry.name_len = nlen.min(256);
                entry.name[..entry.name_len].copy_from_slice(&block_buf[off + 8..off + 8 + entry.name_len]);
                entry.inode = InodeId::new(inner.mount_id, de.inode as u64);
                entry.file_type = FileType::Regular;
                return Ok(true);
            }
            off += de.rec_len as usize;
            i += 1;
        }
        Ok(false)
    }

    fn readlink(&self, _: InodeId, _: &mut [u8]) -> Result<usize, FsError> {
        Err(FsError::NotSymlink)
    }

    fn symlink(
        &self,
        _: InodeId,
        _: &[u8],
        _: &[u8],
        _: InodeMode,
    ) -> Result<InodeId, FsError> {
        Err(FsError::NotSupported)
    }

    fn truncate(&self, inode: InodeId, size: u64) -> Result<(), FsError> {
        let mut raw = self.read_inode_raw(inode.ino as u32)?;
        raw.size = size as u32;
        self.write_inode_raw(inode.ino as u32, &raw)
    }
}
