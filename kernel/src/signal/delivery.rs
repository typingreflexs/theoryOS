//! Async signal delivery on syscall/interrupt return to userspace.

use crate::arch::x86_64::syscall::SyscallFrame;
use crate::console::Console;
use crate::proc::{self, id::Pid};
use crate::signal::types::{
    MContext, SigAction, SignalFrame, UContext, UserSigInfo, SIG_DFL, SIG_IGN, SA_SIGINFO,
};

pub use crate::signal::types::{SigSet, SignalState, SA_RESTART, NSIG};

pub fn init() {
    Console::println("[signal] async delivery framework ready");
}

pub fn kill(pid: Pid, sig: u32) -> Result<(), i64> {
    if sig >= NSIG as u32 {
        return Err(-(crate::syscall::errno::Errno::EINVAL.0));
    }
    if !pid.is_valid() {
        return Err(-(crate::syscall::errno::Errno::ESRCH.0));
    }
    proc::table::with_process_mut(pid, |p| {
        p.signals.deliver(sig, proc::current_thread(|t| t.pid).unwrap_or(Pid::KERNEL));
    })
    .ok_or(-(crate::syscall::errno::Errno::ESRCH.0))?;
    Ok(())
}

pub fn set_action(sig: u32, action: SigAction) -> Result<SigAction, i64> {
    if sig == 0 || sig >= NSIG as u32 {
        return Err(-22);
    }
    proc::current_process_mut(|p| {
        let old = p.signals.actions[sig as usize];
        p.signals.actions[sig as usize] = action;
        old
    })
    .ok_or(-1)
}

pub fn set_blocked(mask: SigSet) -> Result<SigSet, i64> {
    proc::current_process_mut(|p| {
        let old = p.signals.blocked;
        p.signals.blocked = mask;
        old
    })
    .ok_or(-1)
}

pub fn deliver_pending(frame: &mut SyscallFrame) -> bool {
    let tid = proc::current_tid();
    let pid = match proc::table::with_thread(tid, |t| t.pid) {
        Some(p) => p,
        None => return false,
    };
    if pid == Pid::KERNEL {
        return false;
    }

    let result = proc::table::with_process_mut(pid, |p| {
        let sig = p.signals.take_pending()?;
        let action = p.signals.actions[sig as usize];
        if action.handler == SIG_DFL {
            if sig == 9 || sig == 11 || sig == 6 {
                p.state = crate::proc::ProcessState::Dead;
            }
            return None;
        }
        if action.handler == SIG_IGN {
            return None;
        }
        p.signals.handling = true;
        Some((sig, action))
    });

    let Some(Some((sig, action))) = result else {
        return false;
    };

    let user_rsp = frame.user_rsp;
    if user_rsp < core::mem::size_of::<SignalFrame>() as u64 + 128 {
        return false;
    }

    let mut mctx = MContext::empty();
    mctx.gregs[16] = frame.user_rip;
    mctx.gregs[19] = frame.user_rsp;
    mctx.gregs[17] = frame.user_rflags;

    let sigframe = SignalFrame {
        restorer: action.restorer,
        sig,
        pad: 0,
        info: UserSigInfo {
            signo: sig as i32,
            errno: 0,
            code: 1,
            pid: pid.as_u32() as i32,
            uid: proc::table::with_process(pid, |p| p.cred.uid).unwrap_or(0),
        },
        ucontext: UContext {
            uc_flags: 0,
            uc_link: 0,
            uc_stack_ss_sp: 0,
            uc_stack_ss_flags: 0,
            uc_stack_ss_size: 0,
            uc_mcontext: mctx,
            uc_sigmask: proc::table::with_process(pid, |p| p.signals.blocked.0).unwrap_or(0),
        },
        saved_rip: frame.user_rip,
        saved_rsp: user_rsp,
        saved_rflags: frame.user_rflags,
    };

    let frame_size = core::mem::size_of::<SignalFrame>() as u64;
    let new_rsp = user_rsp - frame_size;
    unsafe {
        core::ptr::write(new_rsp as *mut SignalFrame, sigframe);
    }

    frame.user_rsp = new_rsp;
    frame.user_rip = action.handler;
    if action.flags & SA_SIGINFO != 0 {
        let info_off = core::mem::offset_of!(SignalFrame, info) as u64;
        let uc_off = core::mem::offset_of!(SignalFrame, ucontext) as u64;
        frame.rdi = sig as u64;
        frame.rsi = new_rsp + info_off;
        frame.rdx = new_rsp + uc_off;
    } else {
        frame.rdi = sig as u64;
        frame.rsi = 0;
        frame.rdx = 0;
    }
    frame.rax = 0;
    true
}

pub fn sigreturn(frame: &mut SyscallFrame) -> Result<isize, i64> {
    let user_rsp = frame.user_rsp;
    if user_rsp == 0 {
        return Err(-14);
    }
    let saved = unsafe { *(user_rsp as *const SignalFrame) };
    frame.user_rip = saved.saved_rip;
    frame.user_rsp = saved.saved_rsp;
    frame.user_rflags = saved.saved_rflags;
    proc::current_process_mut(|p| {
        p.signals.handling = false;
        p.signals.blocked = SigSet(saved.ucontext.uc_sigmask);
    });
    Ok(0)
}

pub fn has_pending() -> bool {
    proc::current_process_mut(|p| !p.signals.pending.is_empty()).unwrap_or(false)
}

pub fn fork_state(parent: &SignalState) -> SignalState {
    SignalState {
        pending: SigSet(0),
        blocked: parent.blocked,
        actions: parent.actions,
        handling: false,
    }
}
