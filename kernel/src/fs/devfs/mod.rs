//! devfs — `/dev` character devices and standard I/O symlinks.

use alloc::string::String;
use alloc::collections::BTreeMap;
use spin::Mutex;

use crate::console::Console;
use crate::fs::vfs::superblock::{DirEntry, FileSystem, FsError};
use crate::fs::vfs::inode::{FileType, InodeAttr, InodeId, InodeMode};

const ROOT_INO: u64 = 1;
const DEV_CONSOLE: u32 = makedev(5, 1);
const DEV_NULL: u32 = makedev(1, 3);
const DEV_ZERO: u32 = makedev(1, 5);

const fn makedev(major: u32, minor: u32) -> u32 {
    (major << 8) | minor
}

enum DevNodeKind {
    Char { rdev: u32 },
    Symlink { target: String },
}

struct DevNode {
    name: String,
    kind: DevNodeKind,
}

struct DevFsInner {
    mount_id: u32,
    nodes: BTreeMap<u64, DevNode>,
    name_index: BTreeMap<String, u64>,
}

pub struct DevFs {
    inner: Mutex<DevFsInner>,
}

impl DevFs {
    pub fn new(mount_id: u32) -> Self {
        let mut nodes = BTreeMap::new();
        let mut name_index = BTreeMap::new();
        let entries: [(&str, DevNodeKind); 7] = [
            ("console", DevNodeKind::Char { rdev: DEV_CONSOLE }),
            ("null", DevNodeKind::Char { rdev: DEV_NULL }),
            ("zero", DevNodeKind::Char { rdev: DEV_ZERO }),
            ("stdin", DevNodeKind::Symlink { target: String::from("console") }),
            ("stdout", DevNodeKind::Symlink { target: String::from("console") }),
            ("stderr", DevNodeKind::Symlink { target: String::from("console") }),
            ("tty", DevNodeKind::Symlink { target: String::from("console") }),
        ];
        for (i, (name, kind)) in entries.into_iter().enumerate() {
            let ino = (i + 2) as u64;
            nodes.insert(
                ino,
                DevNode {
                    name: String::from(name),
                    kind,
                },
            );
            name_index.insert(String::from(name), ino);
        }
        Self {
            inner: Mutex::new(DevFsInner {
                mount_id,
                nodes,
                name_index,
            }),
        }
    }

    fn mount_id(&self) -> u32 {
        self.inner.lock().mount_id
    }

    fn file_type(kind: &DevNodeKind) -> FileType {
        match kind {
            DevNodeKind::Char { .. } => FileType::CharDev,
            DevNodeKind::Symlink { .. } => FileType::Symlink,
        }
    }
}

impl FileSystem for DevFs {
    fn fs_type(&self) -> &'static str {
        "devfs"
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
        let rdev = match &node.kind {
            DevNodeKind::Char { rdev } => *rdev,
            DevNodeKind::Symlink { .. } => 0,
        };
        Ok(InodeAttr {
            ino: inode.ino,
            mode: InodeMode::default_file(),
            file_type: Self::file_type(&node.kind),
            size: 0,
            nlink: 1,
            generation: 1,
            rdev,
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

    fn create(
        &self,
        _: InodeId,
        _: &[u8],
        _: FileType,
        _: InodeMode,
    ) -> Result<InodeId, FsError> {
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
        let DevNodeKind::Char { rdev } = &node.kind else {
            return Err(FsError::NotFile);
        };
        let _ = offset;
        match *rdev {
            // Console input: no PS/2 backend yet — returns 0 (EOF) not an error.
            // Spec deviation: real Linux would block until input; embedded bring-up uses serial-only output.
            DEV_CONSOLE => Ok(0),
            DEV_NULL => Ok(0),
            DEV_ZERO => {
                for b in buf.iter_mut() {
                    *b = 0;
                }
                Ok(buf.len())
            }
            _ => Err(FsError::NotSupported),
        }
    }

    fn write(&self, inode: InodeId, offset: u64, buf: &[u8]) -> Result<usize, FsError> {
        let inner = self.inner.lock();
        let node = inner.nodes.get(&inode.ino).ok_or(FsError::NotFound)?;
        let DevNodeKind::Char { rdev } = &node.kind else {
            return Err(FsError::NotFile);
        };
        let _ = offset;
        match *rdev {
            DEV_CONSOLE => {
                if let Ok(s) = core::str::from_utf8(buf) {
                    Console::print(s);
                }
                Ok(buf.len())
            }
            DEV_NULL => Ok(buf.len()),
            DEV_ZERO => Ok(buf.len()),
            _ => Err(FsError::NotSupported),
        }
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
        entry.file_type = Self::file_type(&node.kind);
        Ok(true)
    }

    fn readlink(&self, inode: InodeId, buf: &mut [u8]) -> Result<usize, FsError> {
        let inner = self.inner.lock();
        let node = inner.nodes.get(&inode.ino).ok_or(FsError::NotFound)?;
        let DevNodeKind::Symlink { target } = &node.kind else {
            return Err(FsError::NotSymlink);
        };
        let n = target.len().min(buf.len());
        buf[..n].copy_from_slice(&target.as_bytes()[..n]);
        Ok(n)
    }

    fn symlink(
        &self,
        _: InodeId,
        _: &[u8],
        _: &[u8],
        _: InodeMode,
    ) -> Result<InodeId, FsError> {
        Err(FsError::AccessDenied)
    }

    fn truncate(&self, inode: InodeId, _: u64) -> Result<(), FsError> {
        let _ = inode;
        Err(FsError::AccessDenied)
    }
}
