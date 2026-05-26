//! AHCI/SATA driver.
//!
//! Discovers SATA controllers via PCI class 0x01/0x06/0x01, initializes each
//! connected port, sends ATA IDENTIFY, and exposes read/write of 512-byte
//! sectors using DMA via command list + FIS receive area + PRD tables.
//!
//! Designed to be portable to real PCs (LBA48 from the start, no I/O-mapped
//! legacy IDE paths).

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::sync::atomic::{compiler_fence, Ordering};
use spin::Mutex;

use crate::arch::memory::{phys_to_virt, PhysAddr, VirtAddr};
use crate::console::Console;
use crate::mm::layout::PAGE_SIZE;
use crate::mm::numa::NumaNodeId;
use crate::mm::phys;
use crate::net::pci;

use super::{BlockDevice, BlockError, BLOCK_SIZE};

const SATA_SECTOR_SIZE: usize = 512;
const SECTORS_PER_BLOCK: usize = BLOCK_SIZE / SATA_SECTOR_SIZE;

// Global HBA registers (offsets from ABAR).
const HBA_CAP: u32 = 0x00;
const HBA_GHC: u32 = 0x04;
const HBA_IS: u32 = 0x08;
const HBA_PI: u32 = 0x0C;
const HBA_VS: u32 = 0x10;

const GHC_AE: u32 = 1 << 31;
const GHC_HR: u32 = 1 << 0;

// Per-port register offsets relative to port base (0x100 + port*0x80).
const P_CLB: u32 = 0x00;
const P_CLBU: u32 = 0x04;
const P_FB: u32 = 0x08;
const P_FBU: u32 = 0x0C;
const P_IS: u32 = 0x10;
const P_IE: u32 = 0x14;
const P_CMD: u32 = 0x18;
const P_TFD: u32 = 0x20;
const P_SIG: u32 = 0x24;
const P_SSTS: u32 = 0x28;
const P_SERR: u32 = 0x30;
const P_CI: u32 = 0x38;

const PCMD_ST: u32 = 1 << 0;
const PCMD_FRE: u32 = 1 << 4;
const PCMD_FR: u32 = 1 << 14;
const PCMD_CR: u32 = 1 << 15;

const SIG_ATA: u32 = 0x0000_0101;

const ATA_CMD_IDENTIFY: u8 = 0xEC;
const ATA_CMD_READ_DMA_EXT: u8 = 0x25;
const ATA_CMD_WRITE_DMA_EXT: u8 = 0x35;

const FIS_TYPE_REG_H2D: u8 = 0x27;

#[repr(C, align(1024))]
struct CommandList {
    headers: [CommandHeader; 32],
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CommandHeader {
    flags: u16,
    prdt_length: u16,
    prd_byte_count: u32,
    command_table_base_lo: u32,
    command_table_base_hi: u32,
    reserved: [u32; 4],
}

#[repr(C, align(256))]
struct FisReceive {
    dsfis: [u8; 0x1C],
    pad0: [u8; 4],
    psfis: [u8; 0x14],
    pad1: [u8; 12],
    rfis: [u8; 0x14],
    pad2: [u8; 4],
    sdbfis: [u8; 0x08],
    ufis: [u8; 0x40],
    reserved: [u8; 0x60],
}

#[repr(C, align(128))]
struct CommandTable {
    cfis: [u8; 64],
    acmd: [u8; 16],
    reserved: [u8; 48],
    prdt: [PrdtEntry; 8],
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct PrdtEntry {
    data_base_lo: u32,
    data_base_hi: u32,
    reserved: u32,
    /// Bits 0-21: byte-count-1; bit 31: interrupt-on-completion.
    dbc_i: u32,
}

pub struct AhciPort {
    mmio: VirtAddr,
    port_idx: u8,
    sector_count: u64,
    model: alloc::string::String,
    cmd_list: *mut CommandList,
    fis: *mut FisReceive,
    cmd_table: *mut CommandTable,
    cmd_table_phys: u64,
    bounce: *mut u8,
    bounce_phys: u64,
}

unsafe impl Send for AhciPort {}
unsafe impl Sync for AhciPort {}

impl AhciPort {
    fn port_base(&self) -> VirtAddr {
        VirtAddr::new(self.mmio.as_u64() + 0x100 + self.port_idx as u64 * 0x80)
    }

