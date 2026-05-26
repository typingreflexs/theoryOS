//! Anonymous pipes with refcounts, blocking I/O, and EPIPE semantics.

use spin::Mutex;

use super::wait::WaitQueue;

pub const PIPE_BUF: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PipeId(u32);

impl PipeId {
    pub fn new(id: u32) -> Self {
        Self(id)
    }

    pub fn as_u32(self) -> u32 {
        self.0
    }
}

struct PipeInner {
    buf: [u8; PIPE_BUF],
    head: usize,
    tail: usize,
    len: usize,
    read_refs: u32,
    write_refs: u32,
    read_waiters: WaitQueue,
    write_waiters: WaitQueue,
}

impl PipeInner {
    const fn new() -> Self {
        Self {
            buf: [0; PIPE_BUF],
            head: 0,
            tail: 0,
            len: 0,
            read_refs: 0,
            write_refs: 0,
            read_waiters: WaitQueue::new(),
            write_waiters: WaitQueue::new(),
        }
    }

    fn write_once(&mut self, data: &[u8]) -> usize {
        let mut written = 0;
        for &b in data {
            if self.len >= PIPE_BUF {
                break;
            }
            self.buf[self.tail] = b;
            self.tail = (self.tail + 1) % PIPE_BUF;
            self.len += 1;
            written += 1;
        }
        written
    }

    fn read_once(&mut self, out: &mut [u8]) -> usize {
        let mut n = 0;
        while n < out.len() && self.len > 0 {
            out[n] = self.buf[self.head];
            self.head = (self.head + 1) % PIPE_BUF;
            self.len -= 1;
            n += 1;
        }
        n
    }
}

const MAX_PIPES: usize = 64;

static PIPES: Mutex<[Option<PipeInner>; MAX_PIPES]> = Mutex::new([const { None }; MAX_PIPES]);
static NEXT_PIPE: Mutex<u32> = Mutex::new(0);

fn with_pipe<F, R>(id: PipeId, f: F) -> Option<R>
where
    F: FnOnce(&mut PipeInner) -> R,
{
    PIPES.lock()[id.as_u32() as usize].as_mut().map(f)
}

pub struct Pipe;

impl Pipe {
    pub fn create() -> Result<(PipeId, PipeId), ()> {
        let mut next = NEXT_PIPE.lock();
        let id = *next;
        *next += 1;
        if id as usize >= MAX_PIPES {
            return Err(());
        }
        let mut pipes = PIPES.lock();
        let mut inner = PipeInner::new();
        inner.read_refs = 1;
        inner.write_refs = 1;
        pipes[id as usize] = Some(inner);
        Ok((PipeId(id), PipeId(id)))
    }

    pub fn dup_read(id: PipeId) {
        with_pipe(id, |p| p.read_refs += 1);
    }

    pub fn dup_write(id: PipeId) {
        with_pipe(id, |p| p.write_refs += 1);
    }

    pub fn close_read(id: PipeId) {
        with_pipe(id, |p| {
            if p.read_refs > 0 {
                p.read_refs -= 1;
            }
            if p.read_refs == 0 {
                p.write_waiters.wake_all();
            }
        });
    }

    pub fn close_write(id: PipeId) {
        with_pipe(id, |p| {
            if p.write_refs > 0 {
                p.write_refs -= 1;
            }
            if p.write_refs == 0 {
                p.read_waiters.wake_all();
            }
        });
    }

    pub fn write(id: PipeId, data: &[u8]) -> Result<usize, PipeError> {
        loop {
            let mut wrote = 0usize;
            let status = with_pipe(id, |p| {
                if p.read_refs == 0 {
                    return Err(PipeError::Epipe);
                }
                wrote = p.write_once(data);
                if wrote > 0 {
                    p.read_waiters.wake_one();
                    return Ok(wrote);
                }
                if p.len >= PIPE_BUF {
                    Err(PipeError::WouldBlock)
                } else {
                    Ok(0)
                }
            });
            match status {
                Some(Ok(n)) => return Ok(n),
                Some(Err(PipeError::Epipe)) => return Err(PipeError::Epipe),
                Some(Err(PipeError::WouldBlock)) | Some(Ok(0)) => {
                    with_pipe(id, |p| {
                        if p.read_refs == 0 {
                            return;
                        }
                        p.write_waiters.block_on();
                    });
                }
                Some(Err(e)) => return Err(e),
                None => return Err(PipeError::BadFd),
            }
        }
    }

    pub fn read(id: PipeId, out: &mut [u8]) -> Result<usize, PipeError> {
        loop {
            let result = with_pipe(id, |p| {
                if p.len > 0 {
                    let n = p.read_once(out);
                    if n > 0 {
                        p.write_waiters.wake_one();
                    }
                    return Ok(n);
                }
                if p.write_refs == 0 {
                    return Ok(0);
                }
                Err(PipeError::WouldBlock)
            });
            match result {
                Some(Ok(n)) => return Ok(n),
                Some(Err(PipeError::WouldBlock)) => {
                    with_pipe(id, |p| {
                        if p.len > 0 {
                            return;
                        }
                        if p.write_refs == 0 {
                            return;
                        }
                        p.read_waiters.block_on();
                    });
                }
                Some(Err(e)) => return Err(e),
                None => return Err(PipeError::BadFd),
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PipeError {
    BadFd,
    Epipe,
    WouldBlock,
}

pub fn pipe_err_to_errno(err: PipeError) -> crate::syscall::errno::Errno {
    use crate::syscall::errno::Errno;
    match err {
        PipeError::BadFd => Errno::EBADF,
        PipeError::Epipe => Errno::EPIPE,
        PipeError::WouldBlock => Errno::EAGAIN,
    }
}
