use core::fmt::Write;
use core::sync::atomic::{AtomicU32, Ordering};

use spin::Mutex;

use crate::acpi::AcpiContext;
use crate::arch::memory::{phys_to_virt, PhysAddr, VirtAddr};
use crate::console::Console;

const IOAPIC_REG_INDEX: u32 = 0x00;
const IOAPIC_REG_DATA: u32 = 0x10;
const IOAPIC_ID: u32 = 0x00;
const IOAPIC_VER: u32 = 0x01;
const IOAPIC_ARB: u32 = 0x02;
const IOAPIC_REDIRECT: u32 = 0x10;

const MAX_IOAPICS: usize = 8;
const MAX_IRQS: usize = 256;

#[derive(Clone, Copy, Debug)]
struct IoApic {
    mmio: VirtAddr,
    id: u8,
    gsi_base: u32,
    max_redirections: u8,
}

static IOAPICS: Mutex<[Option<IoApic>; MAX_IOAPICS]> = Mutex::new([None; MAX_IOAPICS]);
static IOAPIC_COUNT: AtomicU32 = AtomicU32::new(0);
static IRQ_OVERRIDES: Mutex<[Option<IrqOverride>; MAX_IRQS]> = Mutex::new([None; MAX_IRQS]);

#[derive(Clone, Copy, Debug)]
struct IrqOverride {
    gsi: u32,
    vector: u8,
}

fn ioapic_read(ioapic: IoApic, reg: u32) -> u32 {
    unsafe {
        ioapic
            .mmio
            .as_mut_ptr::<u32>()
            .add((IOAPIC_REG_INDEX / 4) as usize)
            .write_volatile(reg);
        ioapic
            .mmio
            .as_mut_ptr::<u32>()
            .add((IOAPIC_REG_DATA / 4) as usize)
            .read_volatile()
    }
}

fn ioapic_write(ioapic: IoApic, reg: u32, value: u32) {
    unsafe {
        ioapic
            .mmio
            .as_mut_ptr::<u32>()
            .add((IOAPIC_REG_INDEX / 4) as usize)
            .write_volatile(reg);
        ioapic
            .mmio
            .as_mut_ptr::<u32>()
            .add((IOAPIC_REG_DATA / 4) as usize)
            .write_volatile(value);
    }
}

pub fn discover_from_acpi(ctx: &AcpiContext) {
    let hhdm = crate::boot_info().hhdm_offset;
    let mut count = 0usize;

    for entry in ctx.madt().io_apics() {
        if entry.address.as_u64() == 0 {
            continue;
        }
        let mmio = phys_to_virt(hhdm, entry.address);
        // Do not probe IOAPIC MMIO at boot: Limine HHDM may not cover chipset MMIO yet.
        let ioapic = IoApic {
            mmio,
            id: entry.io_apic_id,
            gsi_base: entry.global_system_interrupt_base,
            max_redirections: 24,
        };

        if count < MAX_IOAPICS {
            IOAPICS.lock()[count] = Some(ioapic);
            count += 1;
            let _ = writeln!(
                Console,
                "[ioapic] id={} gsi_base={} entries=24",
                entry.io_apic_id,
                entry.global_system_interrupt_base
            );
        }
    }

    for override_entry in ctx.madt().interrupt_overrides() {
        let vector = if override_entry.irq < 16 {
            32 + override_entry.irq
        } else {
            32 + (override_entry.irq as u32 % 224) as u8
        };
        let idx = override_entry.global_system_interrupt as usize;
        if idx < MAX_IRQS {
            IRQ_OVERRIDES.lock()[idx] = Some(IrqOverride {
                gsi: override_entry.global_system_interrupt,
                vector,
            });
        }
    }

    IOAPIC_COUNT.store(count as u32, Ordering::Release);
}

pub fn mask_all() {
    let _ = writeln!(Console, "[ioapic] legacy IRQ routing deferred (APIC timer only)");
}

pub fn route_irqs() {
    mask_all();
}

fn resolve_vector(gsi: u32, pin: u8) -> u8 {
    if let Some(entry) = IRQ_OVERRIDES.lock()[gsi as usize] {
        return entry.vector;
    }
    32 + (pin % 224)
}

fn set_redirection(ioapic: IoApic, pin: u8, vector: u8) {
    let low = IOAPIC_REDIRECT + (pin as u32 * 2);
    let high = low + 1;
    ioapic_write(ioapic, high, 0);
    ioapic_write(ioapic, low, vector as u32);
}

pub fn remap_irq(irq: u8, vector: u8) {
    let ioapics = IOAPICS.lock();
    let count = IOAPIC_COUNT.load(Ordering::Acquire) as usize;
    for idx in 0..count {
        let Some(ioapic) = ioapics[idx] else { continue };
        if (irq as u32) < ioapic.max_redirections as u32 {
            set_redirection(ioapic, irq, vector);
            return;
        }
    }
}
