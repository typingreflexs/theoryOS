use crate::mm::layout::{align_up, PAGE_SIZE};
use crate::mm::permissions::{MmapFlags, ProtFlags};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VmaKind {
    Anonymous,
    File,
    Stack,
    Heap,
    Mmap,
    Framebuffer,
}

#[derive(Clone, Copy, Debug)]
pub struct Vma {
    pub start: u64,
    pub end: u64,
    pub prot: ProtFlags,
    pub flags: MmapFlags,
    pub kind: VmaKind,
    pub file_offset: u64,
    pub shared: bool,
    pub populated: bool,
}

impl Vma {
    pub fn new(start: u64, length: u64, prot: ProtFlags, flags: MmapFlags, kind: VmaKind) -> Self {
        Self {
            start,
            end: start + length,
            prot,
            flags,
            kind,
            shared: flags.contains(MmapFlags::SHARED),
            file_offset: 0,
            populated: flags.contains(MmapFlags::POPULATE),
        }
    }

    pub fn length(&self) -> u64 {
        self.end - self.start
    }

    pub fn contains(&self, addr: u64) -> bool {
        addr >= self.start && addr < self.end
    }

    pub fn page_count(&self) -> u64 {
        align_up(self.length(), PAGE_SIZE) / PAGE_SIZE
    }
}

const MAX_VMAS: usize = 256;

#[derive(Debug)]
pub struct VmaTree {
    entries: [Option<Vma>; MAX_VMAS],
    len: usize,
}

impl VmaTree {
    pub const fn new() -> Self {
        Self {
            entries: [None; MAX_VMAS],
            len: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn insert(&mut self, vma: Vma) -> Result<(), VmaError> {
        if self.len >= MAX_VMAS || overlaps(self, vma.start, vma.end) {
            return Err(VmaError::NoSpace);
        }
        let idx = self.len;
        self.entries[idx] = Some(vma);
        self.len += 1;
        self.sort_entries();
        Ok(())
    }

    fn sort_entries(&mut self) {
        for i in 1..self.len {
            let mut j = i;
            while j > 0 {
                let left = self.entries[j - 1].unwrap().start;
                let right = self.entries[j].unwrap().start;
                if left <= right {
                    break;
                }
                self.entries.swap(j - 1, j);
                j -= 1;
            }
        }
    }

    pub fn remove(&mut self, start: u64, length: u64) -> Result<Vma, VmaError> {
        let end = start + length;
        for i in 0..self.len {
            if let Some(vma) = self.entries[i] {
                if vma.start == start && vma.end == end {
                    let removed = vma;
                    self.entries[i] = None;
                    self.compact();
                    return Ok(removed);
                }
            }
        }
        Err(VmaError::NotFound)
    }

    pub fn find(&self, addr: u64) -> Option<Vma> {
        for vma in self.entries.iter().take(self.len).flatten() {
            if vma.contains(addr) {
                return Some(*vma);
            }
        }
        None
    }

    pub fn find_gap(&self, hint: u64, length: u64, limit: u64) -> Option<u64> {
        let mut cursor = hint;
        while cursor + length <= limit {
            if !overlaps(self, cursor, cursor + length) {
                return Some(cursor);
            }
            cursor = align_up(cursor + 1, PAGE_SIZE);
        }
        None
    }

    pub fn update_prot(&mut self, start: u64, length: u64, prot: ProtFlags) -> Result<(), VmaError> {
        let end = start + length;
        for entry in self.entries.iter_mut().take(self.len) {
            if let Some(vma) = entry {
                if vma.start == start && vma.end == end {
                    vma.prot = prot;
                    return Ok(());
                }
            }
        }
        Err(VmaError::NotFound)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Vma> {
        self.entries.iter().take(self.len).flatten()
    }

    fn compact(&mut self) {
        let mut out = 0;
        for i in 0..self.len {
            if self.entries[i].is_some() {
                if out != i {
                    self.entries[out] = self.entries[i];
                }
                out += 1;
            }
        }
        for slot in &mut self.entries[out..self.len] {
            *slot = None;
        }
        self.len = out;
    }
}

fn overlaps(tree: &VmaTree, start: u64, end: u64) -> bool {
    tree.iter().any(|vma| start < vma.end && end > vma.start)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VmaError {
    NoSpace,
    NotFound,
    Overlap,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vma_insert_and_find() {
        let mut tree = VmaTree::new();
        let vma = Vma::new(
            0x1000,
            0x2000,
            ProtFlags::READ | ProtFlags::WRITE,
            MmapFlags::PRIVATE,
            VmaKind::Anonymous,
        );
        tree.insert(vma).unwrap();
        assert!(tree.find(0x1500).is_some());
        assert!(tree.find(0x4000).is_none());
    }

    #[test]
    fn vma_find_gap() {
        let mut tree = VmaTree::new();
        let _ = tree.insert(Vma::new(
            0x1000,
            0x1000,
            ProtFlags::READ,
            MmapFlags::PRIVATE,
            VmaKind::Anonymous,
        ));
        let gap = tree.find_gap(0x1000, 0x1000, 0x10000).unwrap();
        assert!(gap >= 0x2000);
    }
}
