//! Pure ELF64 parsing helpers — no kernel side effects, host-testable.

pub const ELF_MAGIC: [u8; 4] = [0x7f, b'E', b'L', b'F'];
pub const ET_EXEC: u16 = 2;
pub const ET_DYN: u16 = 3;
pub const EM_X86_64: u16 = 62;
pub const PT_LOAD: u32 = 1;
pub const PT_INTERP: u32 = 3;
pub const PT_DYNAMIC: u32 = 2;
pub const PT_PHDR: u32 = 6;
pub const PF_X: u32 = 1;
pub const PF_W: u32 = 2;

pub const DT_NULL: u64 = 0;
pub const DT_RELA: u64 = 7;
pub const DT_RELASZ: u64 = 8;
pub const DT_RELAENT: u64 = 9;
pub const DT_JMPREL: u64 = 23;
pub const DT_PLTRELSZ: u64 = 2;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Elf64Ehdr {
    pub e_ident: [u8; 16],
    pub e_type: u16,
    pub e_machine: u16,
    pub e_version: u32,
    pub e_entry: u64,
    pub e_phoff: u64,
    pub e_shoff: u64,
    pub e_flags: u32,
    pub e_ehsize: u16,
    pub e_phentsize: u16,
    pub e_phnum: u16,
    pub e_shentsize: u16,
    pub e_shnum: u16,
    pub e_shstrndx: u16,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Elf64Phdr {
    pub p_type: u32,
    pub p_flags: u32,
    pub p_offset: u64,
    pub p_vaddr: u64,
    pub p_paddr: u64,
    pub p_filesz: u64,
    pub p_memsz: u64,
    pub p_align: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Elf64Rela {
    pub r_offset: u64,
    pub r_info: u64,
    pub r_addend: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParseError {
    Invalid,
    NotElf,
    BadMachine,
    BadType,
}

pub fn parse_ehdr(data: &[u8]) -> Result<Elf64Ehdr, ParseError> {
    if data.len() < core::mem::size_of::<Elf64Ehdr>() {
        return Err(ParseError::Invalid);
    }
    // SAFETY: bounds checked above; ELF header is a packed POD read from byte slice.
    let hdr = unsafe { (data.as_ptr() as *const Elf64Ehdr).read_unaligned() };
    Ok(hdr)
}

pub fn validate_hdr(hdr: &Elf64Ehdr) -> Result<(), ParseError> {
    if hdr.e_ident[..4] != ELF_MAGIC || hdr.e_ident[4] != 2 {
        return Err(ParseError::NotElf);
    }
    // ELFCLASS64 + ELFDATA2LSB required; big-endian or ELF32 rejected per our x86_64 port.
    if hdr.e_ident[5] != 1 {
        return Err(ParseError::NotElf);
    }
    if hdr.e_machine != EM_X86_64 {
        return Err(ParseError::BadMachine);
    }
    if hdr.e_type != ET_EXEC && hdr.e_type != ET_DYN {
        return Err(ParseError::BadType);
    }
    Ok(())
}

pub fn phdr_at(data: &[u8], hdr: &Elf64Ehdr, index: u16) -> Result<Elf64Phdr, ParseError> {
    let off = hdr.e_phoff as usize + index as usize * hdr.e_phentsize as usize;
    if off + core::mem::size_of::<Elf64Phdr>() > data.len() {
        return Err(ParseError::Invalid);
    }
    // SAFETY: offset validated to fit within `data`.
    Ok(unsafe { (data.as_ptr().add(off) as *const Elf64Phdr).read_unaligned() })
}

/// Linux AT_PHDR: virtual address of the program header table in the loaded image.
///
/// Spec note: `e_phoff` is a *file* offset. If no PT_LOAD covers that region, callers must
/// copy PHDRs into a mapped page and return that address instead.
pub fn phdr_vaddr_in_image(data: &[u8], hdr: &Elf64Ehdr, load_bias: u64) -> Result<u64, ParseError> {
    // PT_PHDR segment explicitly describes where PHDRs live (preferred on PIE).
    for i in 0..hdr.e_phnum {
        let ph = phdr_at(data, hdr, i)?;
        if ph.p_type == PT_PHDR {
            return Ok(ph.p_vaddr + load_bias);
        }
    }
    // Fall back: PHDR file range covered by some PT_LOAD segment.
    let phoff = hdr.e_phoff;
    let phsize = hdr.e_phnum as u64 * hdr.e_phentsize as u64;
    for i in 0..hdr.e_phnum {
        let ph = phdr_at(data, hdr, i)?;
        if ph.p_type != PT_LOAD {
            continue;
        }
        let seg_end = ph.p_offset + ph.p_filesz;
        if ph.p_offset <= phoff && seg_end >= phoff + phsize {
            return Ok(ph.p_vaddr + load_bias + (phoff - ph.p_offset));
        }
    }
    Err(ParseError::Invalid)
}

/// Minimum PT_LOAD virtual address — used to pick load bias for ET_DYN PIE binaries.
pub fn min_load_vaddr(data: &[u8], hdr: &Elf64Ehdr) -> Result<u64, ParseError> {
    let mut min = u64::MAX;
    for i in 0..hdr.e_phnum {
        let ph = phdr_at(data, hdr, i)?;
        if ph.p_type == PT_LOAD {
            min = min.min(ph.p_vaddr);
        }
    }
    if min == u64::MAX {
        Err(ParseError::Invalid)
    } else {
        Ok(min)
    }
}

pub fn extract_interp_path<'a>(data: &'a [u8], hdr: &Elf64Ehdr) -> Result<Option<&'a [u8]>, ParseError> {
    for i in 0..hdr.e_phnum {
        let ph = phdr_at(data, hdr, i)?;
        if ph.p_type != PT_INTERP {
            continue;
        }
        let start = ph.p_offset as usize;
        let end = start + ph.p_filesz as usize;
        if end > data.len() {
            return Err(ParseError::Invalid);
        }
        let bytes = &data[start..end];
        let nul = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
        return Ok(Some(&bytes[..nul]));
    }
    Ok(None)
}

pub struct DynamicInfo {
    pub rela: u64,
    pub relasz: u64,
    pub relaent: u64,
    pub jmprel: u64,
    pub pltrelsz: u64,
}

pub fn parse_dynamic(data: &[u8], hdr: &Elf64Ehdr, load_bias: u64) -> Result<Option<DynamicInfo>, ParseError> {
    for i in 0..hdr.e_phnum {
        let ph = phdr_at(data, hdr, i)?;
        if ph.p_type != PT_DYNAMIC {
            continue;
        }
        let mut info = DynamicInfo {
            rela: 0,
            relasz: 0,
            relaent: core::mem::size_of::<Elf64Rela>() as u64,
            jmprel: 0,
            pltrelsz: 0,
        };
        let start = ph.p_offset as usize;
        let end = start + ph.p_filesz as usize;
        if end > data.len() {
            return Err(ParseError::Invalid);
        }
        let mut off = start;
        while off + 16 <= end {
            // SAFETY: loop guard ensures 16 bytes available.
            let tag = unsafe { (data.as_ptr().add(off) as *const u64).read_unaligned() };
            let val = unsafe { (data.as_ptr().add(off + 8) as *const u64).read_unaligned() };
            off += 16;
            match tag {
                DT_NULL => break,
                DT_RELA => info.rela = val + load_bias,
                DT_RELASZ => info.relasz = val,
                DT_RELAENT => info.relaent = val,
                DT_JMPREL => info.jmprel = val + load_bias,
                DT_PLTRELSZ => info.pltrelsz = val,
                _ => {}
            }
        }
        return Ok(Some(info));
    }
    Ok(None)
}

/// Parse `#!` line: returns interpreter path and optional argument bytes after whitespace.
pub fn parse_shebang(data: &[u8]) -> Option<(&[u8], &[u8])> {
    if !data.starts_with(b"#!") {
        return None;
    }
    let line_end = data.iter().position(|&b| b == b'\n').unwrap_or(data.len());
    let line = &data[2..line_end];
    let start = line.iter().position(|&b| !matches!(b, b' ' | b'\t')).unwrap_or(line.len());
    if start >= line.len() {
        return None;
    }
    let rest = &line[start..];
    let path_end = rest
        .iter()
        .position(|&b| matches!(b, b' ' | b'\t'))
        .unwrap_or(rest.len());
    Some((&rest[..path_end], &rest[path_end..]))
}

pub fn is_elf(data: &[u8]) -> bool {
    data.len() >= 4 && data[..4] == ELF_MAGIC
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_elf64() -> Vec<u8> {
        let mut data = vec![0u8; 256];
        data[0..4].copy_from_slice(&ELF_MAGIC);
        data[4] = 2; // ELFCLASS64
        data[5] = 1; // ELFDATA2LSB
        data[6] = 1; // EV_CURRENT
        data[0x10..0x12].copy_from_slice(&ET_EXEC.to_le_bytes());
        data[0x12..0x14].copy_from_slice(&EM_X86_64.to_le_bytes());
        data[0x18..0x20].copy_from_slice(&0x400000u64.to_le_bytes()); // e_entry
        data[0x20..0x28].copy_from_slice(&64u64.to_le_bytes()); // e_phoff
        data[0x34..0x36].copy_from_slice(&64u16.to_le_bytes()); // e_ehsize
        data[0x36..0x38].copy_from_slice(&56u16.to_le_bytes()); // e_phentsize
        data[0x38..0x3a].copy_from_slice(&1u16.to_le_bytes()); // e_phnum
        // PHDR at offset 64
        // PHDR at file offset 64 (0x40)
        data[0x40..0x44].copy_from_slice(&PT_LOAD.to_le_bytes());
        data[0x44..0x48].copy_from_slice(&(PF_X | PF_R()).to_le_bytes());
        data[0x48..0x50].copy_from_slice(&64u64.to_le_bytes()); // p_offset
        data[0x50..0x58].copy_from_slice(&0x400000u64.to_le_bytes()); // p_vaddr
        data[0x60..0x68].copy_from_slice(&0x1000u64.to_le_bytes()); // p_filesz
        data[0x68..0x70].copy_from_slice(&0x1000u64.to_le_bytes()); // p_memsz
        data
    }

    const fn PF_R() -> u32 {
        4
    }

    #[test]
    fn parse_valid_exec_header() {
        let data = minimal_elf64();
        let hdr = parse_ehdr(&data).unwrap();
        validate_hdr(&hdr).unwrap();
        assert_eq!(hdr.e_type, ET_EXEC);
        assert_eq!(hdr.e_machine, EM_X86_64);
    }

    #[test]
    fn phdr_vaddr_from_load_segment() {
        let data = minimal_elf64();
        let hdr = parse_ehdr(&data).unwrap();
        let va = phdr_vaddr_in_image(&data, &hdr, 0).unwrap();
        // PHDR at file offset 64 maps to vaddr 0x400000 + (64 - p_offset)
        assert_eq!(va, 0x400000);
    }

    #[test]
    fn shebang_parses_path_and_args() {
        let line = b"#!/bin/sh -x\n";
        let (path, args) = parse_shebang(line).unwrap();
        assert_eq!(path, b"/bin/sh");
        assert_eq!(args, b" -x");
    }

    #[test]
    fn reject_non_elf() {
        assert!(parse_ehdr(b"not elf").is_err());
    }
}
