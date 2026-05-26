//! NVMe block driver — IDENTIFY, READ, WRITE on namespace 1.
//!
//! Discovers controllers via PCI class 0x01/0x08/0x02 (Non-Volatile Memory
//! Express). Sets up an admin queue pair, identifies the controller and
//! namespace, creates a single I/O queue pair, and exposes the namespace as
//! a [`BlockDevice`]. Polled completion only.

use alloc::boxed::Box;
use alloc::string::String;
use core::sync::atomic::{compiler_fence, Ordering};
use spin::Mutex;

use crate::arch::memory::{phys_to_virt, VirtAddr};
use crate::console::Console;
use crate::mm::numa::NumaNodeId;
use crate::mm::phys;
use crate::net::pci;

use super::{BlockDevice, BlockError, BLOCK_SIZE};

const QUEUE_DEPTH: u16 = 64;

const REG_CAP: u32 = 0x00;
const REG_CC: u32 = 0x14;
const REG_CSTS: u32 = 0x1C;
const REG_AQA: u32 = 0x24;
const REG_ASQ: u32 = 0x28;
const REG_ACQ: u32 = 0x30;

const CC_EN: u32 = 1 << 0;
const CSTS_RDY: u32 = 1 << 0;

const ADMIN_OPC_CREATE_IO_CQ: u8 = 0x05;
const ADMIN_OPC_CREATE_IO_SQ: u8 = 0x01;
const ADMIN_OPC_IDENTIFY: u8 = 0x06;

const NVM_OPC_WRITE: u8 = 0x01;
const NVM_OPC_READ: u8 = 0x02;

#[repr(C, align(64))]
#[derive(Clone, Copy, Default)]
struct SubmissionEntry {
    cdw0: u32,
    nsid: u32,
    cdw2: u32,
    cdw3: u32,
    mptr: u64,
    prp1: u64,
    prp2: u64,
    cdw10: u32,
    cdw11: u32,
    cdw12: u32,
    cdw13: u32,
    cdw14: u32,
    cdw15: u32,
}

#[repr(C, align(16))]
#[derive(Clone, Copy, Default)]
struct CompletionEntry {
    dw0: u32,
    dw1: u32,
    sq_head: u16,
    sq_id: u16,
    cid: u16,
    status: u16,
}

struct Queue {
    sq: *mut SubmissionEntry,
    cq: *mut CompletionEntry,
    sq_phys: u64,
    cq_phys: u64,
    depth: u16,
    sq_tail: u16,
    cq_head: u16,
    phase: u16,
    sq_doorbell: u32,
    cq_doorbell: u32,
}

unsafe impl Send for Queue {}

impl Queue {
    fn submit(&mut self, mmio: VirtAddr, entry: SubmissionEntry) -> u16 {
        let cid = self.sq_tail;
        unsafe {
            let mut entry = entry;
            entry.cdw0 = (entry.cdw0 & 0xFFFF) | ((cid as u32) << 16);
            *self.sq.add(self.sq_tail as usize) = entry;
        }
        self.sq_tail = (self.sq_tail + 1) % self.depth;
        compiler_fence(Ordering::SeqCst);
        unsafe {
            core::ptr::write_volatile(
                (mmio.as_u64() + self.sq_doorbell as u64) as *mut u32,
                self.sq_tail as u32,
            )
        };
        cid
    }

    fn wait_completion(&mut self, mmio: VirtAddr) -> Result<CompletionEntry, BlockError> {
        for _ in 0..50_000_000 {
            unsafe {
                let entry = *self.cq.add(self.cq_head as usize);
                if (entry.status & 1) == self.phase {
                    self.cq_head = (self.cq_head + 1) % self.depth;
                    if self.cq_head == 0 {
                        self.phase ^= 1;
                    }
                    core::ptr::write_volatile(
                        (mmio.as_u64() + self.cq_doorbell as u64) as *mut u32,
                        self.cq_head as u32,
                    );
                    if (entry.status >> 1) & 0xFF != 0 {
                        return Err(BlockError::Io);
                    }
                    return Ok(entry);
                }
            }
            core::hint::spin_loop();
        }
        Err(BlockError::Io)
    }
}

