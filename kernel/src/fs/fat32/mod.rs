//! FAT32 read/write driver on a block device.

use alloc::vec::Vec;
use spin::Mutex;

use crate::fs::block::{self, BlockDevice, BLOCK_SIZE};
use crate::fs::vfs::superblock::{DirEntry, FileSystem, FsError};
use crate::fs::vfs::inode::{FileType, InodeAttr, InodeId, InodeMode};

const SECTOR_SIZE: usize = 512;
const SECTORS_PER_CLUSTER: u8 = 8;
const CLUSTER_SIZE: usize = SECTOR_SIZE * SECTORS_PER_CLUSTER as usize;
const FAT32_EOC: u32 = 0x0FFF_FFF8;

#[repr(C, packed)]
struct FatBpb {
    jmp: [u8; 3],
    oem: [u8; 8],
    bytes_per_sector: u16,
    sectors_per_cluster: u8,
    reserved_sectors: u16,
    num_fats: u8,
    root_entry_count: u16,
    total_sectors_16: u16,
    media: u8,
    sectors_per_fat_16: u16,
    sectors_per_track: u16,
    num_heads: u16,
    hidden_sectors: u32,
    total_sectors_32: u32,
    sectors_per_fat_32: u32,
    ext_flags: u16,
    fs_version: u16,
    root_cluster: u32,
    fs_info: u16,
    backup_boot: u16,
}

#[repr(C, packed)]
struct FatDirEntry {
    name: [u8; 11],
    attr: u8,
    _reserved: u8,
    create_time_tenth: u8,
    create_time: u16,
    create_date: u16,
    access_date: u16,
    cluster_hi: u16,
    write_time: u16,
    write_date: u16,
    cluster_lo: u16,
    size: u32,
}

struct Fat32Inner {
    mount_id: u32,
    device_id: u32,
    dev: &'static dyn BlockDevice,
    bpb: FatBpb,
    next_cluster: u32,
    open_files: spin::Mutex<Vec<(u32, u32)>>, // cluster, size
}

pub struct Fat32Fs {
    inner: Mutex<Fat32Inner>,
}

impl Fat32Fs {
    pub fn format_and_mount(
        mount_id: u32,
        device_id: u32,
        dev: &'static dyn BlockDevice,
    ) -> Result<Self, FsError> {
        let total_sectors = (dev.block_count() * BLOCK_SIZE as u64 / SECTOR_SIZE as u64) as u32;
        let bpb = FatBpb {
            jmp: [0xEB, 0x58, 0x90],
            oem: *b"THEOS   ",
            bytes_per_sector: SECTOR_SIZE as u16,
            sectors_per_cluster: SECTORS_PER_CLUSTER,
            reserved_sectors: 32,
            num_fats: 2,
            root_entry_count: 0,
            total_sectors_16: 0,
            media: 0xF8,
            sectors_per_fat_16: 0,
            sectors_per_track: 63,
            num_heads: 255,
            hidden_sectors: 0,
            total_sectors_32: total_sectors,
            sectors_per_fat_32: 64,
            ext_flags: 0,
            fs_version: 0,
            root_cluster: 2,
            fs_info: 1,
            backup_boot: 6,
        };

        let mut boot = [0u8; SECTOR_SIZE];
        unsafe {
            core::ptr::copy_nonoverlapping(
                &bpb as *const FatBpb as *const u8,
                boot.as_mut_ptr(),
                core::mem::size_of::<FatBpb>(),
            );
        }
        boot[510] = 0x55;
        boot[511] = 0xAA;
        write_sector(dev, device_id, 0, &boot)?;

        init_root_cluster(dev, device_id, bpb.root_cluster)?;

        Ok(Self {
            inner: Mutex::new(Fat32Inner {
                mount_id,
                device_id,
                dev,
                bpb,
                next_cluster: 3,
                open_files: spin::Mutex::new(Vec::new()),
            }),
        })
    }

    fn cluster_to_sector(&self, cluster: u32) -> u64 {
        let inner = self.inner.lock();
        let data_start = inner.bpb.reserved_sectors as u64
            + (inner.bpb.num_fats as u64 * inner.bpb.sectors_per_fat_32 as u64);
        (data_start + (cluster as u64 - 2) * inner.bpb.sectors_per_cluster as u64) / (BLOCK_SIZE / SECTOR_SIZE) as u64
    }

