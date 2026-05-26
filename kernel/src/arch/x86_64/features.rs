//! x86-64 CPU security feature detection and enablement.

use crate::console::Console;

const EFER_MSR: u32 = 0xC000_0080;
const EFER_NXE: u64 = 1 << 11;

const CR4_UMIP: u64 = 1 << 11;
const CR4_SMEP: u64 = 1 << 20;
const CR4_SMAP: u64 = 1 << 21;

#[derive(Clone, Copy, Debug, Default)]
pub struct CpuSecurityFeatures {
    pub nxe: bool,
    pub smep: bool,
    pub smap: bool,
    pub umip: bool,
}

static mut FEATURES: CpuSecurityFeatures = CpuSecurityFeatures {
    nxe: false,
    smep: false,
    smap: false,
    umip: false,
};

pub fn init() -> CpuSecurityFeatures {
    let mut feats = detect();
    enable(&mut feats);
    unsafe { FEATURES = feats };
    let _ = core::fmt::Write::write_fmt(
        &mut Console,
        format_args!(
            "[security] CPU: NXE={} SMEP={} SMAP={} UMIP={}\n",
            feats.nxe, feats.smep, feats.smap, feats.umip
        ),
    );
    feats
}

pub fn features() -> CpuSecurityFeatures {
    unsafe { FEATURES }
}

pub fn smap_enabled() -> bool {
    unsafe { FEATURES.smap }
}

fn detect() -> CpuSecurityFeatures {
    let mut feats = CpuSecurityFeatures::default();
    let (_, _, ecx_1, edx_1) = cpuid(1);
    let (_, _, _, edx_8001) = cpuid(0x8000_0001);

    // NX bit in CPUID.80000001H:EDX[20]
    feats.nxe = edx_8001 & (1 << 20) != 0;
    // SMEP in CPUID.7.EBX[7] — check via leaf 7 if available
    if ecx_1 & (1 << 27) != 0 {
        let (_, ebx_7, ecx_7, _) = cpuid_sub(7, 0);
        feats.smep = ebx_7 & (1 << 7) != 0;
        feats.smap = ebx_7 & (1 << 20) != 0;
        feats.umip = ecx_7 & (1 << 2) != 0;
    }
    let _ = edx_1;
    feats
}

fn enable(feats: &mut CpuSecurityFeatures) {
    if feats.nxe {
        let mut efer = super::cpu::read_msr(EFER_MSR);
        if efer & EFER_NXE == 0 {
            efer |= EFER_NXE;
            super::cpu::write_msr(EFER_MSR, efer);
        }
    } else {
        feats.nxe = false;
    }

    let mut cr4 = super::cpu::read_cr4();
    if feats.smep {
        cr4 |= CR4_SMEP;
    } else {
        cr4 &= !CR4_SMEP;
    }
    if feats.smap {
        cr4 |= CR4_SMAP;
    } else {
        cr4 &= !CR4_SMAP;
    }
    if feats.umip {
        cr4 |= CR4_UMIP;
    } else {
        cr4 &= !CR4_UMIP;
    }
    super::cpu::write_cr4(cr4);
}

fn cpuid_sub(leaf: u32, sub: u32) -> (u32, u32, u32, u32) {
    let eax: u32;
    let ebx: u32;
    let ecx: u32;
    let edx: u32;
    unsafe {
        core::arch::asm!(
            "push rbx",
            "cpuid",
            "mov {ebx:e}, ebx",
            "pop rbx",
            ebx = out(reg) ebx,
            inout("eax") leaf => eax,
            inout("ecx") sub => ecx,
            out("edx") edx,
            options(nomem, nostack),
        );
    }
    (eax, ebx, ecx, edx)
}

fn cpuid(leaf: u32) -> (u32, u32, u32, u32) {
    cpuid_sub(leaf, 0)
}
