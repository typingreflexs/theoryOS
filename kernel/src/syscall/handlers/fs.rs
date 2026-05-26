//! Filesystem syscalls: stat, getdents64, chdir, getcwd, access, unlink, readlink.

use crate::fs::fd::{FdEntry, FD_CLOEXEC};
use crate::fs::vfs::{fs_err_to_errno, stat_path};
use crate::proc;
use crate::syscall::errno::{err, ok, Errno, SysResult};
use crate::syscall::handlers::sys_open;
use crate::syscall::uaccess::{copy_to_user, copy_to_user_obj, str_from_user, user_slice_ok};

#[repr(C)]
#[derive(Clone, Copy)]
struct LinuxStat {
    st_dev: u64,
    st_ino: u64,
    st_nlink: u64,
    st_mode: u32,
    st_uid: u32,
    st_gid: u32,
    pad0: u32,
    st_rdev: u64,
    st_size: i64,
    st_blksize: i32,
    st_blocks: i64,
    st_atime: i64,
    st_atime_nsec: i64,
    st_mtime: i64,
    st_mtime_nsec: i64,
    st_ctime: i64,
    st_ctime_nsec: i64,
    __unused: [i64; 3],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct LinuxDirent64 {
    d_ino: u64,
    d_off: i64,
    d_reclen: u16,
    d_type: u8,
    name: [u8; 256],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Termios {
    iflag: u32,
    oflag: u32,
    cflag: u32,
    lflag: u32,
    line: u8,
    cc: [u8; 19],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Winsize {
    ws_row: u16,
    ws_col: u16,
    ws_xpixel: u16,
    ws_ypixel: u16,
}

const F_DUPFD: u64 = 0;
const F_GETFD: u64 = 1;
const F_SETFD: u64 = 2;
const F_GETFL: u64 = 3;
const F_SETFL: u64 = 4;
const O_NONBLOCK: u32 = 0x800;

const TCGETS: u64 = 0x5401;
const TIOCGWINSZ: u64 = 0x5413;

fn mode_from_attr(attr: &crate::fs::vfs::inode::InodeAttr) -> u32 {
    use crate::fs::vfs::inode::FileType;
    let kind = match attr.file_type {
        FileType::Directory => 0o040000,
        FileType::Symlink => 0o120000,
        FileType::CharDev => 0o020000,
        FileType::BlockDev => 0o060000,
        FileType::Fifo => 0o010000,
        FileType::Socket => 0o140000,
        FileType::Regular => 0o100000,
    };
    kind | 0o755
}

fn fill_stat(attr: &crate::fs::vfs::inode::InodeAttr) -> LinuxStat {
    LinuxStat {
        st_dev: 0,
        st_ino: attr.ino,
        st_nlink: attr.nlink as u64,
        st_mode: mode_from_attr(attr),
        st_uid: 0,
        st_gid: 0,
        pad0: 0,
        st_rdev: attr.rdev as u64,
        st_size: attr.size as i64,
        st_blksize: 4096,
        st_blocks: ((attr.size + 511) / 512) as i64,
        st_atime: 0,
        st_atime_nsec: 0,
        st_mtime: 0,
        st_mtime_nsec: 0,
        st_ctime: 0,
        st_ctime_nsec: 0,
        __unused: [0; 3],
    }
}

pub fn sys_stat(path: u64, buf: u64, _: u64, _: u64, _: u64, _: u64) -> SysResult {
    let mut pathbuf = [0u8; 256];
    str_from_user(&mut pathbuf, path, 255)?;
    let end = pathbuf.iter().position(|&b| b == 0).unwrap_or(pathbuf.len());
    user_slice_ok(buf, core::mem::size_of::<LinuxStat>() as u64)?;
    let attr = stat_path(&pathbuf[..end]).map_err(fs_err_to_errno)?;
    copy_to_user_obj(buf, &fill_stat(&attr))?;
    ok(0)
}

pub fn sys_fstat(fd: u64, buf: u64, _: u64, _: u64, _: u64, _: u64) -> SysResult {
    user_slice_ok(buf, core::mem::size_of::<LinuxStat>() as u64)?;
    proc::current_process_mut(|p| {
        match p.fds.get(fd as u32) {
            Some(FdEntry::Vfs(f)) => {
                let fs = crate::fs::vfs::mount::MountTable::fs(f.inode.mount).ok_or(Errno::ENOENT)?;
                let attr = fs.getattr(f.inode).map_err(fs_err_to_errno)?;
                copy_to_user_obj(buf, &fill_stat(&attr))?;
                ok(0)
            }
            Some(FdEntry::PipeRead(_)) | Some(FdEntry::PipeWrite(_)) => {
                copy_to_user_obj(buf, &fill_stat(&pipe_attr()))?;
                ok(0)
            }
            Some(FdEntry::ConsoleIn) | Some(FdEntry::ConsoleOut) | Some(FdEntry::ConsoleErr) => {
                copy_to_user_obj(buf, &fill_stat(&console_attr()))?;
                ok(0)
            }
            _ => err(Errno::EBADF),
        }
    })
    .unwrap_or(err(Errno::EFAULT))
}

fn pipe_attr() -> crate::fs::vfs::inode::InodeAttr {
    use crate::fs::vfs::inode::{FileType, InodeAttr, InodeMode};
    InodeAttr {
        ino: 0,
        mode: InodeMode::default_file(),
        file_type: FileType::Fifo,
        size: 0,
        nlink: 1,
        generation: 1,
        rdev: 0,
    }
}

fn console_attr() -> crate::fs::vfs::inode::InodeAttr {
    use crate::fs::vfs::inode::{FileType, InodeAttr, InodeMode};
    InodeAttr {
        ino: 0,
        mode: InodeMode::default_file(),
        file_type: FileType::CharDev,
        size: 0,
        nlink: 1,
        generation: 1,
        rdev: (5 << 8) | 1,
    }
}

pub fn sys_getdents64(fd: u64, dirp: u64, count: u64, _: u64, _: u64, _: u64) -> SysResult {
    if count < core::mem::size_of::<LinuxDirent64>() as u64 {
        return err(Errno::EINVAL);
    }
    user_slice_ok(dirp, count)?;
    proc::current_process_mut(|p| {
        let file = p.fds.get_vfs_mut(fd as u32).ok_or(Errno::EBADF)?;
        let fs = crate::fs::vfs::mount::MountTable::fs(file.inode.mount).ok_or(Errno::ENOENT)?;
        let mut entry = crate::fs::vfs::superblock::DirEntry::empty();
        let index = file.offset;
        if !fs.readdir(file.inode, index, &mut entry).map_err(fs_err_to_errno)? {
            return ok(0);
        }
        file.offset += 1;
        let mut dirent = LinuxDirent64 {
            d_ino: entry.inode.ino,
            d_off: file.offset as i64,
            d_reclen: 0,
            d_type: dirent_type(entry.file_type),
            name: [0; 256],
        };
        let nlen = entry.name_len.min(255);
        dirent.name[..nlen].copy_from_slice(&entry.name[..nlen]);
        dirent.d_reclen = (core::mem::offset_of!(LinuxDirent64, name) + nlen + 1 + 7) as u16 & !7;
        let reclen = dirent.d_reclen as u64;
        if reclen > count {
            return err(Errno::EINVAL);
        }
        copy_to_user_obj(dirp, &dirent)?;
        ok(reclen as isize)
    })
    .unwrap_or(err(Errno::EFAULT))
}

fn dirent_type(ft: crate::fs::vfs::inode::FileType) -> u8 {
    use crate::fs::vfs::inode::FileType;
    match ft {
        FileType::Regular => 8,
        FileType::Directory => 4,
        FileType::Symlink => 10,
        FileType::CharDev => 2,
        FileType::BlockDev => 6,
        FileType::Fifo => 1,
        FileType::Socket => 12,
    }
}

pub fn sys_getcwd(buf: u64, size: u64, _: u64, _: u64, _: u64, _: u64) -> SysResult {
    if size == 0 {
        return err(Errno::EINVAL);
    }
    user_slice_ok(buf, size)?;
    proc::current_process_mut(|p| {
        let end = p.cwd.iter().position(|&b| b == 0).unwrap_or(1);
        let cwd = &p.cwd[..end.max(1)];
        if cwd.len() + 1 > size as usize {
            return err(Errno::ERANGE);
        }
        copy_to_user(buf, cwd)?;
        copy_to_user(buf + cwd.len() as u64, b"\0")?;
        ok(buf as isize)
    })
    .unwrap_or(err(Errno::EFAULT))
}

pub fn sys_chdir(path: u64, _: u64, _: u64, _: u64, _: u64, _: u64) -> SysResult {
    let mut pathbuf = [0u8; 256];
    str_from_user(&mut pathbuf, path, 255)?;
    let end = pathbuf.iter().position(|&b| b == 0).unwrap_or(pathbuf.len());
    stat_path(&pathbuf[..end]).map_err(fs_err_to_errno)?;
    proc::current_process_mut(|p| {
        p.cwd[..end].copy_from_slice(&pathbuf[..end]);
        p.cwd[end] = 0;
        ok(0)
    })
    .unwrap_or(err(Errno::EFAULT))
}

pub fn sys_access(path: u64, mode: u64, _: u64, _: u64, _: u64, _: u64) -> SysResult {
    let mut pathbuf = [0u8; 256];
    str_from_user(&mut pathbuf, path, 255)?;
    let end = pathbuf.iter().position(|&b| b == 0).unwrap_or(pathbuf.len());
    let attr = stat_path(&pathbuf[..end]).map_err(fs_err_to_errno)?;
    // Root bypasses permission checks (POSIX); otherwise verify mode bits against 0o755 default.
    if proc::capable(crate::security::CapSet::DAC_OVERRIDE) {
        return ok(0);
    }
    let file_mode = mode_from_attr(&attr) & 0o777;
    let want = mode as u32 & 0o7;
    if want == 0 {
        return ok(0);
    }
    if want & file_mode != 0 || file_mode & 0o5 != 0 {
        ok(0)
    } else {
        err(Errno::EACCES)
    }
}

pub fn sys_readlink(path: u64, buf: u64, bufsz: u64, _: u64, _: u64, _: u64) -> SysResult {
    let mut pathbuf = [0u8; 256];
    str_from_user(&mut pathbuf, path, 255)?;
    let end = pathbuf.iter().position(|&b| b == 0).unwrap_or(pathbuf.len());
    let resolved = crate::fs::vfs::path::resolve_path_at(&pathbuf[..end], false)
        .map_err(fs_err_to_errno)?;
    let fs = crate::fs::vfs::mount::MountTable::fs(resolved.inode.mount).ok_or(Errno::ENOENT)?;
    let mut kbuf = [0u8; 256];
    let n = fs.readlink(resolved.inode, &mut kbuf).map_err(fs_err_to_errno)?;
    let n = n.min(bufsz as usize);
    copy_to_user(buf, &kbuf[..n])?;
    ok(n as isize)
}

pub fn sys_unlink(path: u64, _: u64, _: u64, _: u64, _: u64, _: u64) -> SysResult {
    let mut pathbuf = [0u8; 256];
    str_from_user(&mut pathbuf, path, 255)?;
    let end = pathbuf.iter().position(|&b| b == 0).unwrap_or(pathbuf.len());
    let (parent, name, name_len) =
        crate::fs::vfs::path::lookup_parent(&pathbuf[..end]).map_err(fs_err_to_errno)?;
    let fs = crate::fs::vfs::mount::MountTable::fs(parent.mount).ok_or(Errno::ENOENT)?;
    fs.unlink(parent, &name[..name_len]).map_err(fs_err_to_errno)?;
    ok(0)
}

pub fn sys_openat(_dirfd: u64, path: u64, flags: u64, mode: u64, _: u64, _: u64) -> SysResult {
    let _ = mode;
    sys_open(path, flags, 0, 0, 0, 0)
}

pub fn sys_ioctl(fd: u64, req: u64, arg: u64, _: u64, _: u64, _: u64) -> SysResult {
    proc::current_process_mut(|p| {
        let entry = p.fds.get(fd as u32).ok_or(Errno::EBADF)?;
        match entry {
            FdEntry::ConsoleIn | FdEntry::ConsoleOut | FdEntry::ConsoleErr => match req {
                TCGETS => {
                    user_slice_ok(arg, core::mem::size_of::<Termios>() as u64)?;
                    // Minimal cooked terminal — sufficient for busybox/isatty probes.
                    let t = Termios {
                        iflag: 0,
                        oflag: 0,
                        cflag: 0x4cf,
                        lflag: 0x8a3b,
                        line: 0,
                        cc: [0; 19],
                    };
                    copy_to_user_obj(arg, &t)?;
                    ok(0)
                }
                TIOCGWINSZ => {
                    user_slice_ok(arg, 8)?;
                    let ws = Winsize {
                        ws_row: 24,
                        ws_col: 80,
                        ws_xpixel: 0,
                        ws_ypixel: 0,
                    };
                    copy_to_user_obj(arg, &ws)?;
                    ok(0)
                }
                _ => err(Errno::ENOTTY),
            },
            _ => err(Errno::ENOTTY),
        }
    })
    .unwrap_or(err(Errno::EFAULT))
}

pub fn sys_fcntl(fd: u64, cmd: u64, arg: u64, _: u64, _: u64, _: u64) -> SysResult {
    proc::current_process_mut(|p| -> SysResult {
        match cmd {
            F_DUPFD => {
                let entry = p.fds.dup(fd as u32).map_err(|_| Errno::EBADF)?;
                p.fds
                    .alloc(entry)
                    .map(|n| ok(n as isize))
                    .map_err(|_| Errno::EMFILE)?
            }
            F_GETFD => ok(p.fds.get_fd_flags(fd as u32) as isize),
            F_SETFD => {
                p.fds
                    .set_fd_flags(fd as u32, arg as u8)
                    .map_err(|_| Errno::EBADF)?;
                ok(0)
            }
            F_GETFL => {
                let _ = p.fds.get(fd as u32).ok_or(Errno::EBADF)?;
                let mut fl = p.fds.open_flags(fd as u32).bits();
                if p.fds.get_fd_flags(fd as u32) & FD_CLOEXEC != 0 {
                    fl |= 0x80000;
                }
                ok(fl as isize)
            }
            F_SETFL => {
                p.fds.get(fd as u32).ok_or(Errno::EBADF)?;
                ok(0)
            }
            _ => err(Errno::EINVAL),
        }
    })
    .unwrap_or(err(Errno::EFAULT))
}
