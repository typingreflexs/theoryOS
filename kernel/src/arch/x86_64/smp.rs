use core::fmt::Write;
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use spin::Mutex;

use crate::arch::traits::Arch;
use crate::arch::x86_64::cpu;
use crate::arch::x86_64::ioapic;
use crate::arch::x86_64::lapic;
use crate::arch::x86_64::tss::MAX_CPUS;
use crate::arch::x86_64::X86_64;
use crate::boot::limine::LimineRequests;
use crate::console::Console;

const TRAMPOLINE_PHYS: u64 = 0x1000;
const TRAMPOLINE_SIZE: usize = 4096;

#[derive(Clone, Copy, Debug)]
struct CpuRecord {
    lapic_id: u32,
    processor_id: u32,
    cpu_index: u32,
    online: bool,
}

static CPU_TABLE: Mutex<[Option<CpuRecord>; MAX_CPUS]> = Mutex::new([None; MAX_CPUS]);
static CPU_COUNT: AtomicU32 = AtomicU32::new(0);
static ONLINE_COUNT: AtomicU32 = AtomicU32::new(1);
static BSP_LAPIC: AtomicU32 = AtomicU32::new(0);
static CURRENT_CPU: AtomicU32 = AtomicU32::new(0);
static SMP_READY: AtomicBool = AtomicBool::new(false);

static mut AP_STACKS: [[u8; 16384]; MAX_CPUS] = [[0; 16384]; MAX_CPUS];

pub fn bsp_lapic_id() -> u32 {
    BSP_LAPIC.load(Ordering::Acquire)
}

pub fn current_cpu_index() -> u32 {
    CURRENT_CPU.load(Ordering::Acquire)
}

pub fn cpu_count() -> u32 {
    CPU_COUNT.load(Ordering::Acquire).max(1)
}

pub fn bootstrap_application_processors() -> ! {
    build_cpu_table();
    SMP_READY.store(true, Ordering::Release);

    let count = CPU_COUNT.load(Ordering::Acquire);
    let bsp = BSP_LAPIC.load(Ordering::Acquire);

    install_trampoline();

    for idx in 0..count as usize {
        let Some(record) = CPU_TABLE.lock()[idx] else { continue };
        if record.lapic_id == bsp {
            CURRENT_CPU.store(record.cpu_index, Ordering::Release);
            continue;
        }

        wake_ap(record);
        wait_for_ap(record.cpu_index);
    }

    lapic::calibrate_timer();
    ioapic::route_irqs();
    crate::sched::init();

    let online = ONLINE_COUNT.load(Ordering::Acquire);
    let _ = writeln!(
        Console,
        "[smp] {online}/{count} logical CPUs online (BSP lapic={bsp})"
    );

    crate::sched::start_cpu(0)
}

fn build_cpu_table() {
    let mut table = CPU_TABLE.lock();
    let mut index = 0u32;

    let bsp_from_limine = crate::boot_info().bsp_lapic_id;
    BSP_LAPIC.store(bsp_from_limine, Ordering::Release);
    lapic::set_bsp_lapic_id(bsp_from_limine);

    if let Some(ctx) = crate::acpi::context() {
        for lapic in ctx.madt().local_apics() {
            if !lapic.enabled {
                continue;
            }
            let idx = index as usize;
            if idx >= MAX_CPUS {
                break;
            }
            table[idx] = Some(CpuRecord {
                lapic_id: lapic.apic_id as u32,
                processor_id: lapic.processor_id as u32,
                cpu_index: index,
                online: lapic.apic_id as u32 == bsp_from_limine,
            });
            index += 1;
        }
    }

    if index == 0 {
        // Fallback to Limine SMP response if MADT unavailable.
        unsafe {
            let resp = LimineRequests::smp().response;
            if !resp.is_null() {
                let bsp = (*resp).bsp_lapic_id;
                BSP_LAPIC.store(bsp, Ordering::Release);
                let cpus = core::slice::from_raw_parts((*resp).cpus, (*resp).cpu_count as usize);
                for (i, cpu_ptr) in cpus.iter().enumerate() {
                    if cpu_ptr.is_null() || i >= MAX_CPUS {
                        break;
                    }
                    let info = &**cpu_ptr;
                    table[i] = Some(CpuRecord {
                        lapic_id: info.lapic_id,
                        processor_id: info.processor_id,
                        cpu_index: i as u32,
                        online: info.lapic_id == bsp,
                    });
                    index = (i + 1) as u32;
                }
            }
        }
    }

    if index == 0 {
        table[0] = Some(CpuRecord {
            lapic_id: bsp_from_limine,
            processor_id: 0,
            cpu_index: 0,
            online: true,
        });
        index = 1;
    }

    CPU_COUNT.store(index, Ordering::Release);
}

