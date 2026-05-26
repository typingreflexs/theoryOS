//! Realtek RTL8169 / RTL8168 / RTL8111 Gigabit NIC driver.
//!
//! The 8169-family is the most common cheap Ethernet chip on real bare metal
//! PCs (e.g. RTL8111 onboard the great majority of consumer motherboards).
//! Supports both legacy 8169 (BAR0 I/O + BAR1 MMIO) and 8168/8111
//! (BAR2 64-bit MMIO).
//!
//! Polled RX only, single TX queue, no interrupts (mirrors the e1000 driver).

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::sync::atomic::{compiler_fence, Ordering};
use spin::Mutex;

use crate::arch::memory::{phys_to_virt, VirtAddr};
use crate::mm::numa::NumaNodeId;
use crate::mm::phys;
use crate::net::pci::{self, PciDevice};

use super::super::addr::MacAddr;
use super::super::buffer::PacketBuf;
use super::super::device::{NetDevice, NetError};

const REG_IDR0: u32 = 0x00;
const REG_TNPDS: u32 = 0x20;
const REG_CR: u32 = 0x37;
const REG_IMR: u32 = 0x3C;
const REG_ISR: u32 = 0x3E;
const REG_TCR: u32 = 0x40;
const REG_RCR: u32 = 0x44;
const REG_9346CR: u32 = 0x50;
const REG_CONFIG1: u32 = 0x52;
const REG_PHYAR: u32 = 0x60;
const REG_RMS: u32 = 0xDA;
const REG_C_PLUS_CR: u32 = 0xE0;
const REG_RDSAR: u32 = 0xE4;
const REG_MTPS: u32 = 0xEC;

const CR_RST: u8 = 1 << 4;
const CR_RE: u8 = 1 << 3;
const CR_TE: u8 = 1 << 2;

const RX_RING_LEN: usize = 32;
const TX_RING_LEN: usize = 8;
const RX_BUF_SIZE: usize = 2048;

#[repr(C, align(256))]
struct Descriptor {
    flags: u32,
    vlan: u32,
    buf_lo: u32,
    buf_hi: u32,
}

const DESC_OWN: u32 = 1 << 31;
const DESC_EOR: u32 = 1 << 30;
const DESC_FS: u32 = 1 << 29;
const DESC_LS: u32 = 1 << 28;

pub struct Rtl8169 {
    mmio: VirtAddr,
    mac: MacAddr,
    irq: u8,
    rx: *mut Descriptor,
    tx: *mut Descriptor,
    rx_bufs: Vec<(*mut u8, u64)>,
    tx_bufs: Vec<(*mut u8, u64)>,
    rx_index: Mutex<usize>,
    tx_index: Mutex<usize>,
    rx_queue: Mutex<Vec<PacketBuf>>,
}

unsafe impl Send for Rtl8169 {}
unsafe impl Sync for Rtl8169 {}

impl Rtl8169 {
    fn read8(&self, reg: u32) -> u8 {
        unsafe { core::ptr::read_volatile((self.mmio.as_u64() + reg as u64) as *const u8) }
    }
    fn write8(&self, reg: u32, val: u8) {
        unsafe { core::ptr::write_volatile((self.mmio.as_u64() + reg as u64) as *mut u8, val) };
    }
    fn read16(&self, reg: u32) -> u16 {
        unsafe { core::ptr::read_volatile((self.mmio.as_u64() + reg as u64) as *const u16) }
    }
    fn write16(&self, reg: u32, val: u16) {
        unsafe { core::ptr::write_volatile((self.mmio.as_u64() + reg as u64) as *mut u16, val) };
    }
    fn read32(&self, reg: u32) -> u32 {
        unsafe { core::ptr::read_volatile((self.mmio.as_u64() + reg as u64) as *const u32) }
    }
    fn write32(&self, reg: u32, val: u32) {
        unsafe { core::ptr::write_volatile((self.mmio.as_u64() + reg as u64) as *mut u32, val) };
    }
    fn write64(&self, reg: u32, val: u64) {
        self.write32(reg, val as u32);
        self.write32(reg + 4, (val >> 32) as u32);
    }

