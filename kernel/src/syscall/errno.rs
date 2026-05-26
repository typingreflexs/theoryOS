//! POSIX errno values returned as negative integers in rax.

pub type SysResult = Result<isize, Errno>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Errno(pub i64);

impl Errno {
    pub const EPERM: Self = Self(1);
    pub const ENOENT: Self = Self(2);
    pub const ESRCH: Self = Self(3);
    pub const EINTR: Self = Self(4);
    pub const EIO: Self = Self(5);
    pub const EBADF: Self = Self(9);
    pub const EAGAIN: Self = Self(11);
    pub const ENOMEM: Self = Self(12);
    pub const EFAULT: Self = Self(14);
    pub const EBUSY: Self = Self(16);
    pub const EEXIST: Self = Self(17);
    pub const ENODEV: Self = Self(19);
    pub const EINVAL: Self = Self(22);
    pub const EMFILE: Self = Self(24);
    pub const EPIPE: Self = Self(32);
    pub const ENOTTY: Self = Self(25);
    pub const ENOSYS: Self = Self(38);
    pub const ENOEXEC: Self = Self(8);
    pub const ECHILD: Self = Self(10);
    pub const ERANGE: Self = Self(34);
    pub const ENOSPC: Self = Self(28);
    pub const EACCES: Self = Self(13);
    pub const ENOTDIR: Self = Self(20);
    pub const EISDIR: Self = Self(21);
    pub const ELOOP: Self = Self(40);
    pub const EAFNOSUPPORT: Self = Self(97);
    pub const ENOTSOCK: Self = Self(88);
    pub const ENOTCONN: Self = Self(107);
    pub const ENETUNREACH: Self = Self(101);
    pub const EADDRINUSE: Self = Self(98);
    pub const ECONNREFUSED: Self = Self(111);
    pub const EOPNOTSUPP: Self = Self(95);

    pub fn as_neg(self) -> isize {
        -(self.0 as isize)
    }
}

pub fn ok(val: isize) -> SysResult {
    Ok(val)
}

pub fn err(e: Errno) -> SysResult {
    Err(e)
}
