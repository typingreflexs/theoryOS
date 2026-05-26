//! x86_64 ELF RELA relocation application — pure logic where possible.

use super::parse::Elf64Rela;

/// x86_64 relocation types from ELF spec / psABI.
pub const R_X86_64_NONE: u32 = 0;
pub const R_X86_64_64: u32 = 1;
pub const R_X86_64_GLOB_DAT: u32 = 6;
pub const R_X86_64_JUMP_SLOT: u32 = 7;
pub const R_X86_64_RELATIVE: u32 = 8;
pub const R_X86_64_IRELATIVE: u32 = 37;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RelocError {
    UnknownType(u32),
    BadOffset,
}

/// Apply one RELA entry to a 64-bit in-memory word.
///
/// `resolve` maps a virtual address to the current 64-bit value stored there.
/// `write` stores a new 64-bit value. For unit tests these are in-memory closures;
/// the loader wires them to user page tables.
///
/// Spec deviation: R_X86_64_IRELATIVE invokes resolver at `load_base + addend` after
/// RELATIVE fixup — we follow glibc/musl kernel behavior and call the resolver stub
/// only when `allow_irelative` is true (dynamic linker images).
pub fn apply_rela_entry(
    rela: &Elf64Rela,
    load_base: u64,
    resolve: impl Fn(u64) -> Option<u64>,
    mut write: impl FnMut(u64, u64) -> Result<(), RelocError>,
    allow_irelative: bool,
) -> Result<(), RelocError> {
    let r_type = (rela.r_info & 0xffff_ffff) as u32;
    let offset = rela.r_offset;
    match r_type {
        R_X86_64_NONE => Ok(()),
        R_X86_64_RELATIVE => {
            let value = (load_base as i64).wrapping_add(rela.r_addend) as u64;
            write(offset, value)
        }
        R_X86_64_64 | R_X86_64_GLOB_DAT | R_X86_64_JUMP_SLOT => {
            let sym_value = resolve(rela.r_info >> 32).unwrap_or(0);
            let value = (sym_value as i64).wrapping_add(rela.r_addend) as u64;
            write(offset, value)
        }
        R_X86_64_IRELATIVE if allow_irelative => {
            let resolver = (load_base as i64).wrapping_add(rela.r_addend) as u64;
            let resolved = resolve(resolver).unwrap_or(resolver);
            write(offset, resolved)
        }
        other => Err(RelocError::UnknownType(other)),
    }
}

pub fn apply_rela_table(
    entries: &[Elf64Rela],
    load_base: u64,
    resolve: impl Fn(u64) -> Option<u64>,
    mut write: impl FnMut(u64, u64) -> Result<(), RelocError>,
    allow_irelative: bool,
) -> Result<(), RelocError> {
    for rela in entries {
        apply_rela_entry(rela, load_base, &resolve, &mut write, allow_irelative)?;
    }
    Ok(())
}

pub fn rela_count(relasz: u64, relaent: u64) -> usize {
    if relaent == 0 {
        return 0;
    }
    (relasz / relaent) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_relocation() {
        let rela = Elf64Rela {
            r_offset: 0x1000,
            r_info: R_X86_64_RELATIVE as u64,
            r_addend: 0x20,
        };
        let mut mem = 0u64;
        apply_rela_entry(
            &rela,
            0x400000,
            |_| None,
            |_, v| {
                mem = v;
                Ok(())
            },
            false,
        )
        .unwrap();
        assert_eq!(mem, 0x400020);
    }

    #[test]
    fn glob_dat_addend() {
        let rela = Elf64Rela {
            r_offset: 0x2000,
            r_info: (1u64 << 32) | R_X86_64_GLOB_DAT as u64,
            r_addend: 8,
        };
        let mut mem = 0u64;
        apply_rela_entry(
            &rela,
            0,
            |_| Some(0x5000),
            |_, v| {
                mem = v;
                Ok(())
            },
            false,
        )
        .unwrap();
        assert_eq!(mem, 0x5008);
    }
}
