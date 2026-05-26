//! Security-related syscalls: prctl (seccomp).

use crate::proc;
use crate::security::seccomp;
use crate::security::CapSet;
use crate::syscall::errno::{err, ok, Errno, SysResult};

const PR_SET_SECCOMP: u64 = 22;

pub fn sys_prctl(option: u64, arg2: u64, _: u64, _: u64, _: u64, _: u64) -> SysResult {
    match option {
        PR_SET_SECCOMP => {
            if !proc::capable(CapSet::SYS_ADMIN) {
                return err(Errno::EACCES);
            }
            match arg2 {
                1 => match seccomp::apply_strict() {
                    Ok(()) => ok(0),
                    Err(()) => err(Errno::EINVAL),
                },
                2 => {
                    // Filter mode: allow read/write/exit only by default
                    let allowed = [0u64, 1, 60, 15]; // read, write, exit, sigreturn
                    match seccomp::apply_filter(&allowed) {
                        Ok(()) => ok(0),
                        Err(()) => err(Errno::EINVAL),
                    }
                }
                _ => err(Errno::EINVAL),
            }
        }
        _ => err(Errno::EINVAL),
    }
}
