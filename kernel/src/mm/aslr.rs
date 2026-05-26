use crate::mm::layout::{KERNEL_HEAP_BASE, USER_HEAP_BASE, USER_MMAP_BASE, USER_STACK_TOP};

#[derive(Clone, Copy, Debug, Default)]
pub struct AslrEntropy {
    pub kernel_slide: u64,
    pub mmap_slide: u64,
    pub stack_slide: u64,
    pub heap_slide: u64,
}

static mut ENTROPY: AslrEntropy = AslrEntropy {
    kernel_slide: 0,
    mmap_slide: 0,
    stack_slide: 0,
    heap_slide: 0,
};

const MMAP_ALIGN: u64 = 2 * 1024 * 1024;
const STACK_ALIGN: u64 = 2 * 1024 * 1024;
const HEAP_ALIGN: u64 = 1024 * 1024;
const KERNEL_ALIGN: u64 = 1024 * 1024;

pub fn init() {
    unsafe {
        ENTROPY = AslrEntropy {
            kernel_slide: random_bounded(256) * KERNEL_ALIGN,
            mmap_slide: random_bounded(512) * MMAP_ALIGN,
            stack_slide: random_bounded(256) * STACK_ALIGN,
            heap_slide: random_bounded(128) * HEAP_ALIGN,
        };
    }
}

pub fn entropy() -> AslrEntropy {
    unsafe { ENTROPY }
}

pub fn user_mmap_base() -> u64 {
    USER_MMAP_BASE + unsafe { ENTROPY.mmap_slide }
}

pub fn user_stack_top() -> u64 {
    USER_STACK_TOP - unsafe { ENTROPY.stack_slide }
}

pub fn user_heap_base() -> u64 {
    USER_HEAP_BASE + unsafe { ENTROPY.heap_slide }
}

pub fn kernel_heap_base() -> u64 {
    KERNEL_HEAP_BASE + unsafe { ENTROPY.kernel_slide }
}

pub fn random_page_offset(max_pages: u64) -> u64 {
    random_bounded(max_pages) * crate::mm::layout::PAGE_SIZE
}

fn random_bounded(max: u64) -> u64 {
    if max <= 1 {
        return 0;
    }
    fallback_random() % max
}

fn fallback_random() -> u64 {
    let tsc = unsafe {
        let lo: u32;
        let hi: u32;
        core::arch::asm!("rdtsc", out("eax") lo, out("edx") hi, options(nomem, nostack));
        ((hi as u64) << 32) | lo as u64
    };
    let cr3 = crate::arch::x86_64::cpu::read_cr3();
    tsc ^ cr3 ^ 0x5DEECE66D
}