    fn read_reg(&self, off: u32) -> u32 {
        let base = self.port_base().as_u64();
        unsafe { core::ptr::read_volatile((base + off as u64) as *const u32) }
    }

    fn write_reg(&self, off: u32, val: u32) {
        let base = self.port_base().as_u64();
        unsafe { core::ptr::write_volatile((base + off as u64) as *mut u32, val) };
    }

    fn stop(&self) {
        let mut cmd = self.read_reg(P_CMD);
        cmd &= !(PCMD_ST | PCMD_FRE);
        self.write_reg(P_CMD, cmd);
        for _ in 0..1_000_000 {
            if self.read_reg(P_CMD) & (PCMD_FR | PCMD_CR) == 0 {
                break;
            }
            core::hint::spin_loop();
        }
    }

    fn start(&self) {
        while self.read_reg(P_CMD) & PCMD_CR != 0 {
            core::hint::spin_loop();
        }
        let mut cmd = self.read_reg(P_CMD);
        cmd |= PCMD_FRE | PCMD_ST;
        self.write_reg(P_CMD, cmd);
    }

    fn issue_command(&self, lba: u64, sectors: u16, write: bool) -> Result<(), BlockError> {
        self.write_reg(P_IS, 0xFFFF_FFFF);
        self.write_reg(P_SERR, 0xFFFF_FFFF);

        unsafe {
            let header = &mut (*self.cmd_list).headers[0];
            header.flags = (size_of_fis_h2d_dwords() & 0x1F) as u16;
            if write {
                header.flags |= 1 << 6;
            }
            header.prdt_length = 1;
            header.prd_byte_count = 0;
            header.command_table_base_lo = self.cmd_table_phys as u32;
            header.command_table_base_hi = (self.cmd_table_phys >> 32) as u32;

            let table = &mut *self.cmd_table;
            for b in table.cfis.iter_mut() {
                *b = 0;
            }
            table.cfis[0] = FIS_TYPE_REG_H2D;
            table.cfis[1] = 1 << 7; // command bit
            table.cfis[2] = if write {
                ATA_CMD_WRITE_DMA_EXT
            } else {
                ATA_CMD_READ_DMA_EXT
            };
            table.cfis[4] = lba as u8;
            table.cfis[5] = (lba >> 8) as u8;
            table.cfis[6] = (lba >> 16) as u8;
            table.cfis[7] = 1 << 6; // LBA mode
            table.cfis[8] = (lba >> 24) as u8;
            table.cfis[9] = (lba >> 32) as u8;
            table.cfis[10] = (lba >> 40) as u8;
            table.cfis[12] = sectors as u8;
            table.cfis[13] = (sectors >> 8) as u8;

            table.prdt[0] = PrdtEntry {
                data_base_lo: self.bounce_phys as u32,
                data_base_hi: (self.bounce_phys >> 32) as u32,
                reserved: 0,
                dbc_i: (sectors as u32 * SATA_SECTOR_SIZE as u32 - 1) | (1 << 31),
            };
        }

        compiler_fence(Ordering::SeqCst);
        self.write_reg(P_CI, 1);

        for _ in 0..50_000_000 {
            let ci = self.read_reg(P_CI);
            let is = self.read_reg(P_IS);
            if ci & 1 == 0 {
                if is & (1 << 30) != 0 {
                    return Err(BlockError::Io);
                }
                return Ok(());
            }
            if self.read_reg(P_TFD) & 1 != 0 {
                return Err(BlockError::Io);
            }
            core::hint::spin_loop();
        }
        Err(BlockError::Io)
    }

