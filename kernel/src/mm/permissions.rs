use bitflags::bitflags;

bitflags! {
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct ProtFlags: u32 {
        const NONE = 0;
        const READ = 1 << 0;
        const WRITE = 1 << 1;
        const EXEC = 1 << 2;
        const USER = 1 << 3;
        const GROW_DOWN = 1 << 4;
        const SHARED = 1 << 5;
    }
}

bitflags! {
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct MmapFlags: u32 {
        const NONE = 0;
        const SHARED = 1 << 0;
        const PRIVATE = 1 << 1;
        const ANONYMOUS = 1 << 2;
        const FIXED = 1 << 3;
        const FIXED_NOREPLACE = 1 << 4;
        const POPULATE = 1 << 5;
        const NORESERVE = 1 << 6;
        const STACK = 1 << 7;
        const HUGETLB = 1 << 8;
    }
}

bitflags! {
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct MprotectFlags: u32 {
        const NONE = 0;
        const READ = 1 << 0;
        const WRITE = 1 << 1;
        const EXEC = 1 << 2;
        const GROW_DOWN = 1 << 3;
    }
}

impl From<ProtFlags> for MprotectFlags {
    fn from(prot: ProtFlags) -> Self {
        let mut flags = Self::empty();
        if prot.contains(ProtFlags::READ) {
            flags |= Self::READ;
        }
        if prot.contains(ProtFlags::WRITE) {
            flags |= Self::WRITE;
        }
        if prot.contains(ProtFlags::EXEC) {
            flags |= Self::EXEC;
        }
        if prot.contains(ProtFlags::GROW_DOWN) {
            flags |= Self::GROW_DOWN;
        }
        flags
    }
}

impl ProtFlags {
    pub fn user_readable(self) -> bool {
        self.contains(Self::READ) || self.contains(Self::EXEC)
    }

    pub fn user_writable(self) -> bool {
        self.contains(Self::WRITE)
    }
}
