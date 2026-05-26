use core::mem::size_of;

use spin::Mutex;

core::arch::global_asm!(
    r#"
    .intel_syntax noprefix
    .global gdt_reload
    // void gdt_reload(GdtPointer* ptr)
    gdt_reload:
        lgdt [rdi]
        mov rax, rsp
        push 0x10
        push rax
        pushfq
        push 0x08
        lea rax, [rip + .Lgdt_flush]
        push rax
        iretq
    .Lgdt_flush:
        mov ax, 0x10
        mov ds, ax
        mov es, ax
        mov ss, ax
        mov fs, ax
        mov gs, ax
        ret
    "#,
);

extern "C" {
    fn gdt_reload(ptr: *const GdtPointer);
}

/// GDT segment selectors.
pub mod sel {
    pub const NULL: u16 = 0x00;
    pub const KERNEL_CODE: u16 = 0x08;
    pub const KERNEL_DATA: u16 = 0x10;
    pub const USER_DATA: u16 = 0x18 | 3;
    pub const USER_CODE: u16 = 0x20 | 3;
    pub const TSS: u16 = 0x28;
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
struct SegmentDescriptor {
    limit_low: u16,
    base_low: u16,
    base_mid: u8,
    access: u8,
    limit_high_flags: u8,
    base_high: u8,
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
struct TssDescriptor {
    limit_low: u16,
    base_low: u16,
    base_mid: u8,
    access: u8,
    limit_high_flags: u8,
    base_high: u8,
    base_upper: u32,
    reserved: u32,
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
struct Gdt {
    null: SegmentDescriptor,
    kernel_code: SegmentDescriptor,
    kernel_data: SegmentDescriptor,
    user_data: SegmentDescriptor,
    user_code: SegmentDescriptor,
    tss: TssDescriptor,
}

#[repr(C, packed)]
struct GdtPointer {
    limit: u16,
    base: u64,
}

const ACC_PRESENT: u8 = 1 << 7;
const ACC_RING0_CODE: u8 = 0b10011010;
const ACC_RING0_DATA: u8 = 0b10010010;
const ACC_RING3_CODE: u8 = 0b11111010;
const ACC_RING3_DATA: u8 = 0b11110010;
const ACC_TSS_AVAILABLE: u8 = 0b10001001;
const FLAGS_GRANULARITY: u8 = 1 << 7;
const FLAGS_LONG_MODE: u8 = 1 << 5;
const FLAGS_SIZE_32: u8 = 1 << 6;

use super::tss::MAX_CPUS;

static BSP_GDT: Mutex<Gdt> = Mutex::new(empty_gdt());
static AP_GDTS: Mutex<[Gdt; MAX_CPUS]> = Mutex::new([empty_gdt(); MAX_CPUS]);

const fn empty_gdt() -> Gdt {
    Gdt {
        null: SegmentDescriptor {
            limit_low: 0,
            base_low: 0,
            base_mid: 0,
            access: 0,
            limit_high_flags: 0,
            base_high: 0,
        },
        kernel_code: SegmentDescriptor {
            limit_low: 0,
            base_low: 0,
            base_mid: 0,
            access: 0,
            limit_high_flags: 0,
            base_high: 0,
        },
        kernel_data: SegmentDescriptor {
            limit_low: 0,
            base_low: 0,
            base_mid: 0,
            access: 0,
            limit_high_flags: 0,
            base_high: 0,
        },
        user_data: SegmentDescriptor {
            limit_low: 0,
            base_low: 0,
            base_mid: 0,
            access: 0,
            limit_high_flags: 0,
            base_high: 0,
        },
        user_code: SegmentDescriptor {
            limit_low: 0,
            base_low: 0,
            base_mid: 0,
            access: 0,
            limit_high_flags: 0,
            base_high: 0,
        },
        tss: TssDescriptor {
            limit_low: 0,
            base_low: 0,
            base_mid: 0,
            access: 0,
            limit_high_flags: 0,
            base_high: 0,
            base_upper: 0,
            reserved: 0,
        },
    }
}

fn make_code_segment(ring0: bool) -> SegmentDescriptor {
    SegmentDescriptor {
        limit_low: 0,
        base_low: 0,
        base_mid: 0,
        access: ACC_PRESENT | if ring0 { ACC_RING0_CODE } else { ACC_RING3_CODE },
        limit_high_flags: FLAGS_GRANULARITY | FLAGS_LONG_MODE,
        base_high: 0,
    }
}

fn make_data_segment(ring0: bool) -> SegmentDescriptor {
    SegmentDescriptor {
        limit_low: 0xffff,
        base_low: 0,
        base_mid: 0,
        access: ACC_PRESENT | if ring0 { ACC_RING0_DATA } else { ACC_RING3_DATA },
        limit_high_flags: FLAGS_GRANULARITY | FLAGS_SIZE_32,
        base_high: 0,
    }
}

fn make_tss_descriptor(base: u64, limit: u32) -> TssDescriptor {
    TssDescriptor {
        limit_low: (limit & 0xffff) as u16,
        base_low: (base & 0xffff) as u16,
        base_mid: ((base >> 16) & 0xff) as u8,
        access: ACC_TSS_AVAILABLE,
        limit_high_flags: ((limit >> 16) & 0xf) as u8 | FLAGS_GRANULARITY,
        base_high: ((base >> 24) & 0xff) as u8,
        base_upper: (base >> 32) as u32,
        reserved: 0,
    }
}

fn populate_gdt(gdt: &mut Gdt, tss_base: u64, tss_limit: u32) {
    gdt.null = SegmentDescriptor {
        limit_low: 0,
        base_low: 0,
        base_mid: 0,
        access: 0,
        limit_high_flags: 0,
        base_high: 0,
    };
    gdt.kernel_code = make_code_segment(true);
    gdt.kernel_data = make_data_segment(true);
    gdt.user_data = make_data_segment(false);
    gdt.user_code = make_code_segment(false);
    gdt.tss = make_tss_descriptor(tss_base, tss_limit);
}

unsafe fn load_gdt(gdt: &Gdt) {
    let pointer = GdtPointer {
        limit: (size_of::<Gdt>() - 1) as u16,
        base: gdt as *const Gdt as u64,
    };
    gdt_reload(&pointer);
}

unsafe fn load_tss() {
    core::arch::asm!(
        "ltr {0:x}",
        in(reg) sel::TSS,
        options(nomem, nostack),
    );
}

pub fn init_bsp() {
    let tss = super::tss::bsp_tss();
    let mut gdt = BSP_GDT.lock();
    populate_gdt(
        &mut gdt,
        tss as *const _ as u64,
        super::tss::TSS_LIMIT as u32,
    );
    unsafe {
        load_gdt(&*gdt);
    }
}

pub fn load_tss_bsp() {
    unsafe {
        load_tss();
    }
}

pub fn init_ap(cpu_index: u32) {
    let idx = cpu_index as usize;
    assert!(idx < MAX_CPUS);
    let tss = super::tss::ap_tss(cpu_index);
    let mut gdts = AP_GDTS.lock();
    populate_gdt(
        &mut gdts[idx],
        tss as *const _ as u64,
        super::tss::TSS_LIMIT as u32,
    );
    unsafe {
        load_gdt(&gdts[idx]);
    }
}

pub fn load_tss_ap(_cpu_index: u32) {
    unsafe {
        load_tss();
    }
}

pub fn kernel_code_selector() -> u16 {
    sel::KERNEL_CODE
}

pub fn kernel_data_selector() -> u16 {
    sel::KERNEL_DATA
}

pub fn user_code_selector() -> u16 {
    sel::USER_CODE
}

pub fn user_data_selector() -> u16 {
    sel::USER_DATA
}
