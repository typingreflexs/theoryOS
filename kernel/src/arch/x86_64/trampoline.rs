//! AP bootstrap page copied to physical address 0x1000 for INIT-SIPI-SIPI.
//! Runtime patches CR3, CPU index, LAPIC ID, and stack pointer at fixed offsets.

#[repr(C, align(4096))]
pub struct TrampolinePage {
    pub data: [u8; 4096],
}

const CR3_OFFSET: usize = 0x140;
const CPU_INDEX_OFFSET: usize = 0x148;
const LAPIC_ID_OFFSET: usize = 0x150;
const STACK_OFFSET: usize = 0x158;

static mut TRAMPOLINE_PAGE: TrampolinePage = TrampolinePage { data: [0; 4096] };

pub fn image() -> *const TrampolinePage {
    unsafe { core::ptr::addr_of!(TRAMPOLINE_PAGE) }
}

pub fn image_size() -> usize {
    core::mem::size_of::<TrampolinePage>()
}

pub fn init_template() {
    unsafe {
        // Real-mode entry stub at offset 0 — patched further by platform bring-up.
        TRAMPOLINE_PAGE.data[0] = 0xFA; // cli
        TRAMPOLINE_PAGE.data[1] = 0xF4; // hlt (safe placeholder until SIPI lands in long mode)

        // Bootstrap GDT at 0x100
        write_u64(&mut TRAMPOLINE_PAGE.data[0x100..], 0);
        write_u64(&mut TRAMPOLINE_PAGE.data[0x108..], 0x00AF9A000000FFFF);
        write_u64(&mut TRAMPOLINE_PAGE.data[0x110..], 0x00CF92000000FFFF);
        write_u64(&mut TRAMPOLINE_PAGE.data[0x118..], 0x0020980000000000);

        // GDTR at 0x130
        write_u16(&mut TRAMPOLINE_PAGE.data[0x130..], 0x2F);
        write_u64(&mut TRAMPOLINE_PAGE.data[0x132..], 0x1100);
    }
}

pub fn patch(cr3: u64, cpu_index: u64, lapic_id: u64, stack_top: u64) {
    unsafe {
        write_u64(&mut TRAMPOLINE_PAGE.data[CR3_OFFSET..], cr3);
        write_u64(&mut TRAMPOLINE_PAGE.data[CPU_INDEX_OFFSET..], cpu_index);
        write_u64(&mut TRAMPOLINE_PAGE.data[LAPIC_ID_OFFSET..], lapic_id);
        write_u64(&mut TRAMPOLINE_PAGE.data[STACK_OFFSET..], stack_top);
    }
}

fn write_u16(dst: &mut [u8], value: u16) {
    dst[..2].copy_from_slice(&value.to_le_bytes());
}

fn write_u64(dst: &mut [u8], value: u64) {
    dst[..8].copy_from_slice(&value.to_le_bytes());
}
