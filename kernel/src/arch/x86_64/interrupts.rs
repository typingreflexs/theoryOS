use core::fmt::Write;

use spin::Mutex;

use crate::arch::x86_64::cpu;
use crate::arch::x86_64::lapic;
use crate::console::Console;
use crate::mm::fault::{self, PageFaultOutcome};

/// Saved register frame pushed by the interrupt stub.
#[repr(C)]
pub struct InterruptFrame {
    pub r15: u64,
    pub r14: u64,
    pub r13: u64,
    pub r12: u64,
    pub r11: u64,
    pub r10: u64,
    pub r9: u64,
    pub r8: u64,
    pub rdi: u64,
    pub rsi: u64,
    pub rbp: u64,
    pub rbx: u64,
    pub rdx: u64,
    pub rcx: u64,
    pub rax: u64,
    pub vector: u64,
    pub error_code: u64,
    pub rip: u64,
    pub cs: u64,
    pub rflags: u64,
    pub rsp: u64,
    pub ss: u64,
}

static IRQ_HANDLERS: Mutex<[Option<fn(&InterruptFrame)>; 224]> =
    Mutex::new([None; 224]);

#[no_mangle]
pub extern "C" fn theory_interrupt_dispatch(frame_ptr: *const InterruptFrame) {
    let frame = unsafe { &*frame_ptr };
    let vector = frame.vector as u8;

    if vector < 32 {
        if handle_exception(frame) {
            return;
        }
        cpu::halt_forever();
    } else {
        handle_irq(frame);
    }
}

pub(crate) fn handle_exception(frame: &InterruptFrame) -> bool {
    let vector = frame.vector as u8;

    if vector == 14 {
        return handle_page_fault(frame);
    }

    Console::print("\n=== CPU EXCEPTION ===\n");
    let _ = writeln!(Console, "Vector: {vector} ({})", exception_name(vector));
    let _ = writeln!(Console, "Error code: {:#x}", frame.error_code);
    let _ = writeln!(Console, "RIP: {:#018x}", frame.rip);
    let _ = writeln!(Console, "CR2: {:#018x}", cpu::read_cr2());
    let _ = writeln!(Console, "CR3: {:#018x}", cpu::read_cr3());

    match vector {
        8 | 18 => {
            cpu::halt_forever();
        }
        _ => {
            cpu::halt_forever();
        }
    }
}

fn handle_page_fault(frame: &InterruptFrame) -> bool {
    let outcome = fault::handle_from_interrupt(frame.error_code);
    match outcome {
        PageFaultOutcome::Handled
        | PageFaultOutcome::CowBroken
        | PageFaultOutcome::DemandPaged => true,
        PageFaultOutcome::BadAddress | PageFaultOutcome::ProtectionViolation => {
            let info = fault::decode_error(frame.error_code);
            let _ = writeln!(
                Console,
                "Page fault failed: addr={:#x} write={} user={}",
                info.address, info.write, info.user
            );
            false
        }
    }
}

pub fn handle_irq(frame: &InterruptFrame) {
    let vector = frame.vector as u8;
    if vector >= 32 {
        let irq = vector - 32;
        if irq < 224 {
            let handler = {
                let handlers = IRQ_HANDLERS.lock();
                handlers[irq as usize]
            };
            if let Some(handler) = handler {
                handler(frame);
            }
        }
    }

    lapic::send_eoi();
}

pub fn register_irq_handler(irq: u8, handler: fn(&InterruptFrame)) {
    assert!(irq < 224);
    IRQ_HANDLERS.lock()[irq as usize] = Some(handler);
}

fn exception_name(vector: u8) -> &'static str {
    match vector {
        0 => "Divide Error",
        1 => "Debug",
        2 => "NMI",
        3 => "Breakpoint",
        4 => "Overflow",
        5 => "Bound Range",
        6 => "Invalid Opcode",
        7 => "Device Not Available",
        8 => "Double Fault",
        9 => "Coprocessor Segment Overrun",
        10 => "Invalid TSS",
        11 => "Segment Not Present",
        12 => "Stack-Segment Fault",
        13 => "General Protection Fault",
        14 => "Page Fault",
        15 => "Reserved",
        16 => "x87 FP Exception",
        17 => "Alignment Check",
        18 => "Machine Check",
        19 => "SIMD FP Exception",
        20 => "Virtualization Exception",
        21 => "Control Protection Exception",
        _ => "Reserved/Exception",
    }
}
