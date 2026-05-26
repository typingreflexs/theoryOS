//! Legacy PCI configuration space access.

use crate::arch::x86_64::port;

const CONFIG_ADDRESS: u16 = 0xCF8;
const CONFIG_DATA: u16 = 0xCFC;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PciDevice {
    pub bus: u8,
    pub slot: u8,
    pub func: u8,
    pub vendor_id: u16,
    pub device_id: u16,
    pub bar0: u64,
    pub bar0_is_mmio: bool,
    pub irq_line: u8,
}

fn config_address(bus: u8, slot: u8, func: u8, offset: u8) -> u32 {
    0x8000_0000
        | ((bus as u32) << 16)
        | ((slot as u32) << 11)
        | ((func as u32) << 8)
        | ((offset as u32) & 0xFC)
}

pub fn config_read32(bus: u8, slot: u8, func: u8, offset: u8) -> u32 {
    unsafe {
        port::outl(CONFIG_ADDRESS, config_address(bus, slot, func, offset));
        port::inl(CONFIG_DATA)
    }
}

pub fn config_write32(bus: u8, slot: u8, func: u8, offset: u8, value: u32) {
    unsafe {
        port::outl(CONFIG_ADDRESS, config_address(bus, slot, func, offset));
        port::outl(CONFIG_DATA, value);
    }
}

pub fn config_read16(bus: u8, slot: u8, func: u8, offset: u8) -> u16 {
    let v = config_read32(bus, slot, func, offset & 0xFC);
    let shift = ((offset & 2) * 8) as u32;
    ((v >> shift) & 0xFFFF) as u16
}

pub fn config_write16(bus: u8, slot: u8, func: u8, offset: u8, value: u16) {
    let aligned = offset & 0xFC;
    let mut v = config_read32(bus, slot, func, aligned);
    let shift = ((offset & 2) * 8) as u32;
    v = (v & !(0xFFFF << shift)) | ((value as u32) << shift);
    config_write32(bus, slot, func, aligned, v);
}

pub fn enable_device(dev: &PciDevice) {
    let cmd = config_read16(dev.bus, dev.slot, dev.func, 0x04);
    config_write16(dev.bus, dev.slot, dev.func, 0x04, cmd | 0x0007);
}

pub fn map_bar0(dev: &PciDevice) {
    if !dev.bar0_is_mmio || dev.bar0 == 0 {
        return;
    }
    use crate::arch::memory::{PhysAddr, VirtAddr};
    use crate::mm::paging::{PageFlags, PageTable};

    let table = PageTable::kernel();
    let hhdm = crate::boot_info().hhdm_offset;
    let flags = PageFlags::PRESENT
        | PageFlags::WRITABLE
        | PageFlags::NO_EXECUTE
        | PageFlags::NO_CACHE;
    for page in 0..32u64 {
        let phys = PhysAddr::new(dev.bar0 + page * 4096);
        let virt = VirtAddr::new(hhdm + dev.bar0 + page * 4096);
        let _ = table.map_page(virt, phys, flags);
    }
}

pub fn find_device(vendor: u16, device: u16) -> Option<PciDevice> {
    for slot in 0..32u8 {
        let vendor_id = config_read16(0, slot, 0, 0);
        if vendor_id == 0xFFFF {
            continue;
        }
        let device_id = config_read16(0, slot, 0, 2);
        if vendor_id == vendor && device_id == device {
            let bar0_raw = config_read32(0, slot, 0, 0x10);
            let bar0_is_mmio = bar0_raw & 1 == 0;
            let bar0 = (bar0_raw & 0xFFFF_FFF0) as u64;
            let irq = config_read32(0, slot, 0, 0x3C) as u8;
            return Some(PciDevice {
                bus: 0,
                slot,
                func: 0,
                vendor_id,
                device_id,
                bar0,
                bar0_is_mmio,
                irq_line: irq,
            });
        }
    }
    None
}

pub fn find_virtio_net() -> Option<PciDevice> {
    for slot in 0..32u8 {
        let vendor_id = config_read16(0, slot, 0, 0);
        if vendor_id != 0x1AF4 {
            continue;
        }
        let device_id = config_read16(0, slot, 0, 2);
        if device_id != 0x1000 && device_id != 0x1041 {
            continue;
        }
        let subsystem = config_read16(0, slot, 0, 0x2E);
        if subsystem != 1 && device_id == 0x1000 {
            // modern virtio may use subsystem; accept 0x1000 anyway
        }
        let bar0_raw = config_read32(0, slot, 0, 0x10);
        let bar0 = (bar0_raw & 0xFFFF_FFF0) as u64;
        let irq = config_read32(0, slot, 0, 0x3C) as u8;
        return Some(PciDevice {
            bus: 0,
            slot,
            func: 0,
            vendor_id,
            device_id,
            bar0,
            bar0_is_mmio: bar0_raw & 1 == 0,
            irq_line: irq,
        });
    }
    None
}
