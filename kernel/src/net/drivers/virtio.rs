//! Virtio-net PCI driver (MMIO virtqueues).

use alloc::boxed::Box;
use alloc::vec::Vec;
use spin::Mutex;

use crate::arch::memory::{phys_to_virt, PhysAddr, VirtAddr};
use crate::arch::x86_64::interrupts::{self, InterruptFrame};
use crate::mm::layout::PAGE_SIZE;
use crate::mm::numa::NumaNodeId;
use crate::mm::phys;

use super::super::addr::MacAddr;
use super::super::buffer::PacketBuf;
use super::super::device::{NetDevice, NetError};
use super::super::pci::PciDevice;

const VIRTIO_MAGIC: u32 = 0x7472_6976;
const VIRTIO_DEV_NET: u32 = 1;
const REG_MAGIC: u32 = 0x000;
const REG_DEVICE: u32 = 0x008;
const REG_STATUS: u32 = 0x028;
const REG_QUEUE_SEL: u32 = 0x030;
const REG_QUEUE_NUM: u32 = 0x038;
const REG_QUEUE_READY: u32 = 0x044;
const REG_QUEUE_NOTIFY: u32 = 0x050;
const REG_ISR: u32 = 0x060;
const REG_MAC: u32 = 0x080;

const QUEUE_SIZE: u16 = 16;

#[repr(C)]
struct VirtqDesc {
    addr: u64,
    len: u32,
    flags: u16,
    next: u16,
}

#[repr(C)]
struct VirtqAvail {
    flags: u16,
    idx: u16,
    ring: [u16; QUEUE_SIZE as usize],
}

#[repr(C, align(4096))]
struct VirtqUsedElem {
    id: u32,
    len: u32,
}

pub struct VirtioNet {
    mmio: VirtAddr,
    mac: MacAddr,
    irq: u8,
    rx_queue: Mutex<Vec<PacketBuf>>,
    rx_desc: *mut VirtqDesc,
    tx_desc: *mut VirtqDesc,
    rx_avail: *mut VirtqAvail,
    tx_avail: *mut VirtqAvail,
    rx_bufs: Vec<(*mut u8, u64)>,
    tx_bufs: Vec<(*mut u8, u64)>,
}

static INSTANCE: Mutex<Option<&'static VirtioNet>> = Mutex::new(None);

unsafe impl Send for VirtioNet {}
unsafe impl Sync for VirtioNet {}

impl VirtioNet {
    fn new(pci: &PciDevice) -> Option<Self> {
        let hhdm = crate::boot_info().hhdm_offset;
        let mmio = phys_to_virt(hhdm, PhysAddr::new(pci.bar0));
        if unsafe { core::ptr::read_volatile(mmio.as_ptr::<u32>()) } != VIRTIO_MAGIC {
            return None;
        }
        let device = unsafe { core::ptr::read_volatile(mmio.as_ptr::<u32>().add(2)) };
        if device != VIRTIO_DEV_NET {
            return None;
        }
        unsafe {
            core::ptr::write_volatile(mmio.as_mut_ptr::<u32>().add(0xA), 0);
            core::ptr::write_volatile(mmio.as_mut_ptr::<u32>().add(0xA), 2);
            core::ptr::write_volatile(mmio.as_mut_ptr::<u32>().add(0xA), 6);
        }
        let mac_lo = unsafe { core::ptr::read_volatile(mmio.as_ptr::<u32>().add(REG_MAC as usize / 4)) };
        let mac_hi = unsafe { core::ptr::read_volatile(mmio.as_ptr::<u32>().add(REG_MAC as usize / 4 + 1)) };
        let mac = MacAddr([
            (mac_lo & 0xFF) as u8,
            ((mac_lo >> 8) & 0xFF) as u8,
            ((mac_lo >> 16) & 0xFF) as u8,
            ((mac_lo >> 24) & 0xFF) as u8,
            (mac_hi & 0xFF) as u8,
            ((mac_hi >> 8) & 0xFF) as u8,
        ]);
        let mut dev = Self {
            mmio,
            mac,
            irq: if pci.irq_line > 0 { pci.irq_line } else { 11 },
            rx_queue: Mutex::new(Vec::new()),
            rx_desc: core::ptr::null_mut(),
            tx_desc: core::ptr::null_mut(),
            rx_avail: core::ptr::null_mut(),
            tx_avail: core::ptr::null_mut(),
            rx_bufs: Vec::new(),
            tx_bufs: Vec::new(),
        };
        dev.setup_queue(0)?;
        dev.setup_queue(1)?;
        Some(dev)
    }

