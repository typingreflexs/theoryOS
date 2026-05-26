//! Pathname resolution with symlink following and TOCTOU mitigation.
//!
//! User paths are copied into kernel memory once at syscall entry. Each resolved
//! component records the inode generation; mutating operations revalidate before use.

use spin::Mutex;

use super::dentry::{Dentry, DentryCache};
use super::inode::{FileType, InodeAttr, InodeId};
use super::mount::MountTable;
use super::superblock::FsError;

const MAX_SYMLINK_DEPTH: usize = 8;
const MAX_PATH: usize = 4096;

static DENTRY_CACHE: Mutex<DentryCache> = Mutex::new(DentryCache::new());

#[derive(Clone, Copy, Debug)]
pub struct PathResolution {
    pub path: [u8; MAX_PATH],
    pub path_len: usize,
}

#[derive(Clone, Copy, Debug)]
pub struct ResolvedInode {
    pub inode: InodeId,
    pub attr: InodeAttr,
}

impl PathResolution {
    /// Copy user path into kernel buffer — single snapshot for TOCTOU protection.
    pub fn from_kernel_path(path: &[u8]) -> Result<Self, FsError> {
        if path.is_empty() || path.len() >= MAX_PATH {
            return Err(FsError::Invalid);
        }
        let mut buf = [0u8; MAX_PATH];
        buf[..path.len()].copy_from_slice(path);
        Ok(Self {
            path: buf,
            path_len: path.len(),
        })
    }

    pub fn as_path(&self) -> &[u8] {
        &self.path[..self.path_len]
    }
}

pub fn resolve_path(path: &[u8]) -> Result<ResolvedInode, FsError> {
    let pr = PathResolution::from_kernel_path(path)?;
    resolve_at(&pr, true)
}

pub fn resolve_path_at(path: &[u8], follow_final: bool) -> Result<ResolvedInode, FsError> {
    let pr = PathResolution::from_kernel_path(path)?;
    resolve_at(&pr, follow_final)
}

fn resolve_at(pr: &PathResolution, follow_final: bool) -> Result<ResolvedInode, FsError> {
    let path = pr.as_path();
    let (mount_id, skip) = MountTable::find_mount_for_path(path);
    let fs = MountTable::fs(mount_id).ok_or(FsError::NotFound)?;
    let mut current = InodeId::new(mount_id, fs.root().ino);
    let mut attr = fs.getattr(current)?;

    let rel = if skip < path.len() {
        &path[skip..]
    } else {
        b""
    };

    if rel.is_empty() || rel == b"/" {
        return Ok(ResolvedInode { inode: current, attr });
    }

    let mut components = rel.split(|&b| b == b'/').filter(|c| !c.is_empty());
    let mut symlink_depth = 0;

    while let Some(name) = components.next() {
        let is_last = components.clone().next().is_none();

        if attr.file_type == FileType::Symlink && (follow_final || !is_last) {
            if symlink_depth >= MAX_SYMLINK_DEPTH {
                return Err(FsError::TooManyLinks);
            }
            symlink_depth += 1;
            let mut target = [0u8; MAX_PATH];
            let len = fs.readlink(current, &mut target)?;
            let mut link_path = [0u8; MAX_PATH];
            if target[0] == b'/' {
                link_path[..len].copy_from_slice(&target[..len]);
            } else {
                // relative symlink — not fully implemented; treat as absolute under mount
                link_path[..len].copy_from_slice(&target[..len]);
            }
            let nested = PathResolution {
                path: link_path,
                path_len: len,
            };
            return resolve_at(&nested, follow_final);
        }

        if attr.file_type != FileType::Directory {
            return Err(FsError::NotDir);
        }

        // Dentry cache lookup
        let mut cache = DENTRY_CACHE.lock();
        if let Some(dentry) = cache.get(current, name) {
            drop(cache);
            current = dentry.inode;
            attr = fs.getattr(current)?;
            if attr.generation != dentry.generation {
                DENTRY_CACHE.lock().invalidate_inode(current);
                current = fs.lookup(current, name)?;
                attr = fs.getattr(current)?;
            }
        } else {
            drop(cache);
            let parent_ino = current;
            current = fs.lookup(current, name)?;
            attr = fs.getattr(current)?;
            DENTRY_CACHE.lock().insert(Dentry {
                parent: parent_ino,
                name_hash: DentryCache::hash(parent_ino, name),
                name_len: name.len().min(64) as u8,
                name: {
                    let mut n = [0u8; 64];
                    n[..name.len().min(64)].copy_from_slice(&name[..name.len().min(64)]);
                    n
                },
                inode: current,
                generation: attr.generation,
            });
        }
    }

    if !follow_final && attr.file_type == FileType::Symlink {
        return Ok(ResolvedInode { inode: current, attr });
    }

    Ok(ResolvedInode { inode: current, attr })
}

/// Revalidate inode generation before a mutating operation (TOCTOU check).
pub fn revalidate(resolved: &ResolvedInode) -> Result<(), FsError> {
    let fs = MountTable::fs(resolved.inode.mount).ok_or(FsError::NotFound)?;
    let attr = fs.getattr(resolved.inode)?;
    if attr.generation != resolved.attr.generation {
        return Err(FsError::Invalid);
    }
    Ok(())
}

pub fn invalidate_dentry(inode: InodeId) {
    DENTRY_CACHE.lock().invalidate_inode(inode);
}

pub fn lookup_parent(path: &[u8]) -> Result<(InodeId, [u8; 256], usize), FsError> {
    if path.is_empty() {
        return Err(FsError::Invalid);
    }
    let mut name = [0u8; 256];
    let mut parent_path = [0u8; MAX_PATH];
    parent_path[..path.len()].copy_from_slice(path);
    let path_len = path.len();

    let slash = (0..path_len).rev().find(|&i| path[i] == b'/');
    let (parent_len, name_len) = match slash {
        Some(0) => {
            if path_len == 1 {
                return Err(FsError::Invalid);
            }
            (1, path_len - 1)
        }
        Some(i) => (i, path_len - i - 1),
        None => (0, path_len),
    };

    if name_len == 0 || name_len > 255 {
        return Err(FsError::Invalid);
    }
    name[..name_len].copy_from_slice(&path[path_len - name_len..]);
    let parent = if parent_len <= 1 {
        resolve_path(b"/")?.inode
    } else {
        resolve_path(&parent_path[..parent_len])?.inode
    };
    Ok((parent, name, name_len))
}
