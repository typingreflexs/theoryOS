use super::inode::{FileType, InodeAttr, InodeId, InodeMode};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FsError {
    NotFound,
    Exists,
    NotDir,
    IsDir,
    NotFile,
    NotSymlink,
    Invalid,
    Io,
    NoSpace,
    NotSupported,
    TooManyLinks,
    AccessDenied,
}

pub struct DirEntry {
    pub name: [u8; 256],
    pub name_len: usize,
    pub inode: InodeId,
    pub file_type: FileType,
}

impl DirEntry {
    pub const fn empty() -> Self {
        Self {
            name: [0; 256],
            name_len: 0,
            inode: InodeId::INVALID,
            file_type: FileType::Regular,
        }
    }
}

/// Virtual filesystem operations — one implementation per filesystem type.
pub trait FileSystem: Send + Sync {
    fn fs_type(&self) -> &'static str;

    fn root(&self) -> InodeId;

    fn getattr(&self, inode: InodeId) -> Result<InodeAttr, FsError>;

    fn lookup(&self, dir: InodeId, name: &[u8]) -> Result<InodeId, FsError>;

    fn create(
        &self,
        dir: InodeId,
        name: &[u8],
        file_type: FileType,
        mode: InodeMode,
    ) -> Result<InodeId, FsError>;

    fn mkdir(&self, dir: InodeId, name: &[u8], mode: InodeMode) -> Result<InodeId, FsError>;

    fn unlink(&self, dir: InodeId, name: &[u8]) -> Result<(), FsError>;

    fn read(&self, inode: InodeId, offset: u64, buf: &mut [u8]) -> Result<usize, FsError>;

    fn write(&self, inode: InodeId, offset: u64, buf: &[u8]) -> Result<usize, FsError>;

    fn readdir(&self, dir: InodeId, index: u64, entry: &mut DirEntry) -> Result<bool, FsError>;

    fn readlink(&self, inode: InodeId, buf: &mut [u8]) -> Result<usize, FsError>;

    fn symlink(&self, dir: InodeId, name: &[u8], target: &[u8], mode: InodeMode)
        -> Result<InodeId, FsError>;

    fn truncate(&self, inode: InodeId, size: u64) -> Result<(), FsError>;
}