pub struct NvmeController {
    mmio: VirtAddr,
    doorbell_stride: u32,
    admin: Mutex<Queue>,
    io: Mutex<Queue>,
    block_size: u32,
    block_count: u64,
    bounce: *mut u8,
    bounce_phys: u64,
    model: String,
}

unsafe impl Send for NvmeController {}
unsafe impl Sync for NvmeController {}

impl NvmeController {
    fn read_u32(&self, off: u32) -> u32 {
        unsafe { core::ptr::read_volatile((self.mmio.as_u64() + off as u64) as *const u32) }
    }
    fn write_u32(&self, off: u32, v: u32) {
        unsafe { core::ptr::write_volatile((self.mmio.as_u64() + off as u64) as *mut u32, v) };
    }
    fn read_u64(&self, off: u32) -> u64 {
        unsafe { core::ptr::read_volatile((self.mmio.as_u64() + off as u64) as *const u64) }
    }
    fn write_u64(&self, off: u32, v: u64) {
        unsafe { core::ptr::write_volatile((self.mmio.as_u64() + off as u64) as *mut u64, v) };
    }
}

impl BlockDevice for NvmeController {
    fn block_count(&self) -> u64 {
        self.block_count * self.block_size as u64 / BLOCK_SIZE as u64
    }

    fn read_block(&self, block: u64, buf: &mut [u8]) -> Result<(), BlockError> {
        let lba = block * (BLOCK_SIZE / self.block_size as usize) as u64;
        let count = (BLOCK_SIZE / self.block_size as usize) as u16;
        self.transfer(lba, count, false)?;
        let len = buf.len().min(BLOCK_SIZE);
        unsafe { core::ptr::copy_nonoverlapping(self.bounce, buf.as_mut_ptr(), len) };
        Ok(())
    }

    fn write_block(&self, block: u64, buf: &[u8]) -> Result<(), BlockError> {
        let lba = block * (BLOCK_SIZE / self.block_size as usize) as u64;
        let count = (BLOCK_SIZE / self.block_size as usize) as u16;
        let len = buf.len().min(BLOCK_SIZE);
        unsafe {
            core::ptr::copy_nonoverlapping(buf.as_ptr(), self.bounce, len);
            if len < BLOCK_SIZE {
                core::ptr::write_bytes(self.bounce.add(len), 0, BLOCK_SIZE - len);
            }
        }
        self.transfer(lba, count, true)
    }
}