const TRAMPOLINE_CR3_OFF: u64 = 0x140;
const CR3_OFFSET: usize = 0x140;

fn install_trampoline() {
    super::trampoline::init_template();

    let hhdm = crate::boot_info().hhdm_offset;
    let dst = (TRAMPOLINE_PHYS + hhdm) as *mut u8;
    let src = super::trampoline::image() as *const u8;
    let size = super::trampoline::image_size().min(TRAMPOLINE_SIZE);

    unsafe {
        core::ptr::copy_nonoverlapping(src, dst, size);
        let cr3 = cpu::read_cr3();
        super::trampoline::patch(cr3, 0, 0, 0);
        core::ptr::copy_nonoverlapping(
            src.add(CR3_OFFSET),
            (TRAMPOLINE_PHYS + hhdm + TRAMPOLINE_CR3_OFF) as *mut u8,
            32,
        );
    }
}

fn wake_ap(record: CpuRecord) {
    let hhdm = crate::boot_info().hhdm_offset;
    let idx = record.cpu_index as usize;
    let stack_top = unsafe { AP_STACKS[idx].as_mut_ptr().add(16384) as u64 };

    super::trampoline::patch(
        cpu::read_cr3(),
        record.cpu_index as u64,
        record.lapic_id as u64,
        stack_top,
    );

    unsafe {
        let src = super::trampoline::image() as *const u8;
        core::ptr::copy_nonoverlapping(
            src.add(CR3_OFFSET),
            (TRAMPOLINE_PHYS + hhdm + TRAMPOLINE_CR3_OFF) as *mut u8,
            32,
        );
    }

    lapic::send_init_ipi(record.lapic_id);
    spin_delay_ms(10);
    lapic::send_sipi(record.lapic_id, (TRAMPOLINE_PHYS >> 12) as u8);
    spin_delay_ms(200);
    lapic::send_sipi(record.lapic_id, (TRAMPOLINE_PHYS >> 12) as u8);
}

fn wait_for_ap(cpu_index: u32) {
    for _ in 0..10_000_000 {
        if cpu_online(cpu_index) {
            return;
        }
        cpu::pause();
    }
    panic!("AP cpu_index={cpu_index} failed to come online");
}

fn cpu_online(cpu_index: u32) -> bool {
    CPU_TABLE.lock()[cpu_index as usize]
        .map(|r| r.online)
        .unwrap_or(false)
}

fn spin_delay_ms(ms: u32) {
    for _ in 0..(ms * 100_000) {
        cpu::pause();
    }
}

#[no_mangle]
pub extern "C" fn theory_ap_entry(cpu_index: u32, lapic_id: u32) -> ! {
    while !SMP_READY.load(Ordering::Acquire) {
        cpu::pause();
    }

    CURRENT_CPU.store(cpu_index, Ordering::Release);
    X86_64::init_per_cpu(cpu_index, lapic_id);

    if let Some(record) = CPU_TABLE.lock()[cpu_index as usize].as_mut() {
        record.online = true;
    }
    ONLINE_COUNT.fetch_add(1, Ordering::AcqRel);

    let _ = writeln!(Console, "[smp] AP cpu_index={cpu_index} lapic_id={lapic_id} online");
    crate::sched::start_cpu(cpu_index)
}