    fn alloc(size: usize) -> Option<(*mut u8, u64)> {
        let pages = (size + PAGE_SIZE as usize - 1) / PAGE_SIZE as usize;
        let frame = phys::alloc_frames(phys::order_for_count(pages as u64), NumaNodeId::new(0))?;
        let virt = phys_to_virt(crate::boot_info().hhdm_offset, frame.phys());
        Some((virt.as_mut_ptr(), frame.phys().as_u64()))
    }

    fn setup_queue(&mut self, q: u32) -> Option<()> {
        unsafe {
            core::ptr::write_volatile(self.mmio.as_mut_ptr::<u32>().add(REG_QUEUE_SEL as usize / 4), q);
            core::ptr::write_volatile(self.mmio.as_mut_ptr::<u32>().add(REG_QUEUE_NUM as usize / 4), QUEUE_SIZE as u32);
        }
        let (desc, _) = Self::alloc(core::mem::size_of::<VirtqDesc>() * QUEUE_SIZE as usize)?;
        let (avail, _) = Self::alloc(core::mem::size_of::<VirtqAvail>())?;
        if q == 0 {
            self.rx_desc = desc as *mut VirtqDesc;
            self.rx_avail = avail as *mut VirtqAvail;
            for i in 0..QUEUE_SIZE as usize {
                let (buf, buf_phys) = Self::alloc(2048)?;
                self.rx_bufs.push((buf, buf_phys));
                unsafe {
                    (*self.rx_desc.add(i)).addr = buf_phys;
                    (*self.rx_desc.add(i)).len = 2048;
                    (*self.rx_desc.add(i)).flags = 2; // WRITE
                    (*self.rx_avail).ring[i] = i as u16;
                }
            }
            unsafe {
                (*self.rx_avail).idx = QUEUE_SIZE;
            }
        } else {
            self.tx_desc = desc as *mut VirtqDesc;
            self.tx_avail = avail as *mut VirtqAvail;
            for _ in 0..QUEUE_SIZE as usize {
                self.tx_bufs.push(Self::alloc(2048)?);
            }
        }
        unsafe {
            core::ptr::write_volatile(self.mmio.as_mut_ptr::<u32>().add(REG_QUEUE_READY as usize / 4), 1);
        }
        Some(())
    }

    fn poll_rx_internal(&self) {
        let _ = unsafe { core::ptr::read_volatile(self.mmio.as_ptr::<u32>().add(REG_ISR as usize / 4)) };
        // Simplified: copy from first RX buffer slot
        if !self.rx_bufs.is_empty() {
            let (buf, _) = self.rx_bufs[0];
            let mut pkt = PacketBuf::new();
            unsafe {
                let slice = core::slice::from_raw_parts(buf, 64.min(2048));
                if slice.len() > 14 {
                    pkt.extend_from_slice(slice);
                    self.rx_queue.lock().push(pkt);
                }
            }
        }
    }
}

fn virtio_irq(_frame: &InterruptFrame) {
    if let Some(dev) = *INSTANCE.lock() {
        dev.poll_rx_internal();
        super::super::rx_poll();
    }
}

impl NetDevice for VirtioNet {
    fn name(&self) -> &'static str {
        "virtio-net"
    }

    fn mac(&self) -> MacAddr {
        self.mac
    }

    fn send(&self, pkt: &PacketBuf) -> Result<(), NetError> {
        if self.tx_bufs.is_empty() {
            return Err(NetError::Hardware);
        }
        let (buf, _) = self.tx_bufs[0];
        let len = pkt.len().min(2048);
        unsafe {
            core::ptr::copy_nonoverlapping(pkt.data().as_ptr(), buf, len);
            core::ptr::write_volatile(self.mmio.as_mut_ptr::<u32>().add(REG_QUEUE_NOTIFY as usize / 4), 1);
        }
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

pub fn init(pci: PciDevice) -> Option<&'static VirtioNet> {
    let dev = Box::leak(Box::new(VirtioNet::new(&pci)?));
    interrupts::register_irq_handler(dev.irq, virtio_irq);
    *INSTANCE.lock() = Some(dev);
    Some(dev)
}
