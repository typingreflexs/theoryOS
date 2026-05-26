use core::arch::asm;

/// Fully mask both cascaded 8259 PICs so all external interrupts route through IOAPIC.
pub fn disable_legacy_pic() {
    const MASTER_CMD: u16 = 0x20;
    const MASTER_DATA: u16 = 0x21;
    const SLAVE_CMD: u16 = 0xA0;
    const SLAVE_DATA: u16 = 0xA1;
    const ICW1_INIT: u8 = 0x11;
    const ICW4_8086: u8 = 0x01;

    unsafe {
        // Initialize master PIC.
        asm!("out dx, al", in("dx") MASTER_CMD, in("al") ICW1_INIT, options(nomem, nostack));
        asm!("out dx, al", in("dx") MASTER_DATA, in("al") 0x20u8, options(nomem, nostack));
        asm!("out dx, al", in("dx") MASTER_CMD, in("al") 0x04u8, options(nomem, nostack));
        asm!("out dx, al", in("dx") MASTER_DATA, in("al") ICW4_8086, options(nomem, nostack));

        // Initialize slave PIC.
        asm!("out dx, al", in("dx") SLAVE_CMD, in("al") ICW1_INIT, options(nomem, nostack));
        asm!("out dx, al", in("dx") SLAVE_DATA, in("al") 0x28u8, options(nomem, nostack));
        asm!("out dx, al", in("dx") SLAVE_CMD, in("al") 0x02u8, options(nomem, nostack));
        asm!("out dx, al", in("dx") SLAVE_DATA, in("al") ICW4_8086, options(nomem, nostack));

        // Mask all IRQ lines.
        asm!("out dx, al", in("dx") MASTER_DATA, in("al") 0xFFu8, options(nomem, nostack));
        asm!("out dx, al", in("dx") SLAVE_DATA, in("al") 0xFFu8, options(nomem, nostack));
    }
}
