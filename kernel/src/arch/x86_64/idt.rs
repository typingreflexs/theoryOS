use core::mem::size_of;

use crate::arch::x86_64::gdt::sel;
use crate::arch::x86_64::interrupts::InterruptFrame;

#[repr(C, packed)]
#[derive(Clone, Copy)]
struct IdtEntry {
    offset_low: u16,
    selector: u16,
    ist: u8,
    type_attr: u8,
    offset_mid: u16,
    offset_high: u32,
    zero: u32,
}

#[repr(C, packed)]
struct IdtPointer {
    limit: u16,
    base: u64,
}

const GATE_INTERRUPT: u8 = 0x8E;
const GATE_TRAP: u8 = 0x8F;

static mut IDT: [IdtEntry; 256] = [IdtEntry {
    offset_low: 0,
    selector: 0,
    ist: 0,
    type_attr: 0,
    offset_mid: 0,
    offset_high: 0,
    zero: 0,
}; 256];

extern "C" {
    static theory_vector_table: [u64; 256];
}

fn vector_address(vector: u8) -> u64 {
    unsafe { theory_vector_table[vector as usize] }
}

fn set_gate(vector: u8, handler: u64, ist: u8, trap: bool) {
    unsafe {
        IDT[vector as usize] = IdtEntry {
            offset_low: handler as u16,
            selector: sel::KERNEL_CODE,
            ist,
            type_attr: if trap { GATE_TRAP } else { GATE_INTERRUPT },
            offset_mid: (handler >> 16) as u16,
            offset_high: (handler >> 32) as u32,
            zero: 0,
        };
    }
}

pub fn init() {
    for vector in 0..=255u8 {
        let ist = if vector == 8 { 1 } else { 0 };
        let trap = vector == 1 || vector == 3;
        set_gate(vector, vector_address(vector), ist, trap);
    }
}

pub fn load() {
    let pointer = IdtPointer {
        limit: (size_of::<[IdtEntry; 256]>() - 1) as u16,
        base: unsafe { IDT.as_ptr() as u64 },
    };
    unsafe {
        core::arch::asm!("lidt [{0}]", in(reg) &pointer, options(nomem, nostack));
    }
}

pub fn report_exception(frame: &InterruptFrame) -> bool {
    super::interrupts::handle_exception(frame)
}

pub fn report_irq(frame: &InterruptFrame) {
    super::interrupts::handle_irq(frame);
}
