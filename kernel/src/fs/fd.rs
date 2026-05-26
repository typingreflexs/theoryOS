use crate::fs::pipe::PipeId;
use crate::fs::vfs::OpenFile;
use crate::ipc::unix::UnixSocketId;
use crate::net::socket::SocketId;

pub const MAX_FDS: usize = 256;

bitflags::bitflags! {
    #[derive(Clone, Copy, Debug)]
    pub struct OpenFlags: u32 {
        const O_RDONLY = 0;
        const O_WRONLY = 1;
        const O_RDWR = 2;
        const O_CREAT = 0x40;
        const O_TRUNC = 0x200;
        const O_APPEND = 0x400;
        const O_NOFOLLOW = 0x100;
    }
}

#[derive(Clone, Copy, Debug)]
pub enum FdEntry {
    ConsoleIn,
    ConsoleOut,
    ConsoleErr,
    PipeRead(PipeId),
    PipeWrite(PipeId),
    Vfs(OpenFile),
    Socket(SocketId),
    UnixSocket(UnixSocketId),
    Anon,
}

#[derive(Debug)]
pub struct FdTable {
    entries: [Option<FdEntry>; MAX_FDS],
    fd_flags: [u8; MAX_FDS],
    next_fd: u32,
}

impl FdTable {
    pub const fn new() -> Self {
        Self {
            entries: [None; MAX_FDS],
            fd_flags: [0; MAX_FDS],
            next_fd: 3,
        }
    }

    pub fn init_stdio(&mut self) {
        self.entries[0] = Some(FdEntry::ConsoleIn);
        self.entries[1] = Some(FdEntry::ConsoleOut);
        self.entries[2] = Some(FdEntry::ConsoleErr);
    }

    pub fn alloc(&mut self, entry: FdEntry) -> Result<u32, ()> {
        for fd in 0..MAX_FDS {
            if self.entries[fd].is_none() {
                self.entries[fd] = Some(entry);
                return Ok(fd as u32);
            }
        }
        Err(())
    }

    pub fn get(&self, fd: u32) -> Option<FdEntry> {
        self.entries.get(fd as usize).and_then(|e| *e)
    }

    pub fn get_vfs_mut(&mut self, fd: u32) -> Option<&mut OpenFile> {
        match self.entries.get_mut(fd as usize)? {
            Some(FdEntry::Vfs(f)) => Some(f),
            _ => None,
        }
    }

    pub fn dup(&self, fd: u32) -> Result<FdEntry, ()> {
        self.get(fd).ok_or(())
    }

    pub fn close(&mut self, fd: u32) -> Result<(), ()> {
        if fd < 3 {
            return Err(());
        }
        if self.entries.get_mut(fd as usize).and_then(|e| e.take()).is_some() {
            Ok(())
        } else {
            Err(())
        }
    }

    pub fn dup2(&mut self, oldfd: u32, newfd: u32) -> Result<u32, ()> {
        let entry = self.dup(oldfd)?;
        if newfd as usize >= MAX_FDS {
            return Err(());
        }
        self.entries[newfd as usize] = Some(entry);
        self.fd_flags[newfd as usize] = self.fd_flags[oldfd as usize];
        Ok(newfd)
    }

    /// Close all FDs marked close-on-exec (called from execve).
    pub fn close_on_exec(&mut self) {
        for fd in 3..MAX_FDS {
            if self.fd_flags[fd] & FD_CLOEXEC != 0 {
                self.entries[fd] = None;
                self.fd_flags[fd] = 0;
            }
        }
    }

    pub fn get_fd_flags(&self, fd: u32) -> u8 {
        self.fd_flags.get(fd as usize).copied().unwrap_or(0)
    }

    pub fn set_fd_flags(&mut self, fd: u32, flags: u8) -> Result<(), ()> {
        if fd as usize >= MAX_FDS || self.entries[fd as usize].is_none() {
            return Err(());
        }
        self.fd_flags[fd as usize] = flags;
        Ok(())
    }

    pub fn open_flags(&self, fd: u32) -> OpenFlags {
        match self.entries.get(fd as usize).and_then(|e| *e) {
            Some(FdEntry::ConsoleIn) => OpenFlags::O_RDONLY,
            Some(FdEntry::ConsoleOut) | Some(FdEntry::ConsoleErr) => OpenFlags::O_WRONLY,
            _ => OpenFlags::O_RDWR,
        }
    }
}

pub const FD_CLOEXEC: u8 = 1;
