//! I/O syscalls: read, write, open, close, pipe, dup, dup2, lseek.

use crate::console::Console;
use crate::fs::fd::{FdEntry, OpenFlags};
use crate::fs::pipe::{pipe_err_to_errno, Pipe};
use crate::fs::vfs::{fs_err_to_errno, open_path, read_file, write_file, lseek_file};
use crate::ipc::fifo;
use crate::ipc::unix;
use crate::net::socket as inet;
use crate::proc;
use crate::syscall::errno::{err, ok, Errno, SysResult};
use crate::syscall::handlers::ipc::{close_fd_entry, dup_fd_entry, try_open_fifo_from_vfs};
use crate::syscall::uaccess::{copy_from_user, copy_to_user, str_from_user, user_slice_ok};

pub fn sys_read(fd: u64, buf: u64, count: u64, _: u64, _: u64, _: u64) -> SysResult {
    if count == 0 {
        return ok(0);
    }
    user_slice_ok(buf, count)?;
    let mut kbuf = [0u8; 4096];
    let to_read = count.min(4096) as usize;

    proc::current_process_mut(|p| {
        let entry = p.fds.get(fd as u32).ok_or(Errno::EBADF)?;
        let n = match entry {
            FdEntry::ConsoleIn => 0,
            FdEntry::PipeRead(id) => Pipe::read(id, &mut kbuf[..to_read]).map_err(pipe_err_to_errno)?,
            FdEntry::Vfs(_) => {
                let file = p.fds.get_vfs_mut(fd as u32).ok_or(Errno::EBADF)?;
                read_file(file, &mut kbuf[..to_read]).map_err(fs_err_to_errno)?
            }
            FdEntry::Socket(id) => inet::recvfrom(id, &mut kbuf[..to_read], &mut [0u8; 128])
                .map_err(inet::socket_err_to_errno)?,
            FdEntry::UnixSocket(id) => unix::recv(id, &mut kbuf[..to_read]).map_err(unix::unix_err_to_errno)?,
            _ => return err(Errno::EBADF),
        };
        if n > 0 {
            copy_to_user(buf, &kbuf[..n]).map_err(|_| Errno::EFAULT)?;
        }
        ok(n as isize)
    })
    .unwrap_or(err(Errno::EFAULT))
}

pub fn sys_write(fd: u64, buf: u64, count: u64, _: u64, _: u64, _: u64) -> SysResult {
    if count == 0 {
        return ok(0);
    }
    user_slice_ok(buf, count)?;
    let mut kbuf = [0u8; 4096];
    let to_write = count.min(4096) as usize;
    copy_from_user(&mut kbuf[..to_write], buf)?;

    proc::current_process_mut(|p| {
        let entry = p.fds.get(fd as u32).ok_or(Errno::EBADF)?;
        match entry {
            FdEntry::ConsoleOut | FdEntry::ConsoleErr => {
                if let Ok(s) = core::str::from_utf8(&kbuf[..to_write]) {
                    Console::print(s);
                }
                ok(to_write as isize)
            }
            FdEntry::PipeWrite(id) => {
                let n = Pipe::write(id, &kbuf[..to_write]).map_err(pipe_err_to_errno)?;
                ok(n as isize)
            }
            FdEntry::Vfs(_) => {
                let file = p.fds.get_vfs_mut(fd as u32).ok_or(Errno::EBADF)?;
                let n = write_file(file, &kbuf[..to_write]).map_err(fs_err_to_errno)?;
                ok(n as isize)
            }
            FdEntry::Socket(id) => {
                let n = inet::sendto(id, &kbuf[..to_write], None).map_err(inet::socket_err_to_errno)?;
                ok(n as isize)
            }
            FdEntry::UnixSocket(id) => {
                let n = unix::send(id, &kbuf[..to_write]).map_err(unix::unix_err_to_errno)?;
                ok(n as isize)
            }
            _ => err(Errno::EBADF),
        }
    })
    .unwrap_or(err(Errno::EFAULT))
}

pub fn sys_open(path: u64, flags: u64, _: u64, _: u64, _: u64, _: u64) -> SysResult {
    let mut pathbuf = [0u8; 256];
    str_from_user(&mut pathbuf, path, 255)?;
    let end = pathbuf.iter().position(|&b| b == 0).unwrap_or(pathbuf.len());
    let path = &pathbuf[..end];
    let flags = OpenFlags::from_bits_truncate(flags as u32);

    match open_path(path, flags) {
        Ok(open_file) => proc::current_process_mut(|p| match p.fds.alloc(FdEntry::Vfs(open_file)) {
            Ok(fd) => ok(fd as isize),
            Err(()) => err(Errno::EMFILE),
        })
        .unwrap_or(err(Errno::EFAULT)),
        Err(crate::fs::vfs::superblock::FsError::NotFound) if fifo::lookup(path).is_some() => {
            proc::current_process_mut(|p| {
                let entry = try_open_fifo_from_vfs(path, flags)?;
                match p.fds.alloc(entry) {
                    Ok(fd) => ok(fd as isize),
                    Err(()) => err(Errno::EMFILE),
                }
            })
            .unwrap_or(err(Errno::EFAULT))
        }
        Err(e) => err(fs_err_to_errno(e)),
    }
}

pub fn sys_close(fd: u64, _: u64, _: u64, _: u64, _: u64, _: u64) -> SysResult {
    proc::current_process_mut(|p| {
        let entry = p.fds.get(fd as u32).ok_or(Errno::EBADF)?;
        close_fd_entry(entry);
        match p.fds.close(fd as u32) {
            Ok(()) => ok(0),
            Err(()) => err(Errno::EBADF),
        }
    })
    .unwrap_or(err(Errno::EFAULT))
}

pub fn sys_lseek(fd: u64, offset: u64, whence: u64, _: u64, _: u64, _: u64) -> SysResult {
    proc::current_process_mut(|p| {
        let file = p.fds.get_vfs_mut(fd as u32).ok_or(Errno::EBADF)?;
        match lseek_file(file, offset as i64, whence as u32) {
            Ok(pos) => ok(pos as isize),
            Err(e) => err(fs_err_to_errno(e)),
        }
    })
    .unwrap_or(err(Errno::EBADF))
}

pub fn sys_pipe(fds: u64, _: u64, _: u64, _: u64, _: u64, _: u64) -> SysResult {
    crate::syscall::handlers::sys_pipe2(fds, 0, 0, 0, 0, 0)
}

pub fn sys_dup(fd: u64, _: u64, _: u64, _: u64, _: u64, _: u64) -> SysResult {
    match proc::current_process_mut(|p| {
        let entry = p.fds.dup(fd as u32).map_err(|_| Errno::EBADF)?;
        dup_fd_entry(entry);
        p.fds.alloc(entry).map_err(|_| Errno::EMFILE)
    }) {
        Some(Ok(n)) => ok(n as isize),
        Some(Err(e)) => Err(e),
        None => err(Errno::EFAULT),
    }
}

pub fn sys_dup2(oldfd: u64, newfd: u64, _: u64, _: u64, _: u64, _: u64) -> SysResult {
    match proc::current_process_mut(|p| {
        if oldfd != newfd {
            if let Some(old) = p.fds.get(oldfd as u32) {
                dup_fd_entry(old);
            }
        }
        p.fds.dup2(oldfd as u32, newfd as u32).map_err(|_| Errno::EBADF)
    }) {
        Some(Ok(n)) => ok(n as isize),
        Some(Err(e)) => Err(e),
        None => err(Errno::EFAULT),
    }
}