    fn read_mac(&mut self) {
        let mut mac = [0u8; 6];
        for i in 0..6 {
            mac[i] = self.read8(REG_IDR0 + i as u32);
        }
        self.mac = MacAddr(mac);
    }

    fn reset(&self) {
        self.write8(REG_CR, CR_RST);
        for _ in 0..1_000_000 {
            if self.read8(REG_CR) & CR_RST == 0 {
                return;
            }
            core::hint::spin_loop();
        }
    }

    fn setup_rings(&mut self) -> Option<()> {
        let (rx, rx_phys) = alloc_dma_pages(1)?;
        let (tx, tx_phys) = alloc_dma_pages(1)?;
        self.rx = rx as *mut Descriptor;
        self.tx = tx as *mut Descriptor;

        for i in 0..RX_RING_LEN {
            let (buf, buf_phys) = alloc_dma_pages(1)?;
            self.rx_bufs.push((buf, buf_phys));
            unsafe {
                let mut flags = DESC_OWN | (RX_BUF_SIZE as u32 & 0x3FFF);
                if i == RX_RING_LEN - 1 {
                    flags |= DESC_EOR;
                }
                *self.rx.add(i) = Descriptor {
                    flags,
                    vlan: 0,
                    buf_lo: buf_phys as u32,
                    buf_hi: (buf_phys >> 32) as u32,
                };
            }
        }
        for i in 0..TX_RING_LEN {
            self.tx_bufs.push(alloc_dma_pages(1)?);
            unsafe {
                let mut flags = 0u32;
                if i == TX_RING_LEN - 1 {
                    flags |= DESC_EOR;
                }
                *self.tx.add(i) = Descriptor {
                    flags,
                    vlan: 0,
                    buf_lo: 0,
                    buf_hi: 0,
                };
            }
        }
        compiler_fence(Ordering::SeqCst);
        self.write64(REG_RDSAR, rx_phys);
        self.write64(REG_TNPDS, tx_phys);
        Some(())
    }

    fn enable(&self) {
        // Unlock config registers
        self.write8(REG_9346CR, 0xC0);
        // C+ command register: enable RX/TX checksum offload, BIST off
        self.write16(REG_C_PLUS_CR, 0x2B);
        // Max RX packet size
        self.write16(REG_RMS, 0x1FFF);
        // Max TX packet size (in 128-byte units)
        self.write8(REG_MTPS, 0x3F);
        // RX config: accept broadcast + physical match + multicast, max DMA 1024B
        self.write32(REG_RCR, 0x0000_700E);
        // TX config: IFG normal, max DMA 1024B
        self.write32(REG_TCR, 0x0300_0700);
        // Disable all interrupts (we poll)
        self.write16(REG_IMR, 0);
        self.write16(REG_ISR, 0xFFFF);
        // Enable RX and TX
        self.write8(REG_CR, CR_RE | CR_TE);
        // Lock config
        self.write8(REG_9346CR, 0x00);
    }

    fn poll_rx_internal(&self) {
        let mut idx = self.rx_index.lock();
        loop {
            let desc = unsafe { &*self.rx.add(*idx) };
            if desc.flags & DESC_OWN != 0 {
                break;
            }
            let len = (desc.flags & 0x3FFF) as usize;
            let (buf, _) = self.rx_bufs[*idx];
            let mut pkt = PacketBuf::new();
            unsafe {
                pkt.extend_from_slice(core::slice::from_raw_parts(buf, len.min(2048)));
            }
            self.rx_queue.lock().push(pkt);

            // Return descriptor to the NIC.
            unsafe {
                let mut flags = DESC_OWN | (RX_BUF_SIZE as u32 & 0x3FFF);
                if *idx == RX_RING_LEN - 1 {
                    flags |= DESC_EOR;
                }
                (*self.rx.add(*idx)).flags = flags;
            }
            *idx = (*idx + 1) % RX_RING_LEN;
        }
    }
}

