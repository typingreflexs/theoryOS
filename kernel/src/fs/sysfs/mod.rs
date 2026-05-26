//! sysfs — `/sys` virtual filesystem.

use alloc::string::String;
use alloc::collections::BTreeMap;
use spin::Mutex;

use crate::fs::vfs::superblock::{DirEntry, FileSystem, FsError};
use crate::fs::vfs::inode::{FileType, InodeAttr, InodeId, InodeMode};

const ROOT_INO: u64 = 1;

struct SysNode {
    name: String,
    data: String,
}

struct SysFsInner {
    mount_id: u32,
    nodes: BTreeMap<u64, SysNode>,
    name_index: BTreeMap<String, u64>,
}

pub struct SysFs {
    inner: Mutex<SysFsInner>,
}

impl SysFs {
    pub fn new(mount_id: u32) -> Self {
        let mut nodes = BTreeMap::new();
        let mut name_index = BTreeMap::new();
        let entries = [
            ("kernel", "Theory OS 0.1\n"),
            ("class", ""),
            ("devices", ""),
            ("module", ""),
        ];
        for (i, (name, data)) in entries.iter().enumerate() {
            let ino = (i + 2) as u64;
            nodes.insert(
                ino,
                SysNode {
                    name: String::from(*name),
                    data: String::from(*data),
                },
            );
            name_index.insert(String::from(*name), ino);
        }
        Self {
            inner: Mutex::new(SysFsInner {
                mount_id,
                nodes,
                name_index,
            }),
        }
    }

    fn mount_id(&self) -> u32 {
        self.inner.lock().mount_id
    }
}

impl FileSystem for SysFs {
    fn fs_type(&self) -> &'static str {
        "sysfs"
    }

    fn root(&self) -> InodeId {
        InodeId::new(self.mount_id(), ROOT_INO)
    }

    fn getattr(&self, inode: InodeId) -> Result<InodeAttr, FsError> {
        if inode.mount != self.mount_id() {
            return Err(FsError::NotFound);
        }
        if inode.ino == ROOT_INO {
            return Ok(InodeAttr {
                ino: ROOT_INO,
                mode: InodeMode::default_dir(),
                file_type: FileType::Directory,
                size: 0,
                nlink: 2,
                generation: 1,
                rdev: 0,
            });
        }
        let inner = self.inner.lock();
        let node = inner.nodes.get(&inode.ino).ok_or(FsError::NotFound)?;
        Ok(InodeAttr {
            ino: inode.ino,
            mode: if node.data.is_empty() {
                InodeMode::default_dir()
            } else {
                InodeMode::default_file()
            },
            file_type: if node.data.is_empty() {
                FileType::Directory
            } else {
                FileType::Regular
            },
            size: node.data.len() as u64,
            nlink: 1,
            generation: 1,
            rdev: 0,
        })
    }

    fn lookup(&self, dir: InodeId, name: &[u8]) -> Result<InodeId, FsError> {
        if dir.ino != ROOT_INO {
            return Err(FsError::NotDir);
        }
        let name = core::str::from_utf8(name).map_err(|_| FsError::Invalid)?;
        let inner = self.inner.lock();
        let ino = inner.name_index.get(name).ok_or(FsError::NotFound)?;
        Ok(InodeId::new(self.mount_id(), *ino))
    }

    fn create(&self, _: InodeId, _: &[u8], _: FileType, _: InodeMode) -> Result<InodeId, FsError> {
        Err(FsError::AccessDenied)
    }

    fn mkdir(&self, _: InodeId, _: &[u8], _: InodeMode) -> Result<InodeId, FsError> {
        Err(FsError::AccessDenied)
    }

    fn unlink(&self, _: InodeId, _: &[u8]) -> Result<(), FsError> {
        Err(FsError::AccessDenied)
    }

    fn read(&self, inode: InodeId, offset: u64, buf: &mut [u8]) -> Result<usize, FsError> {
        let inner = self.inner.lock();
        let node = inner.nodes.get(&inode.ino).ok_or(FsError::NotFound)?;
        if node.data.is_empty() {
            return Err(FsError::IsDir);
        }
        if offset as usize >= node.data.len() {
            return Ok(0);
        }
        let start = offset as usize;
        let end = (start + buf.len()).min(node.data.len());
        buf[..end - start].copy_from_slice(node.data.as_bytes()[start..end].as_ref());
        Ok(end - start)
    }

    fn write(&self, _: InodeId, _: u64, _: &[u8]) -> Result<usize, FsError> {
        Err(FsError::AccessDenied)
    }

    fn readdir(&self, dir: InodeId, index: u64, entry: &mut DirEntry) -> Result<bool, FsError> {
        if dir.ino != ROOT_INO {
            return Err(FsError::NotDir);
        }
        let inner = self.inner.lock();
        if index as usize >= inner.nodes.len() {
            return Ok(false);
        }
        let (ino, node) = inner.nodes.iter().nth(index as usize).unwrap();
        entry.name_len = node.name.len().min(256);
        entry.name[..entry.name_len].copy_from_slice(&node.name.as_bytes()[..entry.name_len]);
        entry.inode = InodeId::new(self.mount_id(), *ino);
        entry.file_type = if node.data.is_empty() {
            FileType::Directory
        } else {
            FileType::Regular
        };
        Ok(true)
    }

    fn readlink(&self, _: InodeId, _: &mut [u8]) -> Result<usize, FsError> {
        Err(FsError::NotSymlink)
    }

    fn symlink(&self, _: InodeId, _: &[u8], _: &[u8], _: InodeMode) -> Result<InodeId, FsError> {
        Err(FsError::AccessDenied)
    }

    fn truncate(&self, _: InodeId, _: u64) -> Result<(), FsError> {
        Err(FsError::AccessDenied)
    }
}
