//! Intel e1000 (82540EM) NIC driver.

use alloc::boxed::Box;
use alloc::vec::Vec;
use spin::Mutex;

use crate::arch::memory::{phys_to_virt, PhysAddr, VirtAddr};
use crate::mm::layout::PAGE_SIZE;
use crate::mm::numa::NumaNodeId;
use crate::mm::phys;

use super::super::addr::MacAddr;
use super::super::buffer::PacketBuf;
use super::super::device::{NetDevice, NetError};
use super::super::pci::PciDevice;

const REG_CTRL: u32 = 0x0000;
const REG_RCTL: u32 = 0x0100;
const REG_TCTL: u32 = 0x0400;
const REG_TIPG: u32 = 0x0410;
const REG_RDBAL: u32 = 0x2800;
const REG_RDBAH: u32 = 0x2804;
const REG_RDLEN: u32 = 0x2808;
const REG_RDH: u32 = 0x2810;
const REG_RDT: u32 = 0x2818;
const REG_TDBAL: u32 = 0x3800;
const REG_TDBAH: u32 = 0x3804;
const REG_TDLEN: u32 = 0x3808;
const REG_TDH: u32 = 0x3810;
const REG_TDT: u32 = 0x3818;
const REG_ICR: u32 = 0x00C0;
const REG_IMS: u32 = 0x00D0;
const REG_RAL: u32 = 0x5400;
const REG_RAH: u32 = 0x5404;

const RX_RING_SIZE: usize = 16;
const TX_RING_SIZE: usize = 16;

#[repr(C, align(16))]
struct RxDesc {
    addr: u64,
    len: u16,
    csum: u16,
    status: u8,
    errors: u8,
    special: u16,
}

#[repr(C, align(16))]
struct TxDesc {
    addr: u64,
    len: u16,
    csum: u8,
    cmd: u8,
    status: u8,
    css: u8,
    special: u16,
}

pub struct E1000 {
    mmio: VirtAddr,
    mac: MacAddr,
    irq: u8,
    rx_descs: *mut RxDesc,
    tx_descs: *mut TxDesc,
    rx_bufs: Vec<(*mut u8, u64)>,
    tx_bufs: Vec<(*mut u8, u64)>,
    rx_queue: Mutex<Vec<PacketBuf>>,
}

static INSTANCE: Mutex<Option<&'static E1000>> = Mutex::new(None);

// SAFETY: MMIO pointers are owned exclusively by the driver on CPU 0 IRQ path.
unsafe impl Send for E1000 {}
unsafe impl Sync for E1000 {}

impl E1000 {
    fn new(pci: &PciDevice) -> Option<Self> {
        if !pci.bar0_is_mmio {
            return None;
        }
        let hhdm = crate::boot_info().hhdm_offset;
        let mmio = phys_to_virt(hhdm, PhysAddr::new(pci.bar0));
        let mut dev = Self {
            mmio,
            mac: MacAddr::ZERO,
            irq: if pci.irq_line > 0 { pci.irq_line } else { 11 },
            rx_descs: core::ptr::null_mut(),
            tx_descs: core::ptr::null_mut(),
            rx_bufs: Vec::new(),
            tx_bufs: Vec::new(),
            rx_queue: Mutex::new(Vec::new()),
        };
        dev.reset();
        dev.read_mac();
        dev.setup_rings()?;
        dev.enable();
        Some(dev)
    }

    fn read_reg(&self, reg: u32) -> u32 {
        unsafe { core::ptr::read_volatile(self.mmio.as_mut_ptr::<u32>().add((reg / 4) as usize)) }
    }

    fn write_reg(&self, reg: u32, val: u32) {
        unsafe {
            core::ptr::write_volatile(self.mmio.as_mut_ptr::<u32>().add((reg / 4) as usize), val);
        }
    }

    fn reset(&mut self) {
        self.write_reg(REG_CTRL, self.read_reg(REG_CTRL) | 0x0400_0000);
        for _ in 0..100_000 {
            if self.read_reg(REG_CTRL) & 0x0400_0000 == 0 {
                break;
            }
            core::hint::spin_loop();
        }
    }

    fn read_mac(&mut self) {
        let ral = self.read_reg(REG_RAL);
        let rah = self.read_reg(REG_RAH);
        self.mac = MacAddr([
            (ral & 0xFF) as u8,
            ((ral >> 8) & 0xFF) as u8,
            ((ral >> 16) & 0xFF) as u8,
            ((ral >> 24) & 0xFF) as u8,
            (rah & 0xFF) as u8,
            ((rah >> 8) & 0xFF) as u8,
        ]);
    }

