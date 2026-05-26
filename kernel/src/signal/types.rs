//! Signal types and per-process signal state.

use crate::proc::id::Pid;

pub const NSIG: usize = 64;
pub const SIG_DFL: u64 = 0;
pub const SIG_IGN: u64 = 1;
pub const SA_RESTART: u64 = 0x1000_0000;
pub const SA_SIGINFO: u64 = 0x0000_0004;

#[derive(Clone, Copy, Debug, Default)]
pub struct SigSet(pub u64);

impl SigSet {
    pub fn empty() -> Self {
        Self(0)
    }

    pub fn add(&mut self, sig: u32) {
        if sig > 0 && sig < 64 {
            self.0 |= 1u64 << sig;
        }
    }

    pub fn del(&mut self, sig: u32) {
        if sig > 0 && sig < 64 {
            self.0 &= !(1u64 << sig);
        }
    }

    pub fn contains(&self, sig: u32) -> bool {
        sig > 0 && sig < 64 && (self.0 & (1u64 << sig)) != 0
    }

    pub fn is_empty(&self) -> bool {
        self.0 == 0
    }
}

#[derive(Clone, Copy, Debug)]
pub struct SigAction {
    pub handler: u64,
    pub flags: u64,
    pub restorer: u64,
    pub mask: SigSet,
}

impl SigAction {
    pub const fn default() -> Self {
        Self {
            handler: SIG_DFL,
            flags: 0,
            restorer: 0,
            mask: SigSet(0),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct SignalState {
    pub pending: SigSet,
    pub blocked: SigSet,
    pub actions: [SigAction; NSIG],
    pub handling: bool,
}

impl SignalState {
    pub const fn new() -> Self {
        Self {
            pending: SigSet(0),
            blocked: SigSet(0),
            actions: [SigAction::default(); NSIG],
            handling: false,
        }
    }

    pub fn deliver(&mut self, sig: u32, from: Pid) {
        let _ = from;
        if sig == 0 || sig >= NSIG as u32 {
            return;
        }
        if self.actions[sig as usize].handler == SIG_IGN {
            return;
        }
        self.pending.add(sig);
    }

    pub fn take_pending(&mut self) -> Option<u32> {
        for sig in 1..NSIG as u32 {
            if self.pending.contains(sig) && !self.blocked.contains(sig) {
                self.pending.del(sig);
                return Some(sig);
            }
        }
        None
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct UserSigInfo {
    pub signo: i32,
    pub errno: i32,
    pub code: i32,
    pub pid: i32,
    pub uid: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct MContext {
    pub gregs: [u64; 32],
}

impl MContext {
    pub const fn empty() -> Self {
        Self { gregs: [0; 32] }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct UContext {
    pub uc_flags: u64,
    pub uc_link: u64,
    pub uc_stack_ss_sp: u64,
    pub uc_stack_ss_flags: i32,
    pub uc_stack_ss_size: u64,
    pub uc_mcontext: MContext,
    pub uc_sigmask: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct SignalFrame {
    pub restorer: u64,
    pub sig: u32,
    pub pad: u32,
    pub info: UserSigInfo,
    pub ucontext: UContext,
    pub saved_rip: u64,
    pub saved_rsp: u64,
    pub saved_rflags: u64,
}