impl NvmeController {
    fn transfer(&self, lba: u64, count: u16, write: bool) -> Result<(), BlockError> {
        if count == 0 {
            return Ok(());
        }
        let mut io = self.io.lock();
        let entry = SubmissionEntry {
            cdw0: if write {
                NVM_OPC_WRITE as u32
            } else {
                NVM_OPC_READ as u32
            },
            nsid: 1,
            cdw2: 0,
            cdw3: 0,
            mptr: 0,
            prp1: self.bounce_phys,
            prp2: 0,
            cdw10: lba as u32,
            cdw11: (lba >> 32) as u32,
            cdw12: (count - 1) as u32,
            cdw13: 0,
            cdw14: 0,
            cdw15: 0,
        };
        io.submit(self.mmio, entry);
        io.wait_completion(self.mmio).map(|_| ())
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

static CONTROLLERS: Mutex<alloc::vec::Vec<&'static NvmeController>> = Mutex::new(alloc::vec::Vec::new());

pub fn probe() -> bool {
    match try_probe() {
        Some(()) => true,
        None => false,
    }
}

fn try_probe() -> Option<()> {
    let pci_dev = pci::find_by_class(0x01, 0x08, Some(0x02))?;
    pci::enable_device(&pci_dev);
    let bar0_phys = pci_dev.bar(0);
    if bar0_phys == 0 {
        Console::println("[nvme] BAR0 not assigned");
        return None;
    }
    pci::map_mmio(bar0_phys, 4);
    let hhdm = crate::boot_info().hhdm_offset;
    let mmio = VirtAddr::new(hhdm + bar0_phys);

    let mut ctrl = Box::new(NvmeController {
        mmio,
        doorbell_stride: 4,
        admin: Mutex::new(Queue {
            sq: core::ptr::null_mut(),
            cq: core::ptr::null_mut(),
            sq_phys: 0,
            cq_phys: 0,
            depth: QUEUE_DEPTH,
            sq_tail: 0,
            cq_head: 0,
            phase: 1,
            sq_doorbell: 0,
            cq_doorbell: 0,
        }),
        io: Mutex::new(Queue {
            sq: core::ptr::null_mut(),
            cq: core::ptr::null_mut(),
            sq_phys: 0,
            cq_phys: 0,
            depth: QUEUE_DEPTH,
            sq_tail: 0,
            cq_head: 0,
            phase: 1,
            sq_doorbell: 0,
            cq_doorbell: 0,
        }),
        block_size: 512,
        block_count: 0,
        bounce: core::ptr::null_mut(),
        bounce_phys: 0,
        model: String::new(),
    });

    let cap = ctrl.read_u64(REG_CAP);
    ctrl.doorbell_stride = 1 << (2 + ((cap >> 32) & 0xF));

    // Disable controller
    let mut cc = ctrl.read_u32(REG_CC);
    cc &= !CC_EN;
    ctrl.write_u32(REG_CC, cc);
    for _ in 0..500_000 {
        if ctrl.read_u32(REG_CSTS) & CSTS_RDY == 0 {
            break;
        }
        core::hint::spin_loop();
    }

    // Admin queues
    {
        let (sq_ptr, sq_phys) = alloc_dma_pages(1)?;
        let (cq_ptr, cq_phys) = alloc_dma_pages(1)?;
        let mut admin = ctrl.admin.lock();
        admin.sq = sq_ptr as *mut SubmissionEntry;
        admin.cq = cq_ptr as *mut CompletionEntry;
        admin.sq_phys = sq_phys;
        admin.cq_phys = cq_phys;
        admin.sq_doorbell = 0x1000;
        admin.cq_doorbell = 0x1000 + ctrl.doorbell_stride;
    }
    ctrl.write_u64(REG_ASQ, ctrl.admin.lock().sq_phys);
    ctrl.write_u64(REG_ACQ, ctrl.admin.lock().cq_phys);
    let aqa = ((QUEUE_DEPTH as u32 - 1) << 16) | (QUEUE_DEPTH as u32 - 1);
    ctrl.write_u32(REG_AQA, aqa);

    // Enable controller: IOSQES=6, IOCQES=4 (sizes 64/16), MPS=0, CSS=0, EN=1
    let cc = (6u32 << 16) | (4u32 << 20) | CC_EN;
    ctrl.write_u32(REG_CC, cc);
    let mut ready = false;
    for _ in 0..2_000_000 {
        if ctrl.read_u32(REG_CSTS) & CSTS_RDY != 0 {
            ready = true;
            break;
        }
        core::hint::spin_loop();
    }
    if !ready {
        Console::println("[nvme] controller failed to ready");
        return None;
    }

    // Identify namespace 1
    let (id_buf_ptr, id_buf_phys) = alloc_dma_pages(1)?;
    let identify_entry = SubmissionEntry {
        cdw0: ADMIN_OPC_IDENTIFY as u32,
        nsid: 1,
        prp1: id_buf_phys,
        cdw10: 0, // CNS=0 (namespace)
        ..Default::default()
    };
    {
        let mut admin = ctrl.admin.lock();
        admin.submit(ctrl.mmio, identify_entry);
        if admin.wait_completion(ctrl.mmio).is_err() {
            Console::println("[nvme] IDENTIFY NS failed");
            return None;
        }
    }
    let ns_data = unsafe { core::slice::from_raw_parts(id_buf_ptr, 4096) };
    let nsze = u64::from_le_bytes(ns_data[0..8].try_into().unwrap());
    let flbas = ns_data[26];
    let lba_idx = (flbas & 0x0F) as usize;
    // LBA format entries start at offset 128 each is 4 bytes; LBADS is byte 2 (lower 5 bits).
    let lbads = ns_data[128 + lba_idx * 4 + 2] & 0x1F;
    let block_size = 1u32 << lbads.max(9);
    ctrl.block_size = block_size;
    ctrl.block_count = nsze;

    // Identify controller (for model)
    let id_ctrl_entry = SubmissionEntry {
        cdw0: ADMIN_OPC_IDENTIFY as u32,
        nsid: 0,
        prp1: id_buf_phys,
        cdw10: 1, // CNS=1 (controller)
        ..Default::default()
    };
    {
        let mut admin = ctrl.admin.lock();
        admin.submit(ctrl.mmio, id_ctrl_entry);
        let _ = admin.wait_completion(ctrl.mmio);
    }
    let model_bytes = unsafe { core::slice::from_raw_parts(id_buf_ptr.add(24), 40) };
    ctrl.model = String::from(
        core::str::from_utf8(model_bytes)
            .unwrap_or("")
            .trim_end_matches('\0')
            .trim(),
    );

    // Create I/O completion queue (id=1)
    {
        let (sq_ptr, sq_phys) = alloc_dma_pages(1)?;
        let (cq_ptr, cq_phys) = alloc_dma_pages(1)?;
        let mut io = ctrl.io.lock();
        io.sq = sq_ptr as *mut SubmissionEntry;
        io.cq = cq_ptr as *mut CompletionEntry;
        io.sq_phys = sq_phys;
        io.cq_phys = cq_phys;
        io.sq_doorbell = 0x1000 + 2 * ctrl.doorbell_stride;
        io.cq_doorbell = 0x1000 + 3 * ctrl.doorbell_stride;
    }

    let create_cq = SubmissionEntry {
        cdw0: ADMIN_OPC_CREATE_IO_CQ as u32,
        prp1: ctrl.io.lock().cq_phys,
        cdw10: ((QUEUE_DEPTH as u32 - 1) << 16) | 1,
        cdw11: 1, // PC=1 (physically contiguous)
        ..Default::default()
    };
    {
        let mut admin = ctrl.admin.lock();
        admin.submit(ctrl.mmio, create_cq);
        if admin.wait_completion(ctrl.mmio).is_err() {
            Console::println("[nvme] CREATE_IO_CQ failed");
            return None;
        }
    }
    let create_sq = SubmissionEntry {
        cdw0: ADMIN_OPC_CREATE_IO_SQ as u32,
        prp1: ctrl.io.lock().sq_phys,
        cdw10: ((QUEUE_DEPTH as u32 - 1) << 16) | 1,
        cdw11: (1u32 << 16) | 1, // CQID=1, PC=1
        ..Default::default()
    };
    {
        let mut admin = ctrl.admin.lock();
        admin.submit(ctrl.mmio, create_sq);
        if admin.wait_completion(ctrl.mmio).is_err() {
            Console::println("[nvme] CREATE_IO_SQ failed");
            return None;
        }
    }

    let (bounce, bounce_phys) = alloc_dma_pages(1)?;
    ctrl.bounce = bounce;
    ctrl.bounce_phys = bounce_phys;

    Console::println(&alloc::format!(
        "[nvme] {} — {} blocks of {} bytes ({} MiB)",
        ctrl.model,
        ctrl.block_count,
        ctrl.block_size,
        ctrl.block_count * ctrl.block_size as u64 / (1024 * 1024)
    ));

    let leaked: &'static NvmeController = Box::leak(ctrl);
    CONTROLLERS.lock().push(leaked);
    Some(())
}

pub fn primary() -> Option<&'static NvmeController> {
    CONTROLLERS.lock().first().copied()
}

pub fn controller_count() -> usize {
    CONTROLLERS.lock().len()
}
