use crate::arch::memory::PhysAddr;

#[derive(Clone, Copy, Debug)]
pub struct MemoryAffinity {
    pub base: PhysAddr,
    pub length: u64,
    pub node: u16,
}

#[derive(Debug)]
pub struct Srat {
    memory: [MemoryAffinity; 64],
    memory_len: usize,
    nodes: u16,
}

impl Srat {
    pub fn parse(table: crate::arch::memory::VirtAddr) -> Self {
        let mut srat = Self {
            memory: [MemoryAffinity {
                base: PhysAddr::new(0),
                length: 0,
                node: 0,
            }; 64],
            memory_len: 0,
            nodes: 1,
        };

        unsafe {
            let header = &*(table.as_ptr::<SratHeader>());
            let mut offset = core::mem::size_of::<SratHeader>();
            while offset + 2 <= header.length as usize {
                let ptr = table.as_ptr::<u8>().add(offset);
                let kind = *ptr;
                let len = *ptr.add(1) as usize;
                if len == 0 {
                    break;
                }
                match kind {
                    1 if len >= 24 => {
                        let base = u64::from_le_bytes([
                            *ptr.add(8),
                            *ptr.add(9),
                            *ptr.add(10),
                            *ptr.add(11),
                            *ptr.add(12),
                            *ptr.add(13),
                            *ptr.add(14),
                            *ptr.add(15),
                        ]);
                        let length = u64::from_le_bytes([
                            *ptr.add(16),
                            *ptr.add(17),
                            *ptr.add(18),
                            *ptr.add(19),
                            *ptr.add(20),
                            *ptr.add(21),
                            *ptr.add(22),
                            *ptr.add(23),
                        ]);
                        let node = u16::from_le_bytes([*ptr.add(6), *ptr.add(7)]) & 0x0FFF;
                        if srat.memory_len < srat.memory.len() {
                            srat.memory[srat.memory_len] = MemoryAffinity {
                                base: PhysAddr::new(base),
                                length,
                                node,
                            };
                            srat.memory_len += 1;
                            srat.nodes = srat.nodes.max(node + 1);
                        }
                    }
                    _ => {}
                }
                offset += len;
            }
        }
        srat
    }

    pub fn memory_affinities(&self) -> &[MemoryAffinity] {
        &self.memory[..self.memory_len]
    }

    pub fn node_count(&self) -> u32 {
        self.nodes as u32
    }
}

#[repr(C, packed)]
struct SratHeader {
    signature: [u8; 4],
    length: u32,
    revision: u8,
    checksum: u8,
    oem_id: [u8; 6],
    oem_table_id: [u8; 8],
    oem_revision: u32,
    creator_id: u32,
    creator_revision: u32,
    table_revision: u32,
    reserved: [u8; 8],
}
