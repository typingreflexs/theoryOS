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

    /// Locate the `_S5` named-package and return `(SLP_TYPa, SLP_TYPb)`.
    ///
    /// AML pattern (from ACPI spec): `Name(_S5, Package(){ SLP_TYPa, SLP_TYPb, ... })`.
    /// The bytes appear as `5F 53 35 5F` (`_S5_`) followed by a package-op
    /// (`12`), package length bytes, element count, then `0x0A nn` byte
    /// constants (or raw small integers).
    pub fn find_s5(&self) -> Option<(u8, u8)> {
        let aml = self.aml();
        let mut i = 0;
        while i + 5 < aml.len() {
            if &aml[i..i + 4] == b"_S5_" {
                let mut j = i + 4;
                if j < aml.len() && (aml[j] == 0x12 || aml[j] == 0x11) {
                    j += 1;
                    // Skip pkg length (variable-length, 1–4 bytes)
                    let pkg_lead = aml[j];
                    let pkg_len_bytes = ((pkg_lead >> 6) & 0x3) as usize + 1;
                    j += pkg_len_bytes;
                    if j < aml.len() {
                        j += 1; // element count
                        let a = read_byte_const(&aml, &mut j);
                        let b = read_byte_const(&aml, &mut j);
                        if let (Some(a), Some(b)) = (a, b) {
                            return Some((a & 0x07, b & 0x07));
                        }
                        if let Some(a) = a {
                            return Some((a & 0x07, 0));
                        }
                    }
                }
            }
            i += 1;
        }
        None
    }
}

fn read_byte_const(aml: &[u8], cursor: &mut usize) -> Option<u8> {
    if *cursor >= aml.len() {
        return None;
    }
    let b = aml[*cursor];
    match b {
        0x00 => {
            *cursor += 1;
            Some(0)
        }
        0x01 => {
            *cursor += 1;
            Some(1)
        }
        0x0A => {
            *cursor += 1;
            if *cursor < aml.len() {
                let v = aml[*cursor];
                *cursor += 1;
                Some(v)
            } else {
                None
            }
        }
        0x0B => {
            *cursor += 1;
            if *cursor + 1 < aml.len() {
                let v = aml[*cursor];
                *cursor += 2;
                Some(v)
            } else {
                None
            }
        }
        _ => None,
    }
}
