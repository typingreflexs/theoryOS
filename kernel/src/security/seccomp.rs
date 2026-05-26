//! Seccomp-style syscall filtering.

use crate::syscall::nr::*;

pub const SECCOMP_MODE_DISABLED: u32 = 0;
pub const SECCOMP_MODE_STRICT: u32 = 1;
pub const SECCOMP_MODE_FILTER: u32 = 2;

pub const PR_SET_SECCOMP: u64 = 22;

const ALLOWLIST_SIZE: usize = 64;

#[derive(Clone, Copy, Debug)]
pub struct SeccompFilter {
    pub mode: u32,
    pub allowed: [u64; ALLOWLIST_SIZE],
    pub allowed_count: usize,
}

impl SeccompFilter {
    pub const fn disabled() -> Self {
        Self {
            mode: SECCOMP_MODE_DISABLED,
            allowed: [0; ALLOWLIST_SIZE],
            allowed_count: 0,
        }
    }

    pub fn set_strict(&mut self) {
        self.mode = SECCOMP_MODE_STRICT;
        self.allowed_count = 0;
    }

    pub fn set_filter(&mut self, syscalls: &[u64]) -> Result<(), ()> {
        if syscalls.len() > ALLOWLIST_SIZE {
            return Err(());
        }
        self.mode = SECCOMP_MODE_FILTER;
        self.allowed[..syscalls.len()].copy_from_slice(syscalls);
        self.allowed_count = syscalls.len();
        Ok(())
    }

    pub fn allows(&self, nr: u64) -> bool {
        match self.mode {
            SECCOMP_MODE_DISABLED => true,
            SECCOMP_MODE_STRICT => matches!(nr, SYS_READ | SYS_WRITE | SYS_EXIT | SYS_RT_SIGRETURN),
            SECCOMP_MODE_FILTER => self
                .allowed
                .iter()
                .take(self.allowed_count)
                .any(|&n| n == nr),
            _ => false,
        }
    }
}

pub fn check_current(nr: u64) -> bool {
    crate::proc::current_process_mut(|p| p.seccomp.allows(nr))
        .unwrap_or(true)
}

pub fn apply_strict() -> Result<(), ()> {
    crate::proc::current_process_mut(|p| {
        p.seccomp.set_strict();
        Ok(())
    })
    .unwrap_or(Err(()))
}

pub fn apply_filter(syscalls: &[u64]) -> Result<(), ()> {
    crate::proc::current_process_mut(|p| p.seccomp.set_filter(syscalls))
        .unwrap_or(Err(()))
}
