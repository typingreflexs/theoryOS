use crate::fs::fd::OpenFlags;
use crate::fs::vfs::file::OpenFile;
use crate::fs::vfs::inode::{FileType, InodeMode};
use crate::fs::vfs::mount::MountTable;
use crate::fs::vfs::path::{lookup_parent, revalidate, resolve_path_at, invalidate_dentry, ResolvedInode};
use crate::fs::vfs::superblock::{FileSystem, FsError};
use crate::syscall::errno::Errno;

pub fn fs_err_to_errno(err: FsError) -> Errno {
    match err {
        FsError::NotFound => Errno::ENOENT,
        FsError::Exists => Errno::EEXIST,
        FsError::NotDir => Errno::ENOTDIR,
        FsError::IsDir => Errno::EISDIR,
        FsError::NotFile => Errno::EINVAL,
        FsError::NotSymlink => Errno::EINVAL,
        FsError::Invalid => Errno::EINVAL,
        FsError::Io => Errno::EIO,
        FsError::NoSpace => Errno::ENOSPC,
        FsError::NotSupported => Errno::ENOSYS,
        FsError::TooManyLinks => Errno::ELOOP,
        FsError::AccessDenied => Errno::EACCES,
    }
}

fn fs_for(inode: crate::fs::vfs::inode::InodeId) -> Result<&'static dyn FileSystem, FsError> {
    MountTable::fs(inode.mount).ok_or(FsError::NotFound)
}

pub fn open_path(path: &[u8], flags: OpenFlags) -> Result<OpenFile, FsError> {
    let follow = !flags.contains(OpenFlags::O_NOFOLLOW);
    match resolve_path_at(path, follow) {
        Ok(resolved) => finish_open(resolved, flags),
        Err(FsError::NotFound) if flags.contains(OpenFlags::O_CREAT) => {
            let (parent, name, name_len) = lookup_parent(path)?;
            let fs = fs_for(parent)?;
            let ino = fs.create(parent, &name[..name_len], FileType::Regular, InodeMode::default_file())?;
            invalidate_dentry(parent);
            let attr = fs.getattr(ino)?;
            Ok(OpenFile {
                inode: ino,
                offset: 0,
                flags,
                generation: attr.generation,
            })
        }
        Err(e) => Err(e),
    }
}

fn finish_open(resolved: ResolvedInode, flags: OpenFlags) -> Result<OpenFile, FsError> {
    if flags.contains(OpenFlags::O_TRUNC) {
        revalidate(&resolved)?;
        fs_for(resolved.inode)?.truncate(resolved.inode, 0)?;
    }
    Ok(OpenFile {
        inode: resolved.inode,
        offset: 0,
        flags,
        generation: resolved.attr.generation,
    })
}

pub fn read_file(file: &mut OpenFile, buf: &mut [u8]) -> Result<usize, FsError> {
    let fs = fs_for(file.inode)?;
    let attr = fs.getattr(file.inode)?;
    if attr.generation != file.generation {
        return Err(FsError::Invalid);
    }
    let n = fs.read(file.inode, file.offset, buf)?;
    file.offset += n as u64;
    Ok(n)
}

pub fn write_file(file: &mut OpenFile, buf: &[u8]) -> Result<usize, FsError> {
    let fs = fs_for(file.inode)?;
    let attr = fs.getattr(file.inode)?;
    if attr.generation != file.generation {
        return Err(FsError::Invalid);
    }
    revalidate(&crate::fs::vfs::path::ResolvedInode {
        inode: file.inode,
        attr,
    })?;
    let offset = if file.flags.contains(OpenFlags::O_APPEND) {
        attr.size
    } else {
        file.offset
    };
    let n = fs.write(file.inode, offset, buf)?;
    file.offset = offset + n as u64;
    file.generation = fs.getattr(file.inode)?.generation;
    Ok(n)
}

pub fn lseek_file(file: &mut OpenFile, offset: i64, whence: u32) -> Result<u64, FsError> {
    let fs = fs_for(file.inode)?;
    let size = fs.getattr(file.inode)?.size;
    let new_off = match whence {
        0 => offset as u64,          // SEEK_SET
        1 => (file.offset as i64 + offset) as u64, // SEEK_CUR
        2 => (size as i64 + offset) as u64,          // SEEK_END
        _ => return Err(FsError::Invalid),
    };
    file.offset = new_off;
    Ok(new_off)
}

pub fn stat_path(path: &[u8]) -> Result<crate::fs::vfs::inode::InodeAttr, FsError> {
    Ok(resolve_path_at(path, true)?.attr)
}

const MAX_FILE_READ: usize = 16 * 1024 * 1024;

pub fn read_path(path: &[u8]) -> Result<alloc::vec::Vec<u8>, FsError> {
    let mut file = open_path(path, OpenFlags::O_RDONLY)?;
    let fs = fs_for(file.inode)?;
    let size = fs.getattr(file.inode)?.size as usize;
    let cap = size.min(MAX_FILE_READ);
    let mut buf = alloc::vec::Vec::with_capacity(cap);
    buf.resize(cap, 0);
    let mut total = 0usize;
    loop {
        let chunk = &mut buf[total..];
        if chunk.is_empty() {
            break;
        }
        let n = fs.read(file.inode, file.offset, chunk)?;
        if n == 0 {
            break;
        }
        file.offset += n as u64;
        total += n;
        if total >= cap {
            break;
        }
    }
    buf.truncate(total);
    Ok(buf)
}

pub fn write_path(path: &[u8], data: &[u8], create: bool) -> Result<(), FsError> {
    let flags = if create {
        OpenFlags::O_WRONLY | OpenFlags::O_CREAT | OpenFlags::O_TRUNC
    } else {
        OpenFlags::O_WRONLY | OpenFlags::O_TRUNC
    };
    let mut file = match open_path(path, flags) {
        Ok(f) => f,
        Err(FsError::NotFound) if create => {
            let (parent, name, name_len) = lookup_parent(path)?;
            let fs = fs_for(parent)?;
            let ino = fs.create(parent, &name[..name_len], FileType::Regular, InodeMode::default_file())?;
            invalidate_dentry(parent);
            let attr = fs.getattr(ino)?;
            OpenFile {
                inode: ino,
                offset: 0,
                flags,
                generation: attr.generation,
            }
        }
        Err(e) => return Err(e),
    };
    let fs = fs_for(file.inode)?;
    let mut off = 0u64;
    while off < data.len() as u64 {
        let n = fs.write(file.inode, off, &data[off as usize..])?;
        if n == 0 {
            break;
        }
        off += n as u64;
    }
    Ok(())
}

pub fn mkdir_path(path: &[u8]) -> Result<(), FsError> {
    let (parent, name, name_len) = lookup_parent(path)?;
    let fs = fs_for(parent)?;
    fs.mkdir(parent, &name[..name_len], InodeMode::default_dir())?;
    invalidate_dentry(parent);
    Ok(())
}

pub fn symlink_path(link: &[u8], target: &[u8]) -> Result<(), FsError> {
    let (parent, name, name_len) = lookup_parent(link)?;
    let fs = fs_for(parent)?;
    fs.symlink(parent, &name[..name_len], target, InodeMode::default_file())?;
    invalidate_dentry(parent);
    Ok(())
}