    fn identify(&mut self) -> Result<(), BlockError> {
        self.write_reg(P_IS, 0xFFFF_FFFF);
        unsafe {
            let header = &mut (*self.cmd_list).headers[0];
            header.flags = (size_of_fis_h2d_dwords() & 0x1F) as u16;
            header.prdt_length = 1;
            header.prd_byte_count = 0;
            header.command_table_base_lo = self.cmd_table_phys as u32;
            header.command_table_base_hi = (self.cmd_table_phys >> 32) as u32;

            let table = &mut *self.cmd_table;
            for b in table.cfis.iter_mut() {
                *b = 0;
            }
            table.cfis[0] = FIS_TYPE_REG_H2D;
            table.cfis[1] = 1 << 7;
            table.cfis[2] = ATA_CMD_IDENTIFY;

            table.prdt[0] = PrdtEntry {
                data_base_lo: self.bounce_phys as u32,
                data_base_hi: (self.bounce_phys >> 32) as u32,
                reserved: 0,
                dbc_i: (511) | (1 << 31),
            };
        }

        compiler_fence(Ordering::SeqCst);
        self.write_reg(P_CI, 1);
        for _ in 0..50_000_000 {
            let ci = self.read_reg(P_CI);
            if ci & 1 == 0 {
                break;
            }
            if self.read_reg(P_TFD) & 1 != 0 {
                return Err(BlockError::Io);
            }
            core::hint::spin_loop();
        }

        let data: &[u16; 256] = unsafe { &*(self.bounce as *const [u16; 256]) };
        // LBA48 sector count at words 100-103
        let lba48 = (data[100] as u64)
            | ((data[101] as u64) << 16)
            | ((data[102] as u64) << 32)
            | ((data[103] as u64) << 48);
        let lba28 = (data[60] as u64) | ((data[61] as u64) << 16);
        self.sector_count = if lba48 != 0 { lba48 } else { lba28 };

        // Model string at words 27..47, byte-swapped.
        let mut model = alloc::string::String::new();
        for i in 27..47usize {
            let w = data[i];
            model.push((w >> 8) as u8 as char);
            model.push((w & 0xFF) as u8 as char);
        }
        self.model = alloc::string::String::from(model.trim());
        Ok(())
    }
}

fn size_of_fis_h2d_dwords() -> u16 {
    5
}

impl BlockDevice for AhciPort {
    fn block_count(&self) -> u64 {
        self.sector_count / SECTORS_PER_BLOCK as u64
    }

    fn read_block(&self, block: u64, buf: &mut [u8]) -> Result<(), BlockError> {
        if block >= self.block_count() {
            return Err(BlockError::OutOfRange);
        }
        let lba = block * SECTORS_PER_BLOCK as u64;
        self.issue_command(lba, SECTORS_PER_BLOCK as u16, false)?;
        let len = buf.len().min(BLOCK_SIZE);
        unsafe {
            core::ptr::copy_nonoverlapping(self.bounce, buf.as_mut_ptr(), len);
        }
        Ok(())
    }

    fn write_block(&self, block: u64, buf: &[u8]) -> Result<(), BlockError> {
        if block >= self.block_count() {
            return Err(BlockError::OutOfRange);
        }
        let lba = block * SECTORS_PER_BLOCK as u64;
        let len = buf.len().min(BLOCK_SIZE);
        unsafe {
            core::ptr::copy_nonoverlapping(buf.as_ptr(), self.bounce, len);
            if len < BLOCK_SIZE {
                core::ptr::write_bytes(self.bounce.add(len), 0, BLOCK_SIZE - len);
            }
        }
        self.issue_command(lba, SECTORS_PER_BLOCK as u16, true)
    }
}

static PORTS: Mutex<Vec<&'static AhciPort>> = Mutex::new(Vec::new());

fn alloc_dma_pages(pages: u64) -> Option<(*mut u8, u64)> {
    let order = phys::order_for_count(pages);
    let frame = phys::alloc_frames(order, NumaNodeId::new(0))?;
    let hhdm = crate::boot_info().hhdm_offset;
    let virt = phys_to_virt(hhdm, frame.phys());
    unsafe {
        core::ptr::write_bytes(virt.as_mut_ptr::<u8>(), 0, (pages * PAGE_SIZE) as usize);
    }
    Some((virt.as_mut_ptr(), frame.phys().as_u64()))
}

fn virt_to_phys(virt: *mut u8) -> Option<u64> {
    let hhdm = crate::boot_info().hhdm_offset;
    Some(virt as u64 - hhdm)
}

fn init_port(mmio: VirtAddr, port_idx: u8) -> Option<&'static AhciPort> {
    let port_base = VirtAddr::new(mmio.as_u64() + 0x100 + port_idx as u64 * 0x80);
    let read = |off: u32| -> u32 {
        unsafe { core::ptr::read_volatile((port_base.as_u64() + off as u64) as *const u32) }
    };
    let write = |off: u32, val: u32| unsafe {
        core::ptr::write_volatile((port_base.as_u64() + off as u64) as *mut u32, val);
    };

