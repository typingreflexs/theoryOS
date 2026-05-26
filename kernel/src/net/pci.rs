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
    pub class_code: u8,
    pub subclass: u8,
    pub prog_if: u8,
    pub bar0: u64,
    pub bar0_is_mmio: bool,
    pub irq_line: u8,
}

impl PciDevice {
    pub fn bar(&self, index: u8) -> u64 {
        let offset = 0x10 + index * 4;
        let raw_lo = config_read32(self.bus, self.slot, self.func, offset);
        if raw_lo & 1 == 1 {
            // I/O BAR
            return (raw_lo & !0x3) as u64;
        }
        let kind = (raw_lo >> 1) & 0x3;
        let base_lo = (raw_lo & 0xFFFF_FFF0) as u64;
        if kind == 0x2 {
            let raw_hi = config_read32(self.bus, self.slot, self.func, offset + 4);
            base_lo | ((raw_hi as u64) << 32)
        } else {
            base_lo
        }
    }

    pub fn bar_is_mmio(&self, index: u8) -> bool {
        let offset = 0x10 + index * 4;
        let raw = config_read32(self.bus, self.slot, self.func, offset);
        raw & 1 == 0
    }
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

fn read_device(bus: u8, slot: u8, func: u8) -> Option<PciDevice> {
    let vendor_id = config_read16(bus, slot, func, 0);
    if vendor_id == 0xFFFF {
        return None;
    }
    let device_id = config_read16(bus, slot, func, 2);
    let class_word = config_read32(bus, slot, func, 0x08);
    let prog_if = ((class_word >> 8) & 0xFF) as u8;
    let subclass = ((class_word >> 16) & 0xFF) as u8;
    let class_code = ((class_word >> 24) & 0xFF) as u8;
    let bar0_raw = config_read32(bus, slot, func, 0x10);
    let bar0_is_mmio = bar0_raw & 1 == 0;
    let bar0 = (bar0_raw & 0xFFFF_FFF0) as u64;
    let irq = config_read32(bus, slot, func, 0x3C) as u8;
    Some(PciDevice {
        bus,
        slot,
        func,
        vendor_id,
        device_id,
        class_code,
        subclass,
        prog_if,
        bar0,
        bar0_is_mmio,
        irq_line: irq,
    })
}

pub fn for_each<F: FnMut(&PciDevice)>(mut f: F) {
    for slot in 0..32u8 {
        if let Some(dev) = read_device(0, slot, 0) {
            f(&dev);
            let header_type = config_read32(0, slot, 0, 0x0C) >> 16;
            if header_type & 0x80 != 0 {
                for func in 1..8u8 {
                    if let Some(fdev) = read_device(0, slot, func) {
                        f(&fdev);
                    }
                }
            }
        }
    }
}

pub fn find_device(vendor: u16, device: u16) -> Option<PciDevice> {
    let mut result = None;
    for_each(|dev| {
        if result.is_none() && dev.vendor_id == vendor && dev.device_id == device {
            result = Some(*dev);
        }
    });
    result
}

pub fn find_by_class(class: u8, subclass: u8, prog_if: Option<u8>) -> Option<PciDevice> {
    let mut result = None;
    for_each(|dev| {
        if result.is_some() {
            return;
        }
        if dev.class_code == class && dev.subclass == subclass {
            if let Some(pi) = prog_if {
                if dev.prog_if != pi {
                    return;
                }
            }
            result = Some(*dev);
        }
    });
    result
}

pub fn find_by_vendor_class(vendor: u16, class: u8, subclass: u8) -> Option<PciDevice> {
    let mut result = None;
    for_each(|dev| {
        if result.is_some() {
            return;
        }
        if dev.vendor_id == vendor && dev.class_code == class && dev.subclass == subclass {
            result = Some(*dev);
        }
    });
    result
}

pub fn map_mmio(phys: u64, pages: u64) {
    use crate::arch::memory::{PhysAddr, VirtAddr};
    use crate::mm::paging::{PageFlags, PageTable};
    if phys == 0 {
        return;
    }
    let table = PageTable::kernel();
    let hhdm = crate::boot_info().hhdm_offset;
    let flags =
        PageFlags::PRESENT | PageFlags::WRITABLE | PageFlags::NO_EXECUTE | PageFlags::NO_CACHE;
    for page in 0..pages {
        let p = PhysAddr::new(phys + page * 4096);
        let v = VirtAddr::new(hhdm + phys + page * 4096);
        let _ = table.map_page(v, p, flags);
    }
}

pub fn find_virtio_net() -> Option<PciDevice> {
    let mut result = None;
    for_each(|dev| {
        if result.is_some() {
            return;
        }
        if dev.vendor_id == 0x1AF4 && (dev.device_id == 0x1000 || dev.device_id == 0x1041) {
            result = Some(*dev);
        }
    });
    result
}
