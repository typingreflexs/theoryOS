pub mod errno;
pub mod handlers;
pub mod nr;
pub mod table;
pub mod uaccess;

use crate::arch::x86_64::syscall::SyscallFrame;
use crate::console::Console;
use crate::syscall::errno::SysResult;
use crate::syscall::nr::SYS_RT_SIGRETURN;
use crate::syscall::table::dispatch;

pub use errno::{err, ok, Errno};

pub fn init() {
    crate::arch::x86_64::syscall::init_msrs();
    crate::signal::init();
    Console::println("[syscall] SYSCALL/SYSRET fast path online");
}

/// Called from assembly syscall entry with saved register frame.
#[no_mangle]
pub extern "C" fn theory_syscall_dispatch(frame: *mut SyscallFrame) -> i64 {
    let frame = unsafe { &mut *frame };

    proc::preempt_disable();

    let nr = frame.rax;

    if nr != SYS_RT_SIGRETURN && !crate::security::seccomp_allows(nr) {
        proc::preempt_enable();
        return Errno::EACCES.as_neg() as i64;
    }

    let result = if nr == SYS_RT_SIGRETURN {
        crate::signal::sigreturn(frame).map(|v| v as i64).unwrap_or_else(|e| e)
    } else {
        match dispatch(
            nr, frame.rdi, frame.rsi, frame.rdx, frame.r10, frame.r8, frame.r9,
        ) {
            Ok(v) => v as i64,
            Err(e) => e.as_neg() as i64,
        }
    };

    frame.rax = result as u64;

    // Async signal delivery before returning to userspace
    if crate::signal::has_pending() {
        let _ = crate::signal::deliver_pending(frame);
    }

    crate::security::verify_cpu_stack(crate::arch::x86_64::smp::current_cpu_index() as usize);

    proc::preempt_enable();
    result
}

/// Return current CPU's kernel stack top for syscall entry.
#[no_mangle]
pub extern "C" fn theory_syscall_kernel_stack() -> u64 {
    let cpu = crate::arch::x86_64::smp::current_cpu_index() as usize;
    crate::arch::x86_64::syscall::percpu_kstack(cpu)
}

use crate::proc;
