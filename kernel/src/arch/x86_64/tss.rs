use core::mem::size_of;

use crate::arch::x86_64::gdt::sel;

pub const MAX_CPUS: usize = 256;
pub const TSS_LIMIT: u16 = size_of::<TaskStateSegment>() as u16 - 1;

#[repr(C, packed)]
#[derive(Clone, Copy, Default)]
pub struct TaskStateSegment {
    reserved0: u32,
    pub rsp0: u64,
    pub rsp1: u64,
    pub rsp2: u64,
    reserved1: u64,
    pub ist1: u64,
    pub ist2: u64,
    pub ist3: u64,
    pub ist4: u64,
    pub ist5: u64,
    pub ist6: u64,
    pub ist7: u64,
    reserved2: u64,
    reserved3: u16,
    pub iomap_base: u16,
}

static mut BSP_TSS: TaskStateSegment = TaskStateSegment {
    reserved0: 0,
    rsp0: 0,
    rsp1: 0,
    rsp2: 0,
    reserved1: 0,
    ist1: 0,
    ist2: 0,
    ist3: 0,
    ist4: 0,
    ist5: 0,
    ist6: 0,
    ist7: 0,
    reserved2: 0,
    reserved3: 0,
    iomap_base: 0,
};

static mut AP_TSS: [TaskStateSegment; MAX_CPUS] = [TaskStateSegment {
    reserved0: 0,
    rsp0: 0,
    rsp1: 0,
    rsp2: 0,
    reserved1: 0,
    ist1: 0,
    ist2: 0,
    ist3: 0,
    ist4: 0,
    ist5: 0,
    ist6: 0,
    ist7: 0,
    reserved2: 0,
    reserved3: 0,
    iomap_base: 0,
}; MAX_CPUS];

static mut BSP_KERNEL_STACK: [u8; 16384] = [0; 16384];
static mut AP_KERNEL_STACKS: [[u8; 16384]; MAX_CPUS] = [[0; 16384]; MAX_CPUS];
static mut BSP_DF_STACK: [u8; 8192] = [0; 8192];
static mut AP_DF_STACKS: [[u8; 8192]; MAX_CPUS] = [[0; 8192]; MAX_CPUS];

pub fn init_bsp() {
    unsafe {
        BSP_TSS.iomap_base = TSS_LIMIT + 1;
        BSP_TSS.rsp0 = BSP_KERNEL_STACK.as_mut_ptr().add(16384) as u64;
        BSP_TSS.ist1 = BSP_DF_STACK.as_mut_ptr().add(8192) as u64;
    }
}

pub fn init_ap(cpu_index: u32) {
    let idx = cpu_index as usize;
    assert!(idx < MAX_CPUS);
    unsafe {
        AP_TSS[idx].iomap_base = TSS_LIMIT + 1;
        AP_TSS[idx].rsp0 = AP_KERNEL_STACKS[idx].as_mut_ptr().add(16384) as u64;
        AP_TSS[idx].ist1 = AP_DF_STACKS[idx].as_mut_ptr().add(8192) as u64;
    }
}

pub fn bsp_tss() -> *const TaskStateSegment {
    unsafe { core::ptr::addr_of!(BSP_TSS) }
}

pub fn ap_tss(cpu_index: u32) -> *const TaskStateSegment {
    let idx = cpu_index as usize;
    unsafe { core::ptr::addr_of!(AP_TSS[idx]) }
}

pub fn set_rsp0(cpu_index: u32, rsp0: u64) {
    if cpu_index == 0 {
        unsafe {
            BSP_TSS.rsp0 = rsp0;
        }
    } else {
        let idx = cpu_index as usize;
        if idx < MAX_CPUS {
            unsafe {
                AP_TSS[idx].rsp0 = rsp0;
            }
        }
    }
}

pub fn stamp_stack_canary(canary: u64) {
    unsafe {
        (BSP_KERNEL_STACK.as_mut_ptr() as *mut u64).write(canary);
        for stack in AP_KERNEL_STACKS.iter_mut() {
            (stack.as_mut_ptr() as *mut u64).write(canary);
        }
    }
}

pub fn verify_stack_canary(expected: u64, cpu: u32) {
    unsafe {
        let val = if cpu == 0 {
            (BSP_KERNEL_STACK.as_ptr() as *const u64).read()
        } else {
            let idx = cpu as usize;
            if idx >= MAX_CPUS {
                return;
            }
            (AP_KERNEL_STACKS[idx].as_ptr() as *const u64).read()
        };
        if val != 0 && val != expected {
            crate::console::Console::print("\n*** STACK SMASH DETECTED ***\n");
            crate::arch::halt_forever();
        }
    }
}
