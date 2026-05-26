//! Unix domain sockets (AF_UNIX) — stream and datagram.

use spin::Mutex;

use super::wait::WaitQueue;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UnixSocketId(u32);

impl UnixSocketId {
    pub const INVALID: Self = Self(u32::MAX);

    pub fn as_u32(self) -> u32 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnixType {
    Stream,
    Dgram,
}

const MAX_UNIX: usize = 64;
const UNIX_BUF: usize = 8192;

struct UnixSocket {
    sock_type: UnixType,
    bound_path: [u8; 108],
    bound_len: usize,
    connected: Option<UnixSocketId>,
    listener: bool,
    pending: [Option<UnixSocketId>; 8],
    pending_len: usize,
    buf: [u8; UNIX_BUF],
    head: usize,
    tail: usize,
    len: usize,
    peer_closed: bool,
    read_waiters: WaitQueue,
    write_waiters: WaitQueue,
    accept_waiters: WaitQueue,
}

impl UnixSocket {
    const fn new(sock_type: UnixType) -> Self {
        Self {
            sock_type,
            bound_path: [0; 108],
            bound_len: 0,
            connected: None,
            listener: false,
            pending: [None; 8],
            pending_len: 0,
            buf: [0; UNIX_BUF],
            head: 0,
            tail: 0,
            len: 0,
            peer_closed: false,
            read_waiters: WaitQueue::new(),
            write_waiters: WaitQueue::new(),
            accept_waiters: WaitQueue::new(),
        }
    }

    fn write_bytes(&mut self, data: &[u8]) -> usize {
        let mut n = 0;
        for &b in data {
            if self.len >= UNIX_BUF {
                break;
            }
            self.buf[self.tail] = b;
            self.tail = (self.tail + 1) % UNIX_BUF;
            self.len += 1;
            n += 1;
        }
        n
    }