impl NetDevice for Rtl8169 {
    fn name(&self) -> &'static str {
        "rtl8169"
    }

    fn mac(&self) -> MacAddr {
        self.mac
    }

    fn send(&self, pkt: &PacketBuf) -> Result<(), NetError> {
        let mut idx = self.tx_index.lock();
        let desc = unsafe { &mut *self.tx.add(*idx) };
        if desc.flags & DESC_OWN != 0 {
            return Err(NetError::TxBusy);
        }
        let len = pkt.len().min(RX_BUF_SIZE);
        let (buf, buf_phys) = self.tx_bufs[*idx];
        unsafe {
            core::ptr::copy_nonoverlapping(pkt.data().as_ptr(), buf, len);
        }
        let mut flags = DESC_OWN | DESC_FS | DESC_LS | (len as u32 & 0x3FFF);
        if *idx == TX_RING_LEN - 1 {
            flags |= DESC_EOR;
        }
        desc.buf_lo = buf_phys as u32;
        desc.buf_hi = (buf_phys >> 32) as u32;
        desc.flags = flags;
        compiler_fence(Ordering::SeqCst);
        // Poll TX (NPQ bit at register 0x38)
        unsafe { core::ptr::write_volatile((self.mmio.as_u64() + 0x38) as *mut u8, 0x40) };
        *idx = (*idx + 1) % TX_RING_LEN;
        Ok(())
    }

    fn poll_rx(&self) -> Option<PacketBuf> {
        self.poll_rx_internal();
        self.rx_queue.lock().pop()
    }

    fn irq(&self) -> u8 {
        self.irq
    }
}

fn alloc_dma_pages(pages: u64) -> Option<(*mut u8, u64)> {
    let order = phys::order_for_count(pages);
    let frame = phys::alloc_frames(order, NumaNodeId::new(0))?;
    let hhdm = crate::boot_info().hhdm_offset;
    let virt = phys_to_virt(hhdm, frame.phys());
    unsafe { core::ptr::write_bytes(virt.as_mut_ptr::<u8>(), 0, (pages * 4096) as usize) };
    Some((virt.as_mut_ptr(), frame.phys().as_u64()))
}

pub fn init(pci_dev: PciDevice) -> Option<&'static Rtl8169> {
    // RTL8168/8111 uses BAR2 (64-bit MMIO); legacy 8169 uses BAR1 MMIO.
    let bar2 = pci_dev.bar(2);
    let bar1 = pci_dev.bar(1);
    let (mmio_phys, _is_64bit) = if bar2 != 0 && !pci_dev.bar_is_mmio(0) {
        (bar2, true)
    } else if bar1 != 0 && pci_dev.bar_is_mmio(1) {
        (bar1, false)
    } else {
        return None;
    };
    pci::map_mmio(mmio_phys, 4);
    let hhdm = crate::boot_info().hhdm_offset;
    let mmio = VirtAddr::new(hhdm + mmio_phys);

    let mut dev = Rtl8169 {
        mmio,
        mac: MacAddr::ZERO,
        irq: if pci_dev.irq_line > 0 { pci_dev.irq_line } else { 11 },
        rx: core::ptr::null_mut(),
        tx: core::ptr::null_mut(),
        rx_bufs: Vec::new(),
        tx_bufs: Vec::new(),
        rx_index: Mutex::new(0),
        tx_index: Mutex::new(0),
        rx_queue: Mutex::new(Vec::new()),
    };
    dev.reset();
    dev.read_mac();
    dev.setup_rings()?;
    dev.enable();
    Some(Box::leak(Box::new(dev)))
}

pub fn find_card() -> Option<PciDevice> {
    let mut result = None;
    pci::for_each(|dev| {
        if result.is_some() || dev.vendor_id != 0x10EC {
            return;
        }
        // Common Realtek 8169 family device IDs
        match dev.device_id {
            0x8129 | 0x8136 | 0x8161 | 0x8167 | 0x8168 | 0x8169 => {
                result = Some(*dev);
            }
            _ => {}
        }
    });
    result
}
