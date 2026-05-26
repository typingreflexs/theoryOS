//! Kernel socket layer — BSD-compatible API for syscalls.

use spin::Mutex;

use super::addr::{parse_sockaddr_in, write_sockaddr_in, IpAddr, Ipv4Addr, SOCK_DGRAM, SOCK_STREAM};
use super::tcp;
use super::udp;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SocketId(pub u32);

impl SocketId {
    pub const INVALID: Self = Self(u32::MAX);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SocketType {
    Stream,
    Dgram,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SocketDomain {
    Inet,
    Inet6,
}

#[derive(Debug)]
pub struct Socket {
    pub domain: SocketDomain,
    pub sock_type: SocketType,
    pub bound_port: u16,
    pub tcp_conn: Option<u32>,
    pub remote: Option<(IpAddr, u16)>,
}

const MAX_SOCKETS: usize = 128;

static SOCKETS: Mutex<[Option<Socket>; MAX_SOCKETS]> = Mutex::new([const { None }; MAX_SOCKETS]);
static NEXT_PORT: Mutex<u16> = Mutex::new(49152);

pub fn init() {}

fn alloc_id() -> Option<SocketId> {
    let mut socks = SOCKETS.lock();
    for (i, slot) in socks.iter_mut().enumerate() {
        if slot.is_none() {
            return Some(SocketId(i as u32));
        }
    }
    None
}

pub fn socket(domain: u32, sock_type: u32, _protocol: u32) -> Result<SocketId, SocketError> {
    let domain = match domain {
        2 => SocketDomain::Inet,
        10 => SocketDomain::Inet6,
        _ => return Err(SocketError::Unsupported),
    };
    let sock_type = match sock_type {
        SOCK_STREAM => SocketType::Stream,
        SOCK_DGRAM => SocketType::Dgram,
        _ => return Err(SocketError::Unsupported),
    };
    let id = alloc_id().ok_or(SocketError::NoMemory)?;
    SOCKETS.lock()[id.0 as usize] = Some(Socket {
        domain,
        sock_type,
        bound_port: 0,
        tcp_conn: None,
        remote: None,
    });
    Ok(id)
}

pub fn bind(id: SocketId, addr: &[u8]) -> Result<(), SocketError> {
    let (_, port) = parse_sockaddr_in(addr).ok_or(SocketError::InvalidArg)?;
    let mut socks = SOCKETS.lock();
    let sock = socks.get_mut(id.0 as usize).and_then(|s| s.as_mut()).ok_or(SocketError::BadFd)?;
    sock.bound_port = if port == 0 {
        let mut p = NEXT_PORT.lock();
        *p = p.wrapping_add(1);
        *p
    } else {
        port
    };
    match sock.sock_type {
        SocketType::Dgram => udp::bind_port(sock.bound_port),
        SocketType::Stream => {
            sock.tcp_conn = tcp::create_listener(sock.bound_port);
        }
    }
    Ok(())
}

pub fn listen(id: SocketId, _backlog: u32) -> Result<(), SocketError> {
    with_socket(id, |s| {
        if s.sock_type != SocketType::Stream {
            return Err(SocketError::NotSupported);
        }
        Ok(())
    })?;
    Ok(())
}

pub fn accept(id: SocketId, addr_out: &mut [u8]) -> Result<SocketId, SocketError> {
    let (listener_idx, listener_port) = {
        let socks = SOCKETS.lock();
        let s = socks.get(id.0 as usize).and_then(|x| x.as_ref()).ok_or(SocketError::BadFd)?;
        if s.sock_type != SocketType::Stream {
            return Err(SocketError::NotSupported);
        }
        (s.tcp_conn.ok_or(SocketError::InvalidArg)?, s.bound_port)
    };
    let conn_idx = tcp::accept(listener_idx).ok_or(SocketError::WouldBlock)?;
    let new_id = alloc_id().ok_or(SocketError::NoMemory)?;
    tcp::with_connection(conn_idx, |tcb| {
        if let IpAddr::V4(ip) = tcb.remote {
            write_sockaddr_in(ip, tcb.remote_port, addr_out);
        }
    });
    SOCKETS.lock()[new_id.0 as usize] = Some(Socket {
        domain: SocketDomain::Inet,
        sock_type: SocketType::Stream,
        bound_port: listener_port,
        tcp_conn: Some(conn_idx),
        remote: None,
    });
    Ok(new_id)
}

pub fn connect(id: SocketId, addr: &[u8]) -> Result<(), SocketError> {
    let (ip, port) = parse_sockaddr_in(addr).ok_or(SocketError::InvalidArg)?;
    let mut socks = SOCKETS.lock();
    let sock = socks.get_mut(id.0 as usize).and_then(|s| s.as_mut()).ok_or(SocketError::BadFd)?;
    if sock.sock_type != SocketType::Stream {
        return Err(SocketError::NotSupported);
    }
    let local_port = if sock.bound_port == 0 {
        let mut p = NEXT_PORT.lock();
        *p = p.wrapping_add(1);
        sock.bound_port = *p;
        *p
    } else {
        sock.bound_port
    };
    sock.tcp_conn = tcp::connect(ip, port, local_port);
    sock.remote = Some((ip, port));
    sock.tcp_conn.map(|_| ()).ok_or(SocketError::NetworkUnreachable)
}

pub fn sendto(id: SocketId, buf: &[u8], addr: Option<&[u8]>) -> Result<usize, SocketError> {
    let mut socks = SOCKETS.lock();
    let sock = socks.get_mut(id.0 as usize).and_then(|s| s.as_mut()).ok_or(SocketError::BadFd)?;
    match sock.sock_type {
        SocketType::Dgram => {
            let (ip, port) = if let Some(a) = addr {
                parse_sockaddr_in(a).ok_or(SocketError::InvalidArg)?
            } else if let Some(r) = sock.remote {
                r
            } else {
                return Err(SocketError::InvalidArg);
            };
            let dst_port = port;
            let src_port = if sock.bound_port == 0 {
                let mut p = NEXT_PORT.lock();
                *p = p.wrapping_add(1);
                sock.bound_port = *p;
                *p
            } else {
                sock.bound_port
            };
            udp::sendto(ip, dst_port, src_port, buf).map_err(|_| SocketError::Io)?;
            Ok(buf.len())
        }
        SocketType::Stream => {
            let idx = sock.tcp_conn.ok_or(SocketError::NotConnected)?;
            tcp::send(idx, buf).map_err(|_| SocketError::Io)
        }
    }
}

pub fn recvfrom(id: SocketId, buf: &mut [u8], addr_out: &mut [u8]) -> Result<usize, SocketError> {
    super::rx_poll();
    super::dhcp::poll();
    super::tcp::tick();
    let mut socks = SOCKETS.lock();
    let sock = socks.get_mut(id.0 as usize).and_then(|s| s.as_mut()).ok_or(SocketError::BadFd)?;
    match sock.sock_type {
        SocketType::Dgram => {
            let dg = udp::recv(sock.bound_port).ok_or(SocketError::WouldBlock)?;
            let n = dg.data.len().min(buf.len());
            buf[..n].copy_from_slice(&dg.data[..n]);
            if let IpAddr::V4(ip) = dg.src {
                write_sockaddr_in(ip, dg.src_port, addr_out);
            }
            Ok(n)
        }
        SocketType::Stream => {
            let idx = sock.tcp_conn.ok_or(SocketError::NotConnected)?;
            drop(socks);
            tcp::recv(idx, buf).ok_or(SocketError::WouldBlock)
        }
    }
}

pub fn close(id: SocketId) {
    SOCKETS.lock()[id.0 as usize] = None;
}

fn with_socket<F, R>(id: SocketId, f: F) -> Result<R, SocketError>
where
    F: FnOnce(&Socket) -> Result<R, SocketError>,
{
    SOCKETS.lock()[id.0 as usize]
        .as_ref()
        .ok_or(SocketError::BadFd)
        .and_then(f)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SocketError {
    BadFd,
    InvalidArg,
    NoMemory,
    Unsupported,
    NotSupported,
    NotConnected,
    WouldBlock,
    NetworkUnreachable,
    Io,
}

pub fn socket_err_to_errno(err: SocketError) -> crate::syscall::errno::Errno {
    use crate::syscall::errno::Errno;
    match err {
        SocketError::BadFd => Errno::EBADF,
        SocketError::InvalidArg => Errno::EINVAL,
        SocketError::NoMemory => Errno::ENOMEM,
        SocketError::Unsupported | SocketError::NotSupported => Errno::EAFNOSUPPORT,
        SocketError::NotConnected => Errno::ENOTCONN,
        SocketError::WouldBlock => Errno::EAGAIN,
        SocketError::NetworkUnreachable => Errno::ENETUNREACH,
        SocketError::Io => Errno::EIO,
    }
}
