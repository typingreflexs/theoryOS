//! Intel High Definition Audio controller driver.
//!
//! Detects HDA controllers via PCI class 0x04/0x03, resets the link, brings
//! up CORB/RIRB ring buffers, scans STATESTS for present codecs, and
//! enumerates each codec's function groups and audio output widgets via
//! `GetParameter` verbs.
//!
//! This module establishes everything needed to *initiate* a stream — a
//! follow-up will plumb a PCM ring buffer + Stream Descriptor for actual
//! playback. The codec map is exposed so a future userspace audio daemon
//! can choose an output.

use alloc::boxed::Box;
use alloc::format;
use alloc::vec::Vec;
use core::sync::atomic::{compiler_fence, Ordering};
use spin::Mutex;

use crate::arch::memory::{phys_to_virt, VirtAddr};
use crate::console::Console;
use crate::mm::numa::NumaNodeId;
use crate::mm::phys;
use crate::net::pci;

const REG_GCAP: u32 = 0x00;
const REG_GCTL: u32 = 0x08;
const REG_WAKEEN: u32 = 0x0C;
const REG_STATESTS: u32 = 0x0E;
const REG_INTCTL: u32 = 0x20;
const REG_CORBLBASE: u32 = 0x40;
const REG_CORBUBASE: u32 = 0x44;
const REG_CORBWP: u32 = 0x48;
const REG_CORBRP: u32 = 0x4A;
const REG_CORBCTL: u32 = 0x4C;
const REG_CORBSIZE: u32 = 0x4E;
const REG_RIRBLBASE: u32 = 0x50;
const REG_RIRBUBASE: u32 = 0x54;
const REG_RIRBWP: u32 = 0x58;
const REG_RINTCNT: u32 = 0x5A;
const REG_RIRBCTL: u32 = 0x5C;
const REG_RIRBSIZE: u32 = 0x5E;

const GCTL_CRST: u32 = 1 << 0;

const CORB_RUN: u8 = 1 << 1;
const RIRB_RUN: u8 = 1 << 1;
const RIRB_DMAEN: u8 = 1 << 1;

const VERB_GET_PARAM: u32 = 0x0F00;
const PARAM_NODE_COUNT: u8 = 0x04;
const PARAM_FUNC_GROUP_TYPE: u8 = 0x05;
const PARAM_AUDIO_WIDGET_CAPS: u8 = 0x09;

const FN_GROUP_AUDIO: u8 = 0x01;

#[derive(Clone, Copy, Debug)]
pub struct CodecInfo {
    pub address: u8,
    pub vendor: u16,
    pub device: u16,
    pub output_count: u8,
}

pub struct HdaController {
    mmio: VirtAddr,
    corb: *mut u32,
    rirb: *mut u64,
    corb_phys: u64,
    rirb_phys: u64,
    corb_size: u16,
    rirb_size: u16,
    corb_wp: Mutex<u16>,
    rirb_rp: Mutex<u16>,
    codecs: Vec<CodecInfo>,
}

unsafe impl Send for HdaController {}
unsafe impl Sync for HdaController {}

impl HdaController {
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

    fn reset(&self) -> bool {
        // Bring controller out of reset.
        self.write32(REG_GCTL, GCTL_CRST);
        for _ in 0..1_000_000 {
            if self.read32(REG_GCTL) & GCTL_CRST != 0 {
                break;
            }
            core::hint::spin_loop();
        }
        // Wait 521 µs (per spec) for codecs to come up.
        spin_delay_us(600);
        true
    }

    fn setup_rings(&mut self) -> Option<()> {
        let (corb_ptr, corb_phys) = alloc_dma_pages(1)?;
        let (rirb_ptr, rirb_phys) = alloc_dma_pages(1)?;
        self.corb = corb_ptr as *mut u32;
        self.rirb = rirb_ptr as *mut u64;
        self.corb_phys = corb_phys;
        self.rirb_phys = rirb_phys;

        // Stop CORB/RIRB engines first
        self.write8(REG_CORBCTL, 0);
        self.write8(REG_RIRBCTL, 0);

        // Read supported size (bits 4-6 of CORBSIZE, encoded)
        // Default to 256-entry CORB / 256-entry RIRB.
        self.corb_size = 256;
        self.rirb_size = 256;

        self.write32(REG_CORBLBASE, corb_phys as u32);
        self.write32(REG_CORBUBASE, (corb_phys >> 32) as u32);
        self.write32(REG_RIRBLBASE, rirb_phys as u32);
        self.write32(REG_RIRBUBASE, (rirb_phys >> 32) as u32);

        self.write8(REG_CORBSIZE, 0x02); // 256 entries
        self.write8(REG_RIRBSIZE, 0x02);

        // Reset CORB read pointer
        self.write16(REG_CORBRP, 1 << 15);
        for _ in 0..1_000_000 {
            if self.read16(REG_CORBRP) & (1 << 15) != 0 {
                break;
            }
            core::hint::spin_loop();
        }
        self.write16(REG_CORBRP, 0);
        self.write16(REG_CORBWP, 0);

        // Reset RIRB write pointer
        self.write16(REG_RIRBWP, 1 << 15);
        self.write16(REG_RINTCNT, 0xFF);

        self.write8(REG_CORBCTL, CORB_RUN);
        self.write8(REG_RIRBCTL, RIRB_DMAEN);
        Some(())
    }

