//! Socket syscalls — BSD-compatible API backed by the in-kernel network stack.

use crate::fs::fd::FdEntry;
use crate::ipc::unix::{self, UnixSocketId, UnixType};
use crate::net::addr::{parse_sockaddr_in, SOCK_DGRAM, SOCK_STREAM};
use crate::net::socket::{self as inet, SocketId};
use crate::proc;
use crate::syscall::errno::{err, ok, Errno, SysResult};
use crate::syscall::uaccess::{copy_from_user, copy_to_user, user_slice_ok};

const AF_UNIX: u32 = 1;
const AF_INET: u32 = 2;

pub fn sys_socket(domain: u64, sock_type: u64, protocol: u64, _: u64, _: u64, _: u64) -> SysResult {
    if domain as u32 == AF_UNIX {
        let st = match sock_type as u32 {
            SOCK_STREAM => UnixType::Stream,
            SOCK_DGRAM => UnixType::Dgram,
            _ => return err(Errno::EINVAL),
        };
        let _ = protocol;
        return match unix::socket(st) {
            Ok(id) => proc::current_process_mut(|p| match p.fds.alloc(FdEntry::UnixSocket(id)) {
                Ok(fd) => ok(fd as isize),
                Err(()) => err(Errno::EMFILE),
            })
            .unwrap_or(err(Errno::EFAULT)),
            Err(e) => err(unix::unix_err_to_errno(e)),
        };
    }
    match inet::socket(domain as u32, sock_type as u32, protocol as u32) {
        Ok(id) => proc::current_process_mut(|p| match p.fds.alloc(FdEntry::Socket(id)) {
            Ok(fd) => ok(fd as isize),
            Err(()) => err(Errno::EMFILE),
        })
        .unwrap_or(err(Errno::EFAULT)),
        Err(e) => err(inet::socket_err_to_errno(e)),
    }
}

pub fn sys_bind(fd: u64, addr: u64, addrlen: u64, _: u64, _: u64, _: u64) -> SysResult {
    user_slice_ok(addr, addrlen)?;
    let mut buf = [0u8; 128];
    let len = addrlen.min(128) as usize;
    copy_from_user(&mut buf[..len], addr)?;
    if let Ok(id) = current_unix(fd as u32) {
        let path = unix_path(&buf[..len])?;
        return unix::bind(id, path).map_err(unix::unix_err_to_errno).and_then(|_| ok(0));
    }
    let entry = current_inet(fd as u32)?;
    inet::bind(entry, &buf[..len]).map_err(inet::socket_err_to_errno)?;
    ok(0)
}

pub fn sys_listen(fd: u64, backlog: u64, _: u64, _: u64, _: u64, _: u64) -> SysResult {
    if let Ok(id) = current_unix(fd as u32) {
        return unix::listen(id, backlog as u32)
            .map_err(unix::unix_err_to_errno)
            .and_then(|_| ok(0));
    }
    let entry = current_inet(fd as u32)?;
    inet::listen(entry, backlog as u32)
        .map_err(inet::socket_err_to_errno)?;
    ok(0)
}

pub fn sys_accept(fd: u64, addr: u64, addrlen: u64, _: u64, _: u64, _: u64) -> SysResult {
    if let Ok(id) = current_unix(fd as u32) {
        return match unix::accept(id) {
            Ok(new_sock) => proc::current_process_mut(|p| {
                let newfd = p.fds.alloc(FdEntry::UnixSocket(new_sock)).map_err(|_| Errno::EMFILE)?;
                if addr != 0 && addrlen > 0 {
                    write_unix_addr(addr, addrlen, b"\0")?;
                }
                ok(newfd as isize)
            })
            .unwrap_or(err(Errno::EFAULT)),
            Err(e) => err(unix::unix_err_to_errno(e)),
        };
    }
    let mut addrbuf = [0u8; 128];
    let entry = current_inet(fd as u32)?;
    match inet::accept(entry, &mut addrbuf) {
        Ok(new_sock) => proc::current_process_mut(|p| {
            let newfd = p.fds.alloc(FdEntry::Socket(new_sock)).map_err(|_| Errno::EMFILE)?;
            if addr != 0 && addrlen != 0 {
                let len = core::mem::size_of::<crate::net::addr::SockAddrIn>();
                copy_to_user(addr, &addrbuf[..len]).map_err(|_| Errno::EFAULT)?;
            }
            ok(newfd as isize)
        })
        .unwrap_or(err(Errno::EFAULT)),
        Err(e) => err(inet::socket_err_to_errno(e)),
    }
}

pub fn sys_connect(fd: u64, addr: u64, addrlen: u64, _: u64, _: u64, _: u64) -> SysResult {
    user_slice_ok(addr, addrlen)?;
    let mut buf = [0u8; 128];
    let len = addrlen.min(128) as usize;
    copy_from_user(&mut buf[..len], addr)?;
    if let Ok(id) = current_unix(fd as u32) {
        let path = unix_path(&buf[..len])?;
        return unix::connect(id, path).map_err(unix::unix_err_to_errno).and_then(|_| ok(0));
    }
    let entry = current_inet(fd as u32)?;
    inet::connect(entry, &buf[..len]).map_err(inet::socket_err_to_errno)?;
    ok(0)
}

