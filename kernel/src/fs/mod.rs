//! Virtual filesystem layer — VFS, block devices, and filesystem drivers.

pub mod block;
pub mod devfs;
pub mod embed;
pub mod ext2;
pub mod fat32;
pub mod fd;
pub mod pipe;
pub mod procfs;
pub mod rootfs;
pub mod sysfs;
pub mod tmpfs;
pub mod vfs;

use alloc::boxed::Box;

use block::BlockDevice;
use block::ramdisk;
use devfs::DevFs;
use ext2::Ext2Fs;
use fat32::Fat32Fs;
use procfs::ProcFs;
use sysfs::SysFs;
use tmpfs::TmpFs;
use vfs::mount::MountTable;

pub use fd::{FdEntry, FdTable, OpenFlags};
pub use pipe::{Pipe, PipeId, PIPE_BUF};
pub use vfs::{OpenFile, fs_err_to_errno, open_path, read_file, write_file, lseek_file};

struct RamDiskDev {
    slot: u32,
}

impl BlockDevice for RamDiskDev {
    fn block_count(&self) -> u64 {
        let disk = if self.slot == 0 {
            ramdisk::disk0()
        } else {
            ramdisk::disk1()
        };
        disk.lock().as_ref().map(|d| d.block_count()).unwrap_or(0)
    }

    fn read_block(&self, block: u64, buf: &mut [u8]) -> Result<(), block::BlockError> {
        let disk = if self.slot == 0 {
            ramdisk::disk0()
        } else {
            ramdisk::disk1()
        };
        disk.lock()
            .as_ref()
            .ok_or(block::BlockError::Io)?
            .read_block(block, buf)
    }

    fn write_block(&self, block: u64, buf: &[u8]) -> Result<(), block::BlockError> {
        let disk = if self.slot == 0 {
            ramdisk::disk0()
        } else {
            ramdisk::disk1()
        };
        disk.lock()
            .as_ref()
            .ok_or(block::BlockError::Io)?
            .write_block(block, buf)
    }
}

static RAMDISK0_DEV: RamDiskDev = RamDiskDev { slot: 0 };
static RAMDISK1_DEV: RamDiskDev = RamDiskDev { slot: 1 };

pub fn init() {
    ramdisk::init();

    let root: &'static TmpFs = Box::leak(Box::new(TmpFs::new(0)));
    MountTable::register("", root).expect("root tmpfs");
    root.with_root_dirs(&["proc", "dev", "tmp", "mnt", "bin", "lib", "etc", "root", "home", "var", "sys"]);

    let procfs: &'static ProcFs = Box::leak(Box::new(ProcFs::new(1)));
    MountTable::register("/proc", procfs).expect("procfs");

    let devfs: &'static DevFs = Box::leak(Box::new(DevFs::new(2)));
    MountTable::register("/dev", devfs).expect("devfs");

    let tmp: &'static TmpFs = Box::leak(Box::new(TmpFs::new(3)));
    MountTable::register("/tmp", tmp).expect("tmpfs");

    let sysfs: &'static SysFs = Box::leak(Box::new(SysFs::new(6)));
    MountTable::register("/sys", sysfs).expect("sysfs");

    if let Ok(ext2) = Ext2Fs::format_and_mount(4, 0, &RAMDISK0_DEV) {
        let ext2: &'static Ext2Fs = Box::leak(Box::new(ext2));
        let _ = MountTable::register("/mnt/ext2", ext2);
    }

    if let Ok(fat) = Fat32Fs::format_and_mount(5, 1, &RAMDISK1_DEV) {
        let fat: &'static Fat32Fs = Box::leak(Box::new(fat));
        let _ = MountTable::register("/mnt/fat32", fat);
    }

    crate::console::Console::println("[fs] VFS: tmpfs, procfs, devfs, sysfs, ext2, fat32 mounted");
    rootfs::populate();
}