    fn send_verb(&self, codec: u8, node: u8, verb: u32, data: u8) -> Option<u32> {
        let cmd = ((codec as u32) << 28)
            | ((node as u32) << 20)
            | ((verb & 0xFFFFF) << 8)
            | data as u32;
        let mut wp = self.corb_wp.lock();
        *wp = (*wp + 1) % self.corb_size;
        unsafe {
            *self.corb.add(*wp as usize) = cmd;
        }
        compiler_fence(Ordering::SeqCst);
        self.write16(REG_CORBWP, *wp);

        // Poll RIRB for response.
        for _ in 0..1_000_000 {
            let rirb_wp = self.read16(REG_RIRBWP) & 0xFF;
            let mut rp = self.rirb_rp.lock();
            if rirb_wp != *rp {
                *rp = (*rp + 1) % self.rirb_size;
                let resp = unsafe { *self.rirb.add(*rp as usize) };
                return Some(resp as u32);
            }
            drop(rp);
            core::hint::spin_loop();
        }
        None
    }

    fn enumerate_codecs(&mut self, statests: u16) {
        for addr in 0..15u8 {
            if statests & (1 << addr) == 0 {
                continue;
            }
            let Some(vendor_word) =
                self.send_verb(addr, 0, VERB_GET_PARAM, 0x00)
            else {
                continue;
            };
            let vendor = (vendor_word >> 16) as u16;
            let device = (vendor_word & 0xFFFF) as u16;

            let Some(node_word) =
                self.send_verb(addr, 0, VERB_GET_PARAM, PARAM_NODE_COUNT)
            else {
                continue;
            };
            let starting = (node_word >> 16) as u8;
            let count = (node_word & 0xFF) as u8;

            let mut outputs = 0u8;
            for offset in 0..count {
                let node = starting + offset;
                let Some(fg_word) =
                    self.send_verb(addr, node, VERB_GET_PARAM, PARAM_FUNC_GROUP_TYPE)
                else {
                    continue;
                };
                if (fg_word & 0xFF) as u8 != FN_GROUP_AUDIO {
                    continue;
                }
                // Sub-node range of this function group
                let Some(sub_word) =
                    self.send_verb(addr, node, VERB_GET_PARAM, PARAM_NODE_COUNT)
                else {
                    continue;
                };
                let sub_start = (sub_word >> 16) as u8;
                let sub_count = (sub_word & 0xFF) as u8;
                for sub in 0..sub_count {
                    let widget = sub_start + sub;
                    if let Some(caps) =
                        self.send_verb(addr, widget, VERB_GET_PARAM, PARAM_AUDIO_WIDGET_CAPS)
                    {
                        let widget_type = (caps >> 20) & 0xF;
                        if widget_type == 0 {
                            outputs += 1;
                        }
                    }
                }
            }
            self.codecs.push(CodecInfo {
                address: addr,
                vendor,
                device,
                output_count: outputs,
            });
        }
    }
}

fn spin_delay_us(us: u32) {
    let target = crate::sched::timer::monotonic_ns() + (us as u64) * 1000;
    while crate::sched::timer::monotonic_ns() < target {
        core::hint::spin_loop();
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

static CONTROLLER: Mutex<Option<&'static HdaController>> = Mutex::new(None);

pub fn probe() -> bool {
    let Some(pci_dev) = pci::find_by_class(0x04, 0x03, None) else {
        return false;
    };
    pci::enable_device(&pci_dev);
    let mmio_phys = pci_dev.bar(0);
    if mmio_phys == 0 {
        return false;
    }
    pci::map_mmio(mmio_phys, 4);
    let hhdm = crate::boot_info().hhdm_offset;
    let mmio = VirtAddr::new(hhdm + mmio_phys);

    let mut ctrl = Box::new(HdaController {
        mmio,
        corb: core::ptr::null_mut(),
        rirb: core::ptr::null_mut(),
        corb_phys: 0,
        rirb_phys: 0,
        corb_size: 0,
        rirb_size: 0,
        corb_wp: Mutex::new(0),
        rirb_rp: Mutex::new(0),
        codecs: Vec::new(),
    });

    if !ctrl.reset() {
        Console::println("[hda] reset failed");
        return false;
    }
    if ctrl.setup_rings().is_none() {
        Console::println("[hda] ring allocation failed");
        return false;
    }
    let statests = ctrl.read16(REG_STATESTS);
    if statests == 0 {
        Console::println("[hda] no codecs detected");
        let leaked: &'static HdaController = Box::leak(ctrl);
        *CONTROLLER.lock() = Some(leaked);
        return true;
    }
    ctrl.enumerate_codecs(statests);

    for codec in &ctrl.codecs {
        Console::println(&format!(
            "[hda] codec @ {} vendor={:04X} device={:04X} outputs={}",
            codec.address, codec.vendor, codec.device, codec.output_count
        ));
    }

    let leaked: &'static HdaController = Box::leak(ctrl);
    *CONTROLLER.lock() = Some(leaked);
    true
}

pub fn codecs() -> Vec<CodecInfo> {
    CONTROLLER
        .lock()
        .map(|c| c.codecs.clone())
        .unwrap_or_default()
}

pub fn has_controller() -> bool {
    CONTROLLER.lock().is_some()
}
