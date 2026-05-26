use super::inode::InodeId;
use crate::fs::fd::OpenFlags;

#[derive(Clone, Copy, Debug)]
pub struct OpenFile {
    pub inode: InodeId,
    pub offset: u64,
    pub flags: OpenFlags,
    pub generation: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileMode {
    Read,
    Write,
    ReadWrite,
}

impl From<OpenFlags> for FileMode {
    fn from(flags: OpenFlags) -> Self {
        if flags.contains(OpenFlags::O_RDWR) {
            Self::ReadWrite
        } else if flags.contains(OpenFlags::O_WRONLY) {
            Self::Write
        } else {
            Self::Read
        }
    }
}
