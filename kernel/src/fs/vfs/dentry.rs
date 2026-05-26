use super::inode::InodeId;

const DENTRY_SLOTS: usize = 512;

#[derive(Clone, Copy, Debug)]
pub struct Dentry {
    pub parent: InodeId,
    pub name_hash: u64,
    pub name_len: u8,
    pub name: [u8; 64],
    pub inode: InodeId,
    pub generation: u64,
}

impl Dentry {
    pub const fn empty() -> Self {
        Self {
            parent: InodeId::INVALID,
            name_hash: 0,
            name_len: 0,
            name: [0; 64],
            inode: InodeId::INVALID,
            generation: 0,
        }
    }
}

pub struct DentryCache {
    entries: [Option<Dentry>; DENTRY_SLOTS],
}

impl DentryCache {
    pub const fn new() -> Self {
        Self {
            entries: [const { None }; DENTRY_SLOTS],
        }
    }

    pub fn hash(parent: InodeId, name: &[u8]) -> u64 {
        let mut h = parent.mount as u64 ^ parent.ino.wrapping_mul(0x9E37_79B9);
        for &b in name {
            h = h.wrapping_mul(0x100000001b3).wrapping_add(b as u64);
        }
        h
    }

    pub fn get(&self, parent: InodeId, name: &[u8]) -> Option<Dentry> {
        let hash = Self::hash(parent, name);
        for entry in self.entries.iter().flatten() {
            if entry.parent == parent
                && entry.name_hash == hash
                && entry.name_len as usize == name.len()
                && &entry.name[..entry.name_len as usize] == name
            {
                return Some(*entry);
            }
        }
        None
    }

    pub fn insert(&mut self, dentry: Dentry) {
        let hash = dentry.name_hash;
        for slot in self.entries.iter_mut() {
            if slot.is_none() {
                *slot = Some(dentry);
                return;
            }
            if let Some(e) = slot {
                if e.parent == dentry.parent && e.name_hash == hash {
                    *e = dentry;
                    return;
                }
            }
        }
    }

    pub fn invalidate_inode(&mut self, inode: InodeId) {
        for slot in self.entries.iter_mut() {
            if slot.as_ref().map(|e| e.inode == inode).unwrap_or(false) {
                *slot = None;
            }
        }
    }
}