pub fn sys_sendto(
    fd: u64,
    buf: u64,
    len: u64,
    _: u64,
    addr: u64,
    addrlen: u64,
) -> SysResult {
    if len == 0 {
        return ok(0);
    }
    user_slice_ok(buf, len)?;
    let mut kbuf = [0u8; 4096];
    let to_send = len.min(4096) as usize;
    copy_from_user(&mut kbuf[..to_send], buf)?;
    if let Ok(id) = current_unix(fd as u32) {
        return match unix::send(id, &kbuf[..to_send]) {
            Ok(n) => ok(n as isize),
            Err(e) => err(unix::unix_err_to_errno(e)),
        };
    }
    let entry = current_inet(fd as u32)?;
    let sockaddr = if addr != 0 && addrlen > 0 {
        user_slice_ok(addr, addrlen)?;
        let mut sbuf = [0u8; 128];
        let slen = addrlen.min(128) as usize;
        copy_from_user(&mut sbuf[..slen], addr)?;
        Some(sbuf[..slen].to_vec())
    } else {
        None
    };
    match inet::sendto(entry, &kbuf[..to_send], sockaddr.as_deref()) {
        Ok(n) => ok(n as isize),
        Err(e) => err(inet::socket_err_to_errno(e)),
    }
}

pub fn sys_recvfrom(
    fd: u64,
    buf: u64,
    len: u64,
    _: u64,
    addr: u64,
    addrlen: u64,
) -> SysResult {
    if len == 0 {
        return ok(0);
    }
    user_slice_ok(buf, len)?;
    let mut kbuf = [0u8; 4096];
    let to_read = len.min(4096) as usize;
    if let Ok(id) = current_unix(fd as u32) {
        return match unix::recv(id, &mut kbuf[..to_read]) {
            Ok(n) => {
                copy_to_user(buf, &kbuf[..n]).map_err(|_| Errno::EFAULT)?;
                if addr != 0 && addrlen > 0 {
                    write_unix_addr(addr, addrlen, b"/tmp/unix")?;
                }
                ok(n as isize)
            }
            Err(e) => err(unix::unix_err_to_errno(e)),
        };
    }
    let mut addrbuf = [0u8; 128];
    let entry = current_inet(fd as u32)?;
    match inet::recvfrom(entry, &mut kbuf[..to_read], &mut addrbuf) {
        Ok(n) => {
            copy_to_user(buf, &kbuf[..n]).map_err(|_| Errno::EFAULT)?;
            if addr != 0 && addrlen > 0 {
                let slen = core::mem::size_of::<crate::net::addr::SockAddrIn>();
                copy_to_user(addr, &addrbuf[..slen]).map_err(|_| Errno::EFAULT)?;
            }
            ok(n as isize)
        }
        Err(e) => err(inet::socket_err_to_errno(e)),
    }
}

fn current_inet(fd: u32) -> Result<SocketId, Errno> {
    match proc::current_process_mut(|p| p.fds.get(fd)).flatten() {
        Some(FdEntry::Socket(id)) => Ok(id),
        Some(FdEntry::UnixSocket(_)) => Err(Errno::EOPNOTSUPP),
        Some(_) => Err(Errno::ENOTSOCK),
        None => Err(Errno::EBADF),
    }
}

fn current_unix(fd: u32) -> Result<UnixSocketId, Errno> {
    match proc::current_process_mut(|p| p.fds.get(fd)).flatten() {
        Some(FdEntry::UnixSocket(id)) => Ok(id),
        Some(FdEntry::Socket(_)) => Err(Errno::EOPNOTSUPP),
        Some(_) => Err(Errno::ENOTSOCK),
        None => Err(Errno::EBADF),
    }
}

fn unix_path(addr: &[u8]) -> Result<&[u8], Errno> {
    if addr.len() < 2 {
        return Err(Errno::EINVAL);
    }
    if addr[0] == 0 {
        Ok(&addr[1..])
    } else if addr[1] == 0 {
        Ok(&addr[2..])
    } else {
        parse_sockaddr_in(addr);
        Err(Errno::EINVAL)
    }
}

fn write_unix_addr(addr: u64, addrlen: u64, path: &[u8]) -> Result<(), Errno> {
    let mut buf = [0u8; 128];
    let plen = path.len().min(107);
    buf[0] = 1;
    buf[1..1 + plen].copy_from_slice(&path[..plen]);
    let total = (2 + plen).min(addrlen as usize);
    copy_to_user(addr, &buf[..total]).map_err(|_| Errno::EFAULT)?;
    Ok(())
}
