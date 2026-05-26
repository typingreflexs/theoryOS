use crate::arch::memory::VirtAddr;

#[repr(C, packed)]
struct DsdtHeader {
    signature: [u8; 4],
    length: u32,
    revision: u8,
    checksum: u8,
    oem_id: [u8; 6],
    oem_table_id: [u8; 8],
    oem_revision: u32,
    creator_id: u32,
    creator_revision: u32,
}

#[derive(Debug)]
pub struct Dsdt {
    table: VirtAddr,
    length: u32,
}

impl Dsdt {
    pub fn parse(hhdm: u64, phys: crate::arch::memory::PhysAddr) -> Self {
        let table = crate::arch::memory::phys_to_virt(hhdm, phys);
        let length = unsafe { (*(table.as_ptr::<DsdtHeader>())).length };
        Self { table, length }
    }

    pub fn aml(&self) -> &[u8] {
        let header_size = core::mem::size_of::<DsdtHeader>();
        let total = self.length as usize;
        if total <= header_size {
            return &[];
        }
        unsafe { core::slice::from_raw_parts(self.table.as_ptr::<u8>().add(header_size), total - header_size) }
    }

    pub fn find_named_opregion(&self, name: &[u8; 4]) -> Option<u32> {
        // Minimal AML scanner for \_SB_.PCI0.LNXVIDEO-style opregion discovery.
        let aml = self.aml();
        let mut i = 0usize;
        while i + 4 <= aml.len() {
            if &aml[i..i + 4] == name {
                return Some(i as u32);
            }
            i += 1;
        }
        None
    }

    pub fn signature(&self) -> &[u8; 4] {
        unsafe { &(*(self.table.as_ptr::<DsdtHeader>())).signature }
    }
}