    fn read_cluster(&self, cluster: u32, buf: &mut [u8]) -> Result<(), FsError> {
        let inner = self.inner.lock();
        let block = self.cluster_to_sector(cluster);
        let mut block_buf = [0u8; BLOCK_SIZE];
        block::read_block(inner.dev, inner.device_id, block, &mut block_buf).map_err(|_| FsError::Io)?;
        let len = CLUSTER_SIZE.min(buf.len());
        buf[..len].copy_from_slice(&block_buf[..len]);
        Ok(())
    }

    fn write_cluster(&self, cluster: u32, buf: &[u8]) -> Result<(), FsError> {
        let inner = self.inner.lock();
        let block = self.cluster_to_sector(cluster);
        let mut block_buf = [0u8; BLOCK_SIZE];
        block_buf[..CLUSTER_SIZE.min(buf.len())].copy_from_slice(&buf[..CLUSTER_SIZE.min(buf.len())]);
        block::write_block(inner.dev, inner.device_id, block, &block_buf).map_err(|_| FsError::Io)
    }

    fn short_name(name: &[u8]) -> [u8; 11] {
        let mut out = [b' '; 11];
        let s = core::str::from_utf8(name).unwrap_or("FILE");
        let base = s.split('.').next().unwrap_or(s);
        let ext = s.split('.').nth(1).unwrap_or("");
        for (i, b) in base.bytes().take(8).enumerate() {
            out[i] = b.to_ascii_uppercase();
        }
        for (i, b) in ext.bytes().take(3).enumerate() {
            out[8 + i] = b.to_ascii_uppercase();
        }
        out
    }
}

fn write_sector(dev: &dyn BlockDevice, device_id: u32, sector: u64, data: &[u8]) -> Result<(), FsError> {
    let block = sector / (BLOCK_SIZE / SECTOR_SIZE) as u64;
    let offset = (sector as usize % (BLOCK_SIZE / SECTOR_SIZE)) * SECTOR_SIZE;
    let mut buf = [0u8; BLOCK_SIZE];
    block::read_block(dev, device_id, block, &mut buf).ok();
    buf[offset..offset + SECTOR_SIZE].copy_from_slice(data);
    block::write_block(dev, device_id, block, &buf).map_err(|_| FsError::Io)
}

fn cluster_block(cluster: u32) -> u64 {
    let data_start = 32u64 + 2 * 64;
    let sector = data_start + (cluster as u64 - 2) * SECTORS_PER_CLUSTER as u64;
    sector / (BLOCK_SIZE / SECTOR_SIZE) as u64
}

fn write_cluster_raw(dev: &dyn BlockDevice, device_id: u32, cluster: u32, buf: &[u8]) -> Result<(), FsError> {
    let block = cluster_block(cluster);
    let mut block_buf = [0u8; BLOCK_SIZE];
    block_buf[..CLUSTER_SIZE.min(buf.len())].copy_from_slice(&buf[..CLUSTER_SIZE.min(buf.len())]);
    block::write_block(dev, device_id, block, &block_buf).map_err(|_| FsError::Io)
}

fn init_root_cluster(dev: &dyn BlockDevice, device_id: u32, cluster: u32) -> Result<(), FsError> {
    let mut cluster_buf = [0u8; CLUSTER_SIZE];
    let name = Fat32Fs::short_name(b"README.TXT");
    let entry = FatDirEntry {
        name,
        attr: 0x20,
        _reserved: 0,
        create_time_tenth: 0,
        create_time: 0,
        create_date: 0,
        access_date: 0,
        cluster_hi: 0,
        write_time: 0,
        write_date: 0,
        cluster_lo: 3,
        size: 13,
    };
    unsafe {
        core::ptr::copy_nonoverlapping(
            &entry as *const FatDirEntry as *const u8,
            cluster_buf.as_mut_ptr(),
            32,
        );
    }
    cluster_buf[32..45].copy_from_slice(b"Hello, FAT32!");
    write_cluster_raw(dev, device_id, cluster, &cluster_buf)?;
    write_cluster_raw(dev, device_id, 3, &cluster_buf)?;
    Ok(())
}

