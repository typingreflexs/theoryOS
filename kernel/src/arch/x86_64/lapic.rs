use core::fmt::Write;
use core::sync::atomic::{AtomicU32, Ordering};

use spin::Mutex;

use crate::arch::memory::{phys_to_virt, VirtAddr};
use crate::arch::x86_64::cpu;
use crate::console::Console;

const IA32_APIC_BASE: u32 = 0x1B;
const APIC_BASE_ENABLE: u64 = 1 << 11;
const APIC_SPURIOUS: u32 = 0xF0;
const APIC_LVT_TIMER: u32 = 0x320;
const APIC_TIMER_INIT: u32 = 0x380;
const APIC_TIMER_CURRENT: u32 = 0x390;
const APIC_LVT_ERROR: u32 = 0x370;
const APIC_ERROR: u32 = 0x280;
const APIC_EOI: u32 = 0xB0;
const APIC_ICR_LOW: u32 = 0x300;
const APIC_ICR_HIGH: u32 = 0x310;
const APIC_LVT_PERF: u32 = 0x340;
const APIC_LVT_LINT0: u32 = 0x350;
const APIC_LVT_LINT1: u32 = 0x360;

const APIC_TIMER_DIVIDE: u32 = 0x3E0;
const APIC_BUS_HZ: u32 = 1_000_000_000; // QEMU typical APIC bus frequency
const TIMER_MODE_PERIODIC: u32 = 0b01 << 17;
const TIMER_VECTOR: u32 = 32;
const LVT_MASKED: u32 = 1 << 16;
const TIMER_DIVIDE_16: u32 = 0x3;

static LAPIC_MMIO: Mutex<Option<VirtAddr>> = Mutex::new(None);
static BSP_LAPIC_ID: AtomicU32 = AtomicU32::new(0);
static TIMER_TICKS_PER_MS: AtomicU32 = AtomicU32::new(0);

pub fn init_bsp() {
    let ctx = crate::acpi::context().expect("ACPI required before LAPIC init");
    let apic_phys = ctx.local_apic_address();
    let virt = phys_to_virt(crate::boot_info().hhdm_offset, apic_phys);
    LAPIC_MMIO.lock().replace(virt);

    let bsp_id = crate::boot_info().bsp_lapic_id;
    BSP_LAPIC_ID.store(bsp_id, Ordering::Release);

    enable_local_apic();
    mask_legacy_lvt_lines();
    configure_error_lvt();

    let _ = writeln!(Console, "[lapic] BSP LAPIC id={bsp_id} base={:#x}", apic_phys.as_u64());
}

pub fn init_ap(lapic_id: u32) {
    enable_local_apic();
    mask_legacy_lvt_lines();
    configure_error_lvt();
    let _ = writeln!(Console, "[lapic] AP online lapic_id={lapic_id}");
}

fn lapic_base() -> VirtAddr {
    LAPIC_MMIO.lock().expect("LAPIC not mapped")
}

fn lapic_read(offset: u32) -> u32 {
    let base = lapic_base();
    unsafe { base.as_mut_ptr::<u32>().add((offset / 4) as usize).read_volatile() }
}

fn lapic_write(offset: u32, value: u32) {
    let base = lapic_base();
    unsafe {
        base.as_mut_ptr::<u32>()
            .add((offset / 4) as usize)
            .write_volatile(value)
    }
}

fn enable_local_apic() {
    let mut base = cpu::read_msr(IA32_APIC_BASE);
    base |= APIC_BASE_ENABLE;
    cpu::write_msr(IA32_APIC_BASE, base);

    lapic_write(APIC_SPURIOUS, (0xFF) | (1 << 8));
}

fn mask_legacy_lvt_lines() {
    lapic_write(APIC_LVT_PERF, LVT_MASKED);
    lapic_write(APIC_LVT_LINT0, LVT_MASKED);
    lapic_write(APIC_LVT_LINT1, LVT_MASKED);
}

fn configure_error_lvt() {
    lapic_write(APIC_LVT_ERROR, LVT_MASKED);
    lapic_write(APIC_ERROR, 0);
}

pub fn current_lapic_id() -> u32 {
    (lapic_read(0x20) >> 24) & 0xFF
}

pub fn send_eoi() {
    lapic_write(APIC_EOI, 0);
}

pub fn prepare_timer() {
    // APIC timer counts at bus_hz / divide. Do not use ACPI PM timer frequency here.
    let ticks = APIC_BUS_HZ / 16 / 1000; // ~1 ms at 1 GHz bus, divide-by-16
    lapic_write(APIC_TIMER_DIVIDE, TIMER_DIVIDE_16);
    lapic_write(APIC_LVT_TIMER, LVT_MASKED | TIMER_VECTOR | TIMER_MODE_PERIODIC);
    lapic_write(APIC_TIMER_INIT, ticks);
    TIMER_TICKS_PER_MS.store(ticks, Ordering::Release);
}

pub fn start_timer() {
    let ticks = TIMER_TICKS_PER_MS.load(Ordering::Acquire);
    if ticks == 0 {
        prepare_timer();
    }
    lapic_write(APIC_TIMER_DIVIDE, TIMER_DIVIDE_16);
    lapic_write(APIC_LVT_TIMER, TIMER_VECTOR | TIMER_MODE_PERIODIC);
    lapic_write(APIC_TIMER_INIT, ticks.max(1));
}

pub fn calibrate_timer() {
    prepare_timer();
}

pub fn send_ipi(destination: u32, vector: u8, level: bool) {
    while lapic_read(APIC_ICR_LOW) & (1 << 12) != 0 {
        cpu::pause();
    }
    lapic_write(APIC_ICR_HIGH, destination << 24);
    let mut low = vector as u32;
    if level {
        low |= 1 << 14;
    }
    lapic_write(APIC_ICR_LOW, low);
}

pub fn send_init_ipi(destination: u32) {
    while lapic_read(APIC_ICR_LOW) & (1 << 12) != 0 {
        cpu::pause();
    }
    lapic_write(APIC_ICR_HIGH, destination << 24);
    lapic_write(APIC_ICR_LOW, 0b101 << 8);
}

pub fn send_sipi(destination: u32, vector: u8) {
    while lapic_read(APIC_ICR_LOW) & (1 << 12) != 0 {
        cpu::pause();
    }
    lapic_write(APIC_ICR_HIGH, destination << 24);
    lapic_write(APIC_ICR_LOW, 0b110 << 8 | (vector as u32));
}

pub fn bsp_lapic_id() -> u32 {
    BSP_LAPIC_ID.load(Ordering::Acquire)
}

pub fn set_bsp_lapic_id(id: u32) {
    BSP_LAPIC_ID.store(id, Ordering::Release);
}

pub fn calibrate_timer_ap() {
    if TIMER_TICKS_PER_MS.load(Ordering::Acquire) == 0 {
        prepare_timer();
    }
}
