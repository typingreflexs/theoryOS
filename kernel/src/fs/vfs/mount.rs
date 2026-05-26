use spin::Mutex;

use super::superblock::FileSystem;
use super::inode::InodeId;

pub const MAX_MOUNTS: usize = 16;

pub struct MountPoint {
    pub id: u32,
    pub path: [u8; 256],
    pub path_len: usize,
    pub fs: &'static dyn FileSystem,
    pub root: InodeId,
}

pub struct MountTable {
    mounts: [Option<MountPoint>; MAX_MOUNTS],
    root_mount: u32,
    count: u32,
}

static MOUNTS: Mutex<MountTable> = Mutex::new(MountTable {
    mounts: [const { None }; MAX_MOUNTS],
    root_mount: 0,
    count: 0,
});

impl MountTable {
    pub fn register(path: &str, fs: &'static dyn FileSystem) -> Result<u32, ()> {
        let mut table = MOUNTS.lock();
        let id = table.count as u32;
        if id as usize >= MAX_MOUNTS {
            return Err(());
        }
        let root = fs.root();
        let mut mp = MountPoint {
            id,
            path: [0; 256],
            path_len: 0,
            fs,
            root: InodeId::new(id, root.ino),
        };
        let bytes = path.as_bytes();
        let len = bytes.len().min(255);
        mp.path[..len].copy_from_slice(&bytes[..len]);
        mp.path_len = len;
        table.mounts[id as usize] = Some(mp);
        table.count += 1;
        if id == 0 {
            table.root_mount = 0;
        }
        Ok(id)
    }

    pub fn with_mount<F, R>(mount_id: u32, f: F) -> Option<R>
    where
        F: FnOnce(&MountPoint) -> R,
    {
        MOUNTS.lock().mounts.get(mount_id as usize)?.as_ref().map(f)
    }

    pub fn root_mount() -> u32 {
        MOUNTS.lock().root_mount
    }

    pub fn find_mount_for_path(path: &[u8]) -> (u32, usize) {
        let table = MOUNTS.lock();
        let mut best = (0u32, 0usize);
        for mp in table.mounts.iter().flatten() {
            if mp.path_len == 0 {
                if path.is_empty() || path[0] == b'/' {
                    best = (mp.id, 0);
                }
                continue;
            }
            if path.len() >= mp.path_len && path.starts_with(&mp.path[..mp.path_len]) {
                let next = if path.len() > mp.path_len {
                    path.get(mp.path_len)
                } else {
                    None
                };
                if next == Some(&b'/') || next.is_none() {
                    if mp.path_len >= best.1 {
                        best = (mp.id, mp.path_len);
                    }
                }
            }
        }
        best
    }

    pub fn fs(mount_id: u32) -> Option<&'static dyn FileSystem> {
        MOUNTS.lock().mounts.get(mount_id as usize)?.as_ref().map(|m| m.fs)
    }
}

pub use MountTable as Mount;
