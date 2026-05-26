//! In-memory tmpfs — used for `/`, `/tmp`, and initramfs-style trees.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use spin::Mutex;

use crate::fs::vfs::superblock::{DirEntry, FileSystem, FsError};
use crate::fs::vfs::inode::{FileType, InodeAttr, InodeId, InodeMode};

const ROOT_INO: u64 = 1;

struct TmpInode {
    attr: InodeAttr,
    data: Vec<u8>,
    children: BTreeMap<String, u64>,
    link_target: Vec<u8>,
}

struct TmpFsInner {
    next_ino: u64,
    inodes: BTreeMap<u64, TmpInode>,
}

pub struct TmpFs {
    inner: Mutex<TmpFsInner>,
    mount_id: u32,
}

impl TmpFs {
    pub fn new(mount_id: u32) -> Self {
        let mut inodes = BTreeMap::new();
        inodes.insert(
            ROOT_INO,
            TmpInode {
                attr: InodeAttr {
                    ino: ROOT_INO,
                    mode: InodeMode::default_dir(),
                    file_type: FileType::Directory,
                    size: 0,
                    nlink: 2,
                    generation: 1,
                    rdev: 0,
                },
                data: Vec::new(),
                children: BTreeMap::new(),
                link_target: Vec::new(),
            },
        );
        Self {
            inner: Mutex::new(TmpFsInner {
                next_ino: ROOT_INO + 1,
                inodes,
            }),
            mount_id,
        }
    }

    pub fn with_root_dirs(&self, dirs: &[&str]) {
        let root = InodeId::new(self.mount_id, ROOT_INO);
        for name in dirs {
            let _ = self.mkdir(root, name.as_bytes(), InodeMode::default_dir());
        }
    }

    fn id(&self, inode: InodeId) -> Result<u64, FsError> {
        if inode.mount != self.mount_id {
            return Err(FsError::NotFound);
        }
        Ok(inode.ino)
    }

    fn alloc_inode(
        &self,
        file_type: FileType,
        mode: InodeMode,
    ) -> Result<u64, FsError> {
        let mut inner = self.inner.lock();
        let ino = inner.next_ino;
        inner.next_ino += 1;
        inner.inodes.insert(
            ino,
            TmpInode {
                attr: InodeAttr {
                    ino,
                    mode,
                    file_type,
                    size: 0,
                    nlink: 1,
                    generation: 1,
                    rdev: 0,
                },
                data: Vec::new(),
                children: BTreeMap::new(),
                link_target: Vec::new(),
            },
        );
        Ok(ino)
    }

    fn bump_generation(&self, ino: u64) {
        if let Some(node) = self.inner.lock().inodes.get_mut(&ino) {
            node.attr.generation += 1;
        }
    }
}

