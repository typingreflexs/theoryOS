//! IPC-related syscalls.

use crate::fs::fd::{FdEntry, OpenFlags};
use crate::fs::vfs::path::resolve_path_at;
use crate::fs::vfs::inode::FileType;
use crate::ipc::{fifo, futex, mq, pipe, shm, unix};
use crate::proc;
use crate::syscall::errno::{err, ok, Errno, SysResult};
use crate::syscall::uaccess::{copy_from_user, copy_from_user_obj, copy_to_user, copy_to_user_obj, str_from_user, user_slice_ok};

pub const O_NONBLOCK: u32 = 0x800;

pub fn sys_pipe2(fds: u64, flags: u64, _: u64, _: u64, _: u64, _: u64) -> SysResult {
    let _ = flags & O_NONBLOCK as u64;
    user_slice_ok(fds, 8)?;
    let (read_id, write_id) = pipe::Pipe::create().map_err(|_| Errno::ENOSPC)?;
    proc::current_process_mut(|p| {
        let rfd = p
            .fds
            .alloc(FdEntry::PipeRead(read_id))
            .map_err(|_| Errno::EMFILE)?;
        let wfd = p
            .fds
            .alloc(FdEntry::PipeWrite(write_id))
            .map_err(|_| Errno::EMFILE)?;
        let pair = [rfd as i32, wfd as i32];
        copy_to_user_obj(fds, &pair)?;
        ok(0)
    })
    .unwrap_or(err(Errno::EFAULT))
}

pub fn sys_mkfifo(path: u64, _mode: u64, _: u64, _: u64, _: u64, _: u64) -> SysResult {
    let mut pathbuf = [0u8; 256];
    str_from_user(&mut pathbuf, path, 255)?;
    let end = pathbuf.iter().position(|&b| b == 0).unwrap_or(pathbuf.len());
    let path = &pathbuf[..end];
    let pipe_id = fifo::mkfifo(path).map_err(|_| Errno::ENOSPC)?;
    fifo::register(path, pipe_id);
    ok(0)
}

pub fn sys_msgget(key: u64, flags: u64, _: u64, _: u64, _: u64, _: u64) -> SysResult {
    match mq::msgget(key as i32, flags as i32) {
        Ok(id) => ok(id as isize),
        Err(e) => err(mq::msg_err_to_errno(e)),
    }
}

pub fn sys_msgsnd(qid: u64, msg_ptr: u64, msgsz: u64, _: u64, _: u64, _: u64) -> SysResult {
    if msgsz == 0 || msgsz > 4096 {
        return err(Errno::EINVAL);
    }
    user_slice_ok(msg_ptr, 8 + msgsz)?;
    let mtype: i64 = copy_from_user_obj(msg_ptr)?;
    let mut buf = [0u8; 4096];
    copy_from_user(&mut buf[..msgsz as usize], msg_ptr + 8)?;
    match mq::msgsnd(qid as i32, mtype, &buf[..msgsz as usize]) {
        Ok(()) => ok(0),
        Err(e) => err(mq::msg_err_to_errno(e)),
    }
}

pub fn sys_msgrcv(
    qid: u64,
    msg_ptr: u64,
    msgsz: u64,
    msgtyp: u64,
    _: u64,
    _: u64,
) -> SysResult {
    if msgsz == 0 || msgsz > 4096 {
        return err(Errno::EINVAL);
    }
    user_slice_ok(msg_ptr, 8 + msgsz)?;
    let mut buf = [0u8; 4096];
    match mq::msgrcv(qid as i32, msgtyp as i64, &mut buf[..msgsz as usize], false) {
        Ok((mtype, n)) => {
            copy_to_user_obj(msg_ptr, &mtype)?;
            copy_to_user(msg_ptr + 8, &buf[..n])?;
            ok(n as isize)
        }
        Err(e) => err(mq::msg_err_to_errno(e)),
    }
}

pub fn sys_msgctl(qid: u64, cmd: u64, _: u64, _: u64, _: u64, _: u64) -> SysResult {
    match mq::msgctl(qid as i32, cmd as i32) {
        Ok(()) => ok(0),
        Err(e) => err(mq::msg_err_to_errno(e)),
    }
}

pub fn sys_shmget(key: u64, size: u64, flags: u64, _: u64, _: u64, _: u64) -> SysResult {
    match shm::shmget(key as i32, size, flags as i32) {
        Ok(id) => ok(id as isize),
        Err(e) => err(shm::shm_err_to_errno(e)),
    }
}

pub fn sys_shmat(shmid: u64, addr: u64, flags: u64, _: u64, _: u64, _: u64) -> SysResult {
    match shm::shmat(shmid as i32, addr, flags as i32) {
        Ok(v) => ok(v as isize),
        Err(e) => err(shm::shm_err_to_errno(e)),
    }
}

pub fn sys_shmdt(addr: u64, _: u64, _: u64, _: u64, _: u64, _: u64) -> SysResult {
    match shm::shmdt(addr) {
        Ok(()) => ok(0),
        Err(e) => err(shm::shm_err_to_errno(e)),
    }
}

pub fn sys_shmctl(shmid: u64, cmd: u64, _: u64, _: u64, _: u64, _: u64) -> SysResult {
    match shm::shmctl(shmid as i32, cmd as i32) {
        Ok(()) => ok(0),
        Err(e) => err(shm::shm_err_to_errno(e)),
    }
}

pub fn sys_futex(
    uaddr: u64,
    op: u64,
    val: u64,
    timeout: u64,
    uaddr2: u64,
    val3: u64,
) -> SysResult {
    match futex::futex(
        uaddr,
        op as i32,
        val as u32,
        timeout,
        uaddr2,
        val3 as u32,
    ) {
        Ok(n) => ok(n as isize),
        Err(e) => err(futex::futex_err_to_errno(e)),
    }
}

pub fn close_fd_entry(entry: FdEntry) {
    match entry {
        FdEntry::PipeRead(id) => pipe::Pipe::close_read(id),
        FdEntry::PipeWrite(id) => pipe::Pipe::close_write(id),
        FdEntry::UnixSocket(id) => unix::close(id),
        FdEntry::Socket(id) => crate::net::socket::close(id),
        _ => {}
    }
}

pub fn dup_fd_entry(entry: FdEntry) {
    match entry {
        FdEntry::PipeRead(id) => pipe::Pipe::dup_read(id),
        FdEntry::PipeWrite(id) => pipe::Pipe::dup_write(id),
        _ => {}
    }
}

pub fn try_open_fifo_from_vfs(path: &[u8], flags: OpenFlags) -> Result<FdEntry, Errno> {
    let pipe_id = if let Some(id) = fifo::lookup(path) {
        id
    } else {
        let resolved = resolve_path_at(path, true).map_err(|_| Errno::ENOENT)?;
        if resolved.attr.file_type != FileType::Fifo {
            return Err(Errno::ENOENT);
        }
        fifo::decode_rdev(resolved.attr.rdev).ok_or(Errno::EINVAL)?
    };
    if flags.contains(OpenFlags::O_WRONLY) {
        pipe::Pipe::dup_write(pipe_id);
        Ok(FdEntry::PipeWrite(pipe_id))
    } else {
        pipe::Pipe::dup_read(pipe_id);
        Ok(FdEntry::PipeRead(pipe_id))
    }
}
