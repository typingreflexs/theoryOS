use core::arch::asm;

pub fn disable_interrupts() {
    unsafe {
        asm!("cli", options(nomem, nostack));
    }
}

pub fn enable_interrupts() {
    unsafe {
        asm!("sti", options(nomem, nostack));
    }
}

pub fn halt_forever() -> ! {
    loop {
        disable_interrupts();
        unsafe {
            asm!("hlt", options(nomem, nostack));
        }
    }
}

pub fn verify_long_mode() {
    let (eax, _ebx, _ecx, edx): (u32, u32, u32, u32);
    unsafe {
        asm!(
            "push rbx",
            "cpuid",
            "pop rbx",
            inout("eax") 0x8000_0001u32 => eax,
            out("ecx") _ecx,
            out("edx") edx,
            options(nomem, nostack),
        );
    }
    if edx & (1 << 29) == 0 {
        panic!("CPU does not support long mode");
    }
}

pub fn read_cr2() -> u64 {
    let value: u64;
    unsafe {
        asm!("mov {}, cr2", out(reg) value, options(nomem, nostack));
    }
    value
}

pub fn read_cr3() -> u64 {
    let value: u64;
    unsafe {
        asm!("mov {}, cr3", out(reg) value, options(nomem, nostack));
    }
    value
}

pub fn write_cr3(value: u64) {
    unsafe {
        asm!("mov cr3, {}", in(reg) value, options(nomem, nostack));
    }
}

pub fn read_cr4() -> u64 {
    let value: u64;
    unsafe {
        asm!("mov {}, cr4", out(reg) value, options(nomem, nostack));
    }
    value
}

pub fn write_cr4(value: u64) {
    unsafe {
        asm!("mov cr4, {}", in(reg) value, options(nomem, nostack));
    }
}

pub fn read_rflags() -> u64 {
    let value: u64;
    unsafe {
        asm!("pushfq; pop {}", out(reg) value, options(nomem, nostack));
    }
    value
}

pub fn pause() {
    unsafe {
        asm!("pause", options(nomem, nostack));
    }
}

pub fn rdtsc() -> u64 {
    let lo: u32;
    let hi: u32;
    unsafe {
        asm!("rdtsc", out("eax") lo, out("edx") hi, options(nomem, nostack));
    }
    u64::from(lo) | (u64::from(hi) << 32)
}

pub fn mfence() {
    unsafe {
        asm!("mfence", options(nomem, nostack));
    }
}

pub fn lfence() {
    unsafe {
        asm!("lfence", options(nomem, nostack));
    }
}

pub fn read_msr(msr: u32) -> u64 {
    let (low, high): (u32, u32);
    unsafe {
        asm!(
            "rdmsr",
            in("ecx") msr,
            out("eax") low,
            out("edx") high,
            options(nomem, nostack),
        );
    }
    ((high as u64) << 32) | low as u64
}

pub fn write_msr(msr: u32, value: u64) {
    let low = value as u32;
    let high = (value >> 32) as u32;
    unsafe {
        asm!(
            "wrmsr",
            in("ecx") msr,
            in("eax") low,
            in("edx") high,
            options(nomem, nostack),
        );
    }
}
