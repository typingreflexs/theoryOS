/// Virtual memory layout constants for Theory OS on x86-64.

pub const PAGE_SIZE: u64 = 4096;
pub const PAGE_SHIFT: u32 = 12;
pub const PAGE_MASK: u64 = PAGE_SIZE - 1;

pub const MAX_PHYS_ADDR: u64 = 1 << 48;

/// Userspace canonical region.
pub const USER_ADDR_MIN: u64 = 0x0000_0000_0001_0000;
pub const USER_ADDR_MAX: u64 = 0x0000_7FFF_FFFF_FFFF;

/// Default mmap base (before ASLR slide).
pub const USER_MMAP_BASE: u64 = 0x0000_2000_0000_0000;
pub const USER_MMAP_LIMIT: u64 = 0x0000_6000_0000_0000;

/// Heap grows upward from here.
pub const USER_HEAP_BASE: u64 = 0x0000_1000_0000_0000;

/// Stack grows downward from here.
pub const USER_STACK_TOP: u64 = 0x0000_7000_0000_0000;

/// Framebuffer metadata page (UserFbMeta) for graphical programs.
pub const USER_FB_META_BASE: u64 = 0x0000_4FFF_F000_0000;

/// Framebuffer pixel buffer mapped read/write for all user processes.
pub const USER_FB_BASE: u64 = 0x0000_5000_0000_0000;

/// Kernel higher-half (Limine HHDM covers all physical memory).
pub const KERNEL_VIRT_BASE: u64 = 0xFFFF_FFFF_8000_0000;

/// Kernel heap virtual window.
pub const KERNEL_HEAP_BASE: u64 = 0xFFFF_C000_0000_0000;
pub const KERNEL_HEAP_SIZE: u64 = 256 * 1024 * 1024;

/// Recursive page-table window (PML4 index 510).
pub const RECURSIVE_INDEX: usize = 510;
pub const RECURSIVE_VBASE: u64 = 0xFFFF_8040_2000_0000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageSize {
    Size4KiB,
    Size2MiB,
    Size1GiB,
}

impl PageSize {
    pub const fn bytes(self) -> u64 {
        match self {
            Self::Size4KiB => 4096,
            Self::Size2MiB => 2 * 1024 * 1024,
            Self::Size1GiB => 1024 * 1024 * 1024,
        }
    }

    pub const fn shift(self) -> u32 {
        match self {
            Self::Size4KiB => 12,
            Self::Size2MiB => 21,
            Self::Size1GiB => 30,
        }
    }
}

pub fn is_user_address(addr: u64) -> bool {
    addr >= USER_ADDR_MIN && addr <= USER_ADDR_MAX
}

pub fn is_kernel_address(addr: u64) -> bool {
    addr >= KERNEL_VIRT_BASE
}

pub fn align_down(addr: u64, align: u64) -> u64 {
    addr & !(align - 1)
}

pub fn align_up(addr: u64, align: u64) -> u64 {
    (addr + align - 1) & !(align - 1)
}