    fn alloc_dma(size: usize) -> Option<(*mut u8, u64)> {
        let pages = (size + PAGE_SIZE as usize - 1) / PAGE_SIZE as usize;
        let frame = phys::alloc_frames(phys::order_for_count(pages as u64), NumaNodeId::new(0))?;
        let hhdm = crate::boot_info().hhdm_offset;
        let virt = phys_to_virt(hhdm, frame.phys());
        unsafe {
            core::ptr::write_bytes(virt.as_mut_ptr::<u8>(), 0, size);
        }
        Some((virt.as_mut_ptr(), frame.phys().as_u64()))
    }

    fn setup_rings(&mut self) -> Option<()> {
        let (rx_ptr, rx_phys) = Self::alloc_dma(RX_RING_SIZE * core::mem::size_of::<RxDesc>())?;
        let (tx_ptr, tx_phys) = Self::alloc_dma(TX_RING_SIZE * core::mem::size_of::<TxDesc>())?;
        self.rx_descs = rx_ptr as *mut RxDesc;
        self.tx_descs = tx_ptr as *mut TxDesc;

        for i in 0..RX_RING_SIZE {
            let (buf, buf_phys) = Self::alloc_dma(2048)?;
            self.rx_bufs.push((buf, buf_phys));
            unsafe {
                (*self.rx_descs.add(i)).addr = buf_phys;
                (*self.rx_descs.add(i)).status = 0;
            }
        }
        for _ in 0..TX_RING_SIZE {
            self.tx_bufs.push(Self::alloc_dma(2048)?);
        }

        self.write_reg(REG_RDBAL, rx_phys as u32);
        self.write_reg(REG_RDBAH, (rx_phys >> 32) as u32);
        self.write_reg(REG_RDLEN, (RX_RING_SIZE * core::mem::size_of::<RxDesc>()) as u32);
        self.write_reg(REG_RDH, 0);
        self.write_reg(REG_RDT, (RX_RING_SIZE - 1) as u32);
        self.write_reg(REG_TDBAL, tx_phys as u32);
        self.write_reg(REG_TDBAH, (tx_phys >> 32) as u32);
        self.write_reg(REG_TDLEN, (TX_RING_SIZE * core::mem::size_of::<TxDesc>()) as u32);
        self.write_reg(REG_TDH, 0);
        self.write_reg(REG_TDT, 0);
        self.write_reg(REG_TIPG, 0x0060200A);
        Some(())
    }

    fn enable(&self) {
        self.write_reg(REG_RCTL, 0x0400_8020);
        self.write_reg(REG_TCTL, 0x0000_0106);
        self.write_reg(REG_IMS, 0);
        self.write_reg(REG_CTRL, self.read_reg(REG_CTRL) | 0x0000_0006);
    }

    fn poll_rx_internal(&self) {
        let mut tail = self.read_reg(REG_RDT);
        loop {
            let next = (tail + 1) % RX_RING_SIZE as u32;
            let desc = unsafe { &*self.rx_descs.add(next as usize) };
            if desc.status & 1 == 0 {
                break;
            }
            let len = desc.len as usize;
            let (buf, _) = self.rx_bufs[next as usize];
            let mut pkt = PacketBuf::new();
            unsafe {
                pkt.extend_from_slice(core::slice::from_raw_parts(buf, len.min(2048)));
            }
            self.rx_queue.lock().push(pkt);
            unsafe {
                (*self.rx_descs.add(next as usize)).status = 0;
            }
            tail = next;
        }
        self.write_reg(REG_RDT, tail);
    }
}

impl NetDevice for E1000 {
    fn name(&self) -> &'static str {
        "e1000"
    }

    fn mac(&self) -> MacAddr {
        self.mac
    }

    fn send(&self, pkt: &PacketBuf) -> Result<(), NetError> {
        let tdt = self.read_reg(REG_TDT) as usize % TX_RING_SIZE;
        if unsafe { (*self.tx_descs.add(tdt)).status & 1 } != 0 {
            return Err(NetError::TxBusy);
        }
        let len = pkt.len().min(2048);
        let (buf, buf_phys) = self.tx_bufs[tdt];
        unsafe {
            core::ptr::copy_nonoverlapping(pkt.data().as_ptr(), buf, len);
            (*self.tx_descs.add(tdt)).addr = buf_phys;
            (*self.tx_descs.add(tdt)).len = len as u16;
            (*self.tx_descs.add(tdt)).cmd = 0x0B;
            (*self.tx_descs.add(tdt)).status = 0;
        }
        self.write_reg(REG_TDT, ((tdt + 1) % TX_RING_SIZE) as u32);
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

pub fn init(pci: PciDevice) -> Option<&'static E1000> {
    let dev = Box::leak(Box::new(E1000::new(&pci)?));
    *INSTANCE.lock() = Some(dev);
    Some(dev)
}
