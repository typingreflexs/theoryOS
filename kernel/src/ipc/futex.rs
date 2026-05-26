//! Futex — userspace synchronization primitive.

use spin::Mutex;

use super::wait::WaitQueue;

const MAX_FUTEX: usize = 128;

struct FutexEntry {
    addr: u64,
    waiters: WaitQueue,
}

static FUTEXES: Mutex<[Option<FutexEntry>; MAX_FUTEX]> = Mutex::new([const { None }; MAX_FUTEX]);

pub const FUTEX_WAIT: i32 = 0;
pub const FUTEX_WAKE: i32 = 1;
pub const FUTEX_PRIVATE_FLAG: i32 = 128;

fn find_or_create(addr: u64) -> usize {
    let mut futexes = FUTEXES.lock();
    for (i, slot) in futexes.iter().enumerate() {
        if let Some(e) = slot {
            if e.addr == addr {
                return i;
            }
        }
    }
    for (i, slot) in futexes.iter_mut().enumerate() {
        if slot.is_none() {
            *slot = Some(FutexEntry {
                addr,
                waiters: WaitQueue::new(),
            });
            return i;
        }
    }
    0
}

pub fn futex(
    uaddr: u64,
    op: i32,
    val: u32,
    _timeout: u64,
    _uaddr2: u64,
    _val3: u32,
) -> Result<i32, FutexError> {
    let op = op & !FUTEX_PRIVATE_FLAG;
    match op {
        FUTEX_WAIT => futex_wait(uaddr, val),
        FUTEX_WAKE => futex_wake(uaddr, val),
        _ => Err(FutexError::Inval),
    }
}

fn futex_wait(uaddr: u64, expected: u32) -> Result<i32, FutexError> {
    let current = read_user_u32(uaddr)?;
    if current != expected {
        return Err(FutexError::Again);
    }
    let idx = find_or_create(uaddr);
    loop {
        let current = read_user_u32(uaddr)?;
        if current != expected {
            return Err(FutexError::Again);
        }
        FUTEXES.lock()[idx].as_mut().unwrap().waiters.block_on();
        let current = read_user_u32(uaddr)?;
        if current != expected {
            return Ok(0);
        }
    }
}

fn futex_wake(uaddr: u64, count: u32) -> Result<i32, FutexError> {
    let mut futexes = FUTEXES.lock();
    for slot in futexes.iter_mut() {
        if let Some(e) = slot {
            if e.addr == uaddr {
                let mut woken = 0i32;
                for _ in 0..count {
                    if let Some(tid) = e.waiters.pop() {
                        crate::sched::wake_thread(tid);
                        woken += 1;
                    } else {
                        break;
                    }
                }
                return Ok(woken);
            }
        }
    }
    Ok(0)
}

fn read_user_u32(addr: u64) -> Result<u32, FutexError> {
    crate::syscall::uaccess::copy_from_user_obj(addr).map_err(|_| FutexError::Fault)
}

/// Wake waiters at `uaddr` and zero the word — used by `set_tid_address` thread exit path.
pub fn wake_user(uaddr: u64, count: u32) -> Result<i32, FutexError> {
    let _ = write_user_u32(uaddr, 0);
    futex_wake(uaddr, count)
}

fn write_user_u32(addr: u64, val: u32) -> Result<(), FutexError> {
    crate::syscall::uaccess::copy_to_user_obj(addr, &val).map_err(|_| FutexError::Fault)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FutexError {
    Inval,
    Again,
    Fault,
}

pub fn futex_err_to_errno(err: FutexError) -> crate::syscall::errno::Errno {
    use crate::syscall::errno::Errno;
    match err {
        FutexError::Inval => Errno::EINVAL,
        FutexError::Again => Errno::EAGAIN,
        FutexError::Fault => Errno::EFAULT,
    }
}