impl FileSystem for TmpFs {
    fn fs_type(&self) -> &'static str {
        "tmpfs"
    }

    fn root(&self) -> InodeId {
        InodeId::new(self.mount_id, ROOT_INO)
    }

    fn getattr(&self, inode: InodeId) -> Result<InodeAttr, FsError> {
        let ino = self.id(inode)?;
        self.inner
            .lock()
            .inodes
            .get(&ino)
            .map(|n| n.attr)
            .ok_or(FsError::NotFound)
    }

    fn lookup(&self, dir: InodeId, name: &[u8]) -> Result<InodeId, FsError> {
        let dir_ino = self.id(dir)?;
        let name = core::str::from_utf8(name).map_err(|_| FsError::Invalid)?;
        let inner = self.inner.lock();
        let node = inner.inodes.get(&dir_ino).ok_or(FsError::NotFound)?;
        if node.attr.file_type != FileType::Directory {
            return Err(FsError::NotDir);
        }
        let child = node.children.get(name).ok_or(FsError::NotFound)?;
        Ok(InodeId::new(self.mount_id, *child))
    }

    fn create(
        &self,
        dir: InodeId,
        name: &[u8],
        file_type: FileType,
        mode: InodeMode,
    ) -> Result<InodeId, FsError> {
        let dir_ino = self.id(dir)?;
        let name_str = core::str::from_utf8(name).map_err(|_| FsError::Invalid)?;
        if name_str.is_empty() {
            return Err(FsError::Invalid);
        }
        let ino = self.alloc_inode(file_type, mode)?;
        let mut inner = self.inner.lock();
        let dir_node = inner.inodes.get_mut(&dir_ino).ok_or(FsError::NotFound)?;
        if dir_node.attr.file_type != FileType::Directory {
            return Err(FsError::NotDir);
        }
        if dir_node.children.contains_key(name_str) {
            return Err(FsError::Exists);
        }
        dir_node.children.insert(String::from(name_str), ino);
        dir_node.attr.generation += 1;
        Ok(InodeId::new(self.mount_id, ino))
    }

    fn mkdir(&self, dir: InodeId, name: &[u8], mode: InodeMode) -> Result<InodeId, FsError> {
        self.create(dir, name, FileType::Directory, mode)
    }

    fn unlink(&self, dir: InodeId, name: &[u8]) -> Result<(), FsError> {
        let dir_ino = self.id(dir)?;
        let name_str = core::str::from_utf8(name).map_err(|_| FsError::Invalid)?;
        let mut inner = self.inner.lock();
        let child_ino = {
            let dir_node = inner.inodes.get(&dir_ino).ok_or(FsError::NotFound)?;
            if dir_node.attr.file_type != FileType::Directory {
                return Err(FsError::NotDir);
            }
            *dir_node.children.get(name_str).ok_or(FsError::NotFound)?
        };
        {
            let child = inner.inodes.get(&child_ino).ok_or(FsError::NotFound)?;
            if child.attr.file_type == FileType::Directory && !child.children.is_empty() {
                return Err(FsError::NotSupported);
            }
        }
        inner.inodes.get_mut(&dir_ino).unwrap().children.remove(name_str);
        inner.inodes.remove(&child_ino);
        if let Some(dir_node) = inner.inodes.get_mut(&dir_ino) {
            dir_node.attr.generation += 1;
        }
        Ok(())
    }

    fn read(&self, inode: InodeId, offset: u64, buf: &mut [u8]) -> Result<usize, FsError> {
        let ino = self.id(inode)?;
        let inner = self.inner.lock();
        let node = inner.inodes.get(&ino).ok_or(FsError::NotFound)?;
        match node.attr.file_type {
            FileType::Regular | FileType::Symlink => {}
            _ => return Err(FsError::NotFile),
        }
        let data = if node.attr.file_type == FileType::Symlink {
            &node.link_target
        } else {
            &node.data
        };
        if offset as usize >= data.len() {
            return Ok(0);
        }
        let start = offset as usize;
        let end = (start + buf.len()).min(data.len());
        buf[..end - start].copy_from_slice(&data[start..end]);
        Ok(end - start)
    }

    fn write(&self, inode: InodeId, offset: u64, buf: &[u8]) -> Result<usize, FsError> {
        let ino = self.id(inode)?;
        let mut inner = self.inner.lock();
        let node = inner.inodes.get_mut(&ino).ok_or(FsError::NotFound)?;
        if node.attr.file_type != FileType::Regular {
            return Err(FsError::NotFile);
        }
        let start = offset as usize;
        let end = start + buf.len();
        if end > node.data.len() {
            node.data.resize(end, 0);
        }
        node.data[start..end].copy_from_slice(buf);
        node.attr.size = node.data.len() as u64;
        node.attr.generation += 1;
        Ok(buf.len())
    }

    fn readdir(&self, dir: InodeId, index: u64, entry: &mut DirEntry) -> Result<bool, FsError> {
        let dir_ino = self.id(dir)?;
        let inner = self.inner.lock();
        let node = inner.inodes.get(&dir_ino).ok_or(FsError::NotFound)?;
        if node.attr.file_type != FileType::Directory {
            return Err(FsError::NotDir);
        }
        if index == 0 {
            entry.name_len = 1;
            entry.name[0] = b'.';
            entry.inode = dir;
            entry.file_type = FileType::Directory;
            return Ok(true);
        }
        if index == 1 {
            entry.name_len = 2;
            entry.name[0] = b'.';
            entry.name[1] = b'.';
            entry.inode = dir;
            entry.file_type = FileType::Directory;
            return Ok(true);
        }
        let idx = (index - 2) as usize;
        if idx >= node.children.len() {
            return Ok(false);
        }
        let (name, &child_ino) = node.children.iter().nth(idx).unwrap();
        entry.name_len = name.len().min(256);
        entry.name[..entry.name_len].copy_from_slice(&name.as_bytes()[..entry.name_len]);
        entry.inode = InodeId::new(self.mount_id, child_ino);
        entry.file_type = inner
            .inodes
            .get(&child_ino)
            .map(|n| n.attr.file_type)
            .unwrap_or(FileType::Regular);
        Ok(true)
    }

    fn readlink(&self, inode: InodeId, buf: &mut [u8]) -> Result<usize, FsError> {
        let ino = self.id(inode)?;
        let inner = self.inner.lock();
        let node = inner.inodes.get(&ino).ok_or(FsError::NotFound)?;
        if node.attr.file_type != FileType::Symlink {
            return Err(FsError::NotSymlink);
        }
        let len = buf.len().min(node.link_target.len());
        buf[..len].copy_from_slice(&node.link_target[..len]);
        Ok(len)
    }

    fn symlink(
        &self,
        dir: InodeId,
        name: &[u8],
        target: &[u8],
        mode: InodeMode,
    ) -> Result<InodeId, FsError> {
        let id = self.create(dir, name, FileType::Symlink, mode)?;
        let ino = self.id(id)?;
        let mut inner = self.inner.lock();
        let node = inner.inodes.get_mut(&ino).unwrap();
        node.link_target = target.to_vec();
        node.attr.size = node.link_target.len() as u64;
        Ok(id)
    }

    fn truncate(&self, inode: InodeId, size: u64) -> Result<(), FsError> {
        let ino = self.id(inode)?;
        let mut inner = self.inner.lock();
        let node = inner.inodes.get_mut(&ino).ok_or(FsError::NotFound)?;
        if node.attr.file_type != FileType::Regular {
            return Err(FsError::NotFile);
        }
        node.data.resize(size as usize, 0);
        node.attr.size = size;
        node.attr.generation += 1;
        Ok(())
    }
}