    fn read_bytes(&mut self, out: &mut [u8]) -> usize {
        let mut n = 0;
        while n < out.len() && self.len > 0 {
            out[n] = self.buf[self.head];
            self.head = (self.head + 1) % UNIX_BUF;
            self.len -= 1;
            n += 1;
        }
        n
    }
}

static SOCKETS: Mutex<[Option<UnixSocket>; MAX_UNIX]> = Mutex::new([const { None }; MAX_UNIX]);

fn alloc_id() -> Option<UnixSocketId> {
    let mut socks = SOCKETS.lock();
    for (i, slot) in socks.iter_mut().enumerate() {
        if slot.is_none() {
            return Some(UnixSocketId(i as u32));
        }
    }
    None
}

fn with_sock<F, R>(id: UnixSocketId, f: F) -> Option<R>
where
    F: FnOnce(&mut UnixSocket) -> R,
{
    SOCKETS.lock()[id.0 as usize].as_mut().map(f)
}

fn find_bound(path: &[u8]) -> Option<UnixSocketId> {
    let socks = SOCKETS.lock();
    for (i, slot) in socks.iter().enumerate() {
        if let Some(s) = slot {
            if s.bound_len == path.len() && &s.bound_path[..s.bound_len] == path {
                return Some(UnixSocketId(i as u32));
            }
        }
    }
    None
}

pub fn socket(sock_type: UnixType) -> Result<UnixSocketId, UnixError> {
    let id = alloc_id().ok_or(UnixError::NoMemory)?;
    SOCKETS.lock()[id.0 as usize] = Some(UnixSocket::new(sock_type));
    Ok(id)
}

pub fn bind(id: UnixSocketId, path: &[u8]) -> Result<(), UnixError> {
    if path.len() > 108 {
        return Err(UnixError::InvalidArg);
    }
    if find_bound(path).is_some() {
        return Err(UnixError::AddrInUse);
    }
    with_sock(id, |s| {
        s.bound_len = path.len();
        s.bound_path[..path.len()].copy_from_slice(path);
        Ok(())
    })
    .ok_or(UnixError::BadFd)?
}

pub fn listen(id: UnixSocketId, _backlog: u32) -> Result<(), UnixError> {
    with_sock(id, |s| {
        if s.sock_type != UnixType::Stream {
            return Err(UnixError::NotSupported);
        }
        s.listener = true;
        Ok(())
    })
    .ok_or(UnixError::BadFd)?
}

pub fn accept(id: UnixSocketId) -> Result<UnixSocketId, UnixError> {
    loop {
        let pending = with_sock(id, |s| {
            if !s.listener {
                return Err(UnixError::NotSupported);
            }
            if s.pending_len > 0 {
                s.pending_len -= 1;
                Ok(s.pending[s.pending_len].unwrap())
            } else {
                Err(UnixError::WouldBlock)
            }
        });
        match pending {
            Some(Ok(client_id)) => {
                let conn = socket(UnixType::Stream)?;
                with_sock(client_id, |c| c.connected = Some(conn));
                with_sock(conn, |c| c.connected = Some(client_id));
                return Ok(conn);
            }
            Some(Err(UnixError::WouldBlock)) => {
                with_sock(id, |s| s.accept_waiters.block_on());
            }
            Some(Err(e)) => return Err(e),
            None => return Err(UnixError::BadFd),
        }
    }
}

pub fn connect(id: UnixSocketId, path: &[u8]) -> Result<(), UnixError> {
    let listener = find_bound(path).ok_or(UnixError::ConnRefused)?;
    with_sock(id, |s| {
        if s.sock_type != UnixType::Stream {
            return Err(UnixError::NotSupported);
        }
        s.connected = Some(listener);
        Ok(())
    })
    .ok_or(UnixError::BadFd)??;

    with_sock(listener, |l| {
        if l.pending_len >= 8 {
            return Err(UnixError::NoMemory);
        }
        l.pending[l.pending_len] = Some(id);
        l.pending_len += 1;
        l.accept_waiters.wake_one();
        Ok(())
    })
    .ok_or(UnixError::BadFd)??;
    Ok(())
}

pub fn send(id: UnixSocketId, buf: &[u8]) -> Result<usize, UnixError> {
    loop {
        let peer_id = with_sock(id, |s| match s.sock_type {
            UnixType::Dgram => Some(id),
            UnixType::Stream => s.connected,
        })
        .flatten()
        .ok_or(UnixError::NotConnected)?;

        let result = with_sock(peer_id, |peer| {
            if peer.peer_closed {
                return Err(UnixError::BrokenPipe);
            }
            if peer.len >= UNIX_BUF {
                return Err(UnixError::WouldBlock);
            }
            let n = peer.write_bytes(buf);
            if n > 0 {
                peer.read_waiters.wake_one();
            }
            Ok(n)
        });
        match result {
            Some(Ok(n)) if n > 0 => return Ok(n),
            Some(Ok(_)) => {}
            Some(Err(UnixError::WouldBlock)) => {
                with_sock(peer_id, |peer| peer.write_waiters.block_on());
            }
            Some(Err(e)) => return Err(e),
            None => return Err(UnixError::BadFd),
        }
    }
}

pub fn recv(id: UnixSocketId, buf: &mut [u8]) -> Result<usize, UnixError> {
    loop {
        let result = with_sock(id, |s| {
            if s.len > 0 {
                let n = s.read_bytes(buf);
                s.write_waiters.wake_one();
                return Ok(n);
            }
            if s.peer_closed {
                return Ok(0);
            }
            Err(UnixError::WouldBlock)
        });
        match result {
            Some(Ok(n)) => return Ok(n),
            Some(Err(UnixError::WouldBlock)) => {
                with_sock(id, |s| s.read_waiters.block_on());
            }
            Some(Err(e)) => return Err(e),
            None => return Err(UnixError::BadFd),
        }
    }
}

pub fn close(id: UnixSocketId) {
    if let Some(peer) = with_sock(id, |s| s.connected).flatten() {
        with_sock(peer, |p| {
            p.peer_closed = true;
            p.read_waiters.wake_all();
        });
    }
    SOCKETS.lock()[id.0 as usize] = None;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnixError {
    BadFd,
    InvalidArg,
    NoMemory,
    NotSupported,
    NotConnected,
    WouldBlock,
    AddrInUse,
    ConnRefused,
    BrokenPipe,
}

pub fn unix_err_to_errno(err: UnixError) -> crate::syscall::errno::Errno {
    use crate::syscall::errno::Errno;
    match err {
        UnixError::BadFd => Errno::EBADF,
        UnixError::InvalidArg => Errno::EINVAL,
        UnixError::NoMemory => Errno::ENOMEM,
        UnixError::NotSupported => Errno::EOPNOTSUPP,
        UnixError::NotConnected => Errno::ENOTCONN,
        UnixError::WouldBlock => Errno::EAGAIN,
        UnixError::AddrInUse => Errno::EADDRINUSE,
        UnixError::ConnRefused => Errno::ECONNREFUSED,
        UnixError::BrokenPipe => Errno::EPIPE,
    }
}
