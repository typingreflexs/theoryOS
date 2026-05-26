//! ACPI power management — shutdown (S5) and reset.
//!
//! Shutdown requires:
//! 1. The PM1a (and optionally PM1b) control block from the FADT
//! 2. The `SLP_TYPa` value scraped from the `_S5` package in the DSDT
//! 3. Writing `(SLP_TYPa << 10) | SLP_EN (1<<13)` to PM1a_CNT (and PM1b_CNT)
//!
//! Reset uses the FADT reset register if available, else falls back to the
//! 8042 keyboard controller (which can pulse the CPU reset line).

use crate::arch::x86_64::port;
use crate::console::Console;

/// Shut the machine down (ACPI S5). Returns `false` if the platform doesn't
/// provide the necessary ACPI data; the caller should fall back to halting.
pub fn shutdown() -> bool {
    let ctx = match crate::acpi::context() {
        Some(c) => c,
        None => return false,
    };
    let Some(fadt) = ctx.fadt() else { return false };
    let Some(dsdt) = ctx.dsdt() else { return false };
    let Some((slp_a, slp_b)) = dsdt.find_s5() else {
        Console::println("[acpi] no _S5 in DSDT — cannot shut down");
        return false;
    };
    let Some(pm1a) = fadt.pm1a_cnt_blk() else {
        Console::println("[acpi] FADT missing PM1a_CNT_BLK");
        return false;
    };

    Console::println("[acpi] entering S5 (poweroff)");
    let value_a = ((slp_a as u16) << 10) | (1 << 13);
    unsafe { port::outw(pm1a, value_a) };

    if let Some(pm1b) = fadt.pm1b_cnt_blk() {
        let value_b = ((slp_b as u16) << 10) | (1 << 13);
        unsafe { port::outw(pm1b, value_b) };
    }

    // S5 should fire immediately; spin briefly waiting for power-off.
    for _ in 0..1_000_000 {
        core::hint::spin_loop();
    }
    Console::println("[acpi] poweroff did not complete");
    false
}

/// Reset the machine. Tries the FADT reset register first, then the 8042
/// keyboard controller, and finally triple-faults via a null IDT.
pub fn reboot() -> ! {
    if let Some(ctx) = crate::acpi::context() {
        if let Some(fadt) = ctx.fadt() {
            if let Some(reset) = fadt.reset_register() {
                // address_space 1 = System I/O
                if reset.address_space == 1 {
                    Console::println("[acpi] reboot via ACPI reset register");
                    unsafe { port::outb(reset.address as u16, fadt.reset_value()) };
                    for _ in 0..1_000_000 {
                        core::hint::spin_loop();
                    }
                }
            }
        }
    }

    Console::println("[acpi] reboot via 8042");
    unsafe {
        for _ in 0..10 {
            while port::inb(0x64) & 0x02 != 0 {}
            port::outb(0x64, 0xFE);
        }
    }

    // Last resort: triple fault by loading a null IDT and executing int 3.
    unsafe {
        #[repr(C, packed)]
        struct IdtPtr {
            limit: u16,
            base: u64,
        }
        let null = IdtPtr { limit: 0, base: 0 };
        core::arch::asm!("lidt [{0}]; int3", in(reg) &null, options(noreturn));
    }
}