    let ssts = read(P_SSTS);
    let det = ssts & 0x0F;
    let ipm = (ssts >> 8) & 0x0F;
    if det != 3 || ipm != 1 {
        return None;
    }
    let sig = read(P_SIG);
    if sig != SIG_ATA {
        return None;
    }

    // Stop port to reconfigure
    let mut cmd_reg = read(P_CMD);
    cmd_reg &= !(PCMD_ST | PCMD_FRE);
    write(P_CMD, cmd_reg);
    for _ in 0..1_000_000 {
        if read(P_CMD) & (PCMD_FR | PCMD_CR) == 0 {
            break;
        }
        core::hint::spin_loop();
    }

    let (cmd_list_ptr, _cmd_list_phys) = alloc_dma_pages(1)?;
    let cmd_list = cmd_list_ptr as *mut CommandList;
    let cmd_list_phys = virt_to_phys(cmd_list_ptr)?;
    let fis_phys = cmd_list_phys + 1024;
    let fis = (cmd_list_ptr as u64 + 1024) as *mut FisReceive;
    let cmd_table = (cmd_list_ptr as u64 + 2048) as *mut CommandTable;
    let cmd_table_phys = cmd_list_phys + 2048;
    let (bounce_ptr, bounce_phys) = alloc_dma_pages(1)?;

    write(P_CLB, cmd_list_phys as u32);
    write(P_CLBU, (cmd_list_phys >> 32) as u32);
    write(P_FB, fis_phys as u32);
    write(P_FBU, (fis_phys >> 32) as u32);
    write(P_SERR, 0xFFFF_FFFF);
    write(P_IS, 0xFFFF_FFFF);

    // Start FIS receive + command engine.
    while read(P_CMD) & PCMD_CR != 0 {
        core::hint::spin_loop();
    }
    let cmd_reg = read(P_CMD) | PCMD_FRE | PCMD_ST;
    write(P_CMD, cmd_reg);

    let mut port = Box::new(AhciPort {
        mmio,
        port_idx,
        sector_count: 0,
        model: alloc::string::String::new(),
        cmd_list,
        fis,
        cmd_table,
        cmd_table_phys,
        bounce: bounce_ptr,
        bounce_phys,
    });
    if port.identify().is_err() {
        return None;
    }
    let leaked: &'static AhciPort = Box::leak(port);
    Some(leaked)
}

pub fn probe() -> bool {
    let Some(pci_dev) = pci::find_by_class(0x01, 0x06, Some(0x01)) else {
        return false;
    };
    pci::enable_device(&pci_dev);
    let abar_phys = pci_dev.bar(5);
    if abar_phys == 0 {
        Console::println("[ahci] BAR5 not assigned");
        return false;
    }
    pci::map_mmio(abar_phys, 4);
    let hhdm = crate::boot_info().hhdm_offset;
    let mmio = VirtAddr::new(hhdm + abar_phys);

    unsafe {
        let ghc_ptr = (mmio.as_u64() + HBA_GHC as u64) as *mut u32;
        core::ptr::write_volatile(ghc_ptr, core::ptr::read_volatile(ghc_ptr) | GHC_AE);
    }

    let pi = unsafe { core::ptr::read_volatile((mmio.as_u64() + HBA_PI as u64) as *const u32) };
    let mut found = 0u32;
    for port_idx in 0..32u8 {
        if pi & (1 << port_idx) == 0 {
            continue;
        }
        if let Some(port) = init_port(mmio, port_idx) {
            Console::println(&alloc::format!(
                "[ahci] port {} online ({} MiB, {})",
                port_idx,
                port.sector_count * 512 / (1024 * 1024),
                port.model,
            ));
            PORTS.lock().push(port);
            found += 1;
        }
    }
    if found == 0 {
        Console::println("[ahci] no SATA devices detected");
        false
    } else {
        true
    }
}

pub fn primary() -> Option<&'static AhciPort> {
    PORTS.lock().first().copied()
}

pub fn port_count() -> usize {
    PORTS.lock().len()
}
