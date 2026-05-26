#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct InodeId {
    pub mount: u32,
    pub ino: u64,
}

impl InodeId {
    pub const INVALID: InodeId = InodeId { mount: u32::MAX, ino: 0 };

    pub fn new(mount: u32, ino: u64) -> Self {
        Self { mount, ino }
    }

    pub fn is_valid(self) -> bool {
        self.mount != u32::MAX
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileType {
    Regular,
    Directory,
    Symlink,
    CharDev,
    BlockDev,
    Fifo,
    Socket,
}

bitflags::bitflags! {
    #[derive(Clone, Copy, Debug)]
    pub struct InodeMode: u16 {
        const OWNER_READ = 0o400;
        const OWNER_WRITE = 0o200;
        const OWNER_EXEC = 0o100;
        const GROUP_READ = 0o040;
        const GROUP_WRITE = 0o020;
        const GROUP_EXEC = 0o010;
        const OTHER_READ = 0o004;
        const OTHER_WRITE = 0o002;
        const OTHER_EXEC = 0o001;
    }
}

impl InodeMode {
    pub fn default_file() -> Self {
        Self::OWNER_READ | Self::OWNER_WRITE | Self::GROUP_READ | Self::OTHER_READ
    }

    pub fn default_dir() -> Self {
        Self::OWNER_READ
            | Self::OWNER_WRITE
            | Self::OWNER_EXEC
            | Self::GROUP_READ
            | Self::GROUP_EXEC
            | Self::OTHER_READ
            | Self::OTHER_EXEC
    }
}

#[derive(Clone, Copy, Debug)]
pub struct InodeAttr {
    pub ino: u64,
    pub mode: InodeMode,
    pub file_type: FileType,
    pub size: u64,
    pub nlink: u32,
    pub generation: u64,
    pub rdev: u32,
}