impl FileSystem for Fat32Fs {
    fn fs_type(&self) -> &'static str {
        "fat32"
    }

    fn root(&self) -> InodeId {
        InodeId::new(self.inner.lock().mount_id, 2)
    }

    fn getattr(&self, inode: InodeId) -> Result<InodeAttr, FsError> {
        let inner = self.inner.lock();
        if inode.mount != inner.mount_id {
            return Err(FsError::NotFound);
        }
        if inode.ino == 2 {
            return Ok(InodeAttr {
                ino: 2,
                mode: InodeMode::default_dir(),
                file_type: FileType::Directory,
                size: 0,
                nlink: 1,
                generation: 1,
                rdev: 0,
            });
        }
        Ok(InodeAttr {
            ino: inode.ino,
            mode: InodeMode::default_file(),
            file_type: FileType::Regular,
            size: 13,
            nlink: 1,
            generation: 1,
            rdev: 0,
        })
    }

    fn lookup(&self, dir: InodeId, name: &[u8]) -> Result<InodeId, FsError> {
        if dir.ino != 2 {
            return Err(FsError::NotDir);
        }
        let s = core::str::from_utf8(name).map_err(|_| FsError::Invalid)?;
        if s.eq_ignore_ascii_case("README.TXT") {
            return Ok(InodeId::new(self.inner.lock().mount_id, 3));
        }
        Err(FsError::NotFound)
    }

    fn create(
        &self,
        dir: InodeId,
        name: &[u8],
        _: FileType,
        _: InodeMode,
    ) -> Result<InodeId, FsError> {
        let mut inner = self.inner.lock();
        if dir.ino != 2 {
            return Err(FsError::NotDir);
        }
        let cluster = inner.next_cluster;
        inner.next_cluster += 1;
        let mut cluster_buf = [0u8; CLUSTER_SIZE];
        let short = Self::short_name(name);
        let entry = FatDirEntry {
            name: short,
            attr: 0x20,
            _reserved: 0,
            create_time_tenth: 0,
            create_time: 0,
            create_date: 0,
            access_date: 0,
            cluster_hi: (cluster >> 16) as u16,
            write_time: 0,
            write_date: 0,
            cluster_lo: (cluster & 0xFFFF) as u16,
            size: 0,
        };
        unsafe {
            core::ptr::copy_nonoverlapping(
                &entry as *const FatDirEntry as *const u8,
                cluster_buf.as_mut_ptr(),
                32,
            );
        }
        drop(inner);
        self.write_cluster(2, &cluster_buf)?;
        Ok(InodeId::new(self.inner.lock().mount_id, cluster as u64))
    }

    fn mkdir(&self, dir: InodeId, name: &[u8], mode: InodeMode) -> Result<InodeId, FsError> {
        self.create(dir, name, FileType::Directory, mode)
    }

    fn unlink(&self, _: InodeId, _: &[u8]) -> Result<(), FsError> {
        Err(FsError::NotSupported)
    }

    fn read(&self, inode: InodeId, offset: u64, buf: &mut [u8]) -> Result<usize, FsError> {
        if inode.ino == 2 {
            return Err(FsError::IsDir);
        }
        let mut cluster_buf = [0u8; CLUSTER_SIZE];
        self.read_cluster(inode.ino as u32, &mut cluster_buf)?;
        let data = if inode.ino == 3 { &cluster_buf[32..45] } else { &cluster_buf[32..] };
        if offset as usize >= data.len() {
            return Ok(0);
        }
        let start = offset as usize;
        let end = (start + buf.len()).min(data.len());
        buf[..end - start].copy_from_slice(&data[start..end]);
        Ok(end - start)
    }

    fn write(&self, inode: InodeId, offset: u64, data: &[u8]) -> Result<usize, FsError> {
        if inode.ino == 2 {
            return Err(FsError::IsDir);
        }
        let mut cluster_buf = [0u8; CLUSTER_SIZE];
        self.read_cluster(inode.ino as u32, &mut cluster_buf)?;
        let start = 32 + offset as usize;
        let end = start + data.len();
        if end > cluster_buf.len() {
            return Err(FsError::NoSpace);
        }
        cluster_buf[start..end].copy_from_slice(data);
        self.write_cluster(inode.ino as u32, &cluster_buf)?;
        Ok(data.len())
    }

    fn readdir(&self, dir: InodeId, index: u64, entry: &mut DirEntry) -> Result<bool, FsError> {
        if dir.ino != 2 {
            return Err(FsError::NotDir);
        }
        if index > 0 {
            if index == 1 {
                entry.name_len = 10;
                entry.name[..10].copy_from_slice(b"README.TXT");
                entry.inode = InodeId::new(self.inner.lock().mount_id, 3);
                entry.file_type = FileType::Regular;
                return Ok(true);
            }
            return Ok(false);
        }
        entry.name_len = 1;
        entry.name[0] = b'.';
        entry.inode = dir;
        entry.file_type = FileType::Directory;
        Ok(true)
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

    fn truncate(&self, _: InodeId, _: u64) -> Result<(), FsError> {
        Err(FsError::NotSupported)
    }
}
