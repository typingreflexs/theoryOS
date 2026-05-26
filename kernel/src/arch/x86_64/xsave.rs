use core::arch::asm;

/// Extended state save area (SSE + AVX via XSAVE), 4 KiB aligned.
pub const XSAVE_AREA_SIZE: usize = 4096;

#[repr(C, align(4096))]
#[derive(Clone, Debug)]
pub struct ExtendedState {
    data: [u8; XSAVE_AREA_SIZE],
}

impl ExtendedState {
    pub const fn zeroed() -> Self {
        Self { data: [0; XSAVE_AREA_SIZE] }
    }

    pub fn as_ptr(&self) -> *const u8 {
        self.data.as_ptr()
    }

    pub fn as_mut_ptr(&mut self) -> *mut u8 {
        self.data.as_mut_ptr()
    }
}

const XCR0_SSE: u64 = 1 << 1;
const XCR0_AVX: u64 = 1 << 2;
const XSAVE_MASK: u64 = XCR0_SSE | XCR0_AVX;

/// Enable XSAVE and configure XCR0 for SSE + AVX.
pub fn init() {
    let (eax, _ebx, ecx, _edx): (u32, u32, u32, u32);
    unsafe {
        asm!(
            "push rbx",
            "cpuid",
            "pop rbx",
            inout("eax") 1u32 => eax,
            out("ecx") ecx,
            options(nomem, nostack),
        );
    }
    if ecx & (1 << 27) == 0 {
        return;
    }

    let mut cr4: u64;
    unsafe {
        asm!("mov {}, cr4", out(reg) cr4, options(nomem, nostack));
    }
    cr4 |= 1 << 18; // OSXSAVE
    unsafe {
        asm!("mov cr4, {}", in(reg) cr4, options(nomem, nostack));
    }

    let (eax2, ecx2, _edx2): (u32, u32, u32);
    unsafe {
        asm!(
            "push rbx",
            "cpuid",
            "pop rbx",
            inout("eax") 0x0Du32 => eax2,
            in("ecx") 1u32,
            lateout("ecx") ecx2,
            options(nomem, nostack),
        );
    }
    let mut xcr0 = XCR0_SSE;
    if ecx2 & (1 << 2) != 0 {
        xcr0 |= XCR0_AVX;
    }

    let low = xcr0 as u32;
    let high = (xcr0 >> 32) as u32;
    unsafe {
        asm!(
            "xsetbv",
            in("ecx") 0u32,
            in("eax") low,
            in("edx") high,
            options(nomem, nostack),
        );
    }
}

pub fn save(state: &mut ExtendedState) {
    let lo = XSAVE_MASK as u32;
    let hi = (XSAVE_MASK >> 32) as u32;
    unsafe {
        asm!(
            "xsave [{}]",
            in(reg) state.as_mut_ptr(),
            in("eax") lo,
            in("edx") hi,
            options(nostack),
        );
    }
}

pub fn restore(state: &ExtendedState) {
    let lo = XSAVE_MASK as u32;
    let hi = (XSAVE_MASK >> 32) as u32;
    unsafe {
        asm!(
            "xrstor [{}]",
            in(reg) state.as_ptr(),
            in("eax") lo,
            in("edx") hi,
            options(nostack),
        );
    }
}
