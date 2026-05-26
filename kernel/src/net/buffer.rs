//! Mutable packet buffer for L2–L4 processing.

pub const MAX_PACKET: usize = 2048;

#[derive(Clone)]
pub struct PacketBuf {
    data: [u8; MAX_PACKET],
    len: usize,
}

impl PacketBuf {
    pub const fn new() -> Self {
        Self {
            data: [0; MAX_PACKET],
            len: 0,
        }
    }

    pub fn clear(&mut self) {
        self.len = 0;
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn data(&self) -> &[u8] {
        &self.data[..self.len]
    }

    pub fn data_mut(&mut self) -> &mut [u8] {
        &mut self.data[..self.len]
    }

    pub fn reserve(&mut self, size: usize) -> bool {
        if size > MAX_PACKET {
            return false;
        }
        self.len = size;
        true
    }

    pub fn extend_from_slice(&mut self, slice: &[u8]) -> bool {
        if self.len + slice.len() > MAX_PACKET {
            return false;
        }
        self.data[self.len..self.len + slice.len()].copy_from_slice(slice);
        self.len += slice.len();
        true
    }

    pub fn push(&mut self, byte: u8) -> bool {
        if self.len >= MAX_PACKET {
            return false;
        }
        self.data[self.len] = byte;
        self.len += 1;
        true
    }

    pub fn trim_front(&mut self, n: usize) {
        if n >= self.len {
            self.len = 0;
            return;
        }
        self.data.copy_within(n..self.len, 0);
        self.len -= n;
    }
}
